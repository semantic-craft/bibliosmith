"""Two PDFs whose safe_filename() collides must not share an output directory.

`safe_filename` is many-to-one, and every per-book path derives from it: the
output directory, the assets directory, the `.html`, `_state.json`, and the
chunk scratch under `.temp`. Two colliding books therefore shared a resume
state and a chunk directory, and the second book silently assembled the first
book's OCR results instead of running its own -- with no error anywhere.

Every remote call is stubbed, so these tests never touch the Baidu API.
"""

from __future__ import annotations

import importlib.util
import json
import logging
from pathlib import Path
import sys
from types import SimpleNamespace
from unittest import mock


PACKAGE_ROOT = Path(__file__).resolve().parents[1]
SCRIPTS = PACKAGE_ROOT / "scripts"
sys.path.insert(0, str(SCRIPTS))


def load_paddle_converter():  # type: ignore[no-untyped-def]
    module_name = "ocr_paddle_output_names_test"
    path = SCRIPTS / "pdf_to_html_paddleocr.py"
    spec = importlib.util.spec_from_file_location(module_name, path)
    if spec is None or spec.loader is None:
        raise RuntimeError("Cannot import PaddleOCR converter")
    module = importlib.util.module_from_spec(spec)
    sys.modules[module_name] = module
    spec.loader.exec_module(module)
    return module


paddle = load_paddle_converter()


# ---------------------------------------------------------------------------
# assign_output_names
# ---------------------------------------------------------------------------
def test_colliding_stems_get_distinct_names() -> None:
    files = [Path("Deep Learning.pdf"), Path("Deep_Learning.pdf")]

    names = paddle.assign_output_names(files)

    assert len(set(names.values())) == 2
    # The first source in sorted order keeps the plain name.
    assert names[files[0]] == "Deep_Learning"
    assert names[files[1]].startswith("Deep_Learning_")


def test_names_without_a_collision_are_untouched() -> None:
    """Existing books must keep their directory, or their resume state is lost."""
    files = [Path("Alpha.pdf"), Path("Beta.pdf"), Path("Gamma Delta.pdf")]

    names = paddle.assign_output_names(files)

    assert names == {
        files[0]: "Alpha",
        files[1]: "Beta",
        files[2]: "Gamma_Delta",
    }


def test_suffix_is_derived_from_the_stem_not_the_position() -> None:
    """Adding an unrelated book must not renumber an already-suffixed one."""
    pair = [Path("Deep Learning.pdf"), Path("Deep_Learning.pdf")]
    grown = [Path("Deep Learning.pdf"), Path("Deep_Learning.pdf"), Path("Zeta.pdf")]

    suffixed = paddle.assign_output_names(pair)[pair[1]]
    assert suffixed.startswith("Deep_Learning_")
    assert suffixed == paddle.assign_output_names(grown)[grown[1]]


def test_a_real_file_named_like_the_suffix_does_not_collide() -> None:
    colliding = [Path("Deep Learning.pdf"), Path("Deep_Learning.pdf")]
    suffixed = paddle.assign_output_names(colliding)[colliding[1]]

    files = [*colliding, Path(f"{suffixed}.pdf")]
    names = paddle.assign_output_names(files)

    assert len(set(names.values())) == 3


# ---------------------------------------------------------------------------
# End-to-end through main()
# ---------------------------------------------------------------------------
CONTENT = {
    "Deep Learning": "AAA content that belongs to the spaced book",
    "Deep_Learning": "BBB content that belongs to the underscored book",
}


def jsonl_for(stem: str) -> str:
    return json.dumps(
        {"result": {"layoutParsingResults": [{"markdown": {"text": CONTENT[stem], "images": {}}}]}}
    )


class FakeOCRClient:
    """Returns the OCR text of whichever book actually submitted the chunk."""

    def __init__(self, config) -> None:  # type: ignore[no-untyped-def]
        self.stem: str | None = None

    def submit_job(self, chunk_path: Path, batch_id: str) -> str:
        self.stem = CHUNK_OWNER[chunk_path]
        return f"job-{batch_id}"

    def poll_json_url(self, job_id: str, deadline: float, on_progress=None) -> str:  # type: ignore[no-untyped-def]
        return "https://example.invalid/result.jsonl"

    def download_jsonl(self, json_url: str) -> str:
        assert self.stem is not None
        return jsonl_for(self.stem)


CHUNK_OWNER: dict[Path, str] = {}


def fake_chunk_specs(source: Path, pages, chunk_dir: Path, max_bytes):  # type: ignore[no-untyped-def]
    chunk_path = chunk_dir / f"pages-{pages[0]:04d}-{pages[-1]:04d}.pdf"
    chunk_path.write_bytes(b"%PDF chunk")
    CHUNK_OWNER[chunk_path] = source.stem
    return [(list(pages), chunk_path)]


def run_main(tmp_path: Path, *, workers: int = 1, extra_argv: list[str] | None = None) -> Path:
    CHUNK_OWNER.clear()
    input_dir = tmp_path / "input"
    input_dir.mkdir(parents=True, exist_ok=True)
    (input_dir / "Deep Learning.pdf").write_bytes(b"%PDF A")
    (input_dir / "Deep_Learning.pdf").write_bytes(b"%PDF B")
    output_dir = tmp_path / "output"

    argv = [
        "pdf_to_html_paddleocr.py",
        "--input-dir",
        str(input_dir),
        "--output-dir",
        str(output_dir),
        *(extra_argv or []),
    ]
    with (
        mock.patch.object(
            paddle, "load_config", return_value=SimpleNamespace(workers=workers, max_upload_bytes=1 << 30)
        ),
        mock.patch.object(paddle, "pdf_page_count", return_value=2),
        mock.patch.object(paddle, "make_chunk_specs", side_effect=fake_chunk_specs),
        mock.patch.object(paddle, "BaiduOCRClient", side_effect=FakeOCRClient),
        mock.patch.object(paddle.OperationProgress, "from_environment", return_value=mock.Mock()),
        mock.patch.object(sys, "argv", argv),
    ):
        assert paddle.main() == 0
    return output_dir


def book_dirs(output_dir: Path) -> list[str]:
    return sorted(p.name for p in output_dir.iterdir() if p.is_dir() and p.name != ".temp")


def test_colliding_books_each_keep_their_own_content(tmp_path: Path) -> None:
    output_dir = run_main(tmp_path)

    dirs = book_dirs(output_dir)
    assert len(dirs) == 2, f"each book needs its own directory, got {dirs}"

    bodies = {d: (output_dir / d / f"{d}.html").read_text(encoding="utf-8") for d in dirs}
    joined = "\n".join(bodies.values())
    assert CONTENT["Deep Learning"] in joined
    assert CONTENT["Deep_Learning"] in joined
    # The decisive check: no book carries the other book's OCR text.
    for name, body in bodies.items():
        assert not (
            CONTENT["Deep Learning"] in body and CONTENT["Deep_Learning"] in body
        ), f"{name} mixes both books"


def test_colliding_books_do_not_share_resume_state_or_chunks(tmp_path: Path) -> None:
    output_dir = run_main(tmp_path)

    dirs = book_dirs(output_dir)
    assert len(dirs) == 2, f"each book needs its own directory, got {dirs}"
    for d in dirs:
        assert (output_dir / d / "_state.json").is_file(), f"{d} has no resume state"
    chunk_roots = sorted(p.name for p in (output_dir / ".temp").iterdir() if p.is_dir())
    assert len(chunk_roots) == 2, f"each book needs its own chunk directory, got {chunk_roots}"
    assert chunk_roots == dirs


def test_thread_pool_run_also_separates_the_books(tmp_path: Path) -> None:
    output_dir = run_main(tmp_path, workers=2)

    assert len(book_dirs(output_dir)) == 2


def test_collision_is_reported(tmp_path: Path, caplog) -> None:  # type: ignore[no-untyped-def]
    with caplog.at_level(logging.WARNING):
        run_main(tmp_path)

    messages = [record.getMessage() for record in caplog.records]
    assert any(
        "Deep_Learning.pdf" in message and "collides" in message for message in messages
    ), f"the renamed book must be reported, got {messages}"


def test_filtering_does_not_move_a_book_to_another_directory(tmp_path: Path) -> None:
    """--book must not hand the suffixed book the plain directory name."""
    full = book_dirs(run_main(tmp_path))

    filtered_dir = run_main(tmp_path / "filtered", extra_argv=["--book", "Deep_Learning.pdf"])

    assert book_dirs(filtered_dir) == [name for name in full if name.startswith("Deep_Learning_")]
