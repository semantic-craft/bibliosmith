from __future__ import annotations

import importlib.util
import json
import zipfile
from pathlib import Path


SCRIPT_PATH = Path(__file__).parents[1] / "build_bilingual_epub.py"
SPEC = importlib.util.spec_from_file_location("build_bilingual_epub", SCRIPT_PATH)
assert SPEC and SPEC.loader
BUILDER = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(BUILDER)


def write_fixture(root: Path, source: str, target: str) -> None:
    (root / "chapters/src").mkdir(parents=True)
    (root / "chapters/final").mkdir(parents=True)
    (root / "metadata").mkdir(parents=True)
    (root / "chapters/src/chapter_001.md").write_text(source, encoding="utf-8")
    (root / "chapters/final/chapter_001.md").write_text(target, encoding="utf-8")
    (root / "metadata/source_map.json").write_text(
        json.dumps(
            {
                "chapters": [
                    {
                        "id": "chapter_001",
                        "chapterSourcePath": "chapters/src/chapter_001.md",
                    }
                ]
            }
        ),
        encoding="utf-8",
    )
    (root / "metadata/source_manifest.json").write_text(
        json.dumps(
            {
                "source_file_name": "Bilingual Fixture.epub",
                "source_language": "en",
                "target_language": "zh-Hans",
            }
        ),
        encoding="utf-8",
    )


def epub_member(path: Path, member: str) -> str:
    with zipfile.ZipFile(path) as archive:
        return archive.read(member).decode("utf-8")


def test_split_paragraphs_uses_blank_lines() -> None:
    assert BUILDER.split_paragraphs("one\ncontinued\n\n two \n\n\nthree\n") == [
        "one\ncontinued",
        "two",
        "three",
    ]


def test_builder_interleaves_equal_paragraphs(tmp_path: Path) -> None:
    write_fixture(
        tmp_path,
        "# Chapter\n\nSource one.\n\nSource two.\n",
        "# 第一章\n\n译文一。\n\n译文二。\n",
    )

    epub_path = BUILDER.build_book(tmp_path)
    assert epub_path == tmp_path / "output/reading/book_bilingual.epub"
    chapter = epub_member(epub_path, "EPUB/chapter_001.xhtml")
    body = chapter.split("<body>", 1)[1]

    assert body.index("Chapter") < body.index("第一章")
    assert body.index("Source one.") < body.index("译文一。")
    assert body.index("译文一。") < body.index("Source two.")
    assert chapter.count('class="bitext-unit"') == 3
    package = epub_member(epub_path, "EPUB/package.opf")
    assert "<dc:language>en</dc:language>" in package
    assert "<dc:language>zh-Hans</dc:language>" in package
    assert "<dc:title>Bilingual Fixture</dc:title>" in package
    assert "Unknown" not in package
    with zipfile.ZipFile(epub_path) as archive:
        assert archive.getinfo("mimetype").compress_type == zipfile.ZIP_STORED


def test_builder_falls_back_to_whole_chapter_when_counts_differ(
    tmp_path: Path, capsys
) -> None:
    write_fixture(
        tmp_path,
        "# Chapter\n\nSource one.\n\nSource two.\n",
        "# 第一章\n\n合并译文。\n",
    )

    epub_path = BUILDER.build_book(tmp_path)
    output = capsys.readouterr().out
    chapter = epub_member(epub_path, "EPUB/chapter_001.xhtml")
    body = chapter.split("<body>", 1)[1]

    assert "alignment=chapter-fallback source_paragraphs=3 target_paragraphs=2" in output
    assert 'class="bitext-unit bitext-fallback"' in chapter
    assert body.index("Source two.") < body.index("第一章")
    assert body.index("第一章") < body.index("合并译文。")


# --- Fenced code blocks (issue #125) ----------------------------------------
# The builder split on blank lines only, so a fence holding one was torn into
# two blocks and each half rendered as a paragraph with the ``` delimiters left
# in the prose. The torn halves also inflated the block count, and this file's
# pairing is positional: an inflated count on one side drops the whole chapter
# to chapter-level fallback and loses paragraph pairing everywhere in it.

FENCED_SOURCE = (
    "# Chapter\n\nBefore the code.\n\n"
    '```python\ndef a():\n    pass\n\ndef b():\n    return "# not a heading"\n```\n\n'
    "After the code.\n"
)
FENCED_TARGET = (
    "# 第一章\n\n代码之前。\n\n"
    '```python\ndef a():\n    pass\n\ndef b():\n    return "# not a heading"\n```\n\n'
    "代码之后。\n"
)


def test_a_fence_holding_a_blank_line_stays_one_block() -> None:
    blocks = BUILDER.split_paragraphs(FENCED_SOURCE)

    assert len(blocks) == 4
    assert blocks[2].startswith("```python")
    assert blocks[2].endswith("```")
    assert "def b():" in blocks[2]


def test_a_fenced_block_keeps_paragraph_alignment(tmp_path: Path, capsys) -> None:
    # The decisive check: without the fence-aware split this chapter counted
    # five blocks a side and, more to the point, any divergence between the two
    # sides would have cost the whole chapter its paragraph pairing.
    write_fixture(tmp_path, FENCED_SOURCE, FENCED_TARGET)

    BUILDER.build_book(tmp_path)

    assert (
        "alignment=paragraph source_paragraphs=4 target_paragraphs=4"
        in capsys.readouterr().out
    )


def test_a_fenced_block_renders_as_code_on_both_sides(tmp_path: Path) -> None:
    write_fixture(tmp_path, FENCED_SOURCE, FENCED_TARGET)

    chapter = epub_member(BUILDER.build_book(tmp_path), "EPUB/chapter_001.xhtml")

    assert '<pre class="bitext-source" lang="en" xml:lang="en"><code>def a():' in chapter
    assert '<pre class="bitext-target" lang="zh-Hans" xml:lang="zh-Hans"><code>def a():' in chapter
    # Line breaks and the blank line inside the block survive.
    assert "def a():\n    pass\n\ndef b():" in chapter
    # The delimiters are structure, not prose.
    assert "```" not in chapter


def test_a_comment_after_a_blank_line_in_a_fence_is_not_promoted_to_a_heading(
    tmp_path: Path,
) -> None:
    # The structural half of the bug. Splitting on the blank line left a block
    # that *began* with `# `, and HEADING.fullmatch is DOTALL — so the comment
    # became a real <h1> carrying the trailing ``` into the chapter navigation.
    commented = "# Chapter\n\n```\nsetup()\n\n# a comment line\nteardown()\n```\n"
    write_fixture(tmp_path, commented, commented)

    chapter = epub_member(BUILDER.build_book(tmp_path), "EPUB/chapter_001.xhtml")

    assert "# a comment line" in chapter
    assert "<h1" not in chapter.split("</h1>", 2)[-1]
    # Only the real chapter heading, once per language.
    assert chapter.count("<h1") == 2


def test_tilde_and_nested_fences_are_recognised() -> None:
    tildes = BUILDER.split_paragraphs("~~~\nplain\n\nblock\n~~~\n")
    assert len(tildes) == 1
    assert BUILDER.fenced_code(tildes[0]) == "plain\n\nblock"

    nested = BUILDER.split_paragraphs("````\n```\ninner\n```\n````\n")
    assert len(nested) == 1
    assert BUILDER.fenced_code(nested[0]) == "```\ninner\n```"


def test_an_unclosed_fence_runs_to_the_end_of_the_chapter() -> None:
    blocks = BUILDER.split_paragraphs("Before.\n\n```\nno closing fence\n\nstill code\n")

    assert len(blocks) == 2
    assert BUILDER.fenced_code(blocks[1]) == "no closing fence\n\nstill code"


def test_an_inline_backtick_span_is_not_a_fence() -> None:
    blocks = BUILDER.split_paragraphs("``a `b` c`` is an inline span.\n")

    assert len(blocks) == 1
    assert BUILDER.fenced_code(blocks[0]) is None


def test_the_stylesheet_wraps_code_and_drops_the_prose_indent(tmp_path: Path) -> None:
    # An e-reader page cannot scroll sideways, and the bitext classes carry a
    # first-line indent that must not apply to code.
    write_fixture(tmp_path, FENCED_SOURCE, FENCED_TARGET)

    stylesheet = epub_member(BUILDER.build_book(tmp_path), "EPUB/styles/book.css")

    assert "white-space:pre-wrap" in stylesheet
    assert "pre.bitext-target" in stylesheet
