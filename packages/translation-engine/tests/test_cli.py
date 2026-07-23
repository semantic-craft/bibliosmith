import json
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest

from tests.fixtures import build_run_fixture


class TranslationEngineCliTests(unittest.TestCase):
    def test_fake_provider_translates_one_markdown_chapter_with_structure_intact(self) -> None:
        with tempfile.TemporaryDirectory() as temporary_directory:
            project_root = Path(temporary_directory)
            source_text = (
                "# lowercase `keepCase`\n\n"
                "Visit [example](https://example.com/a?q=1) and keep $E=mc^2$.[^1]\n\n"
                "[^1]: footnote with `literal`.\n"
            )
            manifest_path = build_run_fixture(
                project_root, source_text=source_text, max_tokens=20
            )

            completed = subprocess.run(
                [sys.executable, "-m", "translation_engine", "--manifest", str(manifest_path)],
                check=False,
                capture_output=True,
                text=True,
            )

            self.assertEqual(completed.returncode, 0, completed.stderr)
            report = json.loads(completed.stdout)
            self.assertEqual(report["schema"], "translation-engine-report-v1")
            self.assertEqual(report["summary"], {"total": 1, "completed": 1, "failed": 0})
            self.assertEqual(report["units"][0]["unitId"], "chapter_001")
            self.assertEqual(report["units"][0]["status"], "completed")
            self.assertGreater(report["units"][0]["metrics"]["chunkCount"], 1)
            self.assertEqual(
                report["units"][0]["metrics"]["tokenCounter"],
                "utf8-byte-upper-bound-v1",
            )
            self.assertEqual(
                report["units"][0]["artifact"]["path"],
                "chapters/translated/chapter_001.md",
            )

            translated = (
                project_root / "chapters" / "translated" / "chapter_001.md"
            ).read_text(encoding="utf-8")
            self.assertIn("# LOWERCASE `keepCase`", translated)
            self.assertIn("[EXAMPLE](https://example.com/a?q=1)", translated)
            self.assertIn("$E=mc^2$.[^1]", translated)
            self.assertIn("[^1]: FOOTNOTE WITH `literal`.", translated)
            self.assertNotIn("lowercase", json.dumps(report))
            self.assertNotIn("LOWERCASE", json.dumps(report))

if __name__ == "__main__":
    unittest.main()
