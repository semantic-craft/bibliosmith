"""EPUB extraction round-trip against a real, builder-produced book (issue #98).

The `epub_source` route claims two things that only hold end to end: a genuine
EPUB extracts back into chapter Markdown, and the chapters it produces are the
boundaries `split-policy-v3` will split on. Both are checked here against an
archive built by the project's own `build_epub.cjs` rather than a hand-written
zip, so the extractor is exercised on the same package/nav/XHTML shapes a
shipped book has.

This suite lives at the repository root next to the other EPUB builder gate
because it spans three trees: the launcher's builder scripts, the OCR package's
extractor, and the split contract they have to agree on.
"""

from __future__ import annotations

import json
import re
import shutil
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path
from zipfile import ZipFile

REPO_ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(REPO_ROOT / "packages" / "ocr" / "scripts"))

from epub_to_markdown import extract_book  # noqa: E402


CHAPTERS = {
    "chapter_001.md": (
        "# The Opening\n\n"
        "First chapter prose that has to survive the round trip.\n\n"
        "## An Inner Section\n\n"
        "A deeper heading that must not become a chapter of its own.\n"
    ),
    # Inline code only: `build_epub.cjs` renders single backticks and nothing
    # else, so a fenced block here would be testing the builder's Markdown
    # support rather than the extractor. Fences are covered where the input is
    # the `<pre><code>` a real EPUB actually carries, in
    # packages/ocr/tests/test_epub_to_markdown.py.
    "chapter_002.md": "# The Middle\n\nSecond chapter prose calling `sample()` inline.\n",
    "chapter_003.md": "# The Close\n\nThird chapter prose.\n",
}


def build_real_epub(book_root: Path) -> Path:
    """Run the project's EPUB builder and hand back the archive it wrote."""
    scripts = book_root / "scripts"
    final = book_root / "chapters" / "final"
    metadata = book_root / "metadata"
    scripts.mkdir(parents=True)
    final.mkdir(parents=True)
    metadata.mkdir(parents=True)

    source_scripts = REPO_ROOT / "tools" / "bibliosmith-launcher" / "source" / "scripts"
    shutil.copy(source_scripts / "build_epub.cjs", scripts / "build_epub.cjs")
    shutil.copy(source_scripts / "run_python.cjs", scripts / "run_python.cjs")
    for name, text in CHAPTERS.items():
        (final / name).write_text(text, encoding="utf-8")
    metadata.joinpath("source_manifest.json").write_text(
        json.dumps({"source_file_name": "Round Trip.epub", "target_language": "zh-Hans"}),
        encoding="utf-8",
    )

    completed = subprocess.run(
        ["node", str(scripts / "build_epub.cjs")],
        cwd=book_root,
        check=False,
        capture_output=True,
        text=True,
    )
    assert completed.returncode == 0, completed.stderr
    return book_root / "output" / "reading" / "book.epub"


def chapter_headings(markdown: str) -> list[str]:
    return re.findall(r"^# (.+)$", markdown, flags=re.MULTILINE)


class EpubExtractRoundTripTests(unittest.TestCase):
    def test_a_builder_produced_epub_extracts_back_into_its_chapters(self) -> None:
        with tempfile.TemporaryDirectory() as temporary_directory:
            root = Path(temporary_directory)
            epub_path = build_real_epub(root / "book")
            self.assertTrue(epub_path.is_file(), "the builder wrote no EPUB")
            # Proof the input really is an EPUB and not a stand-in.
            with ZipFile(epub_path) as archive:
                self.assertIn("META-INF/container.xml", archive.namelist())
                self.assertIn("EPUB/package.opf", archive.namelist())

            result = extract_book(epub_path, root / "extracted")

            markdown = result.markdown_path.read_text(encoding="utf-8")
            self.assertEqual(
                chapter_headings(markdown),
                ["The Opening", "The Middle", "The Close"],
            )
            self.assertEqual(result.chapters, 3)
            self.assertIn("First chapter prose that has to survive the round trip.", markdown)
            self.assertIn("Third chapter prose.", markdown)

    def test_inner_headings_stay_inside_their_chapter(self) -> None:
        with tempfile.TemporaryDirectory() as temporary_directory:
            root = Path(temporary_directory)
            epub_path = build_real_epub(root / "book")

            markdown = extract_book(epub_path, root / "extracted").markdown_path.read_text(
                encoding="utf-8"
            )

            # Demoted, not promoted to a chapter boundary: split-policy-v3 cuts at
            # the shallowest heading level, so a surviving `#` here would tear the
            # first chapter in two.
            self.assertIn("## An Inner Section", markdown)
            self.assertNotIn("An Inner Section", chapter_headings(markdown))
            # Inline code comes back in the form the translation engine protects.
            self.assertIn("`sample()`", markdown)

    def test_the_merged_markdown_is_the_only_registerable_artifact(self) -> None:
        # The extract stage scans the output directory and hands off the first
        # `kind="markdown"` it finds, so exactly one Markdown file may appear.
        with tempfile.TemporaryDirectory() as temporary_directory:
            root = Path(temporary_directory)
            epub_path = build_real_epub(root / "book")
            output_dir = root / "extracted"

            result = extract_book(epub_path, output_dir)

            markdown_files = sorted(path.name for path in output_dir.glob("*.md"))
            self.assertEqual(markdown_files, [result.markdown_path.name])
            self.assertNotIn(" ", result.markdown_path.name)

    def test_extraction_is_repeatable_and_overwrites_a_stale_run(self) -> None:
        # Write-if-missing would pass a "the file exists" check while leaving the
        # previous run's text in place, so the sentinel has to be overwritten.
        with tempfile.TemporaryDirectory() as temporary_directory:
            root = Path(temporary_directory)
            epub_path = build_real_epub(root / "book")
            output_dir = root / "extracted"
            first = extract_book(epub_path, output_dir)
            expected = first.markdown_path.read_text(encoding="utf-8")
            first.markdown_path.write_text("# STALE\n\nLeft over from a previous run.\n", "utf-8")

            second = extract_book(epub_path, output_dir)

            self.assertEqual(second.markdown_path.read_text(encoding="utf-8"), expected)
            self.assertNotIn("STALE", second.markdown_path.read_text(encoding="utf-8"))


if __name__ == "__main__":
    unittest.main()
