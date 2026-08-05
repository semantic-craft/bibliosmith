from __future__ import annotations

import json
import sys
from pathlib import Path
from zipfile import ZIP_DEFLATED, ZipFile

import fitz

PACKAGE_ROOT = Path(__file__).resolve().parents[1]
SCRIPTS = PACKAGE_ROOT / "scripts"
sys.path.insert(0, str(PACKAGE_ROOT))
sys.path.insert(0, str(SCRIPTS))

import pdf_text  # noqa: E402
from epub_to_markdown import extract_book  # noqa: E402
from publication_evidence import SourceDocument, build_markdown_evidence  # noqa: E402


def _write_pdf(path: Path) -> None:
    document = fitz.open()
    contents = document.new_page()
    contents.insert_text((72, 72), "Contents", fontsize=24)
    contents.insert_textbox(
        fitz.Rect(72, 95, 500, 150),
        "Chapter One 2. This printed contents entry is long enough to remain ordinary prose.",
        fontsize=12,
    )
    chapter = document.new_page()
    chapter.insert_text((72, 72), "Chapter One", fontsize=24)
    chapter.insert_textbox(
        fitz.Rect(72, 95, 500, 145),
        "Opening body sentence with enough words to remain a normal paragraph in extraction.",
        fontsize=12,
    )
    chapter.insert_text((72, 180), "Section A", fontsize=18)
    chapter.insert_textbox(
        fitz.Rect(72, 200, 500, 250),
        "Section body sentence with enough words to remain a normal paragraph in extraction.",
        fontsize=12,
    )
    document.save(path)
    document.close()


def _write_epub(path: Path) -> None:
    container = """<?xml version="1.0"?>
<container version="1.0" xmlns="urn:oasis:names:tc:opendocument:xmlns:container">
<rootfiles><rootfile full-path="OEBPS/content.opf" media-type="application/oebps-package+xml"/></rootfiles>
</container>"""
    package = """<?xml version="1.0" encoding="utf-8"?>
<package xmlns="http://www.idpf.org/2007/opf" version="3.0" unique-identifier="uid">
<metadata xmlns:dc="http://purl.org/dc/elements/1.1/"><dc:title>Fixture</dc:title><dc:creator>Author</dc:creator><dc:identifier id="uid">fixture</dc:identifier><dc:language>en</dc:language></metadata>
<manifest><item id="nav" href="nav.xhtml" media-type="application/xhtml+xml" properties="nav"/><item id="contents" href="contents.xhtml" media-type="application/xhtml+xml"/><item id="chapter" href="chapter.xhtml" media-type="application/xhtml+xml"/></manifest>
<spine><itemref idref="contents"/><itemref idref="chapter"/></spine></package>"""
    nav = """<html xmlns="http://www.w3.org/1999/xhtml" xmlns:epub="http://www.idpf.org/2007/ops"><body><nav epub:type="toc"><ol><li><a href="contents.xhtml">Contents</a></li><li><a href="chapter.xhtml">Chapter One</a><ol><li><a href="chapter.xhtml#section-a">Section A</a></li></ol></li></ol></nav></body></html>"""
    contents = """<html xmlns="http://www.w3.org/1999/xhtml"><body><h1>Contents</h1><p>Chapter One 2.</p></body></html>"""
    chapter = """<html xmlns="http://www.w3.org/1999/xhtml"><body><h1>Chapter One</h1><p>Opening body sentence.</p><h2 id="section-a">Section A</h2><p>Section body sentence.</p></body></html>"""
    with ZipFile(path, "w", ZIP_DEFLATED) as archive:
        archive.writestr("mimetype", "application/epub+zip")
        archive.writestr("META-INF/container.xml", container)
        archive.writestr("OEBPS/content.opf", package)
        archive.writestr("OEBPS/nav.xhtml", nav)
        archive.writestr("OEBPS/contents.xhtml", contents)
        archive.writestr("OEBPS/chapter.xhtml", chapter)


def _normalized_tree(sections: list[dict[str, object]]) -> list[tuple[str, int, str | None]]:
    title_by_id = {str(section["id"]): str(section["title"]) for section in sections}
    return [
        (
            str(section["title"]),
            int(section["headingLevel"]),
            title_by_id.get(str(section["parentId"])) if section.get("parentId") else None,
        )
        for section in sections
    ]


def test_real_pdf_and_epub_extractors_produce_equivalent_publication_trees(
    tmp_path: Path,
) -> None:
    pdf_path = tmp_path / "fixture.pdf"
    epub_path = tmp_path / "fixture.epub"
    _write_pdf(pdf_path)
    _write_epub(epub_path)

    pdf = pdf_text.extract_markdown(pdf_path)
    pdf_evidence = build_markdown_evidence(
        pdf.markdown,
        source_format="pdf",
        extraction_engine=pdf.engine,
        source_documents=[
            SourceDocument(
                "pdf/fixture.pdf",
                1,
                len(pdf.markdown.splitlines()),
                (1, 2),
                kind="pdf_fixture",
                sha256="a" * 64,
            )
        ],
    )
    epub_result = extract_book(epub_path, tmp_path / "epub-output")
    epub_evidence = json.loads(
        epub_result.publication_evidence_path.read_text(encoding="utf-8")
    )

    assert _normalized_tree(pdf_evidence["sections"]) == _normalized_tree(
        epub_evidence["sections"]
    ) == [
        ("Contents", 1, None),
        ("Chapter One", 1, None),
        ("Section A", 2, "Chapter One"),
    ]
