"""The paddle-ocr route's Markdown when the model is not a layout model.

`process_ocr_route()` assembles two ways. A PaddleOCR-VL model returns its own
Markdown and the layout branch writes that through; an OCR-only model returns
bare per-page text, and that branch used to wrap it in `## Page N` headings —
the same fake-heading defect #135 fixed for the pdf-text route. The split stage
cuts a chapter at every heading of the shallowest level it finds, so an N-page
book arrived in the finished EPUB as N chapters named "Page 1" through
"Page N", with no real chapter name in the table of contents.

The branch is only selected by exporting `BAIDU_PADDLEOCR_MODEL` to a model
outside `LAYOUT_MODELS` — the Launcher never sets that variable — which is why
nothing covered it until now. Every remote call is stubbed, so these tests
never touch the Baidu API.
"""

from __future__ import annotations

import dataclasses
import json
from pathlib import Path
import sys
import tempfile
import time
import unittest
from unittest import mock

import fitz


SCRIPT_DIR = Path(__file__).resolve().parents[1] / "scripts"
sys.path.insert(0, str(SCRIPT_DIR))

from zotero_llm_worker import (  # noqa: E402
    Attachment,
    StateDB,
    get_config,
    md5_file,
    process_ocr_route,
)


PAGE_TEXTS = {
    1: ["Chapter One", "The first page of the body."],
    2: ["It carries on across the page break."],
    3: [],  # a page the OCR returned nothing for
    4: ["And ends here."],
}


class FakeBaiduOCRClient:
    """The shape an OCR-only Baidu model answers with: per-page `rec_texts`.

    The chunk file name carries its own page range, so this stands in for
    however `process_ocr_route()` decides to split the book.
    """

    def __init__(self, config: object) -> None:
        self.config = config

    def submit_job(self, pdf_path: Path, batch_id: str) -> str:
        return pdf_path.stem  # "pages-0001-0004"

    def poll_json_url(self, job_id: str, deadline: float, on_progress=None) -> str:  # type: ignore[no-untyped-def]
        return job_id

    def download_jsonl(self, url: str) -> str:
        _, start, end = url.split("-")
        results = [
            {"prunedResult": {"rec_texts": PAGE_TEXTS[page]}}
            for page in range(int(start), int(end) + 1)
        ]
        return json.dumps({"result": {"ocrResults": results}}, ensure_ascii=False) + "\n"


def write_pdf(path: Path, page_count: int) -> Path:
    doc = fitz.open()
    for number in range(page_count):
        doc.new_page().insert_text((72, 300), f"scanned page {number + 1}", fontsize=16)
    doc.save(path)
    doc.close()
    return path


class OcrRouteMarkdownTests(unittest.TestCase):
    def setUp(self) -> None:
        self.root = Path(self.enterContext(tempfile.TemporaryDirectory()))
        self.pdf = write_pdf(self.root / "Example.pdf", len(PAGE_TEXTS))
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

    def run_route(self, *, baidu_model: str = "PP-OCRv5"):  # type: ignore[no-untyped-def]
        config = dataclasses.replace(
            get_config(),
            output_root=self.root / "out",
            baidu_token="test-token",
            baidu_model=baidu_model,
            max_ocr_pages_per_job=len(PAGE_TEXTS),
            baidu_max_upload_mb=64,
        )
        state = StateDB(self.root / "state.sqlite3")
        with mock.patch("zotero_llm_worker.BaiduOCRClient", FakeBaiduOCRClient):
            status, pages_used = process_ocr_route(
                attachment=self.attachment,
                config=config,
                state=state,
                source_md5=md5_file(self.pdf),
                page_count=len(PAGE_TEXTS),
                pages=sorted(PAGE_TEXTS),
                route_reason="test",
                no_upload=True,
                deadline=time.time() + 600,
                ocr_pages_remaining=len(PAGE_TEXTS),
            )
        staging = config.output_root / ".state" / "staging" / self.attachment.key
        markdown = next(staging.glob("*.md"))
        return status, markdown, markdown.with_suffix(".jsonl"), pages_used

    def test_the_markdown_carries_no_page_headings(self) -> None:
        _, markdown_path, _, _ = self.run_route()

        text = markdown_path.read_text(encoding="utf-8")

        self.assertNotIn("## Page ", text)
        self.assertNotIn("[no extractable text]", text)

    def test_the_book_title_is_the_only_heading_the_route_adds(self) -> None:
        """One shallowest-level heading is one chapter out of the split stage."""
        _, markdown_path, _, _ = self.run_route()

        text = markdown_path.read_text(encoding="utf-8")
        headings = [line for line in text.splitlines() if line.startswith("#")]

        self.assertEqual(["# Example.pdf"], headings)

    def test_the_body_is_the_recognised_text_and_nothing_else(self) -> None:
        """Reading order intact, and page 3 — which OCR read nothing on —
        contributes nothing. The `[no extractable text]` placeholder only ever
        meant anything under a `## Page N` heading saying which page it was."""
        _, markdown_path, _, _ = self.run_route()

        text = markdown_path.read_text(encoding="utf-8")
        body = text.split("# Example.pdf\n", 1)[1]

        self.assertTrue(text.startswith("---"))
        self.assertEqual(
            "Chapter One\nThe first page of the body.\n\n"
            "It carries on across the page break.\n\n"
            "And ends here.",
            body.strip(),
        )

    def test_the_sidecar_still_records_every_page(self) -> None:
        """The per-page raw results are what the route reports pages through."""
        _, _, sidecar_path, pages_used = self.run_route()

        lines = [
            json.loads(line)
            for line in sidecar_path.read_text(encoding="utf-8").splitlines()
            if line.strip()
        ]

        self.assertEqual(sorted(PAGE_TEXTS), [entry["page"] for entry in lines])
        self.assertEqual(len(PAGE_TEXTS), pages_used)

    def test_a_layout_model_still_takes_the_other_branch(self) -> None:
        """The fake answers `ocrResults`, which the layout branch cannot read.

        Without this the tests above would pass just as well if the route had
        stopped discriminating between the two model families.
        """
        _, markdown_path, _, _ = self.run_route(baidu_model="PaddleOCR-VL-1.6")

        text = markdown_path.read_text(encoding="utf-8")

        self.assertNotIn("Chapter One", text)
        self.assertNotIn("# Example.pdf", text)


if __name__ == "__main__":
    unittest.main()
