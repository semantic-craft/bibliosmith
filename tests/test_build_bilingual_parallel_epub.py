from __future__ import annotations

import json
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path
from zipfile import ZIP_STORED, ZipFile


REPO_ROOT = Path(__file__).resolve().parents[1]
SCRIPT = (
    REPO_ROOT
    / "template"
    / "epub_pipeline"
    / "common"
    / "scripts"
    / "build_bilingual_parallel_epub.py"
)


def write_text(path: Path, text: str) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(text, encoding="utf-8")


def write_book(
    book_root: Path,
    *,
    state: dict,
    alignment: dict | list | None,
    source: str = "# Chapter One\n\nSource one.\n\nSource two.\n",
    target: str = "# 第一章\n\n译文一。\n\n译文二。\n",
) -> None:
    write_text(book_root / "state" / "pipeline_state.json", json.dumps(state))
    write_text(
        book_root / "metadata" / "book.yaml",
        "title: Parallel Fixture\nauthor: Test Author\n",
    )
    write_text(book_root / "chapters" / "src" / "chapter_001.md", source)
    write_text(book_root / "chapters" / "final" / "chapter_001.md", target)
    if alignment is not None:
        write_text(
            book_root / "qa" / "bilingual_parallel" / "alignment_map.json",
            json.dumps(alignment),
        )


def run_builder(book_root: Path) -> subprocess.CompletedProcess[str]:
    return subprocess.run(
        [sys.executable, str(SCRIPT), "--book-root", str(book_root)],
        check=False,
        capture_output=True,
        text=True,
    )


def enabled_state() -> dict:
    return {
        "edition_type": "bilingual_parallel",
        "source_language": "en",
        "target_language": "zh-Hans",
    }


def paired_alignment() -> dict:
    return {
        "alignment_units": [
            {
                "id": "u0001",
                "chapter": "chapters/final/chapter_001.md",
                "source_paragraphs": ["s0001"],
                "target_paragraphs": ["t0001"],
            },
            {
                "id": "u0002",
                "chapter": "chapters/final/chapter_001.md",
                "source_paragraphs": ["s0002"],
                "target_paragraphs": ["t0002"],
            },
        ]
    }


class BilingualParallelBuilderTests(unittest.TestCase):
    def test_skips_when_the_bilingual_edition_is_disabled(self) -> None:
        with tempfile.TemporaryDirectory() as temporary_directory:
            book_root = Path(temporary_directory) / "book"
            write_book(
                book_root,
                state={"edition_type": "target_only"},
                alignment=paired_alignment(),
            )

            completed = run_builder(book_root)

            self.assertEqual(completed.returncode, 0, completed.stderr)
            self.assertIn("bilingual EPUB build skipped", completed.stdout)
            self.assertFalse(
                (book_root / "output" / "book_bilingual_parallel.epub").exists()
            )

    def test_builds_the_parallel_edition_from_the_alignment_map(self) -> None:
        with tempfile.TemporaryDirectory() as temporary_directory:
            book_root = Path(temporary_directory) / "book"
            write_book(
                book_root,
                state=enabled_state(),
                alignment=paired_alignment(),
            )
            write_text(
                book_root / "frontmatter" / "book_info.md",
                "# 版本说明\n\n本书为双语对照版。\n",
            )

            completed = run_builder(book_root)

            self.assertEqual(completed.returncode, 0, completed.stderr)
            epub_path = book_root / "output" / "book_bilingual_parallel.epub"
            self.assertTrue(epub_path.is_file())
            self.assertIn("wrote output/book_bilingual_parallel.epub", completed.stdout)

            with ZipFile(epub_path) as archive:
                self.assertEqual(
                    archive.getinfo("mimetype").compress_type, ZIP_STORED
                )
                names = set(archive.namelist())
                chapter = archive.read("EPUB/bilingual_chapter_001.xhtml").decode("utf-8")
                package = archive.read("EPUB/package.opf").decode("utf-8")
                nav = archive.read("EPUB/nav.xhtml").decode("utf-8")

            self.assertIn("EPUB/book_info.xhtml", names)
            self.assertEqual(chapter.count('class="bitext-unit"'), 2)
            self.assertIn('data-align-id="u0001"', chapter)
            self.assertLess(chapter.index("Source one."), chapter.index("译文一。"))
            self.assertLess(chapter.index("译文一。"), chapter.index("Source two."))
            self.assertIn('<dc:language>zh-Hans</dc:language>', package)
            self.assertIn('<dc:language>en</dc:language>', package)
            self.assertIn("（双语对照版）", package)
            self.assertIn('href="bilingual_chapter_001.xhtml"', nav)

    def test_alignment_units_may_carry_their_own_text(self) -> None:
        with tempfile.TemporaryDirectory() as temporary_directory:
            book_root = Path(temporary_directory) / "book"
            write_book(
                book_root,
                state=enabled_state(),
                alignment={
                    "alignment_units": [
                        {
                            "id": "u0001",
                            "chapter": "chapters/final/chapter_001.md",
                            "source_text": "Inline source.",
                            "target_text": "行内译文。",
                        }
                    ]
                },
            )

            completed = run_builder(book_root)

            self.assertEqual(completed.returncode, 0, completed.stderr)
            with ZipFile(book_root / "output" / "book_bilingual_parallel.epub") as archive:
                chapter = archive.read("EPUB/bilingual_chapter_001.xhtml").decode("utf-8")
            self.assertIn("Inline source.", chapter)
            self.assertIn("行内译文。", chapter)

    def test_explicit_paragraph_ids_are_resolvable(self) -> None:
        with tempfile.TemporaryDirectory() as temporary_directory:
            book_root = Path(temporary_directory) / "book"
            write_book(
                book_root,
                state=enabled_state(),
                alignment={
                    "alignment_units": [
                        {
                            "id": "u0001",
                            "chapter": "chapters/final/chapter_001.md",
                            "source_paragraphs": ["src-1"],
                            "target_paragraphs": ["tgt-1"],
                        }
                    ]
                },
                source="# Chapter One\n\n[id:src-1] Source one.\n",
                target="# 第一章\n\n译文一。{#tgt-1}\n",
            )

            completed = run_builder(book_root)

            self.assertEqual(completed.returncode, 0, completed.stderr)
            with ZipFile(book_root / "output" / "book_bilingual_parallel.epub") as archive:
                chapter = archive.read("EPUB/bilingual_chapter_001.xhtml").decode("utf-8")
            self.assertIn("Source one.", chapter)
            self.assertIn("译文一。", chapter)
            self.assertNotIn("[id:src-1]", chapter)
            self.assertNotIn("{#tgt-1}", chapter)

    def test_missing_alignment_map_fails(self) -> None:
        with tempfile.TemporaryDirectory() as temporary_directory:
            book_root = Path(temporary_directory) / "book"
            write_book(book_root, state=enabled_state(), alignment=None)

            completed = run_builder(book_root)

            self.assertNotEqual(completed.returncode, 0)
            self.assertIn("Missing bilingual alignment map", completed.stderr)
            self.assertFalse(
                (book_root / "output" / "book_bilingual_parallel.epub").exists()
            )

    def test_unknown_paragraph_id_fails(self) -> None:
        with tempfile.TemporaryDirectory() as temporary_directory:
            book_root = Path(temporary_directory) / "book"
            write_book(
                book_root,
                state=enabled_state(),
                alignment={
                    "alignment_units": [
                        {
                            "id": "u0001",
                            "chapter": "chapters/final/chapter_001.md",
                            "source_paragraphs": ["s9999"],
                            "target_paragraphs": ["t0001"],
                        }
                    ]
                },
            )

            completed = run_builder(book_root)

            self.assertNotEqual(completed.returncode, 0)
            self.assertIn("references missing source_paragraphs id: s9999", completed.stderr)

    def test_output_editions_can_override_the_artifact_path(self) -> None:
        with tempfile.TemporaryDirectory() as temporary_directory:
            book_root = Path(temporary_directory) / "book"
            write_book(
                book_root,
                state={
                    "source_language": "en",
                    "target_language": "zh-Hans",
                    "output_editions": [
                        {
                            "edition_type": "bilingual_parallel",
                            "enabled": True,
                            "artifact": "output/custom_bilingual.epub",
                        }
                    ],
                },
                alignment=paired_alignment(),
            )

            completed = run_builder(book_root)

            self.assertEqual(completed.returncode, 0, completed.stderr)
            self.assertTrue((book_root / "output" / "custom_bilingual.epub").is_file())
            self.assertFalse(
                (book_root / "output" / "book_bilingual_parallel.epub").exists()
            )

    def test_the_runner_builder_is_a_separate_tool(self) -> None:
        """The launcher runner has its own builder; neither may replace the other."""
        runner_builder = (
            REPO_ROOT
            / "tools"
            / "bibliosmith-launcher"
            / "source"
            / "scripts"
            / "build_bilingual_epub.py"
        )

        self.assertTrue(SCRIPT.is_file())
        self.assertTrue(runner_builder.is_file())
        self.assertIn(
            "output/book_bilingual_parallel.epub",
            SCRIPT.read_text(encoding="utf-8"),
        )
        self.assertIn(
            'project_root / "output" / "book_bilingual.epub"',
            runner_builder.read_text(encoding="utf-8"),
        )


if __name__ == "__main__":
    unittest.main()
