import json
import os
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

from translation_engine.progress import OperationProgress


class OperationProgressTests(unittest.TestCase):
    def test_item_progress_never_regresses_a_preloaded_checkpoint(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / ".book-pipeline-progress"
            progress = OperationProgress(
                path,
                stage_id="translate",
                unit_kind="chunks",
                total=8,
                scope_id=None,
            )

            progress.update_item("chapter_001", 6, "reviewing")
            progress.update_item("chapter_001", 3, "translating")

            document = json.loads(path.read_text(encoding="utf-8"))
            self.assertEqual(document["completed"], 6)

    def test_writes_only_aggregate_progress_using_the_shared_protocol(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / ".book-pipeline-progress"
            with patch.dict(os.environ, {"BIBLIOSMITH_PROGRESS_PATH": str(path)}, clear=False):
                progress = OperationProgress.from_environment(
                    stage_id="translate", unit_kind="chapters", total=3
                )
                progress.start("translating")
                progress.advance("translating")

            document = json.loads(path.read_text(encoding="utf-8"))
            self.assertEqual(document["schema"], "book-pipeline-progress-v1")
            self.assertEqual(document["stageId"], "translate")
            self.assertEqual(document["completed"], 1)
            self.assertEqual(document["total"], 3)
            self.assertEqual(document["unitKind"], "chapters")
            self.assertEqual(document["phase"], "translating")
            self.assertIn("activityAt", document)
            self.assertNotIn("text", document)


if __name__ == "__main__":
    unittest.main()
