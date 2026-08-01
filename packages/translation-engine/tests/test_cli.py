import json
import os
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
            progress_path = project_root / ".book-pipeline-progress"
            child_env = os.environ.copy()
            child_env["BIBLIOSMITH_PROGRESS_PATH"] = str(progress_path)

            completed = subprocess.run(
                [sys.executable, "-m", "translation_engine", "--manifest", str(manifest_path)],
                check=False,
                capture_output=True,
                text=True,
                env=child_env,
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
            progress = json.loads(progress_path.read_text(encoding="utf-8"))
            self.assertEqual(progress["stageId"], "translate")
            self.assertEqual(progress["unitKind"], "chunks")
            self.assertEqual(
                progress["completed"], report["units"][0]["metrics"]["chunkCount"]
            )
            self.assertEqual(progress["total"], progress["completed"])

    def test_two_pass_progress_counts_translation_and_review_chunks(self) -> None:
        with tempfile.TemporaryDirectory() as temporary_directory:
            project_root = Path(temporary_directory)
            manifest_path = build_run_fixture(
                project_root,
                source_text="# chapter\n\nalpha beta gamma delta epsilon.\n",
                max_tokens=12,
                second_pass_enabled=True,
            )
            progress_path = project_root / ".book-pipeline-progress"
            child_env = os.environ.copy()
            child_env["BIBLIOSMITH_PROGRESS_PATH"] = str(progress_path)

            completed = subprocess.run(
                [sys.executable, "-m", "translation_engine", "--manifest", str(manifest_path)],
                check=False,
                capture_output=True,
                text=True,
                env=child_env,
            )

            self.assertEqual(completed.returncode, 0, completed.stderr)
            report = json.loads(completed.stdout)
            metrics = report["units"][0]["metrics"]
            self.assertTrue(metrics["secondPassApplied"])
            progress = json.loads(progress_path.read_text(encoding="utf-8"))
            self.assertEqual(progress["unitKind"], "chunks")
            self.assertEqual(progress["total"], metrics["chunkCount"] * 2)
            self.assertEqual(progress["completed"], progress["total"])
            self.assertEqual(progress["phase"], "reviewing")

if __name__ == "__main__":
    unittest.main()
