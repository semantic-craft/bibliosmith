"""Atomic progress sidecar shared with the Launcher.

The reflow track's writers (`translation_engine.progress`, `packages/ocr/
scripts/progress.py`) track per-unit counts because they translate chapter by
chapter. This track has one subprocess translating one document, so only the
aggregate form is needed and only that is implemented here.

The document carries counts, a bounded phase name and a heartbeat -- never a
title, path, prompt or provider response. `book_pipeline.rs` rejects anything
outside the vocabularies below (`deny_unknown_fields`, plus an allowlist for
`phase` and `unitKind`), so widening either end alone silently drops progress.
"""

from __future__ import annotations

from datetime import datetime, timezone
import json
import os
from pathlib import Path
import tempfile

SCHEMA = "book-pipeline-progress-v1"

# Mirrors the `valid_phase` match in book_pipeline.rs. BabelDOC's own stage
# names are far more granular; `classify_phase` folds them into these.
PHASE_STARTING = "starting"
PHASE_EXTRACTING = "extracting"
PHASE_TRANSLATING = "translating"
PHASE_ASSEMBLING = "assembling"


class LayoutProgress:
    """Single-segment progress for one document."""

    def __init__(
        self,
        path: Path | None,
        *,
        stage_id: str,
        total: int | None,
        scope_id: str | None,
    ) -> None:
        self.path = path
        self.stage_id = stage_id
        self.total = total if total is not None and total > 0 else None
        self.scope_id = scope_id or None
        self.completed = 0

    @classmethod
    def from_environment(
        cls, stage_id: str, total: int | None = None
    ) -> "LayoutProgress":
        raw_path = os.environ.get("BIBLIOSMITH_PROGRESS_PATH", "").strip()
        return cls(
            Path(raw_path) if raw_path else None,
            stage_id=stage_id,
            total=total,
            scope_id=os.environ.get("BIBLIOSMITH_PROGRESS_SCOPE", "").strip() or None,
        )

    def report(self, phase: str, *, fraction: float | None = None) -> None:
        """Record a heartbeat, and a position when the caller knows one.

        `fraction` is BabelDOC's overall progress in 0..1; it is scaled onto the
        page count so the Launcher shows "142 / 380 pages" rather than a percent
        wearing a page label. Without a page count there is nothing honest to
        scale onto, so only the phase advances.
        """

        if fraction is not None and self.total is not None:
            scaled = int(max(0.0, min(1.0, fraction)) * self.total)
            self.completed = max(self.completed, min(scaled, self.total))
        self._write(phase)

    def _write(self, phase: str) -> None:
        if self.path is None:
            return
        document: dict[str, object] = {
            "schema": SCHEMA,
            "stageId": self.stage_id,
            "completed": self.completed,
            "unitKind": "pages",
            "phase": phase,
            "activityAt": datetime.now(timezone.utc).isoformat().replace("+00:00", "Z"),
        }
        if self.scope_id is not None:
            document["scopeId"] = self.scope_id
        if self.total is not None:
            document["total"] = self.total
        self.path.parent.mkdir(parents=True, exist_ok=True)
        descriptor, temporary_name = tempfile.mkstemp(
            prefix=f".{self.path.name}.", dir=self.path.parent
        )
        try:
            with os.fdopen(descriptor, "w", encoding="utf-8") as temporary:
                json.dump(document, temporary, ensure_ascii=False, separators=(",", ":"))
                temporary.write("\n")
                temporary.flush()
                os.fsync(temporary.fileno())
            os.replace(temporary_name, self.path)
        except BaseException:
            try:
                os.unlink(temporary_name)
            except FileNotFoundError:
                pass
            raise


def classify_phase(stage_name: str) -> str:
    """Fold a BabelDOC stage name into the Launcher's phase vocabulary.

    BabelDOC names roughly a dozen stages (ParseHTML, LayoutParser, ILTranslator,
    Typesetting, SavePDF, ...). Matching on substrings rather than the full list
    keeps this from breaking every time upstream renames one; anything
    unrecognised stays on the phase already reported.
    """

    lowered = stage_name.lower()
    if "translat" in lowered or "glossary" in lowered:
        return PHASE_TRANSLATING
    if "typeset" in lowered or "save" in lowered or "render" in lowered:
        return PHASE_ASSEMBLING
    return PHASE_EXTRACTING
