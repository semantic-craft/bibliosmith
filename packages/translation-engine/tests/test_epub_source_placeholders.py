"""Placeholder coverage for Markdown that came out of an EPUB (issue #98).

The `epub_source` route feeds the translation engine text that no OCR engine
wrote, so the shapes the engine protects have to be the shapes the extractor
emits. The extractor is run for real here rather than pasted in as a golden
string: a fixture that drifts from the script would keep passing while the
protection it claims to prove had already broken.

The extractor is stdlib-only, so importing it by path costs this suite nothing.
"""

from __future__ import annotations

import sys
import tempfile
from pathlib import Path
from zipfile import ZIP_DEFLATED, ZipFile

from translation_engine.placeholders import protect_markdown

REPOSITORY_ROOT = Path(__file__).resolve().parents[3]
sys.path.insert(0, str(REPOSITORY_ROOT / "packages" / "ocr" / "scripts"))

from epub_to_markdown import extract_book  # noqa: E402


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

PACKAGE = """<?xml version="1.0" encoding="utf-8"?>
<package xmlns="http://www.idpf.org/2007/opf" version="3.0" unique-identifier="uid">
  <metadata xmlns:dc="http://purl.org/dc/elements/1.1/">
    <dc:title>Protected Shapes</dc:title>
    <dc:identifier id="uid">urn:uuid:fixture</dc:identifier>
    <dc:language>en</dc:language>
  </metadata>
  <manifest>
    <item id="c1" href="chapter.xhtml" media-type="application/xhtml+xml"/>
    <item id="c2" href="notes.xhtml" media-type="application/xhtml+xml"/>
    <item id="img" href="figure.png" media-type="image/png"/>
  </manifest>
  <spine><itemref idref="c1"/><itemref idref="c2"/></spine>
</package>
"""

CHAPTER = """<?xml version="1.0" encoding="utf-8"?>
<html xmlns="http://www.w3.org/1999/xhtml" xmlns:epub="http://www.idpf.org/2007/ops">
<head><title>Chapter</title></head>
<body>
  <h1>Protected Shapes</h1>
  <p>Prose that should reach the model, with a citation
     <sup><a epub:type="noteref" href="notes.xhtml#fn1">1</a></sup>.</p>
  <pre><code>def keep_me():
    return "# still not a heading"
</code></pre>
  <p>Inline <math xmlns="http://www.w3.org/1998/Math/MathML" alttext="E = mc^2"><mi>E</mi></math>
     and display
     <math xmlns="http://www.w3.org/1998/Math/MathML" display="block" alttext="\\int_0^1 x dx">
     <mi>x</mi></math>.</p>
  <p><img src="figure.png" alt="Translatable alt text"/></p>
  <p>A term rendered as <code>inline_code()</code> in the source.</p>
</body>
</html>
"""

NOTES = """<?xml version="1.0" encoding="utf-8"?>
<html xmlns="http://www.w3.org/1999/xhtml" xmlns:epub="http://www.idpf.org/2007/ops">
<head><title>Notes</title></head>
<body>
  <h1>Notes</h1>
  <aside epub:type="footnote" id="fn1"><p>The note body, which is prose too.</p></aside>
</body>
</html>
"""


def extracted_markdown(directory: Path) -> str:
    epub_path = directory / "Protected Shapes.epub"
    with ZipFile(epub_path, "w", ZIP_DEFLATED) as archive:
        archive.writestr("mimetype", "application/epub+zip")
        archive.writestr("META-INF/container.xml", CONTAINER)
        archive.writestr("OEBPS/content.opf", PACKAGE)
        archive.writestr("OEBPS/chapter.xhtml", CHAPTER)
        archive.writestr("OEBPS/notes.xhtml", NOTES)
        archive.writestr("OEBPS/figure.png", PNG)
    return extract_book(epub_path, directory / "out").markdown_path.read_text(encoding="utf-8")


def test_every_fragile_shape_an_epub_produces_is_protected() -> None:
    with tempfile.TemporaryDirectory() as name:
        markdown = extracted_markdown(Path(name))
        protection = protect_markdown(markdown)
        protected = [original for _, original in protection.replacements]

        assert '```\ndef keep_me():\n    return "# still not a heading"\n```' in protected
        assert "$E = mc^2$" in protected
        assert "$$\\int_0^1 x dx$$" in protected
        assert "`inline_code()`" in protected
        assert "[^fn-1-1]" in protected
        assert "Protected_Shapes_assets/figure.png" in protected


def test_protection_round_trips_the_whole_extracted_document() -> None:
    with tempfile.TemporaryDirectory() as name:
        markdown = extracted_markdown(Path(name))
        protection = protect_markdown(markdown)

        assert protection.restore(protection.text) == markdown


def test_prose_stays_outside_the_placeholders_so_it_still_gets_translated() -> None:
    # Over-protection is the opposite failure: a chapter whose body never reaches
    # the model would come back untranslated with every test still green.
    with tempfile.TemporaryDirectory() as name:
        markdown = extracted_markdown(Path(name))
        protection = protect_markdown(markdown)

        assert "Prose that should reach the model" in protection.text
        assert "The note body, which is prose too." in protection.text
        assert "Translatable alt text" in protection.text
        assert "Protected Shapes" in protection.text


def test_a_reserved_placeholder_in_the_book_is_refused_not_silently_mangled() -> None:
    # Books can contain anything; the engine's guard is what stops a literal
    # placeholder in the source from colliding with a generated one.
    try:
        protect_markdown("# Chapter\n\nA line containing ⟦PH_000001⟧ verbatim.\n")
    except ValueError as error:
        assert "reserved placeholder" in str(error)
    else:  # pragma: no cover - the call above must raise
        raise AssertionError("a reserved placeholder must be refused")
