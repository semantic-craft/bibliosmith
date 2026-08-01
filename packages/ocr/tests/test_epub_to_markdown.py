"""Extractor behaviour for the `epub_source` route.

The fixtures are real EPUB archives built here rather than checked-in binaries:
a hand-written zip keeps the structure under test visible in the diff, and the
extractor reads them through the same `container.xml` -> OPF -> spine path a
shipped book takes.
"""

from __future__ import annotations

import sys
import tempfile
from pathlib import Path
from zipfile import ZIP_DEFLATED, ZipFile

SCRIPTS = Path(__file__).resolve().parents[1] / "scripts"
sys.path.insert(0, str(SCRIPTS))

from epub_to_markdown import EpubExtractError, extract_book  # noqa: E402


# A 1x1 PNG, so the sidecar copy moves real bytes.
PNG = bytes.fromhex(
    "89504e470d0a1a0a0000000d49484452000000010000000108060000001f15c4"
    "890000000a49444154789c6360000002000100ffff03000006000557bfabd400"
    "00000049454e44ae426082"
)

CONTAINER = """<?xml version="1.0"?>
<container version="1.0" xmlns="urn:oasis:names:tc:opendocument:xmlns:container">
  <rootfiles>
    <rootfile full-path="OEBPS/content.opf" media-type="application/oebps-package+xml"/>
  </rootfiles>
</container>
"""


def package_document(manifest: str, spine: str) -> str:
    return f"""<?xml version="1.0" encoding="utf-8"?>
<package xmlns="http://www.idpf.org/2007/opf" version="3.0" unique-identifier="uid">
  <metadata xmlns:dc="http://purl.org/dc/elements/1.1/">
    <dc:title>Fixture Book</dc:title>
    <dc:identifier id="uid">urn:uuid:fixture</dc:identifier>
    <dc:language>en</dc:language>
  </metadata>
  <manifest>{manifest}</manifest>
  <spine>{spine}</spine>
</package>
"""


def xhtml(body: str) -> str:
    return f"""<?xml version="1.0" encoding="utf-8"?>
<html xmlns="http://www.w3.org/1999/xhtml" xmlns:epub="http://www.idpf.org/2007/ops">
<head><title>Document</title></head>
<body>{body}</body>
</html>
"""


def build_epub(target: Path, documents: dict[str, str], extra: dict[str, bytes] | None = None) -> None:
    """Write an EPUB whose spine is exactly `documents`, in insertion order."""
    manifest = "".join(
        f'<item id="d{index}" href="{name}" media-type="application/xhtml+xml"/>'
        for index, name in enumerate(documents)
    )
    spine = "".join(f'<itemref idref="d{index}"/>' for index in range(len(documents)))
    for name in extra or {}:
        manifest += f'<item id="{name.replace("/", "_")}" href="{name}" media-type="image/png"/>'

    with ZipFile(target, "w", ZIP_DEFLATED) as archive:
        archive.writestr("mimetype", "application/epub+zip")
        archive.writestr("META-INF/container.xml", CONTAINER)
        archive.writestr("OEBPS/content.opf", package_document(manifest, spine))
        for name, markup in documents.items():
            archive.writestr(f"OEBPS/{name}", markup)
        for name, payload in (extra or {}).items():
            archive.writestr(f"OEBPS/{name}", payload)


def extract(documents: dict[str, str], extra: dict[str, bytes] | None = None, name: str = "Fixture Book"):
    """Extract a freshly built book and hand back its Markdown plus output dir."""
    directory = tempfile.TemporaryDirectory()
    root = Path(directory.name)
    epub_path = root / f"{name}.epub"
    build_epub(epub_path, documents, extra)
    output_dir = root / "out"
    result = extract_book(epub_path, output_dir)
    return result, result.markdown_path.read_text(encoding="utf-8"), output_dir, directory


def test_every_spine_document_becomes_exactly_one_level_one_heading() -> None:
    result, markdown, _, directory = extract(
        {
            "one.xhtml": xhtml("<h1>Opening</h1><p>First body.</p><h2>Inner</h2><p>More.</p>"),
            "two.xhtml": xhtml("<h1>Second</h1><p>Second body.</p>"),
            "three.xhtml": xhtml("<h1>Third</h1><p>Third body.</p>"),
        }
    )
    with directory:
        level_one = [line for line in markdown.splitlines() if line.startswith("# ")]
        assert level_one == ["# Opening", "# Second", "# Third"]
        assert result.chapters == 3
        # The document's own h2 is demoted so it cannot be mistaken for a
        # chapter boundary by split-policy-v3.
        assert "## Inner" in markdown


def test_document_headings_are_demoted_below_the_chapter_heading() -> None:
    _, markdown, _, directory = extract(
        {"one.xhtml": xhtml("<h1>Title</h1><h2>Section</h2><h3>Subsection</h3><p>Body.</p>")}
    )
    with directory:
        assert "# Title" in markdown
        assert "## Section" in markdown
        assert "### Subsection" in markdown
        assert "#### " not in markdown


def test_a_document_whose_shallowest_heading_is_h2_still_starts_at_level_two() -> None:
    # The title heading is consumed, so the *surviving* headings decide the
    # offset. Counting the consumed one used to push every section a level deep.
    _, markdown, _, directory = extract(
        {"one.xhtml": xhtml("<h2>Chapter Two</h2><h3>Part</h3><p>Body.</p>")}
    )
    with directory:
        assert "# Chapter Two" in markdown
        assert "## Part" in markdown
        assert "### Part" not in markdown


def test_body_text_that_looks_like_a_heading_is_escaped() -> None:
    _, markdown, _, directory = extract(
        {"one.xhtml": xhtml("<h1>Real</h1><p># Not a heading at all.</p><p>- not a list</p>")}
    )
    with directory:
        assert markdown.count("\n# ") + markdown.startswith("# ") == 1
        assert "\\# Not a heading at all." in markdown
        assert "\\- not a list" in markdown


def test_chapter_titles_fall_back_to_the_table_of_contents() -> None:
    directory = tempfile.TemporaryDirectory()
    with directory:
        root = Path(directory.name)
        epub_path = root / "Fixture Book.epub"
        nav = xhtml(
            '<nav epub:type="toc"><ol><li><a href="one.xhtml">Labelled By Nav</a></li></ol></nav>'
        )
        with ZipFile(epub_path, "w", ZIP_DEFLATED) as archive:
            archive.writestr("mimetype", "application/epub+zip")
            archive.writestr("META-INF/container.xml", CONTAINER)
            archive.writestr(
                "OEBPS/content.opf",
                package_document(
                    '<item id="nav" href="nav.xhtml" media-type="application/xhtml+xml"'
                    ' properties="nav"/>'
                    '<item id="d0" href="one.xhtml" media-type="application/xhtml+xml"/>',
                    '<itemref idref="d0"/>',
                ),
            )
            archive.writestr("OEBPS/nav.xhtml", nav)
            archive.writestr("OEBPS/one.xhtml", xhtml("<p>A chapter with no heading of its own.</p>"))

        result = extract_book(epub_path, root / "out")
        markdown = result.markdown_path.read_text(encoding="utf-8")
        assert markdown.startswith("# Labelled By Nav")


def test_images_land_in_the_sidecar_and_are_referenced_relatively() -> None:
    result, markdown, output_dir, directory = extract(
        {"one.xhtml": xhtml('<h1>Figures</h1><p><img src="images/figure.png" alt="A figure"/></p>')},
        {"images/figure.png": PNG},
    )
    with directory:
        assert result.images == 1
        sidecar = output_dir / "Fixture_Book.assets"
        assert (sidecar / "figure.png").read_bytes() == PNG
        assert "![A figure](Fixture_Book.assets/figure.png)" in markdown


def test_output_names_have_no_spaces_so_image_urls_stay_protected() -> None:
    # A space in the reference would break Markdown link syntax and would stop
    # the translation engine's link_url placeholder from matching, leaving the
    # path itself exposed to the model.
    result, markdown, _, directory = extract(
        {"one.xhtml": xhtml('<h1>T</h1><p><img src="images/figure.png" alt=""/></p>')},
        {"images/figure.png": PNG},
        name="A Book With Spaces",
    )
    with directory:
        assert result.markdown_path.name == "A_Book_With_Spaces.md"
        assert "](A_Book_With_Spaces.assets/figure.png)" in markdown


def test_footnotes_are_pulled_into_the_chapter_that_cites_them() -> None:
    result, markdown, _, directory = extract(
        {
            "one.xhtml": xhtml(
                '<h1>Citing</h1><p>Claim'
                '<sup><a epub:type="noteref" href="notes.xhtml#fn1">1</a></sup>.</p>'
            ),
            "notes.xhtml": xhtml(
                '<h1>Notes</h1>'
                '<aside epub:type="footnote" id="fn1"><p>The note body.</p></aside>'
            ),
        }
    )
    with directory:
        assert "Claim[^fn-1-1]." in markdown
        assert "[^fn-1-1]: The note body." in markdown
        # The endnotes document had nothing else in it, so it does not survive as
        # a chapter and the body is not printed twice.
        assert markdown.count("The note body.") == 1
        assert result.chapters == 1


def test_calibre_style_notes_are_recognised_without_any_epub_type() -> None:
    # Regression, found against a real Calibre-produced book: the reference is a
    # superscripted cross-document link carrying no epub:type and no role, and
    # the id marking the note body sits on the back-link *inside* the paragraph
    # rather than on the paragraph. Matching only epub:type found none of them,
    # and every note in the book was silently reduced to a bare digit.
    result, markdown, _, directory = extract(
        {
            "part0009.html": xhtml(
                '<h1>Structures</h1><p>A claim'
                '<sup class="calibre9"><a id="ch4foot1" href="part0010.html#foot1ch4">1</a></sup>'
                " in the body.</p>"
            ),
            "part0010.html": xhtml(
                '<h1>Notes</h1><p class="notet">CHAPTER 4</p>'
                '<p class="note"><a id="foot1ch4" href="part0009.html#ch4foot1">1</a>'
                "  The wet season is the time for oral instruction.</p>"
            ),
        }
    )
    with directory:
        assert "A claim[^fn-1-1] in the body." in markdown
        # The back-link digit is dropped: it would otherwise prefix the note.
        assert "[^fn-1-1]: The wet season is the time for oral instruction." in markdown
        assert markdown.count("The wet season") == 1
        # The endnotes page still carries its own section labels, so it survives
        # as a chapter -- just without the note body that moved.
        assert result.chapters == 2
        assert "CHAPTER 4" in markdown


def test_a_superscripted_link_with_no_note_body_stays_plain_text() -> None:
    # The <sup> heuristic must not turn an ordinary superscripted link into a
    # footnote reference pointing at nothing.
    _, markdown, _, directory = extract(
        {"one.xhtml": xhtml('<h1>A</h1><p>Ordinal 1<sup><a href="#nowhere">st</a></sup>.</p>')}
    )
    with directory:
        assert "Ordinal 1st." in markdown
        assert "[^" not in markdown


def test_an_endnotes_document_keeps_the_notes_nobody_cited() -> None:
    _, markdown, _, directory = extract(
        {
            "one.xhtml": xhtml(
                '<h1>Citing</h1><p>Claim<a epub:type="noteref" href="notes.xhtml#fn1">1</a>.</p>'
            ),
            "notes.xhtml": xhtml(
                '<h1>Notes</h1>'
                '<p id="fn1">Cited note.</p>'
                '<p id="fn2">Orphan note nobody points at.</p>'
            ),
        }
    )
    with directory:
        assert "[^fn-1-1]: Cited note." in markdown
        assert markdown.count("Cited note.") == 1
        assert "Orphan note nobody points at." in markdown


def test_code_blocks_and_maths_use_protected_markdown_forms() -> None:
    _, markdown, _, directory = extract(
        {
            "one.xhtml": xhtml(
                "<h1>Technical</h1>"
                '<pre><code>def f():\n    return "# not a heading"\n</code></pre>'
                '<p>Inline <math xmlns="http://www.w3.org/1998/Math/MathML" alttext="E = mc^2">'
                "<mi>E</mi></math> maths.</p>"
                '<p>Block <math xmlns="http://www.w3.org/1998/Math/MathML" display="block"'
                ' alttext="a^2 + b^2 = c^2"><mi>a</mi></math></p>"'
            )
        }
    )
    with directory:
        assert '```\ndef f():\n    return "# not a heading"\n```' in markdown
        assert "$E = mc^2$" in markdown
        assert "$$a^2 + b^2 = c^2$$" in markdown


def test_mathml_without_a_tex_source_keeps_its_symbols_as_inline_code() -> None:
    _, markdown, _, directory = extract(
        {
            "one.xhtml": xhtml(
                "<h1>Technical</h1>"
                '<p><math xmlns="http://www.w3.org/1998/Math/MathML">'
                "<mi>x</mi><mo>+</mo><mn>1</mn></math></p>"
            )
        }
    )
    with directory:
        assert "`x+1`" in markdown


def test_a_tex_annotation_is_preferred_over_raw_symbols() -> None:
    _, markdown, _, directory = extract(
        {
            "one.xhtml": xhtml(
                "<h1>Technical</h1>"
                '<p><math xmlns="http://www.w3.org/1998/Math/MathML"><semantics><mi>x</mi>'
                '<annotation encoding="application/x-tex">\\alpha</annotation>'
                "</semantics></math></p>"
            )
        }
    )
    with directory:
        assert "$\\alpha$" in markdown


def test_unclosed_paragraphs_do_not_swallow_the_rest_of_the_chapter() -> None:
    _, markdown, _, directory = extract(
        {"one.xhtml": xhtml("<h1>Loose</h1><p>First paragraph.<p>Second paragraph.")}
    )
    with directory:
        assert "First paragraph.\n\nSecond paragraph." in markdown


def test_lists_tables_and_quotes_survive_as_markdown() -> None:
    _, markdown, _, directory = extract(
        {
            "one.xhtml": xhtml(
                "<h1>Shapes</h1>"
                "<ul><li>first</li><li>second</li></ul>"
                "<ol><li>step one</li></ol>"
                "<table><tr><th>Term</th><th>Gloss</th></tr>"
                "<tr><td>alpha</td><td>first</td></tr></table>"
                "<blockquote><p>Quoted.</p></blockquote>"
            )
        }
    )
    with directory:
        assert "- first\n- second" in markdown
        assert "1. step one" in markdown
        assert "| Term | Gloss |\n| --- | --- |\n| alpha | first |" in markdown
        assert "> Quoted." in markdown


def test_internal_cross_references_keep_their_words_and_drop_the_link() -> None:
    # Every spine document is merged into one file, so a link to two.xhtml would
    # point at a file that no longer exists.
    _, markdown, _, directory = extract(
        {
            "one.xhtml": xhtml('<h1>A</h1><p>See <a href="two.xhtml">the next part</a>.</p>'),
            "two.xhtml": xhtml("<h1>B</h1><p>Here.</p>"),
        }
    )
    with directory:
        assert "See the next part." in markdown
        assert "two.xhtml" not in markdown


def test_external_links_are_kept_whole() -> None:
    _, markdown, _, directory = extract(
        {"one.xhtml": xhtml('<h1>A</h1><p>See <a href="https://example.org/x">the site</a>.</p>')}
    )
    with directory:
        assert "[the site](https://example.org/x)" in markdown


def test_a_blank_spine_document_produces_no_chapter() -> None:
    result, markdown, _, directory = extract(
        {
            "one.xhtml": xhtml("<h1>Real</h1><p>Body.</p>"),
            "blank.xhtml": xhtml("<div></div>"),
        }
    )
    with directory:
        assert result.chapters == 1
        assert markdown.count("\n# ") + markdown.startswith("# ") == 1


def test_an_archive_that_is_not_an_epub_is_reported_not_crashed() -> None:
    with tempfile.TemporaryDirectory() as name:
        root = Path(name)
        broken = root / "broken.epub"
        broken.write_bytes(b"not a zip at all")
        try:
            extract_book(broken, root / "out")
        except EpubExtractError as error:
            assert "readable EPUB archive" in str(error)
        else:  # pragma: no cover - the call above must raise
            raise AssertionError("a non-archive should raise EpubExtractError")


def test_an_epub_without_a_spine_is_reported() -> None:
    with tempfile.TemporaryDirectory() as name:
        root = Path(name)
        epub_path = root / "empty.epub"
        with ZipFile(epub_path, "w", ZIP_DEFLATED) as archive:
            archive.writestr("mimetype", "application/epub+zip")
            archive.writestr("META-INF/container.xml", CONTAINER)
            archive.writestr("OEBPS/content.opf", package_document("", ""))
        try:
            extract_book(epub_path, root / "out")
        except EpubExtractError as error:
            assert "empty spine" in str(error)
        else:  # pragma: no cover - the call above must raise
            raise AssertionError("an empty spine should raise EpubExtractError")
