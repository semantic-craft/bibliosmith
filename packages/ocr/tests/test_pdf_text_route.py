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
    markdown_page_numbers,
    md5_file,
    process_text_route,
    sidecar_page_numbers,
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
        status = process_text_route(
            attachment=self.attachment,
            config=config,
            state=state,
            source_md5=md5_file(self.pdf),
            page_count=4,
            pages=[1, 2, 3, 4],
            route_reason="test",
            no_upload=True,
        )
        staging = config.output_root / ".state" / "staging" / self.attachment.key
        markdown = next(staging.glob("*.md"))
        return status, markdown, markdown.with_suffix(".jsonl")

    def test_the_markdown_carries_no_page_headings(self) -> None:
        _, markdown_path, _ = self.run_route()

        text = markdown_path.read_text(encoding="utf-8")

        self.assertNotIn("## Page ", text)
        self.assertNotIn("[no extractable text]", text)

    def test_the_book_title_is_still_the_one_heading_the_route_adds(self) -> None:
        _, markdown_path, _ = self.run_route()

        text = markdown_path.read_text(encoding="utf-8")

        self.assertTrue(text.startswith("---"))
        self.assertIn("\n# Example.pdf\n", text)
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

    def test_the_page_list_survives_for_the_upload_status(self) -> None:
        """The sidecar answers whichever engine won.

        Structured Markdown carries no page markers at all — page breaks are
        not part of what pdf-inspector reconstructs — and the PyMuPDF fallback
        carries them only as comments. Neither is a heading any more, so the
        page list has to come from the sidecar to survive.
        """
        _, markdown_path, sidecar_path = self.run_route()

        self.assertEqual([1, 2, 3, 4], sidecar_page_numbers(sidecar_path))
        self.assertNotIn("## Page ", markdown_path.read_text(encoding="utf-8"))
        self.assertIn(markdown_page_numbers(markdown_path), ([], [1, 2, 3, 4]))


class PageNumberReaderTests(unittest.TestCase):
    def setUp(self) -> None:
        self.root = Path(self.enterContext(tempfile.TemporaryDirectory()))

    def write(self, text: str) -> Path:
        path = self.root / "book.md"
        path.write_text(text, encoding="utf-8")
        return path

    def test_a_file_converted_before_this_change_still_reads(self) -> None:
        path = self.write("# Book\n\n## Page 1\n\nBody.\n\n## Page 2\n\nMore.\n")

        self.assertEqual([1, 2], markdown_page_numbers(path))

    def test_the_anchor_the_ocr_assembler_writes_reads_too(self) -> None:
        path = self.write("# Book\n\n<!-- page: 1 -->\n\nBody.\n\n<!-- page: 2 -->\n\nMore.\n")

        self.assertEqual([1, 2], markdown_page_numbers(path))

    def test_an_anchor_quoted_inside_a_line_is_not_a_page(self) -> None:
        path = self.write("# Book\n\nThe anchor <!-- page: 9 --> is written like this.\n")

        self.assertEqual([], markdown_page_numbers(path))

    def test_a_sidecar_that_is_not_json_is_not_an_error(self) -> None:
        path = self.root / "book.jsonl"
        path.write_text('{"raw": {}}\n{"raw": {}}\n', encoding="utf-8")

        self.assertEqual([], sidecar_page_numbers(path))


if __name__ == "__main__":
    unittest.main()
