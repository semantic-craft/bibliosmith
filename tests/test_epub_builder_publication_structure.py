from __future__ import annotations

import base64
import http.server
import json
import re
import subprocess
import threading
from pathlib import Path
from xml.etree import ElementTree as ET
from zipfile import ZipFile

import pytest


REPO_ROOT = Path(__file__).resolve().parents[1]
BUILDER = REPO_ROOT / "tools/bibliosmith-launcher/source/scripts/build_epub.cjs"
AUDITOR = REPO_ROOT / "tools/bibliosmith-launcher/source/scripts/audit_epub.py"
EPUBCHECK_JAR = (
    REPO_ROOT
    / "books/node_modules/epubchecker/vendors/epubcheck-5.2.1/epubcheck.jar"
)


def assert_epubcheck_passes(epub_path: Path, report_path: Path) -> None:
    if not EPUBCHECK_JAR.is_file():
        pytest.skip("books npm dependencies are not installed")
    completed = subprocess.run(
        ["java", "-jar", str(EPUBCHECK_JAR), str(epub_path), "--json", str(report_path), "-q"],
        capture_output=True,
        text=True,
        check=False,
    )
    assert completed.returncode in {0, 1}, completed.stderr
    checker = json.loads(report_path.read_text(encoding="utf-8"))["checker"]
    assert checker["nFatal"] == 0 and checker["nError"] == 0, checker


def write_contract(book_root: Path) -> None:
    (book_root / "chapters/final").mkdir(parents=True)
    (book_root / "metadata").mkdir(parents=True)
    (book_root / "source").mkdir(parents=True)
    (book_root / "source/source.md").write_text(
        "\n".join("fixture" for _ in range(120)) + "\n", encoding="utf-8"
    )
    (book_root / "chapters/final/chapter_001.md").write_text(
        "# 第一部\n\n## 第一章\n\n第一段正文。\n", encoding="utf-8"
    )
    (book_root / "chapters/final/chapter_002.md").write_text(
        "### 第一节\n\n第二段正文。\n", encoding="utf-8"
    )
    (book_root / "chapters/final/chapter_003.md").write_text(
        "#### 第一小节\n\n第三段正文。\n", encoding="utf-8"
    )
    (book_root / "metadata/publication_map.json").write_text(
        json.dumps(
            {
                "schema": "local-reading-publication-map-v1",
                "audit": {"status": "passed", "source": "fixture", "confidence": 1},
                "sections": [
                    {
                        "id": "section_001",
                        "ordinal": 1,
                        "title": "Part One",
                        "shortTitle": "Part One",
                        "readerTitle": "第一部",
                        "readerShortTitle": "第一部",
                        "headingLevel": 1,
                        "parentId": None,
                        "role": "bodymatter",
                        "kind": "part",
                        "sourceStartLine": 1,
                        "sourceEndLine": 15,
                    },
                    {
                        "id": "section_002",
                        "ordinal": 2,
                        "title": "Chapter One",
                        "shortTitle": "Chapter One",
                        "readerTitle": "第一章",
                        "readerShortTitle": "第一章",
                        "headingLevel": 2,
                        "parentId": "section_001",
                        "role": "bodymatter",
                        "kind": "chapter",
                        "sourceStartLine": 1,
                        "sourceEndLine": 15,
                    },
                    {
                        "id": "section_003",
                        "ordinal": 3,
                        "title": "Section One",
                        "shortTitle": "Section One",
                        "readerTitle": "第一节",
                        "readerShortTitle": "第一节",
                        "headingLevel": 3,
                        "parentId": "section_002",
                        "role": "bodymatter",
                        "kind": "section",
                        "sourceStartLine": 6,
                        "sourceEndLine": 15,
                    },
                    {
                        "id": "section_004",
                        "ordinal": 4,
                        "title": "Subsection One",
                        "shortTitle": "Subsection One",
                        "readerTitle": "第一小节",
                        "readerShortTitle": "第一小节",
                        "headingLevel": 4,
                        "parentId": "section_003",
                        "role": "bodymatter",
                        "kind": "section",
                        "sourceStartLine": 11,
                        "sourceEndLine": 15,
                    },
                ],
                "notes": [],
            },
            ensure_ascii=False,
        ),
        encoding="utf-8",
    )
    (book_root / "metadata/source_map.json").write_text(
        json.dumps(
            {
                "schema": "local-reading-source-map-v2",
                "translationUnits": [
                    {"id": "chapter_001", "publicationSectionId": "section_002", "sourceStartLine": 1, "sourceEndLine": 5},
                    {"id": "chapter_002", "publicationSectionId": "section_003", "sourceStartLine": 6, "sourceEndLine": 10},
                    {"id": "chapter_003", "publicationSectionId": "section_004", "sourceStartLine": 11, "sourceEndLine": 15},
                ],
            }
        ),
        encoding="utf-8",
    )
    (book_root / "metadata/source_manifest.json").write_text(
        json.dumps({"target_language": "zh-Hans"}), encoding="utf-8"
    )
    (book_root / "metadata/book.yaml").write_text(
        "title: 测试学术书\nauthor: 测试作者\nlanguage: zh-Hans\n", encoding="utf-8"
    )


def test_translation_units_are_reassembled_behind_nested_publication_navigation(
    tmp_path: Path,
) -> None:
    write_contract(tmp_path)

    completed = subprocess.run(
        ["node", str(BUILDER)], cwd=tmp_path, capture_output=True, text=True, check=False
    )

    assert completed.returncode == 0, completed.stderr
    with ZipFile(tmp_path / "output/reading/book.epub") as archive:
        names = set(archive.namelist())
        nav = archive.read("EPUB/nav.xhtml").decode("utf-8")
        chapter = archive.read("EPUB/section_001.xhtml").decode("utf-8")
        package = archive.read("EPUB/package.opf").decode("utf-8")
    assert "EPUB/chapter_001.xhtml" not in names
    assert "EPUB/chapter_002.xhtml" not in names
    assert nav.count("<ol>") >= 5
    assert "第一部" in nav and "第一章" in nav and "第一节" in nav and "第一小节" in nav
    assert "chapter_001" not in nav and "chapter_002" not in nav
    assert 'id="section_001"' in chapter
    assert 'id="section_002"' in chapter
    assert 'id="section_003"' in chapter
    assert 'id="section_004"' in chapter
    assert chapter.index("第一段正文") < chapter.index("第一节") < chapter.index("第二段正文")
    assert chapter.index("第二段正文") < chapter.index("第一小节") < chapter.index("第三段正文")
    assert "<dc:title>测试学术书</dc:title>" in package
    assert "<dc:creator>测试作者</dc:creator>" in package


@pytest.mark.parametrize("failure", ["parent_depth", "parent_range", "source_upper_bound"])
def test_standard_builder_rejects_an_invalid_publication_tree(
    tmp_path: Path, failure: str
) -> None:
    write_contract(tmp_path)
    publication_path = tmp_path / "metadata/publication_map.json"
    publication = json.loads(publication_path.read_text(encoding="utf-8"))
    if failure == "parent_depth":
        publication["sections"][1]["headingLevel"] = 1
    elif failure == "parent_range":
        publication["sections"][2]["sourceEndLine"] = 16
    else:
        publication["sections"][0]["sourceEndLine"] = 121
    publication_path.write_text(json.dumps(publication), encoding="utf-8")

    completed = subprocess.run(
        ["node", str(BUILDER)],
        cwd=tmp_path,
        capture_output=True,
        text=True,
        check=False,
    )

    assert completed.returncode != 0
    assert any(
        message in completed.stderr
        for message in ["invalid parent depth", "parent source range", "invalid source range"]
    )


def test_standard_builder_packages_a_configured_cover_with_semantic_roles(tmp_path: Path) -> None:
    write_contract(tmp_path)
    (tmp_path / "source").mkdir(exist_ok=True)
    (tmp_path / "source/cover.png").write_bytes(
        base64.b64decode(
            "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mNk+A8AAQUBAScY42YAAAAASUVORK5CYII="
        )
    )
    with (tmp_path / "metadata/book.yaml").open("a", encoding="utf-8") as stream:
        stream.write("cover: source/cover.png\n")

    subprocess.run(["node", str(BUILDER)], cwd=tmp_path, check=True)

    with ZipFile(tmp_path / "output/reading/book.epub") as archive:
        package = archive.read("EPUB/package.opf").decode("utf-8")
        nav = archive.read("EPUB/nav.xhtml").decode("utf-8")
        cover = archive.read("EPUB/cover.xhtml").decode("utf-8")
        names = set(archive.namelist())
    cover_href = re.search(r'<img src="([^"]+)"', cover)
    assert cover_href is not None
    assert re.fullmatch(r"images/[0-9a-f]{64}\.png", cover_href.group(1))
    assert f"EPUB/{cover_href.group(1)}" in names
    assert 'properties="cover-image"' in package
    assert '<itemref idref="cover-page"' in package
    assert '<meta name="cover" content="cover-image"' in package
    assert 'epub:type="cover" href="cover.xhtml"' in nav
    assert 'epub:type="cover" class="publication-cover"' in cover


def test_standard_builder_keeps_same_named_cover_and_body_images_distinct(
    tmp_path: Path,
) -> None:
    write_contract(tmp_path)
    (tmp_path / "source").mkdir(exist_ok=True)
    (tmp_path / "assets").mkdir()
    cover_bytes = b"cover-image-content"
    body_bytes = b"body-image-content"
    (tmp_path / "source/cover.png").write_bytes(cover_bytes)
    (tmp_path / "assets/cover.png").write_bytes(body_bytes)
    with (tmp_path / "metadata/book.yaml").open("a", encoding="utf-8") as stream:
        stream.write("cover: source/cover.png\n")
    with (tmp_path / "chapters/final/chapter_003.md").open("a", encoding="utf-8") as stream:
        stream.write("\n![正文插图](../../assets/cover.png)\n")

    subprocess.run(["node", str(BUILDER)], cwd=tmp_path, check=True)

    with ZipFile(tmp_path / "output/reading/book.epub") as archive:
        cover_page = archive.read("EPUB/cover.xhtml").decode("utf-8")
        body_page = archive.read("EPUB/section_001.xhtml").decode("utf-8")
        package = ET.fromstring(archive.read("EPUB/package.opf"))
        cover_href = re.search(r'<img src="([^"]+)"', cover_page).group(1)
        body_href = re.search(r'<img src="([^"]+)" alt="正文插图"', body_page).group(1)
        assert cover_href != body_href
        assert archive.read(f"EPUB/{cover_href}") == cover_bytes
        assert archive.read(f"EPUB/{body_href}") == body_bytes
    manifest_hrefs = [
        item.get("href", "")
        for item in package.findall(".//{http://www.idpf.org/2007/opf}item")
    ]
    assert len(manifest_hrefs) == len(set(manifest_hrefs))


def test_standard_builder_escapes_untrusted_container_html(tmp_path: Path) -> None:
    write_contract(tmp_path)
    with (tmp_path / "chapters/final/chapter_003.md").open("a", encoding="utf-8") as stream:
        stream.write(
            '\n<div onclick="steal()"><script>steal()</script>'
            '<a href="javascript:steal()">恶意链接</a>'
            '<img src="https://example.test/pixel.png" onerror="steal()" /></div>\n'
        )

    subprocess.run(["node", str(BUILDER)], cwd=tmp_path, check=True)

    with ZipFile(tmp_path / "output/reading/book.epub") as archive:
        chapter = archive.read("EPUB/section_001.xhtml").decode("utf-8")
    document = ET.fromstring(chapter)
    assert "&lt;div" in chapter
    assert all(element.tag.rsplit("}", 1)[-1] != "script" for element in document.iter())
    assert all(
        not any(name.lower().startswith("on") for name in element.attrib)
        for element in document.iter()
    )
    assert all(
        not value.lower().startswith("javascript:")
        for element in document.iter()
        for name, value in element.attrib.items()
        if name.rsplit("}", 1)[-1] == "href"
    )


def test_standard_builder_rejects_noncanonical_contract_ids(tmp_path: Path) -> None:
    cases = [
        "root_section_traversal",
        "section_attribute_text",
        "translation_unit_traversal",
        "note_attribute_text",
        "reference_attribute_text",
    ]
    for case in cases:
        book_root = tmp_path / case
        write_contract(book_root)
        publication_path = book_root / "metadata/publication_map.json"
        source_map_path = book_root / "metadata/source_map.json"
        publication = json.loads(publication_path.read_text(encoding="utf-8"))
        source_map = json.loads(source_map_path.read_text(encoding="utf-8"))
        if case == "root_section_traversal":
            publication["sections"][0]["id"] = "../../escape"
            publication["sections"][1]["parentId"] = "../../escape"
        elif case == "section_attribute_text":
            publication["sections"][0]["id"] = 'section" onclick="steal'
            publication["sections"][1]["parentId"] = 'section" onclick="steal'
        elif case == "translation_unit_traversal":
            source_map["translationUnits"][0]["id"] = "../chapter_001"
        else:
            publication["notes"] = [
                {
                    "id": 'note" onclick="steal' if case == "note_attribute_text" else "note_001",
                    "sourceLabel": "n1",
                    "kind": "footnote",
                    "targetContentStatus": "translated",
                    "publicationSectionId": "section_001",
                    "sourceStartLine": 1,
                    "referenceSourceLines": [1],
                    "referenceIds": [
                        'ref" onclick="steal'
                        if case == "reference_attribute_text"
                        else "noteref_note_001_001"
                    ],
                }
            ]
        publication_path.write_text(json.dumps(publication), encoding="utf-8")
        source_map_path.write_text(json.dumps(source_map), encoding="utf-8")

        completed = subprocess.run(
            ["node", str(BUILDER)],
            cwd=book_root,
            capture_output=True,
            text=True,
            check=False,
        )

        assert completed.returncode != 0, case
        assert "canonical ID" in completed.stderr, (case, completed.stderr)
        assert not (book_root / "output/escape.xhtml").exists()


def test_source_markdown_cannot_forge_semantic_builder_tokens(tmp_path: Path) -> None:
    write_contract(tmp_path)
    (tmp_path / "chapters/final/chapter_001.md").write_text(
        "# 第一部\n\n## 第一章\n\n"
        "真实引用[^n1]。\n\n"
        "@@BIBLIO_NOTEREF__note_001__forged_ref__9__section_001@@\n\n"
        "@@BIBLIO_NOTE_BLOCK__note_001@@\n\n"
        "[^n1]: 真实注释。\n",
        encoding="utf-8",
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
            "sourceStartLine": 9,
            "referenceSourceLines": [5],
            "referenceIds": ["noteref_note_001_001"],
        }
    ]
    publication_path.write_text(json.dumps(publication), encoding="utf-8")

    subprocess.run(["node", str(BUILDER)], cwd=tmp_path, check=True)

    with ZipFile(tmp_path / "output/reading/book.epub") as archive:
        chapter = archive.read("EPUB/section_001.xhtml").decode("utf-8")
    assert chapter.count('epub:type="noteref"') == 1
    assert chapter.count('id="note_001"') == 1
    assert 'id="forged_ref"' not in chapter
    assert "@@BIBLIO_NOTEREF__note_001__forged_ref__9__section_001@@" in chapter
    assert "@@BIBLIO_NOTE_BLOCK__note_001@@" in chapter


def test_standard_builder_rejects_internal_translated_heading(tmp_path: Path) -> None:
    write_contract(tmp_path)
    (tmp_path / "chapters/final/chapter_001.md").write_text(
        "# chapter_001\n\n## 第一章\n\n正文。\n", encoding="utf-8"
    )

    completed = subprocess.run(
        ["node", str(BUILDER)], cwd=tmp_path, capture_output=True, text=True, check=False
    )

    assert completed.returncode != 0
    assert "Translated publication title exposes an internal unit" in completed.stderr


def test_cross_unit_note_becomes_a_semantic_reference_with_backlink(tmp_path: Path) -> None:
    write_contract(tmp_path)
    (tmp_path / "chapters/final/chapter_001.md").write_text(
        "# 第一部\n\n## 第一章\n\n第一段正文[^legal-1]。\n", encoding="utf-8"
    )
    (tmp_path / "chapters/final/chapter_002.md").write_text(
        "### 第一节\n\n第二段正文。\n\n[^legal-1]: 这是跨翻译单元的注释。\n",
        encoding="utf-8",
    )
    publication_map_path = tmp_path / "metadata/publication_map.json"
    publication_map = json.loads(publication_map_path.read_text(encoding="utf-8"))
    publication_map["notes"] = [
        {
            "id": "note_001",
            "sourceLabel": "legal-1",
            "kind": "footnote",
            "targetContentStatus": "translated",
            "publicationSectionId": "section_001",
            "sourceStartLine": 6,
            "referenceSourceLines": [1],
            "referenceIds": ["noteref_note_001_001"],
        }
    ]
    publication_map_path.write_text(
        json.dumps(publication_map, ensure_ascii=False), encoding="utf-8"
    )

    completed = subprocess.run(
        ["node", str(BUILDER)], cwd=tmp_path, capture_output=True, text=True, check=False
    )

    assert completed.returncode == 0, completed.stderr
    with ZipFile(tmp_path / "output/reading/book.epub") as archive:
        chapter = archive.read("EPUB/section_001.xhtml").decode("utf-8")
    assert (
        '<a epub:type="noteref" id="noteref_note_001_001" href="section_001.xhtml#note_001">[1]</a>'
        in chapter
    )
    assert 'epub:type="footnote"' in chapter and 'id="note_001"' in chapter
    assert 'href="section_001.xhtml#noteref_note_001_001"' in chapter
    assert "这是跨翻译单元的注释" in chapter
    assert "[^legal-1]" not in chapter


def test_book_endnote_links_across_bodymatter_and_backmatter_documents(tmp_path: Path) -> None:
    write_contract(tmp_path)
    (tmp_path / "chapters/final/chapter_001.md").write_text(
        "# 第一部\n\n## 第一章\n\n正文引用[^book-end]。\n", encoding="utf-8"
    )
    (tmp_path / "chapters/final/chapter_004.md").write_text(
        "# 全书注释\n\n[^book-end]: 书末注正文。\n", encoding="utf-8"
    )
    publication_path = tmp_path / "metadata/publication_map.json"
    publication = json.loads(publication_path.read_text(encoding="utf-8"))
    publication["sections"].append(
            {
                "id": "section_005",
                "ordinal": 5,
            "title": "Book Notes",
            "shortTitle": "Notes",
            "readerTitle": "全书注释",
            "readerShortTitle": "注释",
            "headingLevel": 1,
            "parentId": None,
            "role": "backmatter",
                "kind": "notes",
                "sourceStartLine": 100,
                "sourceEndLine": 110,
            }
    )
    publication["notes"] = [
        {
            "id": "note_001",
            "ordinal": 1,
            "sourceLabel": "book-end",
            "kind": "endnote",
            "targetContentStatus": "translated",
            "publicationSectionId": "section_005",
            "sourceStartLine": 101,
            "referenceSourceLines": [3],
            "referenceIds": ["noteref_note_001_001"],
        }
    ]
    publication_path.write_text(json.dumps(publication, ensure_ascii=False), encoding="utf-8")
    source_map_path = tmp_path / "metadata/source_map.json"
    source_map = json.loads(source_map_path.read_text(encoding="utf-8"))
    source_map["translationUnits"].append(
        {
            "id": "chapter_004",
            "publicationSectionId": "section_005",
            "sourceStartLine": 100,
            "sourceEndLine": 110,
        }
    )
    source_map_path.write_text(json.dumps(source_map), encoding="utf-8")

    completed = subprocess.run(
        ["node", str(BUILDER)], cwd=tmp_path, capture_output=True, text=True, check=False
    )

    assert completed.returncode == 0, completed.stderr
    with ZipFile(tmp_path / "output/reading/book.epub") as archive:
        body = archive.read("EPUB/section_001.xhtml").decode("utf-8")
        notes = archive.read("EPUB/section_005.xhtml").decode("utf-8")
    assert 'href="section_005.xhtml#note_001"' in body
    assert 'href="section_001.xhtml#noteref_note_001_001"' in notes
    assert 'epub:type="endnote"' in notes


def test_structural_audit_passes_the_publication_contract(tmp_path: Path) -> None:
    write_contract(tmp_path)
    subprocess.run(["node", str(BUILDER)], cwd=tmp_path, check=True)
    report_path = tmp_path / "output/structural_readability.json"

    completed = subprocess.run(
        [
            "python3",
            str(AUDITOR),
            "--epub",
            str(tmp_path / "output/reading/book.epub"),
            "--publication-map",
            str(tmp_path / "metadata/publication_map.json"),
            "--output",
            str(report_path),
        ],
        capture_output=True,
        text=True,
        check=False,
    )

    assert completed.returncode == 0, completed.stderr
    report = json.loads(report_path.read_text(encoding="utf-8"))
    assert report["status"] == "passed", report["findings"]
    assert all(report["checks"].values())
    assert [item["width"] for item in report["metrics"]["viewportSmoke"]] == [390, 430]
    assert all(item["status"] == "passed" for item in report["metrics"]["viewportSmoke"])
    assert all(item["documents"] == 1 for item in report["metrics"]["viewportSmoke"])
    assert all(item["renderer"] for item in report["metrics"]["viewportSmoke"])
    assert_epubcheck_passes(
        tmp_path / "output/reading/book.epub", tmp_path / "output/fixed-epubcheck.json"
    )


def test_structural_audit_rejects_manifest_collisions_and_missing_targets(
    tmp_path: Path,
) -> None:
    write_contract(tmp_path)
    subprocess.run(["node", str(BUILDER)], cwd=tmp_path, check=True)
    source_epub = tmp_path / "output/reading/book.epub"
    broken_epub = tmp_path / "output/reading/broken-manifest.epub"
    with ZipFile(source_epub) as source, ZipFile(broken_epub, "w") as target:
        for info in source.infolist():
            content = source.read(info.filename)
            if info.filename == "EPUB/package.opf":
                content = content.decode("utf-8").replace(
                    "</manifest>",
                    '<item id="nav" href="duplicate.xhtml" media-type="application/xhtml+xml" />'
                    '<item id="duplicate-href" href="nav.xhtml" media-type="application/xhtml+xml" />'
                    '<item id="missing-target" href="missing.bin" media-type="application/octet-stream" />'
                    '<item id="" href="styles/book.css" media-type="text/css" />'
                    '<item id="empty-href" href="" media-type="application/octet-stream" />'
                    "</manifest>",
                ).encode("utf-8")
            target.writestr(info, content)

    report_path = tmp_path / "output/broken-manifest-audit.json"
    subprocess.run(
        [
            "python3",
            str(AUDITOR),
            "--epub",
            str(broken_epub),
            "--publication-map",
            str(tmp_path / "metadata/publication_map.json"),
            "--output",
            str(report_path),
        ],
        check=True,
    )

    report = json.loads(report_path.read_text(encoding="utf-8"))
    codes = {finding["code"] for finding in report["findings"]}
    assert report["status"] == "failed"
    assert {
        "manifest.id",
        "manifest.href",
        "manifest.duplicate_id",
        "manifest.duplicate_href",
    } <= codes
    assert report["checks"]["packageManifest"] is False


def test_structural_audit_checks_ids_and_local_urls_in_every_packaged_xhtml(
    tmp_path: Path,
) -> None:
    write_contract(tmp_path)
    subprocess.run(["node", str(BUILDER)], cwd=tmp_path, check=True)
    source_epub = tmp_path / "output/reading/book.epub"
    broken_epub = tmp_path / "output/reading/broken-auxiliary-xhtml.epub"
    auxiliary = """<?xml version="1.0" encoding="utf-8"?>
<html xmlns="http://www.w3.org/1999/xhtml">
  <head><title>Auxiliary</title><link rel="stylesheet" href="broken.css" /></head>
  <body>
    <object id="section_001" data="missing-object.bin">
      <video poster="missing-poster.png">
        <source src="missing-source.mp4" srcset="missing-one.png 1x, missing-two.png 2x" />
      </video>
      <a href="missing-page.xhtml#missing">broken local link</a>
      <a href="file:///tmp/secret">unsafe local-file link</a>
      <a href="javascript:alert(1)">unsafe executable link</a>
      <img src="symbols.svg#missing-symbol" alt="missing SVG fragment" />
    </object>
  </body>
</html>
"""
    with ZipFile(source_epub) as source, ZipFile(broken_epub, "w") as target:
        for info in source.infolist():
            content = source.read(info.filename)
            if info.filename == "EPUB/package.opf":
                content = content.decode("utf-8").replace(
                    "</manifest>",
                    '<item id="auxiliary" href="auxiliary.xhtml" media-type="application/xhtml+xml" />'
                    '<item id="auxiliary-css" href="broken.css" media-type="text/css" />'
                    '<item id="symbols" href="symbols.svg" media-type="image/svg+xml" />'
                    "</manifest>",
                ).encode("utf-8")
            target.writestr(info, content)
        target.writestr("EPUB/auxiliary.xhtml", auxiliary)
        target.writestr(
            "EPUB/broken.css",
            '@import "missing-import.css"; body { background: url(missing-css.png); }',
        )
        target.writestr(
            "EPUB/symbols.svg",
            '<svg xmlns="http://www.w3.org/2000/svg"><path id="present" /></svg>',
        )

    report_path = tmp_path / "output/broken-auxiliary-xhtml-audit.json"
    subprocess.run(
        [
            "python3",
            str(AUDITOR),
            "--epub",
            str(broken_epub),
            "--publication-map",
            str(tmp_path / "metadata/publication_map.json"),
            "--output",
            str(report_path),
        ],
        check=True,
    )

    report = json.loads(report_path.read_text(encoding="utf-8"))
    codes = {finding["code"] for finding in report["findings"]}
    messages = "\n".join(finding["message"] for finding in report["findings"])
    assert report["status"] == "failed"
    assert "xhtml.duplicate_id" in codes
    assert "resource.href" in codes
    assert report["checks"]["xhtmlIntegrity"] is False
    assert report["checks"]["resourceResolution"] is False
    assert sum(
        finding["code"] == "resource.external" for finding in report["findings"]
    ) == 2
    for missing in (
        "missing-object.bin",
        "missing-poster.png",
        "missing-source.mp4",
        "missing-one.png",
        "missing-two.png",
        "missing-page.xhtml",
        "missing-symbol",
        "missing-import.css",
        "missing-css.png",
    ):
        assert missing in messages


def test_structural_audit_accepts_an_inline_data_url_srcset(tmp_path: Path) -> None:
    write_contract(tmp_path)
    subprocess.run(["node", str(BUILDER)], cwd=tmp_path, check=True)
    source_epub = tmp_path / "output/reading/book.epub"
    data_srcset_epub = tmp_path / "output/reading/data-srcset.epub"
    with ZipFile(source_epub) as source, ZipFile(data_srcset_epub, "w") as target:
        for info in source.infolist():
            content = source.read(info.filename)
            if info.filename == "EPUB/section_001.xhtml":
                content = content.decode("utf-8").replace(
                    "</main>",
                    '<img src="data:image/png;base64,AAAA" '
                    'srcset="data:image/png;base64,AAAA 1x" alt="inline" /></main>',
                ).encode("utf-8")
            target.writestr(info, content)

    report_path = tmp_path / "output/data-srcset-audit.json"
    subprocess.run(
        [
            "python3",
            str(AUDITOR),
            "--epub",
            str(data_srcset_epub),
            "--publication-map",
            str(tmp_path / "metadata/publication_map.json"),
            "--output",
            str(report_path),
        ],
        check=True,
    )

    report = json.loads(report_path.read_text(encoding="utf-8"))
    assert report["status"] == "passed", report["findings"]


def test_structural_audit_rejects_external_resources_without_requesting_them(
    tmp_path: Path,
) -> None:
    requests: list[str] = []

    class ProbeHandler(http.server.BaseHTTPRequestHandler):
        def do_GET(self) -> None:  # noqa: N802 - stdlib handler contract
            requests.append(self.path)
            self.send_response(200)
            self.send_header("Content-Type", "image/png")
            self.end_headers()
            self.wfile.write(b"not-a-real-image")

        def log_message(self, *_args: object) -> None:
            return

    server = http.server.ThreadingHTTPServer(("127.0.0.1", 0), ProbeHandler)
    server_thread = threading.Thread(target=server.serve_forever, daemon=True)
    server_thread.start()
    try:
        write_contract(tmp_path)
        subprocess.run(["node", str(BUILDER)], cwd=tmp_path, check=True)
        source_epub = tmp_path / "output/reading/book.epub"
        hostile_epub = tmp_path / "output/reading/external-resource.epub"
        remote = f"http://127.0.0.1:{server.server_port}/probe.png"
        outside_css = tmp_path / "outside-epub.css"
        outside_css.write_text("main{display:none!important}", encoding="utf-8")
        with ZipFile(source_epub) as source, ZipFile(hostile_epub, "w") as target:
            for info in source.infolist():
                content = source.read(info.filename)
                if info.filename == "EPUB/section_001.xhtml":
                    text = content.decode("utf-8").replace(
                        "</head>",
                        f'<link rel="stylesheet" href="{outside_css.as_uri()}" /></head>',
                    ).replace(
                        "</main>",
                        f'<img src="{remote}" alt="probe" />'
                        '<img src="https://example.invalid/probe.png" alt="https" />'
                        '<img src="ftp://example.invalid/probe.png" alt="ftp" />'
                        '<img src="ftps://example.invalid/probe.png" alt="ftps" />'
                        '<img src="ws://example.invalid/probe.png" alt="ws" />'
                        '<img src="wss://example.invalid/probe.png" alt="wss" />'
                        '<img src="//example.invalid/probe.png" alt="relative-network" />'
                        '<img src="data:image/png;base64,aGVsbG8=" alt="inline" />'
                        "<script>document.querySelector('main').remove();</script></main>",
                    )
                    content = text.encode("utf-8")
                target.writestr(info, content)

        report_path = tmp_path / "output/external-resource-audit.json"
        completed = subprocess.run(
            [
                "python3",
                str(AUDITOR),
                "--epub",
                str(hostile_epub),
                "--publication-map",
                str(tmp_path / "metadata/publication_map.json"),
                "--output",
                str(report_path),
            ],
            capture_output=True,
            text=True,
            check=False,
        )
        report = json.loads(report_path.read_text(encoding="utf-8"))
    finally:
        server.shutdown()
        server.server_close()
        server_thread.join(timeout=5)

    assert completed.returncode == 0, completed.stderr
    assert sum(item["code"] == "resource.external" for item in report["findings"]) == 8
    assert requests == []
    assert all(item["status"] == "passed" for item in report["metrics"]["viewportSmoke"])


def test_structural_audit_rejects_absolute_and_root_escaping_resource_paths(
    tmp_path: Path,
) -> None:
    write_contract(tmp_path)
    subprocess.run(["node", str(BUILDER)], cwd=tmp_path, check=True)
    source_epub = tmp_path / "output/reading/book.epub"
    hostile_epub = tmp_path / "output/reading/path-escape.epub"
    with ZipFile(source_epub) as source, ZipFile(hostile_epub, "w") as target:
        for info in source.infolist():
            content = source.read(info.filename)
            if info.filename == "EPUB/package.opf":
                content = content.decode("utf-8").replace(
                    "</manifest>",
                    '<item id="escape" href="../../outside.png" media-type="image/png" />'
                    "</manifest>",
                ).encode("utf-8")
            elif info.filename == "EPUB/section_001.xhtml":
                content = content.decode("utf-8").replace(
                    "</main>",
                    '<img src="/absolute.png" alt="absolute" />'
                    '<img src="\\absolute.png" alt="backslash" />'
                    '<img src="../../outside.png" alt="escape" /></main>',
                ).encode("utf-8")
            target.writestr(info, content)

    report_path = tmp_path / "output/path-escape-audit.json"
    subprocess.run(
        [
            "python3",
            str(AUDITOR),
            "--epub",
            str(hostile_epub),
            "--publication-map",
            str(tmp_path / "metadata/publication_map.json"),
            "--output",
            str(report_path),
        ],
        check=True,
    )

    report = json.loads(report_path.read_text(encoding="utf-8"))
    assert report["status"] == "failed"
    assert sum(item["code"] == "resource.external" for item in report["findings"]) == 4
    assert all(item["status"] == "passed" for item in report["metrics"]["viewportSmoke"])


def test_multiline_endnote_with_multiple_references_closes_every_link(tmp_path: Path) -> None:
    write_contract(tmp_path)
    (tmp_path / "chapters/final/chapter_001.md").write_text(
        "# 第一部\n\n## 第一章\n\n第一处[^end-1]，第二处[^end-1]。\n",
        encoding="utf-8",
    )
    (tmp_path / "chapters/final/chapter_002.md").write_text(
        "### 第一节\n\n正文。\n\n[^end-1]: 第一段章末注。\n"
        "    第二段含[链接](https://example.test)。\n",
        encoding="utf-8",
    )
    publication_map_path = tmp_path / "metadata/publication_map.json"
    publication_map = json.loads(publication_map_path.read_text(encoding="utf-8"))
    publication_map["notes"] = [
        {
            "id": "note_001",
            "sourceLabel": "end-1",
            "kind": "endnote",
            "targetContentStatus": "translated",
            "publicationSectionId": "section_001",
            "sourceStartLine": 6,
            "referenceSourceLines": [1, 1],
            "referenceIds": ["noteref_note_001_001", "noteref_note_001_002"],
            "backlinkIds": ["noteref_note_001_001", "noteref_note_001_002"],
        }
    ]
    publication_map_path.write_text(json.dumps(publication_map, ensure_ascii=False), encoding="utf-8")

    subprocess.run(["node", str(BUILDER)], cwd=tmp_path, check=True)
    report_path = tmp_path / "output/endnote-audit.json"
    subprocess.run(
        [
            "python3",
            str(AUDITOR),
            "--epub",
            str(tmp_path / "output/reading/book.epub"),
            "--publication-map",
            str(publication_map_path),
            "--output",
            str(report_path),
        ],
        check=True,
    )

    with ZipFile(tmp_path / "output/reading/book.epub") as archive:
        chapter = archive.read("EPUB/section_001.xhtml").decode("utf-8")
    assert chapter.count('epub:type="noteref"') == 2
    assert 'epub:type="endnote"' in chapter and 'id="note_001"' in chapter
    assert chapter.count('epub:type="backlink"') == 2
    assert "第二段含" in chapter
    assert '<a href="https://example.test">链接</a>' in chapter
    report = json.loads(report_path.read_text(encoding="utf-8"))
    assert report["status"] == "passed", report["findings"]


def test_structural_audit_rejects_misdirected_note_links(tmp_path: Path) -> None:
    write_contract(tmp_path)
    (tmp_path / "chapters/final/chapter_001.md").write_text(
        "# 第一部\n\n## 第一章\n\n正文[^n1]。\n\n[^n1]: 注释正文。\n",
        encoding="utf-8",
    )
    publication_map_path = tmp_path / "metadata/publication_map.json"
    publication_map = json.loads(publication_map_path.read_text(encoding="utf-8"))
    publication_map["notes"] = [
        {
            "id": "note_001",
            "sourceLabel": "n1",
            "kind": "footnote",
            "targetContentStatus": "translated",
            "publicationSectionId": "section_001",
            "sourceStartLine": 1,
            "referenceSourceLines": [1],
            "referenceIds": ["noteref_note_001_001"],
        }
    ]
    publication_map_path.write_text(json.dumps(publication_map), encoding="utf-8")
    subprocess.run(["node", str(BUILDER)], cwd=tmp_path, check=True)
    good_epub = tmp_path / "output/reading/book.epub"
    broken_epub = tmp_path / "output/reading/misdirected.epub"
    with ZipFile(good_epub) as source, ZipFile(broken_epub, "w") as target:
        for info in source.infolist():
            content = source.read(info.filename)
            if info.filename == "EPUB/section_001.xhtml":
                text = content.decode("utf-8")
                text = text.replace('href="section_001.xhtml#note_001"', 'href="#wrong_note"', 1)
                text = text.replace(
                    'href="section_001.xhtml#noteref_note_001_001"', 'href="#wrong_reference"', 1
                )
                content = text.encode("utf-8")
            target.writestr(info, content)
    report_path = tmp_path / "output/misdirected-note-audit.json"
    subprocess.run(
        [
            "python3",
            str(AUDITOR),
            "--epub",
            str(broken_epub),
            "--publication-map",
            str(publication_map_path),
            "--output",
            str(report_path),
        ],
        check=True,
    )

    report = json.loads(report_path.read_text(encoding="utf-8"))
    codes = {finding["code"] for finding in report["findings"]}
    assert report["status"] == "failed"
    assert "notes.reference_target" in codes
    assert "notes.note_backlinks" in codes


def test_structural_audit_rejects_generic_navigation_and_empty_body(tmp_path: Path) -> None:
    write_contract(tmp_path)
    subprocess.run(["node", str(BUILDER)], cwd=tmp_path, check=True)
    publication_map_path = tmp_path / "metadata/publication_map.json"
    publication_map = json.loads(publication_map_path.read_text(encoding="utf-8"))
    publication_map["notes"] = [
        {
            "id": "note_001",
            "sourceLabel": "missing",
            "kind": "footnote",
            "targetContentStatus": "translated",
            "publicationSectionId": "section_001",
            "sourceStartLine": 1,
            "referenceSourceLines": [1],
            "referenceIds": ["noteref_note_001_001"],
        }
    ]
    publication_map_path.write_text(json.dumps(publication_map), encoding="utf-8")
    epub_path = tmp_path / "output/reading/book.epub"
    replacement = tmp_path / "broken.epub"
    with ZipFile(epub_path) as source, ZipFile(replacement, "w") as target:
        for info in source.infolist():
            content = source.read(info.filename)
            if info.filename == "EPUB/nav.xhtml":
                text = content.decode("utf-8")
                toc_start = text.index('<nav epub:type="toc"')
                ol_start = text.index("<ol>", toc_start) + len("<ol>")
                ol_end = text.index("</ol></nav>", ol_start)
                generic = "".join(
                    f'<li><a href="section_001.xhtml#section_001">chapter_{index:03}</a></li>'
                    for index in range(1, 14)
                )
                text = text[:ol_start] + generic + text[ol_end:]
                content = text.encode("utf-8")
            elif info.filename == "EPUB/section_001.xhtml":
                text = content.decode("utf-8")
                body_start = text.index("<body")
                body_open_end = text.index(">", body_start) + 1
                body_end = text.index("</body>")
                empty_structure = (
                    '<h1 id="section_001">这是一个足够长、会掩盖空正文的学术专著标题</h1><h2 id="section_002"></h2>'
                    '<h3 id="section_003"></h3><h4 id="section_004"></h4>'
                    "<p>迈克尔·非常非常长的作者署名</p>"
                )
                content = (text[:body_open_end] + empty_structure + text[body_end:]).encode("utf-8")
            target.writestr(info, content)
    report_path = tmp_path / "output/broken-structure.json"

    subprocess.run(
        [
            "python3",
            str(AUDITOR),
            "--epub",
            str(replacement),
            "--publication-map",
            str(publication_map_path),
            "--output",
            str(report_path),
        ],
        check=True,
    )

    report = json.loads(report_path.read_text(encoding="utf-8"))
    assert report["status"] == "failed"
    codes = {finding["code"] for finding in report["findings"]}
    assert "navigation.label" in codes
    assert "landmarks.body_content" in codes
    assert "heading.empty" in codes
    assert "notes.references" in codes
    assert "notes.bodies" in codes
    assert report["metrics"]["navigationEntries"] == 13
    assert_epubcheck_passes(replacement, tmp_path / "output/broken-epubcheck.json")
