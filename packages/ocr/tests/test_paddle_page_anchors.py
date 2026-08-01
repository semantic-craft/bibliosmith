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
from types import SimpleNamespace
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


def test_anchors_render_as_the_visible_separator() -> None:
    rendered = paddle.page_anchors_to_html("a\n\n<!-- page: 7 -->\n\nb")

    assert '<div class="page-break">— Page 7 —</div>' in rendered
    assert "<!-- page: 7 -->" not in rendered


def test_every_anchor_in_a_document_is_converted() -> None:
    body = "\n\n".join(f"<!-- page: {n} -->\n\ntext {n}" for n in (1, 2, 3))

    rendered = paddle.page_anchors_to_html(body)

    assert rendered.count('class="page-break"') == 3
    assert "<!-- page:" not in rendered


def test_a_comment_that_is_not_a_page_anchor_is_left_alone() -> None:
    body = "<!-- keep me -->\n\n<!-- page: 1 -->"

    rendered = paddle.page_anchors_to_html(body)

    assert "<!-- keep me -->" in rendered
    assert '<div class="page-break">— Page 1 —</div>' in rendered


# ---------------------------------------------------------------------------
# End-to-end through process_book
# ---------------------------------------------------------------------------
class FakeOCRClient:
    def __init__(self, config) -> None:  # type: ignore[no-untyped-def]
        self.jsonl = ""

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
                            {"markdown": {"text": f"Body of page {n}", "images": {}}}
                        ]
                    }
                }
            )
            for n in (1, 2)
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

    # Capture the assembled Markdown exactly as the handoff would receive it.
    assembled: dict[str, str] = {}
    real_to_html = paddle.page_anchors_to_html

    def spy(md_text: str) -> str:
        assembled["full_md"] = md_text
        return real_to_html(md_text)

    with (
        mock.patch.object(paddle, "pdf_page_count", return_value=2),
        mock.patch.object(paddle, "make_chunk_specs", side_effect=fake_chunk_specs),
        mock.patch.object(paddle, "BaiduOCRClient", side_effect=FakeOCRClient),
        mock.patch.object(paddle, "page_anchors_to_html", side_effect=spy),
    ):
        html_path = paddle.process_book(
            pdf_path, output_dir, config, temp_root, mock.Mock()
        )
    return html_path, assembled["full_md"]


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
    assert "<!-- page:" not in html
