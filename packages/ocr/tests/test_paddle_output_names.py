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
import unicodedata
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
    """Adding an unrelated book must not renumber an already-suffixed one.

    The added book sorts *before* the colliding pair, so it shifts their index.
    A suffix derived from position would change; one derived from the stem
    cannot.
    """
    pair = [Path("Deep Learning.pdf"), Path("Deep_Learning.pdf")]
    grown = [Path("AAA First.pdf"), Path("Deep Learning.pdf"), Path("Deep_Learning.pdf")]

    suffixed = paddle.assign_output_names(pair)[pair[1]]
    assert suffixed.startswith("Deep_Learning_")
    assert suffixed == paddle.assign_output_names(grown)[grown[2]]


def folded(name: str) -> str:
    """What the filesystem sees. Deliberately independent of the module under test."""
    return unicodedata.normalize("NFC", name).casefold()


def test_three_colliding_stems_all_get_distinct_names() -> None:
    files = [Path("Deep Learning.pdf"), Path("Deep_Learning.pdf"), Path("Deep:Learning.pdf")]

    names = paddle.assign_output_names(files)

    assert len({folded(name) for name in names.values()}) == 3, names
    assert names[files[0]] == "Deep_Learning"


def test_removing_a_colliding_sibling_does_not_rename_the_rest() -> None:
    """Pins the digest to each book's own stem.

    A digest taken from the shared base still yields distinct names — the
    widening loop rescues that — but it makes every suffix depend on how many
    siblings sort ahead, so deleting one book renames another.
    """
    three = [Path("Deep Learning.pdf"), Path("Deep_Learning.pdf"), Path("Deep:Learning.pdf")]
    without_middle = [three[0], three[2]]

    assert (
        paddle.assign_output_names(three)[three[2]]
        == paddle.assign_output_names(without_middle)[three[2]]
    )


def test_names_differing_only_by_case_are_treated_as_one_directory() -> None:
    """APFS and NTFS fold case, so these are the same path despite differing."""
    files = [Path("Deep Learning.pdf"), Path("deep_learning.pdf")]

    names = paddle.assign_output_names(files)

    keys = {folded(name) for name in names.values()}
    assert len(keys) == 2, f"names collide in the filesystem namespace: {names}"


def test_names_differing_only_by_unicode_normalization_collide() -> None:
    """APFS folds NFC/NFD, so the composed and decomposed stems are one path."""
    composed = "Caf\u00e9 Notes.pdf"        # e-acute as one codepoint
    decomposed = "Cafe\u0301_Notes.pdf"     # e + combining acute
    assert composed != decomposed
    files = [Path(composed), Path(decomposed)]

    names = paddle.assign_output_names(files)

    assert len(set(names.values())) == 2
    keys = {folded(name) for name in names.values()}
    assert len(keys) == 2, f"names collide in the filesystem namespace: {names}"


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
        # baidu_token: these books route to remote OCR (the fixture bytes are
        # not a readable PDF, so the text-layer sample finds nothing), and since
        # #137 main() refuses that route without a credential.
        mock.patch.object(
            paddle,
            "load_config",
            return_value=SimpleNamespace(
                workers=workers,
                max_upload_bytes=1 << 30,
                baidu_token="token",
                baidu_model="PaddleOCR-VL-1.6",
            ),
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


def test_a_book_never_resumes_from_another_book_s_state(tmp_path: Path) -> None:
    """The directory alone must not be enough to inherit a finished run.

    Which book keeps the plain name depends on sorted order, so a PDF added
    later can land on a directory another book already converted. Chunks are
    named by page range only, so an inherited `chunks_done` would make the
    newcomer skip OCR entirely and assemble the previous book's text under its
    own title.
    """
    input_dir = tmp_path / "input"
    input_dir.mkdir(parents=True)
    output_dir = tmp_path / "output"

    # A finished run for a different book, sitting where the newcomer will land.
    book_dir = output_dir / "Deep_Learning"
    book_dir.mkdir(parents=True)
    (book_dir / "_state.json").write_text(
        json.dumps(
            {
                "source_name": "Some Other Book.pdf",
                "chunks_done": ["pages-0001-0002.pdf"],
                "pages_total": 2,
                "pages_done": 2,
            }
        ),
        encoding="utf-8",
    )
    stale_chunks = output_dir / ".temp" / "Deep_Learning" / "chunks"
    stale_chunks.mkdir(parents=True)
    (stale_chunks / "pages-0001-0002.jsonl").write_text(
        jsonl_for("Deep_Learning"), encoding="utf-8"
    )

    (input_dir / "Deep Learning.pdf").write_bytes(b"%PDF A")
    CHUNK_OWNER.clear()
    with (
        mock.patch.object(
            paddle,
            "load_config",
            return_value=SimpleNamespace(
                workers=1,
                max_upload_bytes=1 << 30,
                baidu_token="token",
                baidu_model="PaddleOCR-VL-1.6",
            ),
        ),
        mock.patch.object(paddle, "pdf_page_count", return_value=2),
        mock.patch.object(paddle, "make_chunk_specs", side_effect=fake_chunk_specs),
        mock.patch.object(paddle, "BaiduOCRClient", side_effect=FakeOCRClient),
        mock.patch.object(paddle.OperationProgress, "from_environment", return_value=mock.Mock()),
        mock.patch.object(
            sys,
            "argv",
            [
                "pdf_to_html_paddleocr.py",
                "--input-dir",
                str(input_dir),
                "--output-dir",
                str(output_dir),
            ],
        ),
    ):
        assert paddle.main() == 0

    body = (book_dir / "Deep_Learning.html").read_text(encoding="utf-8")
    assert CONTENT["Deep Learning"] in body, "the book must OCR its own pages"
    assert CONTENT["Deep_Learning"] not in body, "inherited another book's OCR text"
    owner = json.loads((book_dir / "_state.json").read_text(encoding="utf-8"))["source_name"]
    assert owner == "Deep Learning.pdf"


def test_state_written_before_this_field_existed_still_resumes(tmp_path: Path) -> None:
    """Older state has no owner recorded; discarding it would re-bill the OCR."""
    input_dir = tmp_path / "input"
    input_dir.mkdir(parents=True)
    output_dir = tmp_path / "output"
    book_dir = output_dir / "Deep_Learning"
    book_dir.mkdir(parents=True)
    (book_dir / "_state.json").write_text(
        json.dumps({"chunks_done": ["pages-0001-0002.pdf"], "pages_total": 2, "pages_done": 2}),
        encoding="utf-8",
    )
    chunks = output_dir / ".temp" / "Deep_Learning" / "chunks"
    chunks.mkdir(parents=True)
    (chunks / "pages-0001-0002.jsonl").write_text(jsonl_for("Deep Learning"), encoding="utf-8")

    (input_dir / "Deep Learning.pdf").write_bytes(b"%PDF A")
    CHUNK_OWNER.clear()
    clients: list[FakeOCRClient] = []

    def track(config):  # type: ignore[no-untyped-def]
        client = FakeOCRClient(config)
        clients.append(client)
        return client

    with (
        mock.patch.object(
            paddle,
            "load_config",
            return_value=SimpleNamespace(
                workers=1,
                max_upload_bytes=1 << 30,
                baidu_token="token",
                baidu_model="PaddleOCR-VL-1.6",
            ),
        ),
        mock.patch.object(paddle, "pdf_page_count", return_value=2),
        mock.patch.object(paddle, "make_chunk_specs", side_effect=fake_chunk_specs),
        mock.patch.object(paddle, "BaiduOCRClient", side_effect=track),
        mock.patch.object(paddle.OperationProgress, "from_environment", return_value=mock.Mock()),
        mock.patch.object(
            sys,
            "argv",
            [
                "pdf_to_html_paddleocr.py",
                "--input-dir",
                str(input_dir),
                "--output-dir",
                str(output_dir),
            ],
        ),
    ):
        assert paddle.main() == 0

    assert clients and clients[0].stem is None, "ownerless state must still resume"


def test_filtering_does_not_move_a_book_to_another_directory(tmp_path: Path) -> None:
    """--book must not hand the suffixed book the plain directory name."""
    full = book_dirs(run_main(tmp_path))

    filtered_dir = run_main(tmp_path / "filtered", extra_argv=["--book", "Deep_Learning.pdf"])

    assert book_dirs(filtered_dir) == [name for name in full if name.startswith("Deep_Learning_")]
