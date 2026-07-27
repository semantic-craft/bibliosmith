#!/usr/bin/env python3.11
"""Ask the launcher whether deleting a source PDF has been approved.

The launcher records a `source_cleanup` approval against the book's built
reading artifacts, and treats it as spent the moment those artifacts change --
an approval is a statement about specific bytes. These cleanup scripts are the
only thing in the tree that actually deletes a source PDF, so they are where
that record has to be consulted.

The launcher exposes this as a Tauri command, which a script cannot call, so the
check reads the same durable state file the command reads. It is read-only.

Deliberately *not* a hard gate on every book: books converted before the
approval mechanism existed have no record at all, and refusing those would stop
these scripts from doing anything. The rule is narrower and stricter where it
counts -- if the launcher knows about the book, its verdict binds; if it has
never heard of it, this check has no opinion and says so.
"""

from __future__ import annotations

import json
import os
import sys
from dataclasses import dataclass
from pathlib import Path

CLEANUP_GATE_ID = "source_cleanup"


@dataclass(frozen=True)
class ApprovalVerdict:
    """Whether the launcher permits deleting this source, and why."""

    known: bool
    approved: bool
    reason: str

    @property
    def blocks_delete(self) -> bool:
        """A verdict only blocks for a book the launcher actually tracks."""
        return self.known and not self.approved


def launcher_state_path() -> Path:
    """Where the launcher keeps Book Pipeline state, mirroring default_state_dir."""
    override = os.environ.get("BIBLIOSMITH_PIPELINE_STATE")
    if override:
        return Path(override)
    if sys.platform == "darwin":
        base = Path.home() / "Library" / "Application Support"
    elif sys.platform == "win32":
        base = Path(os.environ.get("LOCALAPPDATA", Path.home() / "AppData" / "Local"))
    else:
        base = Path(os.environ.get("XDG_DATA_HOME", Path.home() / ".local" / "share"))
    return base / "BiblioSmith" / "launcher" / "book-pipeline" / "jobs.json"


def _load_jobs(state_path: Path) -> list[dict] | None:
    try:
        payload = json.loads(state_path.read_text(encoding="utf-8"))
    except (OSError, ValueError):
        return None
    jobs = payload.get("jobs")
    return jobs if isinstance(jobs, list) else None


def _reading_artifact_hashes(job: dict) -> dict[str, str]:
    """The reading artifacts an approval binds, keyed by artifact id.

    Mirrors the launcher's cleanup_bound_artifact_hashes: child artifacts first,
    then the job's own, keeping only the built book and only entries carrying a
    digest.
    """
    hashes: dict[str, str] = {}
    children = job.get("children") or []
    artifacts = [
        artifact
        for child in children
        if isinstance(child, dict)
        for artifact in (child.get("artifacts") or [])
    ] + list(job.get("artifacts") or [])
    for artifact in artifacts:
        if not isinstance(artifact, dict):
            continue
        kind = artifact.get("kind") or ""
        sha256 = artifact.get("sha256")
        if kind.startswith("reading_") and sha256:
            hashes[artifact.get("artifactId") or ""] = sha256
    return hashes


def _job_matches(job: dict, source_ref: str) -> bool:
    source = job.get("source") or {}
    return source_ref in {source.get("selector"), source.get("path"), job.get("id")}


def approval_for(source_ref: str, *, state_path: Path | None = None) -> ApprovalVerdict:
    """Look up the launcher's cleanup verdict for one source reference."""
    path = state_path or launcher_state_path()
    jobs = _load_jobs(path)
    if jobs is None:
        return ApprovalVerdict(
            known=False,
            approved=False,
            reason=f"no readable launcher state at {path}",
        )
    job = next((item for item in jobs if isinstance(item, dict) and _job_matches(item, source_ref)), None)
    if job is None:
        return ApprovalVerdict(
            known=False,
            approved=False,
            reason="the launcher has no job for this source",
        )
    approval = next(
        (
            item
            for item in (job.get("approvalReferences") or [])
            if isinstance(item, dict) and item.get("gateId") == CLEANUP_GATE_ID
        ),
        None,
    )
    if approval is None:
        return ApprovalVerdict(
            known=True,
            approved=False,
            reason="the launcher tracks this book but no source-cleanup approval was recorded",
        )
    bound = approval.get("boundArtifactHashes") or {}
    if not bound:
        return ApprovalVerdict(
            known=True,
            approved=False,
            reason="the recorded approval binds no artifacts",
        )
    if bound != _reading_artifact_hashes(job):
        return ApprovalVerdict(
            known=True,
            approved=False,
            reason="the approved artifacts changed since the approval; re-approve before deleting",
        )
    return ApprovalVerdict(
        known=True,
        approved=True,
        reason=f"approved at {approval.get('decidedAt') or 'an unrecorded time'}",
    )


def refuse_unapproved_delete(source_ref: str, *, logger, ignore_approval: bool = False) -> bool:
    """Return True when the caller may delete this source PDF.

    Logs either way: a book the launcher never saw is deleted as before but says
    it went unverified, so a silent gap never passes for a checked one.
    """
    verdict = approval_for(source_ref)
    if ignore_approval:
        logger.warning(
            "CLEANUP-APPROVAL bypassed for %s (%s)", source_ref, verdict.reason
        )
        return True
    if verdict.blocks_delete:
        logger.error("CLEANUP-APPROVAL refused for %s: %s", source_ref, verdict.reason)
        return False
    if verdict.approved:
        logger.info("CLEANUP-APPROVAL ok for %s: %s", source_ref, verdict.reason)
    else:
        logger.warning(
            "CLEANUP-APPROVAL unverified for %s: %s", source_ref, verdict.reason
        )
    return True
