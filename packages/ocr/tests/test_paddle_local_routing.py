"""A local PDF folder must not pay for OCR on books that carry their own text.

Before #137 `process_book()` went straight from `make_chunk_specs()` to
`BaiduOCRClient.submit_job()` for every PDF in the folder, with no text-layer
check anywhere on the path — while the Zotero route next door had been sampling
the text layer all along. On the live corpus that meant uploading roughly nine
books in ten that PyMuPDF could read for free.

The remote boundary is stubbed with an object that fails the test if it is
touched at all, so "no Baidu API call" is asserted rather than assumed.
"""

from __future__ import annotations

import importlib.util
import json
from pathlib import Path
import sys
from unittest import mock

import pytest


PACKAGE_ROOT = Path(__file__).resolve().parents[1]
SCRIPTS = PACKAGE_ROOT / "scripts"
sys.path.insert(0, str(SCRIPTS))


def load_paddle_converter():  # type: ignore[no-untyped-def]
    module_name = "ocr_paddle_local_routing_test"
    path = SCRIPTS / "pdf_to_html_paddleocr.py"
    spec = importlib.util.spec_from_file_location(module_name, path)
    if spec is None or spec.loader is None:
        raise RuntimeError("Cannot import PaddleOCR converter")
    module = importlib.util.module_from_spec(spec)
    sys.modules[module_name] = module
    spec.loader.exec_module(module)
    return module


paddle = load_paddle_converter()


def fake_config(token: str = "token"):  # type: ignore[no-untyped-def]
    return paddle.Config(
        baidu_token=token,
        baidu_job_url="https://example.invalid/jobs",
        baidu_model="PaddleOCR-VL",
        max_ocr_pages_per_job=12,
        baidu_max_upload_mb=49,
        request_timeout=30,
        poll_seconds=1,
        workers=1,
    )


class ForbiddenOCRClient:
    """Any construction at all is a paid upload this route must never reach."""

    def __init__(self, config):  # type: ignore[no-untyped-def]
        raise AssertionError("a direct-text book must not open an OCR client")


def sample(*, extractable: bool, degraded: bool = False, chars: int = 2000):  # type: ignore[no-untyped-def]
    import zotero_llm_worker

    return zotero_llm_worker.TextLayerSample(
        extractable=extractable,
        chars=chars,
        sample_pages=[1, 2, 3],
        degraded=degraded,
        reason="private_use_ratio=0.900>=0.005" if degraded else "",
    )


def book(tmp_path: Path, name: str = "Born Digital.pdf") -> Path:
    input_dir = tmp_path / "input"
    input_dir.mkdir(exist_ok=True)
    path = input_dir / name
    path.write_bytes(b"%PDF fixture")
    return path


# ---------------------------------------------------------------------------
# Classification
# ---------------------------------------------------------------------------
def test_a_readable_text_layer_routes_away_from_the_paid_engine(tmp_path: Path) -> None:
    with mock.patch(
        "zotero_llm_worker.sample_text_layer", return_value=sample(extractable=True)
    ):
        route, reason = paddle.classify_route(book(tmp_path), 240, object())

    assert route == paddle.ROUTE_DIRECT_TEXT
    assert "chars=2000" in reason


def test_a_scan_still_goes_to_remote_ocr(tmp_path: Path) -> None:
    with mock.patch(
        "zotero_llm_worker.sample_text_layer",
        return_value=sample(extractable=False, chars=3),
    ):
        route, reason = paddle.classify_route(book(tmp_path), 240, object())

    assert route == paddle.ROUTE_REMOTE_PADDLEOCR
    assert "low extractable text" in reason


def test_a_degraded_text_layer_goes_to_ocr_rather_than_being_extracted(
    tmp_path: Path,
) -> None:
    """Mojibake is the one case where paying to re-read the page is right.

    The Zotero route holds such a book for manual MinerU review; this entry
    point has no review step to hold it in, so it must not quietly extract the
    broken glyphs into the translation source instead.
    """
    with mock.patch(
        "zotero_llm_worker.sample_text_layer",
        return_value=sample(extractable=True, degraded=True),
    ):
        route, reason = paddle.classify_route(book(tmp_path), 240, object())

    assert route == paddle.ROUTE_REMOTE_PADDLEOCR
    assert "degraded text layer" in reason


def test_an_unreadable_file_falls_back_to_ocr_without_ending_the_run(
    tmp_path: Path,
) -> None:
    path = book(tmp_path, "Not Really A PDF.pdf")

    plan = paddle.plan_routes([path])

    assert plan[path][0] == paddle.ROUTE_REMOTE_PADDLEOCR
    assert "page count failed" in plan[path][1]


# ---------------------------------------------------------------------------
# Overrides
# ---------------------------------------------------------------------------
def test_a_forced_route_is_taken_without_sampling_the_book(tmp_path: Path) -> None:
    """The wizard's chip is the decision; re-deriving one that disagrees is drift."""
    direct = book(tmp_path, "Forced Direct.pdf")
    ocr = book(tmp_path, "Forced OCR.pdf")

    with mock.patch("zotero_llm_worker.sample_text_layer") as sampler:
        plan = paddle.plan_routes(
            [direct, ocr],
            force_text=["Forced Direct.pdf"],
            force_ocr=["Forced OCR.pdf"],
        )

    sampler.assert_not_called()
    assert plan[direct] == (paddle.ROUTE_DIRECT_TEXT, "forced by route override")
    assert plan[ocr] == (paddle.ROUTE_REMOTE_PADDLEOCR, "forced by route override")


def test_a_book_cannot_be_forced_both_ways(tmp_path: Path) -> None:
    path = book(tmp_path, "Contradiction.pdf")

    with pytest.raises(paddle.ConverterError, match="Contradiction.pdf"):
        paddle.plan_routes(
            [path], force_text=["Contradiction.pdf"], force_ocr=["Contradiction.pdf"]
        )


# ---------------------------------------------------------------------------
# The route plan the launcher reads
# ---------------------------------------------------------------------------
def test_the_plan_marker_matches_the_launcher_contract(tmp_path: Path, caplog) -> None:  # type: ignore[no-untyped-def]
    """Pins both halves of the contract parse_local_pdf_route_plan reads.

    The marker prefix and the schema string are literals in
    book_pipeline/contract.rs; a rename on this side that is not made there
    silently returns the route preview to naming the paid engine for every book.
    """
    path = book(tmp_path, "Born Digital.pdf")

    with caplog.at_level("INFO"):
        paddle.emit_route_plan({path: (paddle.ROUTE_DIRECT_TEXT, "extractable")})

    line = next(m for m in caplog.messages if paddle.ROUTE_PLAN_MARKER in m)
    prefix, payload = line.split(f"{paddle.ROUTE_PLAN_MARKER} ", 1)
    assert prefix == ""
    assert json.loads(payload) == {
        "schemaVersion": "local-pdf-route-plan-v1",
        "path": str(path),
        "name": "Born Digital.pdf",
        "route": "direct_text",
        "reason": "extractable",
    }
    assert paddle.ROUTE_PLAN_MARKER == "BOOK_PIPELINE_LOCAL_PDF_ROUTE"


# ---------------------------------------------------------------------------
# The direct-text conversion itself
# ---------------------------------------------------------------------------
def run_direct(tmp_path: Path, markdown: str = "# Chapter One\n\nBody text."):  # type: ignore[no-untyped-def]
    import pdf_text

    pdf_path = book(tmp_path)
    output_dir = tmp_path / "output"
    output_dir.mkdir(exist_ok=True)
    temp_root = output_dir / ".temp"
    temp_root.mkdir(exist_ok=True)
    extracted = pdf_text.PdfTextResult(
        markdown=markdown,
        engine="pdf-inspector",
        fallback_reason="",
        chars=len(markdown),
        page_count=240,
    )
    with (
        mock.patch.object(paddle.pdf_text, "extract_markdown", return_value=extracted),
        mock.patch.object(paddle, "BaiduOCRClient", ForbiddenOCRClient),
        mock.patch.object(paddle, "make_chunk_specs", side_effect=AssertionError),
    ):
        html_path = paddle.process_book(
            pdf_path,
            output_dir,
            fake_config(token=""),
            temp_root,
            mock.Mock(),
            None,
            paddle.ROUTE_DIRECT_TEXT,
        )
    return html_path, output_dir


def test_a_direct_text_book_converts_with_no_remote_call_at_all(tmp_path: Path) -> None:
    html_path, output_dir = run_direct(tmp_path)

    book_dir = output_dir / "Born_Digital"
    assert html_path == book_dir / "Born_Digital.html"
    md_path = book_dir / "Born_Digital.md"
    assert md_path.is_file(), "the translation handoff reads this file"

    body = md_path.read_text(encoding="utf-8")
    assert body.startswith("# Born Digital\n")
    assert "Chapter One" in body
    assert "<html" not in body
    assert "Chapter One" in html_path.read_text(encoding="utf-8")


def test_the_direct_route_leaves_no_empty_assets_sidecar(tmp_path: Path) -> None:
    """A text layer has no images, and the sidecar travels into every project."""
    _, output_dir = run_direct(tmp_path)

    assert not (output_dir / "Born_Digital" / "Born_Digital_assets").exists()


def test_the_direct_route_records_the_engine_it_used(tmp_path: Path) -> None:
    _, output_dir = run_direct(tmp_path)

    state = json.loads(
        (output_dir / "Born_Digital" / "_state.json").read_text(encoding="utf-8")
    )
    assert state["route"] == paddle.ROUTE_DIRECT_TEXT
    assert state["engine"] == "pdf-inspector"
    assert state["source_name"] == "Born Digital.pdf"
    assert state["pages_done"] == state["pages_total"] == 240


def test_a_rerun_rewrites_the_markdown_rather_than_leaving_stale_text(
    tmp_path: Path,
) -> None:
    """Local extraction has nothing to resume, so a rerun must not skip the write.

    A stale body rather than a deleted file: a write-if-missing implementation
    would still repair a deletion, so only overwriting proves the rerun wrote.
    """
    _, output_dir = run_direct(tmp_path)
    md_path = output_dir / "Born_Digital" / "Born_Digital.md"
    first = md_path.read_text(encoding="utf-8")
    md_path.write_text("STALE", encoding="utf-8")

    run_direct(tmp_path)

    assert md_path.read_text(encoding="utf-8") == first


def test_process_book_classifies_for_itself_when_no_route_was_planned(
    tmp_path: Path,
) -> None:
    """A caller converting one book on its own still gets the routing decision."""
    pdf_path = book(tmp_path)
    output_dir = tmp_path / "output"
    output_dir.mkdir(exist_ok=True)

    with (
        mock.patch.object(paddle, "pdf_page_count", return_value=240),
        mock.patch(
            "zotero_llm_worker.sample_text_layer", return_value=sample(extractable=True)
        ),
        mock.patch.object(
            paddle.pdf_text,
            "extract_markdown",
            return_value=__import__("pdf_text").PdfTextResult(
                markdown="Body.", engine="pymupdf", fallback_reason="", chars=5, page_count=240
            ),
        ),
        mock.patch.object(paddle, "BaiduOCRClient", ForbiddenOCRClient),
    ):
        html_path = paddle.process_book(
            pdf_path, output_dir, fake_config(token=""), output_dir / ".temp", mock.Mock()
        )

    assert html_path.is_file()


# ---------------------------------------------------------------------------
# Credentials
# ---------------------------------------------------------------------------
def test_a_missing_token_is_no_longer_an_error_before_any_pdf_is_looked_at() -> None:
    """A folder of born-digital books converts on a machine with no OCR account."""
    with mock.patch.dict("os.environ", {"BAIDU_PADDLEOCR_TOKEN": ""}, clear=False):
        config = paddle.load_config()

    assert config.baidu_token == ""


def test_the_ocr_client_still_refuses_to_run_without_a_credential() -> None:
    with pytest.raises(paddle.ConverterError, match="BAIDU_PADDLEOCR_TOKEN"):
        paddle.BaiduOCRClient(fake_config(token=""))
