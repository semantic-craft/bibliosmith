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


def test_progress_does_not_regress_when_provider_temporarily_omits_counts() -> None:
    with tempfile.TemporaryDirectory() as directory:
        path = Path(directory) / ".book-pipeline-progress"
        with patch.dict(os.environ, {"BIBLIOSMITH_PROGRESS_PATH": str(path)}, clear=False):
            progress = OperationProgress.from_environment("extract", "pages")
            progress.update(completed=117, total=450, phase="extracting")
            progress.update(completed=0, total=None, phase="extracting")

        document = json.loads(path.read_text(encoding="utf-8"))
        assert document["completed"] == 117
        assert document["total"] == 450
        assert document["phase"] == "extracting"


def test_start_resets_counts_for_a_new_operation() -> None:
    with tempfile.TemporaryDirectory() as directory:
        path = Path(directory) / ".book-pipeline-progress"
        with patch.dict(os.environ, {"BIBLIOSMITH_PROGRESS_PATH": str(path)}, clear=False):
            progress = OperationProgress.from_environment("extract", "pages")
            progress.start("uploading", total=450)
            progress.update(completed=117, total=450, phase="extracting")
            progress.start("uploading", total=20)

        document = json.loads(path.read_text(encoding="utf-8"))
        assert document["completed"] == 0
        assert document["total"] == 20
        assert document["phase"] == "uploading"


def test_start_preserves_total_supplied_when_progress_was_created() -> None:
    with tempfile.TemporaryDirectory() as directory:
        path = Path(directory) / ".book-pipeline-progress"
        with patch.dict(os.environ, {"BIBLIOSMITH_PROGRESS_PATH": str(path)}, clear=False):
            progress = OperationProgress.from_environment("extract", "pages", total=32)
            progress.start("starting")

        document = json.loads(path.read_text(encoding="utf-8"))
        assert document["completed"] == 0
        assert document["total"] == 32
        assert document["phase"] == "starting"


def test_item_progress_accumulates_across_multiple_provider_batches() -> None:
    with tempfile.TemporaryDirectory() as directory:
        path = Path(directory) / ".book-pipeline-progress"
        with patch.dict(os.environ, {"BIBLIOSMITH_PROGRESS_PATH": str(path)}, clear=False):
            progress = OperationProgress.from_environment("extract", "pages")
            progress.start("uploading", total=450)
            progress.update_item("part-1", 200, "extracting", total=200)
            progress.update_item("part-2", 75, "extracting", total=200)
            progress.update_item("part-2", 200, "extracting", total=200)
            progress.update_item("part-3", 50, "extracting", total=50)

        document = json.loads(path.read_text(encoding="utf-8"))
        assert document["completed"] == 450
        assert document["total"] == 450
        assert document["phase"] == "extracting"
