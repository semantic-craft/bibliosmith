"""Fold BabelDOC's warnings into the Launcher's worker-marker vocabulary.

BabelDOC warns through the stdlib logger, in free English text that interpolates
page numbers and occasionally file paths. `parse_allowlisted_worker_markers` in
book_pipeline.rs will not carry free text into a job log -- every marker field is
checked against an allowlist -- so this module classifies each record into one of
a handful of tokens and reports counts. The Launcher shows "3 pages were too
large to translate"; the message text itself stays in the subprocess.

Adding a kind here means adding it to `LAYOUT_PDF_WARNING_KINDS` in
book_pipeline.rs too, or the marker is parsed and then dropped.
"""

from __future__ import annotations

from collections import Counter
import logging
import re

# A page BabelDOC considers too large to lay out; it translates what it can and
# the result on that page may be untouched. babeldoc/format/pdf/legacy_parse.py.
KIND_LARGE_PAGE = "large_page"
# Everything else BabelDOC saw fit to warn about. Deliberately not subdivided:
# the point is to tell the user "the run was not clean, open the run log",
# not to mirror upstream's message catalogue.
KIND_OTHER = "other"

_LARGE_PAGE_PATTERN = re.compile(r"page\s+\S+\s+is too large", re.IGNORECASE)

MARKER_PREFIX = "BOOK_PIPELINE_MARKER"


def classify(message: str) -> str:
    if _LARGE_PAGE_PATTERN.search(message):
        return KIND_LARGE_PAGE
    return KIND_OTHER


class WarningCollector(logging.Handler):
    """Counts warning records by kind. Attach to the `babeldoc` logger tree."""

    def __init__(self) -> None:
        super().__init__(level=logging.WARNING)
        self.counts: Counter[str] = Counter()

    def emit(self, record: logging.LogRecord) -> None:
        if record.levelno < logging.WARNING:
            return
        try:
            message = record.getMessage()
        except Exception:  # noqa: BLE001 - a broken record must not fail the run
            message = ""
        self.counts[classify(message)] += 1

    def markers(self) -> list[str]:
        """One marker line per kind seen, in a stable order."""

        return [
            f"{MARKER_PREFIX} warning={kind} count={self.counts[kind]}"
            for kind in (KIND_LARGE_PAGE, KIND_OTHER)
            if self.counts[kind]
        ]
