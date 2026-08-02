"""Page markers must not become body text in the translation source.

The assembled Markdown is what the translation handoff copies into a reading
project, so the per-page separator has to be invisible to the splitter and the
translator while still recording which page a passage came from. The standalone
HTML keeps the visible dashed separator it always had.

Every remote call is stubbed, so these tests never touch the Baidu API.
"""

from __future__ import annotations

import importlib.util
import json
from pathlib import Path
import sys
from unittest import mock


PACKAGE_ROOT = Path(__file__).resolve().parents[1]
SCRIPTS = PACKAGE_ROOT / "scripts"
sys.path.insert(0, str(SCRIPTS))


def load_paddle_converter():  # type: ignore[no-untyped-def]
    module_name = "ocr_paddle_page_anchor_test"
    path = SCRIPTS / "pdf_to_html_paddleocr.py"
    spec = importlib.util.spec_from_file_location(module_name, path)
    if spec is None or spec.loader is None:
        raise RuntimeError("Cannot import PaddleOCR converter")
    module = importlib.util.module_from_spec(spec)
    sys.modules[module_name] = module
    spec.loader.exec_module(module)
    return module


paddle = load_paddle_converter()


def test_page_anchor_is_an_html_comment() -> None:
    assert paddle.page_anchor(42) == "<!-- page: 42 -->"


def test_page_separator_is_the_visible_html_form() -> None:
    assert paddle.page_separator(7) == '<div class="page-break">— Page 7 —</div>'


# ---------------------------------------------------------------------------
# End-to-end through process_book
# ---------------------------------------------------------------------------
# Page 2 quotes a page anchor, the way a book about this pipeline would. A
# substitution pass over the assembled document would rewrite it; assembling the
# Markdown and the HTML separately cannot.
PAGE_BODIES = ["Body of page 1", "Body of page 2\n\n<!-- page: 999 -->"]


class FakeOCRClient:
    def __init__(self, config) -> None:  # type: ignore[no-untyped-def]
        pass

    def submit_job(self, chunk_path: Path, batch_id: str) -> str:
        return "job"

    def poll_json_url(self, job_id: str, deadline: float, on_progress=None) -> str:  # type: ignore[no-untyped-def]
        return "https://example.invalid/result.jsonl"

    def download_jsonl(self, json_url: str) -> str:
        return "\n".join(
            json.dumps(
                {
                    "result": {
                        "layoutParsingResults": [
                            {"markdown": {"text": body, "images": {}}}
                        ]
                    }
                }
            )
            for body in PAGE_BODIES
        )


def fake_chunk_specs(source: Path, pages, chunk_dir: Path, max_bytes):  # type: ignore[no-untyped-def]
    chunk_path = chunk_dir / f"pages-{pages[0]:04d}-{pages[-1]:04d}.pdf"
    chunk_path.write_bytes(b"%PDF chunk")
    return [(list(pages), chunk_path)]


def run_process_book(tmp_path: Path):  # type: ignore[no-untyped-def]
    input_dir = tmp_path / "input"
    input_dir.mkdir(parents=True, exist_ok=True)
    pdf_path = input_dir / "Sample Book.pdf"
    pdf_path.write_bytes(b"%PDF fixture")
    output_dir = tmp_path / "output"
    temp_root = output_dir / ".temp"
    temp_root.mkdir(parents=True, exist_ok=True)

    config = paddle.Config(
        baidu_token="t",
        baidu_job_url="https://example.invalid/jobs",
        baidu_model="PaddleOCR-VL",
        max_ocr_pages_per_job=12,
        baidu_max_upload_mb=49,
        request_timeout=30,
        poll_seconds=1,
        workers=1,
    )

    with (
        mock.patch.object(paddle, "pdf_page_count", return_value=2),
        mock.patch.object(paddle, "make_chunk_specs", side_effect=fake_chunk_specs),
        mock.patch.object(paddle, "BaiduOCRClient", side_effect=FakeOCRClient),
    ):
        html_path = paddle.process_book(
            pdf_path, output_dir, config, temp_root, mock.Mock()
        )
    # The .md on disk is exactly what the translation handoff copies.
    return html_path, html_path.with_suffix(".md").read_text(encoding="utf-8")


def test_assembled_markdown_carries_anchors_not_html_separators(tmp_path: Path) -> None:
    _, full_md = run_process_book(tmp_path)

    assert "<!-- page: 1 -->" in full_md
    assert "<!-- page: 2 -->" in full_md
    # The whole point: no visible page scaffolding in the translation source.
    assert "page-break" not in full_md
    assert "— Page" not in full_md
    assert "Body of page 1" in full_md


def test_rendered_html_keeps_the_visible_separator(tmp_path: Path) -> None:
    html_path, _ = run_process_book(tmp_path)

    html = html_path.read_text(encoding="utf-8")
    assert '<div class="page-break">— Page 1 —</div>' in html
    assert '<div class="page-break">— Page 2 —</div>' in html
    # The stylesheet rule that makes it a dashed rule is still shipped.
    assert ".page-break {" in html


def test_an_anchor_that_came_from_the_book_is_not_turned_into_a_separator(
    tmp_path: Path,
) -> None:
    html_path, full_md = run_process_book(tmp_path)

    # It survives verbatim in the Markdown the handoff receives...
    assert "<!-- page: 999 -->" in full_md
    # ...and never becomes a page break the book never had.
    html = html_path.read_text(encoding="utf-8")
    assert "— Page 999 —" not in html
