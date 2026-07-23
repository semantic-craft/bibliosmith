from __future__ import annotations

import shutil
import subprocess
import tempfile
import unittest
from pathlib import Path
from zipfile import ZipFile


REPO_ROOT = Path(__file__).resolve().parents[1]


class EpubBuilderLandmarksTests(unittest.TestCase):
    def test_chapter_only_book_has_a_bodymatter_landmark(self) -> None:
        with tempfile.TemporaryDirectory() as temporary_directory:
            book_root = Path(temporary_directory) / "book"
            scripts = book_root / "scripts"
            final = book_root / "chapters" / "final"
            scripts.mkdir(parents=True)
            final.mkdir(parents=True)
            source_scripts = REPO_ROOT / "template" / "epub_pipeline" / "common" / "scripts"
            shutil.copy(source_scripts / "build_epub.js", scripts / "build_epub.js")
            shutil.copy(source_scripts / "run_python.js", scripts / "run_python.js")
            (final / "chapter_001.md").write_text(
                "# 第一章\n\n正文。\n", encoding="utf-8"
            )

            completed = subprocess.run(
                ["node", str(scripts / "build_epub.js")],
                check=False,
                capture_output=True,
                text=True,
            )

            self.assertEqual(completed.returncode, 0, completed.stderr)
            with ZipFile(book_root / "output" / "book.epub") as archive:
                nav = archive.read("EPUB/nav.xhtml").decode("utf-8")
            self.assertIn(
                '<li><a epub:type="bodymatter" href="chapter_001.xhtml">正文</a></li>',
                nav,
            )


if __name__ == "__main__":
    unittest.main()
