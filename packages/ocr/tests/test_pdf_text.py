"""Tests for the hybrid pdf-inspector / PyMuPDF extractor.

The real-corpus cases at the bottom are the point of this file. Hand-built
fixtures are all born-digital and all parse, so they are blind to the four ways
pdf-inspector behaves on this library: Chinese downloads with a broken file
trailer, CID fonts it cannot decode, scan-backed layouts it gives up on, and
documents it parses while dropping much of the text. Those tests skip when
~/Zotero/storage is not present, which is the case on CI.
"""

from __future__ import annotations

import dataclasses
import importlib.util
from pathlib import Path
import sys
import tempfile
import unittest
from unittest import mock

import fitz


PACKAGE_ROOT = Path(__file__).resolve().parents[1]
REAL_CORPUS = Path.home() / "Zotero" / "storage"
PRIVATE_USE = ""


def load_pdf_text_module():  # type: ignore[no-untyped-def]
    module_name = "ocr_pdf_text_test"
    spec = importlib.util.spec_from_file_location(module_name, PACKAGE_ROOT / "pdf_text.py")
    if spec is None or spec.loader is None:
        raise RuntimeError("Cannot import pdf_text")
    module = importlib.util.module_from_spec(spec)
    sys.modules[module_name] = module
    spec.loader.exec_module(module)
    return module


pdf_text = load_pdf_text_module()


def write_pdf(path: Path, page_texts: list[str]) -> Path:
    doc = fitz.open()
    for text in page_texts:
        page = doc.new_page()
        if text:
            page.insert_text((72, 300), text, fontsize=16)
    doc.save(path)
    doc.close()
    return path


def nonspace(text: str) -> int:
    return sum(1 for ch in text if not ch.isspace())


class StubResult:
    """Stands in for pdf_inspector.PdfResult, which is a Rust extension type."""

    def __init__(self, markdown: str, page_count: int = 3) -> None:
        self.markdown = markdown
        self.page_count = page_count


def refuse_to_parse(*_args, **_kwargs):
    raise ValueError("PDF parsing error: couldn't parse input: invalid file trailer")


class HybridChainTests(unittest.TestCase):
    def setUp(self) -> None:
        self.tmp = Path(self.enterContext(tempfile.TemporaryDirectory()))

    def test_born_digital_pdf_is_extracted_by_pdf_inspector(self) -> None:
        pdf = write_pdf(self.tmp / "born.pdf", ["Alpha the quick brown fox", "Beta the lazy dog"])

        result = pdf_text.extract_markdown(pdf)

        self.assertEqual(result.engine, pdf_text.ENGINE_INSPECTOR)
        self.assertEqual(result.fallback_reason, "")
        self.assertIn("Alpha the quick brown fox", result.markdown)
        self.assertIn("Beta the lazy dog", result.markdown)
        self.assertEqual(result.page_count, 2)
        self.assertEqual(result.chars, nonspace(result.markdown))

    def test_page_selection_is_one_indexed(self) -> None:
        pdf = write_pdf(
            self.tmp / "three.pdf", ["PageOneMarker", "PageTwoMarker", "PageThreeMarker"]
        )

        result = pdf_text.extract_markdown(pdf, pages=[2])

        self.assertIn("PageTwoMarker", result.markdown)
        self.assertNotIn("PageOneMarker", result.markdown)
        self.assertNotIn("PageThreeMarker", result.markdown)

    def test_pdf_without_a_text_layer_falls_back_to_pymupdf(self) -> None:
        pdf = write_pdf(self.tmp / "blank.pdf", ["", ""])

        result = pdf_text.extract_markdown(pdf)

        self.assertEqual(result.engine, pdf_text.ENGINE_PYMUPDF)
        self.assertEqual(result.fallback_reason, "empty_markdown")

    def test_mojibake_markdown_falls_back_to_pymupdf(self) -> None:
        pdf = write_pdf(self.tmp / "clean.pdf", ["Readable text the extractor can use"])
        mojibake = "文" * 1000 + PRIVATE_USE * 60

        with mock.patch.object(
            pdf_text.pdf_inspector, "process_pdf", return_value=StubResult(mojibake)
        ):
            result = pdf_text.extract_markdown(pdf)

        self.assertEqual(result.engine, pdf_text.ENGINE_PYMUPDF)
        self.assertIn("dirty_text_layer", result.fallback_reason)
        self.assertIn("private_use_ratio", result.fallback_reason)
        self.assertIn("Readable text the extractor can use", result.markdown)

    def test_mojibake_thresholds_come_from_the_caller(self) -> None:
        pdf = write_pdf(self.tmp / "clean.pdf", ["Readable text the extractor can use"])
        mojibake = "文" * 1000 + PRIVATE_USE * 60

        with mock.patch.object(
            pdf_text.pdf_inspector, "process_pdf", return_value=StubResult(mojibake)
        ):
            result = pdf_text.extract_markdown(
                pdf, dirty_text=pdf_text.DirtyTextConfig(dirty_text_guard=False)
            )

        self.assertEqual(result.engine, pdf_text.ENGINE_INSPECTOR)
        self.assertEqual(result.markdown, mojibake)

    def test_markdown_holding_less_text_than_pymupdf_falls_back(self) -> None:
        pdf = write_pdf(
            self.tmp / "long.pdf",
            ["Paragraph one of the document body", "Paragraph two of the document body"],
        )

        with mock.patch.object(
            pdf_text.pdf_inspector, "process_pdf", return_value=StubResult("# Paragraph one")
        ):
            result = pdf_text.extract_markdown(pdf)

        self.assertEqual(result.engine, pdf_text.ENGINE_PYMUPDF)
        self.assertIn("less_text_than_pymupdf", result.fallback_reason)
        self.assertIn("Paragraph two of the document body", result.markdown)

    def test_parse_failure_is_repaired_and_retried_before_falling_back(self) -> None:
        pdf = write_pdf(self.tmp / "trailer.pdf", ["Recovered"])
        calls: list[str] = []

        def refuse(_path, _pages=None):
            calls.append("process_pdf")
            refuse_to_parse()

        def accept(_data, _pages=None):
            calls.append("process_pdf_bytes")
            return StubResult("# Recovered after the repair save", page_count=1)

        with mock.patch.object(pdf_text.pdf_inspector, "process_pdf", refuse), mock.patch.object(
            pdf_text.pdf_inspector, "process_pdf_bytes", accept
        ):
            result = pdf_text.extract_markdown(pdf)

        self.assertEqual(calls, ["process_pdf", "process_pdf_bytes"])
        self.assertEqual(result.engine, pdf_text.ENGINE_INSPECTOR_REPAIRED)
        self.assertIn("invalid file trailer", result.fallback_reason)
        self.assertEqual(result.markdown, "# Recovered after the repair save")

    def test_parse_failure_the_repair_cannot_fix_falls_back_to_pymupdf(self) -> None:
        pdf = write_pdf(self.tmp / "trailer.pdf", ["Still readable by PyMuPDF"])

        with mock.patch.object(
            pdf_text.pdf_inspector, "process_pdf", refuse_to_parse
        ), mock.patch.object(pdf_text.pdf_inspector, "process_pdf_bytes", refuse_to_parse):
            result = pdf_text.extract_markdown(pdf)

        self.assertEqual(result.engine, pdf_text.ENGINE_PYMUPDF)
        self.assertIn("repair_failed", result.fallback_reason)
        self.assertIn("Still readable by PyMuPDF", result.markdown)

    def test_pymupdf_fallback_marks_pages_with_comments_not_headings(self) -> None:
        pdf = write_pdf(self.tmp / "blank.pdf", ["", ""])

        result = pdf_text.extract_markdown(pdf)

        self.assertIn("<!-- page: 1 -->", result.markdown)
        self.assertIn("<!-- page: 2 -->", result.markdown)
        self.assertNotIn("## Page", result.markdown)

    def test_file_neither_engine_can_read_raises(self) -> None:
        not_a_pdf = self.tmp / "notes.pdf"
        not_a_pdf.write_text('{"itemType": "book"}', encoding="utf-8")

        with self.assertRaises(pdf_text.PdfTextError):
            pdf_text.extract_markdown(not_a_pdf)


class OcrRoutingBoundaryTests(unittest.TestCase):
    """The result must not hand a caller pdf-inspector's per-page OCR verdict.

    Measured on this corpus it flags hundreds of pages that PyMuPDF reads
    without trouble, so a caller acting on it would buy paid OCR for nothing.
    """

    def test_result_carries_only_markdown_and_provenance(self) -> None:
        names = {field.name for field in dataclasses.fields(pdf_text.PdfTextResult)}

        self.assertEqual(names, {"markdown", "engine", "fallback_reason", "chars", "page_count"})

    def test_the_module_offers_a_single_entry_point(self) -> None:
        functions = {
            name
            for name, value in vars(pdf_text).items()
            if not name.startswith("_")
            and callable(value)
            and not isinstance(value, type)
            and getattr(value, "__module__", "") == pdf_text.__name__
        }

        self.assertEqual(functions, {"extract_markdown"})


@unittest.skipUnless(REAL_CORPUS.is_dir(), "no ~/Zotero/storage on this machine")
class RealCorpusTests(unittest.TestCase):
    def extract(self, pattern: str):  # type: ignore[no-untyped-def]
        matches = sorted(REAL_CORPUS.glob(f"*/{pattern}"))
        if not matches:
            self.skipTest(f"no book matching {pattern} in the local library")
        return pdf_text.extract_markdown(matches[0])

    def test_born_digital_article_gains_structure(self) -> None:
        """The 88% of the corpus this whole module exists for."""
        result = self.extract("Purtova_Newell_2026_Against_Data_Fixation.pdf")

        self.assertEqual(result.engine, pdf_text.ENGINE_INSPECTOR)
        self.assertEqual(result.fallback_reason, "")
        self.assertGreater(len([line for line in result.markdown.splitlines() if line.startswith("#")]), 10)

    def test_cid_font_book_keeps_its_text(self) -> None:
        """pdf-inspector decodes this one's fonts to `!"#$%&'()` and gives up."""
        result = self.extract("*广松涉_2013_资本论的哲学.pdf")

        self.assertEqual(result.engine, pdf_text.ENGINE_PYMUPDF)
        self.assertEqual(result.fallback_reason, "empty_markdown")
        self.assertGreater(result.chars, 300_000)

    def test_scan_backed_layout_book_keeps_its_text(self) -> None:
        """Its text layer decodes cleanly; pdf-inspector's layout stage bails."""
        result = self.extract("Esser_1972_Vorverst*richterlicher Entschei.pdf")

        self.assertEqual(result.engine, pdf_text.ENGINE_PYMUPDF)
        self.assertEqual(result.fallback_reason, "empty_markdown")
        self.assertGreater(result.chars, 500_000)

    def test_broken_trailer_the_repair_save_rescues(self) -> None:
        """Half the broken-trailer files come back after PyMuPDF rewrites them."""
        result = self.extract("杜颖_2017_网络交易平台上的知识产权恶意投诉及其应对.pdf")

        self.assertEqual(result.engine, pdf_text.ENGINE_INSPECTOR_REPAIRED)
        self.assertIn("invalid file trailer", result.fallback_reason)

    def test_broken_trailer_the_repair_save_cannot_rescue(self) -> None:
        """The other half keeps today's PyMuPDF-only behaviour rather than failing."""
        result = self.extract("叶名怡_2018_论个人信息权的基本范畴.pdf")

        self.assertEqual(result.engine, pdf_text.ENGINE_PYMUPDF)
        self.assertIn("invalid file trailer", result.fallback_reason)
        self.assertGreater(result.chars, 10_000)

    def test_over_flagged_book_is_still_extracted_in_full(self) -> None:
        """pdf-inspector wants OCR on 304 of its 346 pages; every page has text."""
        result = self.extract("阿马蒂亚·森_2013_以自由看待发展.pdf")

        self.assertGreater(result.chars, 300_000)
        self.assertFalse(hasattr(result, "pages_needing_ocr"))


if __name__ == "__main__":
    unittest.main()
