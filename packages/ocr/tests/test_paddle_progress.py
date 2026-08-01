from __future__ import annotations

import importlib.util
from pathlib import Path
from types import SimpleNamespace
import sys
from unittest import mock


PACKAGE_ROOT = Path(__file__).resolve().parents[1]
SCRIPTS = PACKAGE_ROOT / "scripts"
sys.path.insert(0, str(SCRIPTS))


def load_paddle_converter():  # type: ignore[no-untyped-def]
    module_name = "ocr_paddle_progress_test"
    path = PACKAGE_ROOT / "scripts" / "pdf_to_html_paddleocr.py"
    spec = importlib.util.spec_from_file_location(module_name, path)
    if spec is None or spec.loader is None:
        raise RuntimeError("Cannot import PaddleOCR converter")
    module = importlib.util.module_from_spec(spec)
    sys.modules[module_name] = module
    spec.loader.exec_module(module)
    return module


paddle = load_paddle_converter()


def run_main(tmp_path: Path, *, failure: Exception | None = None):  # type: ignore[no-untyped-def]
    input_dir = tmp_path / "input"
    input_dir.mkdir()
    (input_dir / "book.pdf").write_bytes(b"pdf-fixture")
    progress = mock.Mock()
    process_result: object = tmp_path / "output" / "book.html"
    if failure is not None:
        process_result = failure
    with (
        mock.patch.object(paddle, "load_config", return_value=SimpleNamespace(workers=1)),
        mock.patch.object(paddle, "pdf_page_count", return_value=3),
        mock.patch.object(
            paddle,
            "process_book",
            side_effect=process_result if failure else None,
            return_value=process_result,
        ),
        mock.patch.object(paddle.OperationProgress, "from_environment", return_value=progress),
        mock.patch.object(
            sys,
            "argv",
            [
                "pdf_to_html_paddleocr.py",
                "--input-dir",
                str(input_dir),
                "--output-dir",
                str(tmp_path / "output"),
            ],
        ),
    ):
        result = paddle.main()
    return result, progress


def test_successful_paddle_run_reports_completed_total(tmp_path: Path) -> None:
    result, progress = run_main(tmp_path)

    assert result == 0
    progress.start.assert_called_once_with("starting")
    progress.update.assert_called_once_with(completed=3, total=3, phase="completed")
    progress.touch.assert_not_called()


def test_failed_paddle_book_reports_failed_and_returns_nonzero(tmp_path: Path) -> None:
    result, progress = run_main(tmp_path, failure=paddle.ConverterError("remote failure"))

    assert result == 1
    progress.touch.assert_called_once_with("failed")
    progress.update.assert_not_called()
