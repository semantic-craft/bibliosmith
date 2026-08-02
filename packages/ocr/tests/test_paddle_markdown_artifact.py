"""The PaddleOCR wrapper must leave its assembled Markdown on disk.

The translation handoff picks a job's `markdown` artifact off the extraction
output directory. Before this coverage existed the wrapper rendered the
assembled Markdown straight into HTML and dropped it, so a local PDF folder
routed through PaddleOCR completed with no Markdown artifact and the handoff
stopped with "no cleaned Markdown artifact".

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
    module_name = "ocr_paddle_markdown_artifact_test"
    path = SCRIPTS / "pdf_to_html_paddleocr.py"
    spec = importlib.util.spec_from_file_location(module_name, path)
    if spec is None or spec.loader is None:
        raise RuntimeError("Cannot import PaddleOCR converter")
    module = importlib.util.module_from_spec(spec)
    sys.modules[module_name] = module
    spec.loader.exec_module(module)
    return module


paddle = load_paddle_converter()


def fake_config():  # type: ignore[no-untyped-def]
    return paddle.Config(
        baidu_token="token",
        baidu_job_url="https://example.invalid/jobs",
        baidu_model="PaddleOCR-VL",
        max_ocr_pages_per_job=12,
        baidu_max_upload_mb=49,
        request_timeout=30,
        poll_seconds=1,
        workers=1,
    )


def jsonl_page(text: str) -> str:
    """One JSONL line shaped like a Baidu layout-parsing result."""
    return json.dumps(
        {"result": {"layoutParsingResults": [{"markdown": {"text": text, "images": {}}}]}}
    )


class FakeOCRClient:
    """Stands in for BaiduOCRClient; records calls, never hits the network."""

    def __init__(self, config, jsonl_text: str) -> None:  # type: ignore[no-untyped-def]
        self.config = config
        self.jsonl_text = jsonl_text
        self.submitted: list[str] = []

    def submit_job(self, chunk_path: Path, batch_id: str) -> str:
        self.submitted.append(batch_id)
        return f"job-{batch_id}"

    def poll_json_url(self, job_id: str, deadline: float, on_progress=None) -> str:  # type: ignore[no-untyped-def]
        if on_progress is not None:
            on_progress(1, 1)
        return f"https://example.invalid/results/{job_id}.jsonl"

    def download_jsonl(self, json_url: str) -> str:
        return self.jsonl_text


def run_process_book(tmp_path: Path, *, pages: int = 2, page_text: str = "Chapter One"):  # type: ignore[no-untyped-def]
    """Drive the real process_book with only the remote boundary stubbed."""
    input_dir = tmp_path / "input"
    input_dir.mkdir(exist_ok=True)
    pdf_path = input_dir / "Sample Book.pdf"
    pdf_path.write_bytes(b"%PDF fixture")

    output_dir = tmp_path / "output"
    output_dir.mkdir(exist_ok=True)
    temp_root = output_dir / ".temp"
    temp_root.mkdir(exist_ok=True)

    jsonl_text = "\n".join(jsonl_page(page_text) for _ in range(pages))
    clients: list[FakeOCRClient] = []

    def make_client(config):  # type: ignore[no-untyped-def]
        client = FakeOCRClient(config, jsonl_text)
        clients.append(client)
        return client

    def fake_chunk_specs(source, page_numbers, chunk_dir, max_bytes):  # type: ignore[no-untyped-def]
        chunk_path = chunk_dir / f"pages-{page_numbers[0]:04d}-{page_numbers[-1]:04d}.pdf"
        chunk_path.write_bytes(b"%PDF chunk")
        return [(list(page_numbers), chunk_path)]

    progress = mock.Mock()
    with (
        mock.patch.object(paddle, "pdf_page_count", return_value=pages),
        mock.patch.object(paddle, "make_chunk_specs", side_effect=fake_chunk_specs),
        mock.patch.object(paddle, "BaiduOCRClient", side_effect=make_client),
    ):
        html_path = paddle.process_book(
            pdf_path, output_dir, fake_config(), temp_root, progress
        )
    return html_path, output_dir, clients


def test_process_book_writes_markdown_next_to_html(tmp_path: Path) -> None:
    html_path, output_dir, _ = run_process_book(tmp_path)

    md_path = html_path.with_suffix(".md")
    assert md_path.is_file(), "assembled Markdown must be written to disk"
    assert md_path == output_dir / "Sample_Book" / "Sample_Book.md"

    body = md_path.read_text(encoding="utf-8")
    assert body.startswith("# Sample Book\n")
    assert "Chapter One" in body
    # The Markdown is the assembled source, not the rendered HTML.
    assert "<html" not in body


def test_resumed_run_rewrites_the_same_markdown_file(tmp_path: Path) -> None:
    html_path, _, first_clients = run_process_book(tmp_path)
    md_path = html_path.with_suffix(".md")
    first_body = md_path.read_text(encoding="utf-8")
    assert first_clients[0].submitted, "the first run must submit its chunk"

    # A stale body, not a deleted file: a write-if-missing implementation would
    # still repair a deletion, so only overwriting proves the rerun rewrote it.
    md_path.write_text("STALE", encoding="utf-8")

    # Same output directory, so the second run resumes from _state.json.
    _, _, second_clients = run_process_book(tmp_path)

    assert second_clients[0].submitted == [], "a resumed run must not resubmit chunks"
    assert md_path.read_text(encoding="utf-8") == first_body


def test_markdown_is_not_shadowed_by_state_or_assets(tmp_path: Path) -> None:
    """The new file must not collide with the sibling state file or assets dir."""
    html_path, _, _ = run_process_book(tmp_path)
    book_dir = html_path.parent

    assert (book_dir / "_state.json").is_file()
    assert (book_dir / "Sample_Book_assets").is_dir()
    # Exactly one Markdown artifact for the launcher's scan to pick up.
    assert [p.name for p in book_dir.rglob("*.md")] == ["Sample_Book.md"]
