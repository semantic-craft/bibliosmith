"""CLI behaviour, driven through the translate seam so no BabelDOC is needed."""

from __future__ import annotations

import json
import os
from pathlib import Path
import tempfile
import unittest
from unittest import mock

from layout_pdf.cli import (
    API_KEY_ENV,
    BASE_URL_ENV,
    MODEL_ENV,
    bilingual_output_name,
    main,
)
from layout_pdf.contract import TranslationOutcome
from layout_pdf.progress import PHASE_TRANSLATING

ENDPOINT_ENV = {
    BASE_URL_ENV: "https://example.invalid/v1",
    API_KEY_ENV: "test-key",
    MODEL_ENV: "test-model",
}


def write_pdf(path: Path, body: bytes = b"%PDF-1.7\n") -> Path:
    path.write_bytes(body)
    return path


class StubTranslator:
    """Stands in for BabelDOC: writes a dual PDF where the real one would."""

    def __init__(self, *, pages: int | None = 12, fraction: float | None = 0.5):
        self.requests = []
        self.pages = pages
        self.fraction = fraction

    def __call__(self, request, progress):
        self.requests.append(request)
        if self.pages:
            progress.total = self.pages
        progress.report(PHASE_TRANSLATING, fraction=self.fraction)
        dual = request.output_dir / "book.zh-CN.dual.pdf"
        dual.write_bytes(b"%PDF-1.7 dual\n")
        return TranslationOutcome(dual_pdf_path=dual, page_count=self.pages)


class LayoutPdfCliTests(unittest.TestCase):
    def test_writes_exactly_the_bilingual_pdf_into_the_output_directory(self) -> None:
        with tempfile.TemporaryDirectory() as raw:
            root = Path(raw)
            source = write_pdf(root / "book.pdf")
            output_dir = root / "out"
            translator = StubTranslator()

            with mock.patch.dict(os.environ, ENDPOINT_ENV, clear=False):
                exit_code = main(
                    ["--input", str(source), "--output-dir", str(output_dir)],
                    translate_document=translator,
                )

            self.assertEqual(exit_code, 0)
            # The Launcher registers every PDF under the job output root as a
            # deliverable, so BabelDOC's scratch output must not land here.
            self.assertEqual(
                sorted(entry.name for entry in output_dir.iterdir()),
                ["book.zh-CN.bilingual.pdf"],
            )

    def test_staging_directory_is_removed_after_the_run(self) -> None:
        with tempfile.TemporaryDirectory() as raw:
            root = Path(raw)
            source = write_pdf(root / "book.pdf")
            translator = StubTranslator()

            with mock.patch.dict(os.environ, ENDPOINT_ENV, clear=False):
                main(
                    ["--input", str(source), "--output-dir", str(root / "out")],
                    translate_document=translator,
                )

            staging = translator.requests[0].output_dir
            self.assertNotEqual(staging, root / "out")
            self.assertFalse(staging.exists())

    def test_output_name_carries_the_target_language(self) -> None:
        self.assertEqual(
            bilingual_output_name(Path("/books/Weber 1922.pdf"), "zh-CN"),
            "Weber 1922.zh-CN.bilingual.pdf",
        )

    def test_endpoint_comes_from_the_environment(self) -> None:
        with tempfile.TemporaryDirectory() as raw:
            root = Path(raw)
            source = write_pdf(root / "book.pdf")
            translator = StubTranslator()

            with mock.patch.dict(os.environ, ENDPOINT_ENV, clear=False):
                main(
                    ["--input", str(source), "--output-dir", str(root / "out")],
                    translate_document=translator,
                )

            request = translator.requests[0]
            self.assertEqual(request.base_url, "https://example.invalid/v1")
            self.assertEqual(request.api_key, "test-key")
            self.assertEqual(request.model, "test-model")
            self.assertEqual(request.lang_in, "en")
            self.assertEqual(request.lang_out, "zh-CN")

    def test_missing_endpoint_is_refused_before_any_work(self) -> None:
        with tempfile.TemporaryDirectory() as raw:
            root = Path(raw)
            source = write_pdf(root / "book.pdf")
            translator = StubTranslator()
            environment = dict(ENDPOINT_ENV)
            environment[API_KEY_ENV] = ""

            with mock.patch.dict(os.environ, environment, clear=False):
                with self.assertRaises(SystemExit) as raised:
                    main(
                        ["--input", str(source), "--output-dir", str(root / "out")],
                        translate_document=translator,
                    )

            self.assertIn(API_KEY_ENV, str(raised.exception))
            self.assertEqual(translator.requests, [])

    def test_non_pdf_input_is_refused(self) -> None:
        with tempfile.TemporaryDirectory() as raw:
            root = Path(raw)
            source = root / "book.epub"
            source.write_bytes(b"PK\x03\x04")

            with mock.patch.dict(os.environ, ENDPOINT_ENV, clear=False):
                with self.assertRaises(SystemExit) as raised:
                    main(
                        ["--input", str(source), "--output-dir", str(root / "out")],
                        translate_document=StubTranslator(),
                    )

            self.assertIn("only accepts PDFs", str(raised.exception))

    def test_a_translator_that_writes_nothing_fails_the_run(self) -> None:
        def writes_nothing(request, progress):
            return TranslationOutcome(dual_pdf_path=request.output_dir / "absent.pdf")

        with tempfile.TemporaryDirectory() as raw:
            root = Path(raw)
            source = write_pdf(root / "book.pdf")
            output_dir = root / "out"

            with mock.patch.dict(os.environ, ENDPOINT_ENV, clear=False):
                with self.assertRaises(SystemExit) as raised:
                    main(
                        ["--input", str(source), "--output-dir", str(output_dir)],
                        translate_document=writes_nothing,
                    )

            self.assertIn("wrote no bilingual PDF", str(raised.exception))
            self.assertEqual(list(output_dir.iterdir()), [])

    def test_progress_sidecar_tracks_the_run(self) -> None:
        with tempfile.TemporaryDirectory() as raw:
            root = Path(raw)
            source = write_pdf(root / "book.pdf")
            sidecar = root / "out" / ".book-pipeline-progress"
            environment = dict(ENDPOINT_ENV)
            environment["BIBLIOSMITH_PROGRESS_PATH"] = str(sidecar)
            environment["BIBLIOSMITH_PROGRESS_SCOPE"] = "child-1"

            with mock.patch.dict(os.environ, environment, clear=False):
                main(
                    ["--input", str(source), "--output-dir", str(root / "out")],
                    translate_document=StubTranslator(pages=12, fraction=0.5),
                )

            document = json.loads(sidecar.read_text(encoding="utf-8"))
            self.assertEqual(document["schema"], "book-pipeline-progress-v1")
            self.assertEqual(document["stageId"], "extract")
            self.assertEqual(document["unitKind"], "pages")
            self.assertEqual(document["phase"], PHASE_TRANSLATING)
            self.assertEqual(document["scopeId"], "child-1")
            self.assertEqual(document["total"], 12)
            self.assertEqual(document["completed"], 6)

    def test_warning_markers_reach_stdout(self) -> None:
        import logging

        def warns(request, progress):
            logger = logging.getLogger("babeldoc.format.pdf.legacy_parse")
            logger.warning("page %s is too large, maybe unable to translate", 41)
            logger.warning("page %s is too large, maybe unable to translate", 42)
            logger.warning("something else entirely")
            dual = request.output_dir / "book.zh-CN.dual.pdf"
            dual.write_bytes(b"%PDF-1.7 dual\n")
            return TranslationOutcome(dual_pdf_path=dual)

        with tempfile.TemporaryDirectory() as raw:
            root = Path(raw)
            source = write_pdf(root / "book.pdf")

            with mock.patch.dict(os.environ, ENDPOINT_ENV, clear=False):
                with mock.patch("builtins.print") as printed:
                    main(
                        ["--input", str(source), "--output-dir", str(root / "out")],
                        translate_document=warns,
                    )

        lines = [call.args[0] for call in printed.call_args_list]
        self.assertIn("BOOK_PIPELINE_MARKER warning=large_page count=2", lines)
        self.assertIn("BOOK_PIPELINE_MARKER warning=other count=1", lines)

    def test_the_warning_handler_is_detached_after_the_run(self) -> None:
        import logging

        with tempfile.TemporaryDirectory() as raw:
            root = Path(raw)
            source = write_pdf(root / "book.pdf")
            before = list(logging.getLogger("babeldoc").handlers)

            with mock.patch.dict(os.environ, ENDPOINT_ENV, clear=False):
                main(
                    ["--input", str(source), "--output-dir", str(root / "out")],
                    translate_document=StubTranslator(),
                )

            self.assertEqual(logging.getLogger("babeldoc").handlers, before)


if __name__ == "__main__":
    unittest.main()
