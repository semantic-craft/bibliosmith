"""The pdf-text route's Markdown, as the rest of the pipeline receives it.

`## Page N` was never a heading any book had. The split stage cuts a chapter at
every heading of the shallowest level it finds, so a 629-page PDF arrived in
the finished EPUB as 629 chapters named "Page 1" through "Page 629" — a table
of contents with no chapter name in it anywhere.
"""

from __future__ import annotations

import dataclasses
import json
from pathlib import Path
import sys
import tempfile
import unittest

import fitz


SCRIPT_DIR = Path(__file__).resolve().parents[1] / "scripts"
sys.path.insert(0, str(SCRIPT_DIR))

from zotero_llm_worker import (  # noqa: E402
    Attachment,
    StateDB,
    get_config,
    md5_file,
    process_text_route,
    reconcile_staged_conversion,
)


def write_pdf(path: Path, page_texts: list[str]) -> Path:
    doc = fitz.open()
    for text in page_texts:
        page = doc.new_page()
        if text:
            page.insert_text((72, 300), text, fontsize=16)
    doc.save(path)
    doc.close()
    return path


class TextRouteMarkdownTests(unittest.TestCase):
    def setUp(self) -> None:
        self.root = Path(self.enterContext(tempfile.TemporaryDirectory()))
        self.pdf = write_pdf(
            self.root / "Example.pdf",
            [f"Page {n} of the body, with enough words to be extracted." for n in range(1, 5)],
        )
        self.attachment = Attachment(
            key="KXPSMW4C",
            title="Example.pdf",
            path=self.pdf,
            parent_key="XEMDH9G8",
            parent_item_type="book",
            parent_title="Example",
            parent_creators=[],
            parent_date="2026",
            content_type="application/pdf",
        )

    def run_route(self):  # type: ignore[no-untyped-def]
        config = dataclasses.replace(get_config(), output_root=self.root / "out")
        state = StateDB(self.root / "state.sqlite3")
        with fitz.open(self.pdf) as document:
            page_count = document.page_count
        status = process_text_route(
            attachment=self.attachment,
            config=config,
            state=state,
            source_md5=md5_file(self.pdf),
            page_count=page_count,
            pages=list(range(1, page_count + 1)),
            route_reason="test",
            no_upload=True,
        )
        self.state = state
        self.page_count = page_count
        staging = config.output_root / ".state" / "staging" / self.attachment.key
        markdown = next(staging.glob("*.md"))
        return status, markdown, markdown.with_suffix(".jsonl")

    def test_the_markdown_carries_no_page_headings(self) -> None:
        _, markdown_path, _ = self.run_route()

        text = markdown_path.read_text(encoding="utf-8")

        self.assertNotIn("## Page ", text)
        self.assertNotIn("[no extractable text]", text)

    def test_the_route_does_not_invent_a_heading_from_the_attachment_name(self) -> None:
        _, markdown_path, _ = self.run_route()

        text = markdown_path.read_text(encoding="utf-8")

        self.assertTrue(text.startswith("---"))
        self.assertNotIn("\n# Example.pdf\n", text)
        self.assertIn("of the body", text)

    def test_the_sidecar_keeps_its_per_page_character_counts(self) -> None:
        """`pages: [{page, chars}]` has readers outside this module."""
        _, _, sidecar_path = self.run_route()

        sidecar = json.loads(sidecar_path.read_text(encoding="utf-8"))

        self.assertEqual("pdf-text", sidecar["route"])
        self.assertEqual([1, 2, 3, 4], [entry["page"] for entry in sidecar["pages"]])
        self.assertTrue(all(entry["chars"] > 0 for entry in sidecar["pages"]))

    def test_the_sidecar_records_which_engine_produced_the_text(self) -> None:
        _, _, sidecar_path = self.run_route()

        sidecar = json.loads(sidecar_path.read_text(encoding="utf-8"))

        self.assertIn(sidecar["engine"], {"pdf-inspector", "pdf-inspector-repaired", "pymupdf"})
        self.assertIn("running_heads_removed", sidecar)

    def test_the_adapter_commits_page_coverage_to_the_reconciliation_seam(self) -> None:
        _, markdown_path, sidecar_path = self.run_route()

        outcome = reconcile_staged_conversion(
            attachment=self.attachment,
            state=self.state,
            page_count=self.page_count,
        )

        self.assertTrue(outcome.accepted)
        self.assertEqual(tuple(range(1, self.page_count + 1)), outcome.selected_pages)
        self.assertNotIn("## Page ", markdown_path.read_text(encoding="utf-8"))
        self.assertTrue(sidecar_path.is_file())

    def test_a_strict_page_subset_reconciles_as_local_partial(self) -> None:
        config = dataclasses.replace(get_config(), output_root=self.root / "partial-out")
        state = StateDB(self.root / "partial.sqlite3")

        status = process_text_route(
            attachment=self.attachment,
            config=config,
            state=state,
            source_md5=md5_file(self.pdf),
            page_count=4,
            pages=[1, 3],
            route_reason="partial test",
            no_upload=True,
        )
        outcome = reconcile_staged_conversion(
            attachment=self.attachment,
            state=state,
            page_count=4,
        )

        self.assertEqual("local_partial", status)
        self.assertTrue(outcome.accepted)
        self.assertEqual("local_partial", outcome.status)
        self.assertEqual((1, 3), outcome.selected_pages)

    def test_real_pdf_route_emits_a_stable_semantic_note_contract(self) -> None:
        self.pdf = write_pdf(
            self.root / "Notes.pdf",
            ["# Chapter\n\nClaim[^pdf-1].\n\n[^pdf-1]: PDF note."],
        )
        self.attachment = dataclasses.replace(
            self.attachment,
            title="Notes.pdf",
            path=self.pdf,
        )

        _, markdown_path, _ = self.run_route()
        evidence = json.loads(
            markdown_path.with_suffix(".publication.json").read_text(encoding="utf-8")
        )

        self.assertEqual("pdf", evidence["sourceFormat"])
        self.assertEqual("note_001", evidence["notes"][0]["id"])
        self.assertEqual("pdf-1", evidence["notes"][0]["sourceLabel"])
        self.assertEqual(
            ["noteref_note_001_001"], evidence["notes"][0]["referenceIds"]
        )
        self.assertEqual([], evidence["notes"][0]["anomalies"])

if __name__ == "__main__":
    unittest.main()
