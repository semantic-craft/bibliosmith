import json
import os
import sys
import tempfile
from pathlib import Path
from unittest.mock import patch

SCRIPTS = Path(__file__).resolve().parents[1] / "scripts"
sys.path.insert(0, str(SCRIPTS))

from progress import OperationProgress  # noqa: E402


def test_progress_can_move_and_heartbeat_without_inventing_a_total() -> None:
    with tempfile.TemporaryDirectory() as directory:
        path = Path(directory) / ".book-pipeline-progress"
        with patch.dict(os.environ, {"BIBLIOSMITH_PROGRESS_PATH": str(path)}, clear=False):
            progress = OperationProgress.from_environment("extract", "pages")
            progress.update(completed=0, total=None, phase="uploading")
            first = json.loads(path.read_text(encoding="utf-8"))
            progress.update(completed=7, total=20, phase="extracting")

        document = json.loads(path.read_text(encoding="utf-8"))
        assert "total" not in first
        assert document["completed"] == 7
        assert document["total"] == 20
        assert document["phase"] == "extracting"
