"""The real BabelDOC call.

Kept in its own module and imported lazily by `cli.py`: everything else in this
package has to stay importable in the plain workspace venv, where the `babeldoc`
extra is not installed. Nothing here is unit-tested against the real library --
`cli.py` takes the translate step as a seam and the tests drive a stub through
it. This module is exercised by running a book.
"""

from __future__ import annotations

import asyncio
import logging
from pathlib import Path

from .contract import TranslationOutcome, TranslationRequest
from .progress import PHASE_EXTRACTING, LayoutProgress, classify_phase

logger = logging.getLogger(__name__)


def _page_count(path: Path) -> int | None:
    """Page count for the progress bar, or None if the file will not open.

    A document BabelDOC cannot open fails soon enough on its own; refusing to
    start here would only trade a real error message for a worse one.
    """

    try:
        import pymupdf

        with pymupdf.open(path) as document:
            return document.page_count or None
    except Exception as error:  # noqa: BLE001 - progress is not worth failing over
        logger.warning("Could not read the page count: %s", error)
        return None


def _build_config(request: TranslationRequest, working_dir: Path):
    from babeldoc.docvision.doclayout import DocLayoutModel
    from babeldoc.format.pdf.translation_config import (
        TranslationConfig,
        WatermarkOutputMode,
    )
    from babeldoc.translator.translator import (
        OpenAITranslator,
        set_translate_rate_limiter,
    )

    translator = OpenAITranslator(
        lang_in=request.lang_in,
        lang_out=request.lang_out,
        model=request.model,
        base_url=request.base_url,
        api_key=request.api_key,
        ignore_cache=False,
    )
    set_translate_rate_limiter(request.qps)
    return TranslationConfig(
        translator=translator,
        input_file=str(request.input_path),
        lang_in=request.lang_in,
        lang_out=request.lang_out,
        doc_layout_model=DocLayoutModel.load_onnx(),
        output_dir=str(request.output_dir),
        working_dir=str(working_dir),
        qps=request.qps,
        # The deliverable is the side-by-side bilingual PDF. The mono
        # (translation-only) PDF is a second full render of the same book that
        # nothing downstream reads, so it is not produced at all.
        no_mono=True,
        no_dual=False,
        # BabelDOC stamps its own watermark by default. This output goes back
        # into the user's Zotero library as the book they will read.
        watermark_output_mode=WatermarkOutputMode.NoWatermark,
        # The reflow track owns glossary work; here it would be a second pass of
        # LLM calls over a book whose output nothing else consumes.
        auto_extract_glossary=False,
    )


async def _run(request: TranslationRequest, progress: LayoutProgress) -> Path:
    import babeldoc.assets.assets
    import babeldoc.format.pdf.high_level

    babeldoc.format.pdf.high_level.init()
    # Downloads the layout model and the CJK fonts on first use and is a no-op
    # afterwards. Doing it before the config is built keeps the download out of
    # the middle of a translation run.
    babeldoc.assets.assets.warmup()

    working_dir = request.output_dir / "working"
    working_dir.mkdir(parents=True, exist_ok=True)
    config = _build_config(request, working_dir)

    phase = PHASE_EXTRACTING
    async for event in babeldoc.format.pdf.high_level.async_translate(config):
        event_type = event.get("type")
        if event_type == "error":
            raise RuntimeError(f"BabelDOC failed: {event.get('error')}")
        if event_type == "finish":
            result = event["translate_result"]
            dual = result.no_watermark_dual_pdf_path or result.dual_pdf_path
            if dual is None:
                raise RuntimeError("BabelDOC finished without producing a dual PDF.")
            return Path(dual)
        if event_type in {"progress_start", "progress_update", "progress_end"}:
            stage = event.get("stage")
            if stage:
                phase = classify_phase(str(stage))
            overall = event.get("overall_progress")
            progress.report(
                phase,
                fraction=float(overall) / 100.0 if overall is not None else None,
            )
    raise RuntimeError("BabelDOC stopped without reporting a result.")


def translate_document(
    request: TranslationRequest, progress: LayoutProgress
) -> TranslationOutcome:
    pages = _page_count(request.input_path)
    if pages:
        progress.total = pages
    dual_pdf_path = asyncio.run(_run(request, progress))
    return TranslationOutcome(dual_pdf_path=dual_pdf_path, page_count=pages)
