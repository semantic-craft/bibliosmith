"""Extractor behaviour for the `epub_source` route.

The fixtures are real EPUB archives built here rather than checked-in binaries:
a hand-written zip keeps the structure under test visible in the diff, and the
extractor reads them through the same `container.xml` -> OPF -> spine path a
shipped book takes.
"""

from __future__ import annotations

import json
import hashlib
import sys
import tempfile
from pathlib import Path
from zipfile import ZIP_DEFLATED, ZipFile

SCRIPTS = Path(__file__).resolve().parents[1] / "scripts"
sys.path.insert(0, str(SCRIPTS))

from epub_to_markdown import (  # noqa: E402
    EpubExtractError,
    extract_book,
    unique_output_stems,
)


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
    <dc:creator>Fixture Author</dc:creator>
    <dc:publisher>Fixture Press</dc:publisher>
    <dc:date>2026</dc:date>
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


def test_nested_nav_is_exported_as_structure_evidence_even_with_shared_spine_document() -> None:
    directory = tempfile.TemporaryDirectory()
    with directory:
        root = Path(directory.name)
        epub_path = root / "Fixture Book.epub"
        nav = xhtml(
            '<nav epub:type="toc"><ol>'
            '<li><a href="one.xhtml">Part I</a><ol>'
            '<li><a href="one.xhtml#chapter">Chapter 1</a><ol>'
            '<li><a href="one.xhtml#section-a">Section A</a></li>'
            '<li><a href="one.xhtml#section-b">Section B</a></li>'
            '</ol></li></ol></li></ol></nav>'
            '<nav epub:type="landmarks"><ol><li><a epub:type="bodymatter" '
            'href="one.xhtml">Start</a></li></ol></nav>'
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
            archive.writestr(
                "OEBPS/one.xhtml",
                xhtml(
                    '<h1>Part I</h1><h2 id="chapter">Chapter 1</h2>'
                    '<h3 id="section-a">Section A</h3><p>A.</p>'
                    '<h3 id="section-b">Section B</h3><p>B.</p>'
                ),
            )

        result = extract_book(epub_path, root / "out")
        structure = json.loads(result.publication_evidence_path.read_text(encoding="utf-8"))

        assert structure["schema"] == "publication-extraction-evidence-v2"
        assert structure["creator"] == "Fixture Author"
        assert structure["publisher"] == "Fixture Press"
        assert structure["date"] == "2026"
        assert [section["title"] for section in structure["sections"]] == [
            "Part I",
            "Chapter 1",
            "Section A",
            "Section B",
        ]
        assert [section["parentId"] for section in structure["sections"]] == [
            None,
            "epub_section_001",
            "epub_section_002",
            "epub_section_002",
        ]
        assert structure["sections"][1]["sourceHref"].endswith("one.xhtml#chapter")
        assert structure["sections"][0]["role"] == "bodymatter"
        lines = result.markdown_path.read_text(encoding="utf-8").splitlines()
        for section in structure["sections"]:
            assert 1 <= section["sourceStartLine"] <= section["sourceEndLine"] <= len(lines)
            section_lines = lines[section["sourceStartLine"] - 1 : section["sourceEndLine"]]
            assert any(f"bibliosmith-nav:{section['id']}:" in line for line in section_lines)


def test_epub3_nav_is_authoritative_when_a_legacy_ncx_is_also_packaged() -> None:
    directory = tempfile.TemporaryDirectory()
    with directory:
        root = Path(directory.name)
        epub_path = root / "Dual Navigation.epub"
        nav = xhtml(
            '<nav epub:type="toc"><ol><li><a href="one.xhtml">Chapter</a>'
            '</li></ol></nav>'
        )
        ncx = """<?xml version="1.0" encoding="utf-8"?>
<ncx xmlns="http://www.daisy.org/z3986/2005/ncx/">
  <navMap><navPoint id="legacy"><navLabel><text>Legacy Chapter</text></navLabel>
  <content src="one.xhtml"/></navPoint></navMap>
</ncx>
"""
        package = package_document(
            '<item id="nav" href="nav.xhtml" media-type="application/xhtml+xml" properties="nav"/>'
            '<item id="ncx" href="toc.ncx" media-type="application/x-dtbncx+xml"/>'
            '<item id="chapter" href="one.xhtml" media-type="application/xhtml+xml"/>',
            '<itemref idref="chapter"/>',
        ).replace("<spine>", '<spine toc="ncx">')
        with ZipFile(epub_path, "w", ZIP_DEFLATED) as archive:
            archive.writestr("mimetype", "application/epub+zip")
            archive.writestr("META-INF/container.xml", CONTAINER)
            archive.writestr("OEBPS/content.opf", package)
            archive.writestr("OEBPS/nav.xhtml", nav)
            archive.writestr("OEBPS/toc.ncx", ncx)
            archive.writestr("OEBPS/one.xhtml", xhtml("<h1>Chapter</h1><p>Body.</p>"))

        result = extract_book(epub_path, root / "out")
        evidence = json.loads(result.publication_evidence_path.read_text(encoding="utf-8"))
        navigation_documents = [
            document
            for document in evidence["sourceDocuments"]
            if document["kind"] in {"epub_navigation", "epub_ncx"}
        ]

        assert {document["kind"] for document in navigation_documents} == {
            "epub_navigation",
            "epub_ncx",
        }
        assert all(
            (document["startLine"], document["endLine"]) == (0, 0)
            for document in navigation_documents
        )
        assert all(document["anomalies"] for document in navigation_documents)
        nav_document = next(
            document["path"]
            for document in navigation_documents
            if document["kind"] == "epub_navigation"
        )
        ncx_document = next(
            document["path"]
            for document in navigation_documents
            if document["kind"] == "epub_ncx"
        )
        assert nav_document in evidence["sections"][0]["sourceFiles"]
        assert ncx_document not in evidence["sections"][0]["sourceFiles"]
        assert evidence["sections"][0]["navigationSourceHref"].endswith("nav.xhtml")


def test_nested_nav_ranges_and_source_files_contain_children_across_spine_documents() -> None:
    directory = tempfile.TemporaryDirectory()
    with directory:
        root = Path(directory.name)
        epub_path = root / "Fixture Book.epub"
        nav = xhtml(
            '<nav epub:type="toc"><ol>'
            '<li><a href="part.xhtml">Part I</a><ol>'
            '<li><a href="chapter.xhtml#chapter">Chapter 1</a><ol>'
            '<li><a href="chapter.xhtml#section">Section A</a></li>'
            '</ol></li></ol></li></ol></nav>'
        )
        with ZipFile(epub_path, "w", ZIP_DEFLATED) as archive:
            archive.writestr("mimetype", "application/epub+zip")
            archive.writestr("META-INF/container.xml", CONTAINER)
            archive.writestr(
                "OEBPS/content.opf",
                package_document(
                    '<item id="nav" href="nav.xhtml" media-type="application/xhtml+xml" properties="nav"/>'
                    '<item id="part" href="part.xhtml" media-type="application/xhtml+xml"/>'
                    '<item id="chapter" href="chapter.xhtml" media-type="application/xhtml+xml"/>',
                    '<itemref idref="part"/><itemref idref="chapter"/>',
                ),
            )
            archive.writestr("OEBPS/nav.xhtml", nav)
            archive.writestr(
                "OEBPS/part.xhtml",
                xhtml("<h1>Part I</h1><p>Part introduction.</p>"),
            )
            archive.writestr(
                "OEBPS/chapter.xhtml",
                xhtml(
                    '<h1 id="chapter">Chapter 1</h1>'
                    '<h2 id="section">Section A</h2><p>Section body.</p>'
                ),
            )

        result = extract_book(epub_path, root / "out")
        evidence = json.loads(result.publication_evidence_path.read_text(encoding="utf-8"))
        lines = result.markdown_path.read_text(encoding="utf-8").splitlines()
        part, chapter, section = evidence["sections"]

        assert part["sourceStartLine"] < chapter["sourceStartLine"]
        assert part["sourceEndLine"] >= chapter["sourceEndLine"]
        assert chapter["sourceEndLine"] >= section["sourceEndLine"]
        for node in (part, chapter, section):
            selected = lines[node["sourceStartLine"] - 1 : node["sourceEndLine"]]
            assert any(f"bibliosmith-nav:{node['id']}:" in line for line in selected)

        xhtml_documents = [
            document
            for document in evidence["sourceDocuments"]
            if document["kind"] == "epub_xhtml"
        ]
        assert len(xhtml_documents) == 2
        part_source = next(
            document["path"]
            for document in xhtml_documents
            if b"Part I" in (result.markdown_path.parent / document["path"]).read_bytes()
        )
        chapter_source = next(
            document["path"]
            for document in xhtml_documents
            if b"Chapter 1" in (result.markdown_path.parent / document["path"]).read_bytes()
        )
        assert {part_source, chapter_source}.issubset(set(part["sourceFiles"]))
        assert chapter_source in chapter["sourceFiles"]
        assert part_source not in chapter["sourceFiles"]


def test_nav_ranges_use_targets_not_titles_for_non_heading_and_duplicate_anchors() -> None:
    directory = tempfile.TemporaryDirectory()
    with directory:
        root = Path(directory.name)
        epub_path = root / "Fixture Book.epub"
        nav = xhtml(
            '<nav epub:type="toc"><ol>'
            '<li><a href="one.xhtml#first">Repeated</a></li>'
            '<li><a href="one.xhtml#topic">Topic label</a></li>'
            '<li><a href="one.xhtml#second">Repeated</a></li>'
            '</ol></nav>'
        )
        with ZipFile(epub_path, "w", ZIP_DEFLATED) as archive:
            archive.writestr("mimetype", "application/epub+zip")
            archive.writestr("META-INF/container.xml", CONTAINER)
            archive.writestr(
                "OEBPS/content.opf",
                package_document(
                    '<item id="nav" href="nav.xhtml" media-type="application/xhtml+xml" properties="nav"/>'
                    '<item id="d0" href="one.xhtml" media-type="application/xhtml+xml"/>',
                    '<itemref idref="d0"/>',
                ),
            )
            archive.writestr("OEBPS/nav.xhtml", nav)
            archive.writestr(
                "OEBPS/one.xhtml",
                xhtml(
                    '<h1 id="first">Repeated</h1><p>First.</p>'
                    '<p id="topic">A non-heading target.</p>'
                    '<h2 id="second">Repeated</h2><p>Second.</p>'
                ),
            )

        result = extract_book(epub_path, root / "out")
        evidence = json.loads(result.publication_evidence_path.read_text(encoding="utf-8"))
        starts = [section["sourceStartLine"] for section in evidence["sections"]]
        assert starts == sorted(starts)
        assert len(set(starts)) == 3
        assert all(section["sourceEndLine"] >= section["sourceStartLine"] for section in evidence["sections"])

        source_document = evidence["sourceDocuments"][0]
        persisted = result.markdown_path.parent / source_document["path"]
        assert persisted.is_file()
        assert hashlib.sha256(persisted.read_bytes()).hexdigest() == source_document["sha256"]
        navigation_documents = [
            document
            for document in evidence["sourceDocuments"]
            if document["kind"] == "epub_navigation"
        ]
        assert len(navigation_documents) == 1
        navigation_path = result.markdown_path.parent / navigation_documents[0]["path"]
        assert navigation_path.is_file()
        assert all(
            navigation_documents[0]["path"] in section["sourceFiles"]
            for section in evidence["sections"]
        )


def test_images_land_in_the_sidecar_and_are_referenced_relatively() -> None:
    result, markdown, output_dir, directory = extract(
        {"one.xhtml": xhtml('<h1>Figures</h1><p><img src="images/figure.png" alt="A figure"/></p>')},
        {"images/figure.png": PNG},
    )
    with directory:
        assert result.images == 1
        sidecar = output_dir / "Fixture_Book_assets"
        assert (sidecar / "figure.png").read_bytes() == PNG
        assert "![A figure](Fixture_Book_assets/figure.png)" in markdown


def test_declared_cover_is_exported_as_a_first_class_source_asset() -> None:
    directory = tempfile.TemporaryDirectory()
    with directory:
        root = Path(directory.name)
        epub_path = root / "Fixture Book.epub"
        with ZipFile(epub_path, "w", ZIP_DEFLATED) as archive:
            archive.writestr("mimetype", "application/epub+zip")
            archive.writestr("META-INF/container.xml", CONTAINER)
            archive.writestr(
                "OEBPS/content.opf",
                package_document(
                    '<item id="d0" href="one.xhtml" media-type="application/xhtml+xml"/>'
                    '<item id="cover" href="images/cover.png" media-type="image/png"'
                    ' properties="cover-image"/>',
                    '<itemref idref="d0"/>',
                ),
            )
            archive.writestr("OEBPS/one.xhtml", xhtml("<h1>Opening</h1><p>Body.</p>"))
            archive.writestr("OEBPS/images/cover.png", PNG)

        result = extract_book(epub_path, root / "out")
        evidence = json.loads(result.publication_evidence_path.read_text(encoding="utf-8"))

        assert evidence["coverPath"] == "Fixture_Book_assets/cover.png"
        assert (root / "out/Fixture_Book_assets/cover.png").read_bytes() == PNG


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
        assert "](A_Book_With_Spaces_assets/figure.png)" in markdown


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
        evidence = json.loads(
            result.publication_evidence_path.read_text(encoding="utf-8")
        )
        assert evidence["notes"][0]["id"] == "note_001"
        assert evidence["notes"][0]["sourceLabel"] == "fn-1-1"
        assert evidence["notes"][0]["referenceIds"] == [
            "noteref_note_001_001"
        ]
        note_files = evidence["notes"][0]["sourceFiles"]
        assert len(note_files) == 2
        assert evidence["notes"][0]["sourceAnchor"].endswith(
            "notes.xhtml#fn1"
        )
        retained_xhtml = [
            document
            for document in evidence["sourceDocuments"]
            if document["kind"] == "epub_xhtml"
        ]
        assert len(retained_xhtml) == 2
        assert all(
            (result.markdown_path.parent / document["path"]).is_file()
            for document in retained_xhtml
        )
        assert evidence["notes"][0]["anomalies"] == []
        # The endnotes document had nothing else in it, so it does not survive as
        # a chapter and the body is not printed twice.
        assert markdown.count("The note body.") == 1
        assert result.chapters == 1


def test_explicit_footnote_bodies_are_not_silently_dropped_by_length() -> None:
    body = "Long legal note " + ("substantive evidence " * 140)
    result, markdown, _, directory = extract(
        {
            "one.xhtml": xhtml(
                '<h1>Citing</h1><p>Claim<a epub:type="noteref" '
                'href="notes.xhtml#fn-long">1</a>.</p>'
            ),
            "notes.xhtml": xhtml(
                '<aside epub:type="footnote" id="fn-long"><p>'
                f"{body}</p></aside>"
            ),
        }
    )
    with directory:
        assert len(body) > 2000
        assert "Claim[^fn-1-1]." in markdown
        assert body.strip() in markdown
        evidence = json.loads(
            result.publication_evidence_path.read_text(encoding="utf-8")
        )
        assert len(evidence["notes"]) == 1
        assert evidence["notes"][0]["sourceAnchor"].endswith(
            "notes.xhtml#fn-long"
        )


def test_endnote_semantics_and_navigation_survive_when_the_note_page_is_consumed() -> None:
    directory = tempfile.TemporaryDirectory()
    with directory:
        root = Path(directory.name)
        epub_path = root / "Endnote Navigation.epub"
        nav = xhtml(
            '<nav epub:type="toc"><ol>'
            '<li><a href="one.xhtml">Chapter</a></li>'
            '<li><a href="notes.xhtml">Endnotes</a></li>'
            '</ol></nav>'
        )
        with ZipFile(epub_path, "w", ZIP_DEFLATED) as archive:
            archive.writestr("mimetype", "application/epub+zip")
            archive.writestr("META-INF/container.xml", CONTAINER)
            archive.writestr(
                "OEBPS/content.opf",
                package_document(
                    '<item id="nav" href="nav.xhtml" media-type="application/xhtml+xml" properties="nav"/>'
                    '<item id="chapter" href="one.xhtml" media-type="application/xhtml+xml"/>'
                    '<item id="notes" href="notes.xhtml" media-type="application/xhtml+xml"/>',
                    '<itemref idref="chapter"/><itemref idref="notes"/>',
                ),
            )
            archive.writestr("OEBPS/nav.xhtml", nav)
            archive.writestr(
                "OEBPS/one.xhtml",
                xhtml(
                    '<h1>Chapter</h1><p>Claim<a epub:type="noteref" '
                    'href="notes.xhtml#n1">1</a>.</p>'
                ),
            )
            archive.writestr(
                "OEBPS/notes.xhtml",
                xhtml('<aside epub:type="endnote" id="n1"><p>Endnote body.</p></aside>'),
            )

        result = extract_book(epub_path, root / "out")
        markdown = result.markdown_path.read_text(encoding="utf-8")
        evidence = json.loads(result.publication_evidence_path.read_text(encoding="utf-8"))

        assert evidence["notes"][0]["kind"] == "endnote"
        assert [section["title"] for section in evidence["sections"]] == [
            "Chapter",
            "Endnotes",
        ]
        assert "# Endnotes" in markdown
        assert "bibliosmith-nav:epub_section_002:" in markdown
        assert markdown.count("Endnote body.") == 1


def test_a_declared_noteref_with_an_unrecoverable_target_fails_closed() -> None:
    directory = tempfile.TemporaryDirectory()
    with directory:
        root = Path(directory.name)
        epub_path = root / "Missing Note.epub"
        build_epub(
            epub_path,
            {
                "one.xhtml": xhtml(
                    '<h1>Citing</h1><p>Claim<a epub:type="noteref" '
                    'href="notes.xhtml#missing">1</a>.</p>'
                ),
                "notes.xhtml": xhtml("<h1>Notes</h1><p>No matching note.</p>"),
            },
        )

        try:
            extract_book(epub_path, root / "out")
        except EpubExtractError as error:
            assert "declared note reference target could not be recovered" in str(error)
            assert "notes.xhtml#missing" in str(error)
        else:  # pragma: no cover - extraction must not certify missing notes
            raise AssertionError("a declared noteref with no source body must fail")


def test_one_external_note_cited_from_two_spine_documents_remains_one_note() -> None:
    result, markdown, _, directory = extract(
        {
            "one.xhtml": xhtml(
                '<h1>First</h1><p>First claim<sup><a epub:type="noteref" '
                'href="notes.xhtml#fn1">1</a></sup>.</p>'
            ),
            "two.xhtml": xhtml(
                '<h1>Second</h1><p>Second claim<sup><a epub:type="noteref" '
                'href="notes.xhtml#fn1">1</a></sup>.</p>'
            ),
            "notes.xhtml": xhtml(
                '<h1>Notes</h1><aside epub:type="footnote" id="fn1">'
                '<p>Shared note body.</p></aside>'
            ),
        }
    )
    with directory:
        assert markdown.count("[^fn-1-1]") == 3
        assert markdown.count("[^fn-1-1]: Shared note body.") == 1
        evidence = json.loads(
            result.publication_evidence_path.read_text(encoding="utf-8")
        )
        assert len(evidence["notes"]) == 1
        assert evidence["notes"][0]["referenceIds"] == [
            "noteref_note_001_001",
            "noteref_note_001_002",
        ]
        assert len(evidence["notes"][0]["sourceFiles"]) == 3
        assert evidence["notes"][0]["sourceAnchor"].endswith(
            "notes.xhtml#fn1"
        )


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


# --- Review findings on PR #119 ---------------------------------------------


def test_an_ordinary_fragment_link_is_not_turned_into_a_footnote() -> None:
    # A fragment href alone used to trigger a note lookup, and since every short
    # id-bearing block is indexed as a candidate body, a plain cross-reference
    # would pull its target paragraph out of the chapter and re-emit it as a
    # footnote definition.
    _, markdown, _, directory = extract(
        {
            "one.xhtml": xhtml(
                '<h1>A</h1><p>See <a href="#details">the discussion</a> below.</p>'
                '<p id="details">The discussion itself, which belongs right here.</p>'
            )
        }
    )
    with directory:
        assert "See the discussion below." in markdown
        assert "[^" not in markdown
        # Still in its own place, not relocated into a definition list.
        assert "The discussion itself, which belongs right here." in markdown
        assert markdown.index("See the discussion") < markdown.index("The discussion itself")


def test_a_declared_noteref_still_works_without_a_superscript() -> None:
    _, markdown, _, directory = extract(
        {
            "one.xhtml": xhtml(
                '<h1>A</h1><p>Claim<a epub:type="noteref" href="#fn1">1</a>.</p>'
                '<p id="fn1">The note body.</p>'
            )
        }
    )
    with directory:
        assert "Claim[^fn-1-1]." in markdown
        assert "[^fn-1-1]: The note body." in markdown


def test_percent_encoded_hrefs_resolve_to_their_archive_entries() -> None:
    # Package hrefs are URI references, so a space arrives as %20 while the ZIP
    # entry has a real space. Undecoded, the spine document is skipped and a book
    # whose file names all have spaces yields no chapters at all.
    directory = tempfile.TemporaryDirectory()
    with directory:
        root = Path(directory.name)
        epub_path = root / "Encoded.epub"
        with ZipFile(epub_path, "w", ZIP_DEFLATED) as archive:
            archive.writestr("mimetype", "application/epub+zip")
            archive.writestr("META-INF/container.xml", CONTAINER)
            archive.writestr(
                "OEBPS/content.opf",
                package_document(
                    '<item id="d0" href="chapter%201.xhtml"'
                    ' media-type="application/xhtml+xml"/>'
                    '<item id="img" href="my%20image.png" media-type="image/png"/>',
                    '<itemref idref="d0"/>',
                ),
            )
            archive.writestr(
                "OEBPS/chapter 1.xhtml",
                xhtml('<h1>Encoded</h1><p><img src="my%20image.png" alt="Fig"/></p>'),
            )
            archive.writestr("OEBPS/my image.png", PNG)

        result = extract_book(epub_path, root / "out")

        markdown = result.markdown_path.read_text(encoding="utf-8")
        assert result.chapters == 1
        assert markdown.startswith("# Encoded")
        assert result.images == 1
        assert "![Fig](Encoded_assets/my_image.png)" in markdown or "my image.png" in markdown


def test_stems_that_fold_together_do_not_overwrite_each_other() -> None:
    with tempfile.TemporaryDirectory() as name:
        root = Path(name)
        for book in ("A B.epub", "A_B.epub"):
            build_epub(root / book, {"one.xhtml": xhtml(f"<h1>{book}</h1><p>Body.</p>")})

        stems = unique_output_stems(sorted(root.glob("*.epub")))

        assert len(set(stems.values())) == 2
        assert sorted(stems.values()) == ["A_B", "A_B-2"]


def test_a_stem_with_parentheses_cannot_end_the_image_url_early() -> None:
    # `link_url` in the translation engine is `[^)\s]+`, so a `)` in the path
    # ends the protected span and leaves the rest of it open to translation.
    result, markdown, _, directory = extract(
        {"one.xhtml": xhtml('<h1>T</h1><p><img src="images/figure.png" alt=""/></p>')},
        {"images/figure.png": PNG},
        name="Book (2024)",
    )
    with directory:
        assert result.markdown_path.name == "Book_2024.md"
        assert "](Book_2024_assets/figure.png)" in markdown
        reference = markdown.split("](", 1)[1].split(")", 1)[0]
        assert reference == "Book_2024_assets/figure.png"
