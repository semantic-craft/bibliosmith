"""The launcher's source-cleanup approval, as the deletion scripts see it."""

from __future__ import annotations

import json
import logging
import sys
from pathlib import Path

import pytest

SCRIPTS = Path(__file__).resolve().parents[1] / "scripts"
sys.path.insert(0, str(SCRIPTS))

from cleanup_approval import (  # noqa: E402
    approval_for,
    launcher_state_path,
    refuse_unapproved_delete,
)


def write_state(tmp_path: Path, job: dict | None) -> Path:
    path = tmp_path / "jobs.json"
    path.write_text(json.dumps({"jobs": [job] if job else []}), encoding="utf-8")
    return path


def approved_job(*, hashes: dict[str, str], bound: dict[str, str] | None = None) -> dict:
    return {
        "id": "job-1",
        "source": {"kind": "zotero_attachment", "selector": "PDFKEY1"},
        "children": [
            {
                "id": "child-1",
                "artifacts": [
                    {"artifactId": key, "kind": "reading_epub", "sha256": value}
                    for key, value in hashes.items()
                ],
            }
        ],
        "approvalReferences": [
            {
                "gateId": "source_cleanup",
                "approvalId": "approval-1",
                "decidedAt": "2026-07-27T00:00:00Z",
                "boundArtifactHashes": bound if bound is not None else dict(hashes),
            }
        ],
    }


def test_a_current_approval_permits_the_delete(tmp_path: Path) -> None:
    state = write_state(tmp_path, approved_job(hashes={"art-1": "aaaa"}))
    verdict = approval_for("PDFKEY1", state_path=state)
    assert verdict.known and verdict.approved
    assert not verdict.blocks_delete


def test_a_rebuilt_book_spends_its_approval(tmp_path: Path) -> None:
    """An approval is a statement about specific bytes; new bytes need a new one."""
    job = approved_job(hashes={"art-1": "bbbb"}, bound={"art-1": "aaaa"})
    verdict = approval_for("PDFKEY1", state_path=write_state(tmp_path, job))
    assert verdict.known and not verdict.approved
    assert verdict.blocks_delete
    assert "changed" in verdict.reason


def test_a_tracked_book_without_an_approval_blocks(tmp_path: Path) -> None:
    job = approved_job(hashes={"art-1": "aaaa"})
    job["approvalReferences"] = []
    verdict = approval_for("PDFKEY1", state_path=write_state(tmp_path, job))
    assert verdict.known and not verdict.approved
    assert verdict.blocks_delete


@pytest.mark.parametrize("job", [None, "missing-file"])
def test_a_book_the_launcher_never_saw_does_not_block(tmp_path: Path, job) -> None:
    """Books converted before the mechanism existed have no record at all.

    Refusing those would stop these scripts doing anything, so the check has no
    opinion on them — it must not silently become a blanket gate.
    """
    state = tmp_path / "absent.json" if job == "missing-file" else write_state(tmp_path, None)
    verdict = approval_for("PDFKEY1", state_path=state)
    assert not verdict.known
    assert not verdict.blocks_delete


def test_refusal_is_logged_and_stops_the_delete(tmp_path: Path, monkeypatch, caplog) -> None:
    job = approved_job(hashes={"art-1": "bbbb"}, bound={"art-1": "aaaa"})
    state = write_state(tmp_path, job)
    monkeypatch.setattr("cleanup_approval.launcher_state_path", lambda: state)
    logger = logging.getLogger("cleanup-approval-test")

    with caplog.at_level(logging.INFO):
        assert refuse_unapproved_delete("PDFKEY1", logger=logger) is False
        # An untracked book still deletes, but says it went unverified rather
        # than passing for a checked one.
        assert refuse_unapproved_delete("UNKNOWNKEY", logger=logger) is True
        # The escape hatch stays available and announces itself.
        assert refuse_unapproved_delete("PDFKEY1", logger=logger, ignore_approval=True) is True

    text = caplog.text
    assert "refused for PDFKEY1" in text
    assert "unverified for UNKNOWNKEY" in text
    assert "bypassed for PDFKEY1" in text


def test_state_path_is_overridable_for_a_relocated_launcher(monkeypatch, tmp_path: Path) -> None:
    monkeypatch.setenv("BIBLIOSMITH_PIPELINE_STATE", str(tmp_path / "elsewhere.json"))
    assert launcher_state_path() == tmp_path / "elsewhere.json"
