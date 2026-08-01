from __future__ import annotations

from datetime import datetime, timezone
import json
import os
from pathlib import Path
import tempfile
import threading


class OperationProgress:
    """Atomic aggregate progress sidecar shared with the Launcher.

    The document intentionally has no title, path, prompt, source text, or
    provider response. Workers may report only counts, a bounded phase name,
    and a heartbeat timestamp.
    """

    def __init__(
        self,
        path: Path | None,
        *,
        stage_id: str,
        unit_kind: str,
        total: int | None,
        scope_id: str | None,
    ) -> None:
        self.path = path
        self.stage_id = stage_id
        self.unit_kind = unit_kind
        self.total = total if total is not None and total > 0 else None
        self.scope_id = scope_id or None
        self.completed = 0
        self._item_completed: dict[str, int] = {}
        self._lock = threading.Lock()

    @classmethod
    def from_environment(
        cls, stage_id: str, unit_kind: str, total: int | None = None
    ) -> "OperationProgress":
        raw_path = os.environ.get("BIBLIOSMITH_PROGRESS_PATH", "").strip()
        return cls(
            Path(raw_path) if raw_path else None,
            stage_id=stage_id,
            unit_kind=unit_kind,
            total=total,
            scope_id=os.environ.get("BIBLIOSMITH_PROGRESS_SCOPE", "").strip() or None,
        )

    def start(self, phase: str = "starting") -> None:
        self.update(completed=0, total=self.total, phase=phase)

    def touch(self, phase: str) -> None:
        with self._lock:
            self._write(phase)

    def advance(self, phase: str, amount: int = 1) -> None:
        with self._lock:
            self.completed = max(0, self.completed + amount)
            if self.total is not None:
                self.completed = min(self.completed, self.total)
            self._write(phase)

    def update(
        self, *, completed: int, total: int | None, phase: str
    ) -> None:
        with self._lock:
            self.total = total if total is not None and total > 0 else None
            self.completed = max(0, completed)
            if self.total is not None:
                self.completed = min(self.completed, self.total)
            self._write(phase)

    def update_item(self, item_id: str, completed: int, phase: str) -> None:
        with self._lock:
            self._item_completed[item_id] = max(
                self._item_completed.get(item_id, 0), completed, 0
            )
            self.completed = sum(self._item_completed.values())
            if self.total is not None:
                self.completed = min(self.completed, self.total)
            self._write(phase)

    def _write(self, phase: str) -> None:
        if self.path is None:
            return
        document: dict[str, object] = {
            "schema": "book-pipeline-progress-v1",
            "stageId": self.stage_id,
            "completed": self.completed,
            "unitKind": self.unit_kind,
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
