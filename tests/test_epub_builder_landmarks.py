from __future__ import annotations

import json
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
            metadata = book_root / "metadata"
            scripts.mkdir(parents=True)
            final.mkdir(parents=True)
            metadata.mkdir(parents=True)
            source_scripts = (
                REPO_ROOT
                / "tools"
                / "bibliosmith-launcher"
                / "source"
                / "scripts"
            )
            shutil.copy(source_scripts / "build_epub.js", scripts / "build_epub.js")
            shutil.copy(source_scripts / "run_python.js", scripts / "run_python.js")
            (final / "chapter_001.md").write_text(
                "# 第一章\n\n正文。\n", encoding="utf-8"
            )
            (metadata / "source_manifest.json").write_text(
                json.dumps(
                    {
                        "source_file_name": "Bilingual Fixture.epub",
                        "target_language": "zh-Hans",
                    }
                ),
                encoding="utf-8",
            )

            completed = subprocess.run(
                ["node", str(scripts / "build_epub.js")],
                check=False,
                capture_output=True,
                text=True,
            )

            self.assertEqual(completed.returncode, 0, completed.stderr)
            with ZipFile(book_root / "output" / "reading" / "book.epub") as archive:
                nav = archive.read("EPUB/nav.xhtml").decode("utf-8")
                package = archive.read("EPUB/package.opf").decode("utf-8")
            self.assertIn(
                '<li><a epub:type="bodymatter" href="chapter_001.xhtml">正文</a></li>',
                nav,
            )
            self.assertIn("<dc:title>Bilingual Fixture</dc:title>", package)
            self.assertIn("<dc:language>zh-Hans</dc:language>", package)
            self.assertNotIn("Unknown", package)
            self.assertNotIn("BiblioSmith 书坊", package)
            self.assertTrue(
                (book_root / "output" / "reading" / "html" / "chapter_001.xhtml").is_file()
            )


if __name__ == "__main__":
    unittest.main()
