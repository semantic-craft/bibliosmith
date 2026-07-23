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
        json.dumps({"source_language": "en", "target_language": "zh-Hans"}),
        encoding="utf-8",
    )
    (root / "metadata/book.yaml").write_text(
        "title: Bilingual Fixture\nauthor: Test Author\n",
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
    chapter = epub_member(epub_path, "EPUB/chapter_001.xhtml")
    body = chapter.split("<body>", 1)[1]

    assert body.index("Chapter") < body.index("第一章")
    assert body.index("Source one.") < body.index("译文一。")
    assert body.index("译文一。") < body.index("Source two.")
    assert chapter.count('class="bitext-unit"') == 3
    package = epub_member(epub_path, "EPUB/package.opf")
    assert "<dc:language>en</dc:language>" in package
    assert "<dc:language>zh-Hans</dc:language>" in package
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
