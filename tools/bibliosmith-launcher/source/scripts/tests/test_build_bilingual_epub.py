from __future__ import annotations

import importlib.util
import base64
import json
import subprocess
import zipfile
from pathlib import Path

import pytest


SCRIPT_PATH = Path(__file__).parents[1] / "build_bilingual_epub.py"
SPEC = importlib.util.spec_from_file_location("build_bilingual_epub", SCRIPT_PATH)
assert SPEC and SPEC.loader
BUILDER = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(BUILDER)


def write_fixture(root: Path, source: str, target: str) -> None:
    target_heading = next(
        (line.removeprefix("# ").strip() for line in target.splitlines() if line.startswith("# ")),
        "第一章",
    )
    if target_heading.lower().startswith(("chapter_", "unit_", "continuation")):
        target_heading = "第一章"
    (root / "chapters/src").mkdir(parents=True)
    (root / "chapters/final").mkdir(parents=True)
    (root / "metadata").mkdir(parents=True)
    (root / "source").mkdir(parents=True, exist_ok=True)
    (root / "source/source.md").write_text(
        "\n".join("fixture" for _ in range(100)) + "\n", encoding="utf-8"
    )
    (root / "chapters/src/chapter_001.md").write_text(source, encoding="utf-8")
    (root / "chapters/final/chapter_001.md").write_text(target, encoding="utf-8")
    (root / "metadata/source_map.json").write_text(
        json.dumps(
            {
                "schema": "local-reading-source-map-v2",
                "translationUnits": [
                    {
                        "id": "chapter_001",
                        "publicationSectionId": "section_001",
                        "sourceUnitPath": "chapters/src/chapter_001.md",
                        "sourceStartLine": 1,
                        "sourceEndLine": 100,
                    }
                ]
            }
        ),
        encoding="utf-8",
    )
    (root / "metadata/publication_map.json").write_text(
        json.dumps(
            {
                "schema": "local-reading-publication-map-v1",
                "audit": {"status": "passed", "source": "fixture", "confidence": 1},
                "sections": [
                    {
                        "id": "section_001",
                        "ordinal": 1,
                        "title": "Chapter",
                        "shortTitle": "Chapter",
                        "readerTitle": target_heading,
                        "readerShortTitle": target_heading,
                        "headingLevel": 1,
                        "parentId": None,
                        "role": "bodymatter",
                        "kind": "chapter",
                        "sourceStartLine": 1,
                        "sourceEndLine": 100,
                    }
                ],
                "notes": [],
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
    (root / "metadata/book.yaml").write_text(
        "title: Bilingual Fixture\nauthor: Fixture Author\npublisher: Fixture Press\n"
        "date: 2026\nlanguage: zh-Hans\n",
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
    chapter = epub_member(epub_path, "EPUB/section_001.xhtml")
    body = chapter.split("<body>", 1)[1]

    assert body.index("Chapter") < body.index("第一章")
    assert body.index("Source one.") < body.index("译文一。")
    assert body.index("译文一。") < body.index("Source two.")
    assert chapter.count('class="bitext-unit"') == 3
    package = epub_member(epub_path, "EPUB/package.opf")
    assert "<dc:language>en</dc:language>" in package
    assert "<dc:language>zh-Hans</dc:language>" in package
    assert "<dc:title>Bilingual Fixture</dc:title>" in package
    assert "<dc:creator>Fixture Author</dc:creator>" in package
    assert "<dc:publisher>Fixture Press</dc:publisher>" in package
    assert "<dc:date>2026</dc:date>" in package
    assert "Unknown" not in package
    with zipfile.ZipFile(epub_path) as archive:
        assert archive.getinfo("mimetype").compress_type == zipfile.ZIP_STORED


def test_bilingual_builder_packages_a_configured_cover(tmp_path: Path) -> None:
    (tmp_path / "source").mkdir(exist_ok=True)
    (tmp_path / "source/cover.png").write_bytes(
        base64.b64decode(
            "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mNk+A8AAQUBAScY42YAAAAASUVORK5CYII="
        )
    )
    write_fixture(tmp_path, "# Chapter\n\nSource.\n", "# 第一章\n\n译文。\n")
    with (tmp_path / "metadata/book.yaml").open("a", encoding="utf-8") as stream:
        stream.write("cover: source/cover.png\n")

    epub_path = BUILDER.build_book(tmp_path)

    package = epub_member(epub_path, "EPUB/package.opf")
    nav = epub_member(epub_path, "EPUB/nav.xhtml")
    cover = epub_member(epub_path, "EPUB/cover.xhtml")
    with zipfile.ZipFile(epub_path) as archive:
        assert "EPUB/images/cover.png" in archive.namelist()
    assert 'properties="cover-image"' in package
    assert '<itemref idref="cover-page"' in package
    assert 'epub:type="cover" href="cover.xhtml"' in nav
    assert 'epub:type="cover" class="publication-cover"' in cover


def test_standard_and_bilingual_builders_share_structure_validation(tmp_path: Path) -> None:
    write_fixture(tmp_path, "# Chapter\n\nSource.\n", "# 第一章\n\n译文。\n")
    publication_path = tmp_path / "metadata/publication_map.json"
    publication = json.loads(publication_path.read_text(encoding="utf-8"))
    publication["sections"].append({**publication["sections"][0]})
    publication_path.write_text(json.dumps(publication), encoding="utf-8")

    standard = subprocess.run(
        ["node", str(SCRIPT_PATH.with_name("build_epub.cjs"))],
        cwd=tmp_path,
        capture_output=True,
        text=True,
        check=False,
    )
    with pytest.raises(ValueError, match="Duplicate publication section ID"):
        BUILDER.build_book(tmp_path)

    assert standard.returncode != 0
    assert "Duplicate publication section ID" in standard.stderr


@pytest.mark.parametrize("failure", ["invalid_id", "source_upper_bound"])
def test_standard_and_bilingual_builders_share_strict_structure_bounds(
    tmp_path: Path, failure: str
) -> None:
    write_fixture(tmp_path, "# Chapter\n\nSource.\n", "# 第一章\n\n译文。\n")
    publication_path = tmp_path / "metadata/publication_map.json"
    publication = json.loads(publication_path.read_text(encoding="utf-8"))
    if failure == "invalid_id":
        publication["sections"][0]["id"] = "../escape"
    else:
        publication["sections"][0]["sourceEndLine"] = 101
    publication_path.write_text(json.dumps(publication), encoding="utf-8")

    standard = subprocess.run(
        ["node", str(SCRIPT_PATH.with_name("build_epub.cjs"))],
        cwd=tmp_path,
        capture_output=True,
        text=True,
        check=False,
    )
    expected = "canonical ID" if failure == "invalid_id" else "invalid source range"
    with pytest.raises(ValueError, match=expected):
        BUILDER.build_book(tmp_path)

    assert standard.returncode != 0
    assert expected in standard.stderr


def test_bilingual_builder_rejects_internal_translated_heading(tmp_path: Path) -> None:
    write_fixture(tmp_path, "# Chapter\n\nSource.\n", "# chapter_001\n\n译文。\n")

    with pytest.raises(ValueError, match="Translated publication title exposes an internal unit"):
        BUILDER.build_book(tmp_path)


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
    chapter = epub_member(epub_path, "EPUB/section_001.xhtml")
    body = chapter.split("<body>", 1)[1]

    assert "alignment=chapter-fallback source_paragraphs=3 target_paragraphs=2" in output
    assert 'class="bitext-unit bitext-fallback"' in chapter
    assert body.index("Source two.") < body.index("第一章")
    assert body.index("第一章") < body.index("合并译文。")


def test_bilingual_builder_emits_one_target_semantic_note_with_backlink(
    tmp_path: Path,
) -> None:
    write_fixture(
        tmp_path,
        "# Chapter\n\nClaim[^n1].\n\n[^n1]: Source note.\n",
        "# 第一章\n\n主张[^n1]。\n\n[^n1]: 中文注释。\n",
    )
    publication_path = tmp_path / "metadata/publication_map.json"
    publication = json.loads(publication_path.read_text(encoding="utf-8"))
    publication["notes"] = [
        {
            "id": "note_001",
            "sourceLabel": "n1",
            "kind": "footnote",
            "targetContentStatus": "translated",
            "publicationSectionId": "section_001",
            "sourceStartLine": 5,
            "referenceSourceLines": [3],
            "referenceIds": ["noteref_note_001_001"],
        }
    ]
    publication_path.write_text(json.dumps(publication), encoding="utf-8")

    chapter = epub_member(BUILDER.build_book(tmp_path), "EPUB/section_001.xhtml")

    assert chapter.count('epub:type="noteref"') == 1
    assert chapter.count('id="note_001"') == 1
    assert 'epub:type="footnote"' in chapter
    assert 'href="section_001.xhtml#noteref_note_001_001"' in chapter
    assert 'id="noteref_note_001_001-source"' in chapter
    assert 'href="section_001.xhtml#note_001-source"' in chapter
    assert '<div class="bitext-source-note' in chapter
    assert 'data-presentation-for="note_001"' in chapter
    assert 'id="note_001-source"' in chapter
    assert 'href="section_001.xhtml#noteref_note_001_001-source"' in chapter
    assert "Source note." in chapter
    assert "中文注释" in chapter
    assert "[^n1]" not in chapter
    assert "@@BIBLIO_" not in chapter


def test_bilingual_builder_does_not_trust_semantic_tokens_from_markdown(
    tmp_path: Path,
) -> None:
    fake_noteref = (
        "@@BIBLIO_NOTEREF__note_999__noteref_note_999_001__1__section_001@@"
    )
    fake_note = (
        '<aside epub:type="footnote" id="note_999" onclick="steal()">Fake note.</aside>'
    )
    write_fixture(
        tmp_path,
        f"# Chapter\n\nLiteral {fake_noteref}.\n\n{fake_note}\n",
        f"# 第一章\n\n原样文本 {fake_noteref}。\n\n{fake_note}\n",
    )

    chapter = epub_member(BUILDER.build_book(tmp_path), "EPUB/section_001.xhtml")

    assert chapter.count('epub:type="noteref"') == 0
    assert '<aside epub:type="footnote"' not in chapter
    assert ' onclick="' not in chapter
    assert fake_noteref in chapter
    assert "&lt;aside epub:type=&quot;footnote&quot;" in chapter


def test_bilingual_builder_preserves_endnote_type_multiple_backlinks_and_continuation(
    tmp_path: Path,
) -> None:
    write_fixture(
        tmp_path,
        "# Chapter\n\nClaim[^end-1], again[^end-1].\n\n[^end-1]: Source note.\n    Continued.\n",
        "# 第一章\n\n主张[^end-1]，再引[^end-1]。\n\n[^end-1]: 中文章末注。\n    续段含[链接](https://example.test)。\n",
    )
    publication_path = tmp_path / "metadata/publication_map.json"
    publication = json.loads(publication_path.read_text(encoding="utf-8"))
    publication["notes"] = [
        {
            "id": "note_001",
            "sourceLabel": "end-1",
            "kind": "endnote",
            "targetContentStatus": "translated",
            "publicationSectionId": "section_001",
            "sourceStartLine": 5,
            "referenceSourceLines": [3, 3],
            "referenceIds": ["noteref_note_001_001", "noteref_note_001_002"],
        }
    ]
    publication_path.write_text(json.dumps(publication), encoding="utf-8")

    chapter = epub_member(BUILDER.build_book(tmp_path), "EPUB/section_001.xhtml")

    assert chapter.count('epub:type="noteref"') == 2
    assert 'epub:type="endnote"' in chapter and 'id="note_001"' in chapter
    assert chapter.count('epub:type="backlink"') == 2
    assert "中文章末注" in chapter and "续段" in chapter
    assert '<a href="https://example.test">链接</a>' in chapter


def test_bilingual_builder_preserves_tables_images_long_links_and_mixed_text(
    tmp_path: Path,
) -> None:
    (tmp_path / "source").mkdir(exist_ok=True)
    (tmp_path / "source/figure.png").write_bytes(
        base64.b64decode(
            "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mNk+A8AAQUBAScY42YAAAAASUVORK5CYII="
        )
    )
    long_url = "https://example.test/" + "sehr-langer-pfad-" * 12
    write_fixture(
        tmp_path,
        "# Kapitel\n\n| Begriff | Wert |\n| --- | --- |\n| Geschäftsgeheimnis | 价值 |\n\n"
        "![Schaubild](../../source/figure.png)\n\n"
        f"Deutsch 中文 [Quelle]({long_url}).\n",
        "# 第一章\n\n| 术语 | Wert |\n| --- | --- |\n| 商业秘密 | 价值 |\n\n"
        "![图示](../../source/figure.png)\n\n"
        f"中文 Deutsch [来源]({long_url})。\n",
    )

    epub_path = BUILDER.build_book(tmp_path)
    chapter = epub_member(epub_path, "EPUB/section_001.xhtml")
    stylesheet = epub_member(epub_path, "EPUB/styles/book.css")
    package = epub_member(epub_path, "EPUB/package.opf")

    assert chapter.count("<table") == 2
    assert chapter.count("<img") == 2
    assert f'<a href="{long_url}">来源</a>' in chapter
    assert "Geschäftsgeheimnis" in chapter and "商业秘密" in chapter
    assert "overflow-x:auto" in stylesheet and "overflow-wrap:anywhere" in stylesheet
    with zipfile.ZipFile(epub_path) as archive:
        image_members = [name for name in archive.namelist() if name.startswith("EPUB/images/")]
        assert len(image_members) == 1
        assert image_members[0].removeprefix("EPUB/") in package


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

    chapter = epub_member(BUILDER.build_book(tmp_path), "EPUB/section_001.xhtml")

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

    chapter = epub_member(BUILDER.build_book(tmp_path), "EPUB/section_001.xhtml")

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


# --- Review findings on PR #126 ---------------------------------------------


def test_an_indented_opening_fence_has_its_indent_measured_before_trimming() -> None:
    # The whole-text strip used to eat the opener's indentation before it could
    # be measured, so the width came out as zero and the matching indent stayed
    # on every rendered code line.
    blocks = BUILDER.split_paragraphs("  ```\n  indented body\n  ```\n")

    assert len(blocks) == 1
    assert BUILDER.fenced_code(blocks[0]) == "indented body"


def test_blank_lines_before_the_closing_fence_are_part_of_the_code() -> None:
    blocks = BUILDER.split_paragraphs("```\nrow\n\n\n```\n")

    assert BUILDER.fenced_code(blocks[0]) == "row\n\n"


def test_leading_blank_lines_still_do_not_become_a_block() -> None:
    assert BUILDER.split_paragraphs("\n\n  \n\nonly paragraph\n\n") == ["only paragraph"]


# --- Page anchors (issue #108) ----------------------------------------------
# The PaddleOCR assembler writes `<!-- page: N -->` between pages and calls it
# invisible, but every block here goes through `inline_text`, which escapes it.
# Each anchor therefore reached the reader as a paragraph of literal
# `<!-- page: N -->`, once per page, in every book converted that way.

ANCHORED_SOURCE = "# Chapter\n\n<!-- page: 1 -->\n\nSource one.\n\n<!-- page: 2 -->\n\nSource two.\n"
ANCHORED_TARGET = "# 第一章\n\n<!-- page: 1 -->\n\n译文一。\n\n<!-- page: 2 -->\n\n译文二。\n"


def test_a_standalone_page_anchor_is_not_a_block() -> None:
    assert BUILDER.split_paragraphs(ANCHORED_SOURCE) == [
        "# Chapter",
        "Source one.",
        "Source two.",
    ]


def test_page_anchors_never_reach_the_reader(tmp_path: Path) -> None:
    write_fixture(tmp_path, ANCHORED_SOURCE, ANCHORED_TARGET)

    chapter = epub_member(BUILDER.build_book(tmp_path), "EPUB/section_001.xhtml")

    # Neither as the escaped text the reader used to see, nor as a real comment
    # smuggled through unescaped: the anchor is gone from the book entirely.
    assert "page:" not in chapter
    assert "&lt;!--" not in chapter
    assert "<!--" not in chapter.split("<body>", 1)[1]
    # The prose either side of the dropped anchors is untouched and still paired.
    assert "Source one." in chapter and "译文一。" in chapter


def test_dropping_an_anchor_keeps_the_two_sides_in_step(
    tmp_path: Path, capsys
) -> None:
    # The case a fix applied to one side alone would corrupt. The translator
    # dropped the anchors, so the source carries two blocks the target does not:
    # filtering only where the source is rendered would leave five blocks facing
    # three and cost the chapter its pairing, and filtering after pairing would
    # have married `Source one.` to `译文二。`.
    write_fixture(
        tmp_path, ANCHORED_SOURCE, "# 第一章\n\n译文一。\n\n译文二。\n"
    )

    epub_path = BUILDER.build_book(tmp_path)
    chapter = epub_member(epub_path, "EPUB/section_001.xhtml")
    body = chapter.split("<body>", 1)[1]

    assert (
        "alignment=paragraph source_paragraphs=3 target_paragraphs=3"
        in capsys.readouterr().out
    )
    assert chapter.count('class="bitext-unit"') == 3
    # Each source paragraph still sits with its own translation, in order.
    assert body.index("Source one.") < body.index("译文一。")
    assert body.index("译文一。") < body.index("Source two.")
    assert body.index("Source two.") < body.index("译文二。")


def test_a_comment_inside_a_paragraph_is_still_that_paragraph(tmp_path: Path) -> None:
    # The over-eager mutation of the fix: strip comments from a block rather
    # than drop blocks that are only comments, and this sentence loses its
    # middle -- or the whole paragraph goes, because what is left still parses.
    write_fixture(
        tmp_path,
        "# Chapter\n\nA sentence <!-- an aside --> carrying a comment.\n",
        "# 第一章\n\n一句带注释的话。\n",
    )

    chapter = epub_member(BUILDER.build_book(tmp_path), "EPUB/section_001.xhtml")

    assert "A sentence &lt;!-- an aside --&gt; carrying a comment." in chapter


def test_a_fenced_comment_is_a_code_sample_not_an_anchor() -> None:
    # A book about this pipeline quotes the anchor in a listing. Fences skip the
    # comment filter, so the sample survives with its delimiters intact.
    blocks = BUILDER.split_paragraphs("```html\n<!-- page: 42 -->\n```\n")

    assert len(blocks) == 1
    assert BUILDER.fenced_code(blocks[0]) == "<!-- page: 42 -->"


def test_a_multi_line_or_repeated_comment_block_also_goes() -> None:
    assert BUILDER.is_comment_only("<!-- page: 1 -->\n<!-- page: 2 -->")
    assert BUILDER.is_comment_only("<!--\n  page: 1\n-->")
    assert not BUILDER.is_comment_only("<!-- page: 1 --> and then prose")
    assert not BUILDER.is_comment_only("prose <!-- page: 1 -->")
    assert not BUILDER.is_comment_only("<!-- an unterminated comment")
    # The false positive a `startswith("<!--") and endswith("-->")` test would
    # produce: a real sentence between two comments, silently deleted.
    assert not BUILDER.is_comment_only("<!-- page: 1 --> a real sentence <!-- x -->")


def test_blank_lines_inside_a_comment_do_not_split_it_into_visible_blocks() -> None:
    assert BUILDER.split_paragraphs(
        "<!--\n\npage note\n-->\n\nReadable paragraph.\n"
    ) == ["Readable paragraph."]


def test_an_unclosed_comment_cannot_swallow_prose_before_a_later_comment() -> None:
    block = "<!-- broken\nreal prose\n<!-- page: 1 -->"

    assert not BUILDER.is_comment_only(block)
    assert BUILDER.split_paragraphs(block) == [block]


def test_an_unclosed_comment_does_not_swallow_the_rest_of_the_chapter() -> None:
    text = (
        "<!-- no close\n\nLater paragraph.\n\n## Later heading\n\n"
        "```html\n<div>Sample</div>\n```\n"
    )

    assert BUILDER.split_paragraphs(text) == [
        "<!-- no close",
        "Later paragraph.",
        "## Later heading",
        "```html\n<div>Sample</div>\n```",
    ]


def test_a_comment_sample_in_a_fence_cannot_close_an_earlier_opener() -> None:
    text = (
        "<!-- no close\n\nLater paragraph.\n\n"
        "```html\n<!-- code sample -->\n```\n\nAfter fence.\n"
    )

    assert BUILDER.split_paragraphs(text) == [
        "<!-- no close",
        "Later paragraph.",
        "```html\n<!-- code sample -->\n```",
        "After fence.",
    ]
