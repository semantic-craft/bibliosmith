"""Launcher-facing entry point for the layout-preserving PDF track.

Invoked by `book_pipeline.rs` as

    uv run --package layout-pdf --extra babeldoc layout-pdf \\
        --input <source.pdf> --output-dir <job output dir>

with the active model slot's endpoint in the environment. Exactly one file --
the bilingual PDF -- is written into the output directory; BabelDOC's own
output and scratch files stay in a temporary directory, because the Launcher
registers every PDF it finds under the job output root as a deliverable.
"""

from __future__ import annotations

import argparse
import logging
import os
from pathlib import Path
import shutil
import sys
import tempfile

from .contract import TranslateDocument, TranslationRequest
from .progress import PHASE_STARTING, LayoutProgress
from .warnings import WarningCollector

BASE_URL_ENV = "LAYOUT_PDF_BASE_URL"
API_KEY_ENV = "LAYOUT_PDF_API_KEY"
MODEL_ENV = "LAYOUT_PDF_MODEL"

# The Launcher's stage vocabulary, not BabelDOC's: this whole run is one
# `extract` stage as far as the pipeline is concerned.
STAGE_ID = "extract"


def _default_translate_document() -> TranslateDocument:
    # Imported here rather than at module scope so the argument parsing, the
    # environment contract and the warning classifier stay testable in the plain
    # workspace venv, where the `babeldoc` extra is deliberately not installed.
    from .babeldoc_runner import translate_document

    return translate_document


def _require_env(name: str) -> str:
    value = os.environ.get(name, "").strip()
    if not value:
        raise SystemExit(
            f"{name} is not set. The layout-preserving track needs an "
            "OpenAI-compatible endpoint from the active model slot."
        )
    return value


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(
        prog="layout-pdf",
        description="Translate a text PDF into a bilingual PDF that keeps the original layout.",
    )
    parser.add_argument("--input", required=True, type=Path, help="Source PDF.")
    parser.add_argument(
        "--output-dir",
        required=True,
        type=Path,
        help="Directory the bilingual PDF is written into.",
    )
    parser.add_argument("--lang-in", default="en", help="Source language (default: en).")
    parser.add_argument(
        "--lang-out", default="zh-CN", help="Target language (default: zh-CN)."
    )
    parser.add_argument(
        "--qps",
        type=int,
        default=4,
        help="Requests per second allowed against the provider (default: 4).",
    )
    return parser


def bilingual_output_name(source: Path, lang_out: str) -> str:
    return f"{source.stem}.{lang_out}.bilingual.pdf"


def main(
    argv: list[str] | None = None,
    *,
    translate_document: TranslateDocument | None = None,
) -> int:
    arguments = build_parser().parse_args(argv)
    source: Path = arguments.input
    if not source.is_file():
        raise SystemExit(f"Source PDF not found: {source}")
    if source.suffix.lower() != ".pdf":
        raise SystemExit(f"The layout-preserving track only accepts PDFs: {source}")

    output_dir: Path = arguments.output_dir
    output_dir.mkdir(parents=True, exist_ok=True)

    translate = translate_document or _default_translate_document()
    progress = LayoutProgress.from_environment(STAGE_ID)
    progress.report(PHASE_STARTING)

    collector = WarningCollector()
    babeldoc_logger = logging.getLogger("babeldoc")
    babeldoc_logger.addHandler(collector)
    try:
        # BabelDOC writes the mono PDF, an optional glossary and assorted scratch
        # files next to its output. Keeping all of that outside the job output
        # directory is what stops the Launcher's artifact scan from offering an
        # intermediate file as the finished book.
        with tempfile.TemporaryDirectory(prefix="babeldoc-") as staging:
            request = TranslationRequest(
                input_path=source,
                output_dir=Path(staging),
                lang_in=arguments.lang_in,
                lang_out=arguments.lang_out,
                base_url=_require_env(BASE_URL_ENV),
                api_key=_require_env(API_KEY_ENV),
                model=_require_env(MODEL_ENV),
                qps=arguments.qps,
            )
            outcome = translate(request, progress)
            if not outcome.dual_pdf_path.is_file():
                raise SystemExit(
                    "BabelDOC reported success but wrote no bilingual PDF at "
                    f"{outcome.dual_pdf_path}"
                )
            destination = output_dir / bilingual_output_name(source, arguments.lang_out)
            # move, not copy: the staging directory is about to be removed, and
            # on the same filesystem this is a rename rather than a second full
            # write of a book-sized PDF.
            shutil.move(str(outcome.dual_pdf_path), str(destination))
    finally:
        babeldoc_logger.removeHandler(collector)

    for marker in collector.markers():
        print(marker)
    print(f"Bilingual PDF written to {destination}")
    return 0


if __name__ == "__main__":  # pragma: no cover - exercised through the console script
    sys.exit(main())
