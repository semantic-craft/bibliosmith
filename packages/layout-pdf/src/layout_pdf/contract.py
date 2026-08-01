"""What the CLI hands the translate step, and what it expects back.

Its own module so `babeldoc_runner` does not have to import `cli` -- the CLI is
what imports the runner, and only lazily.
"""

from __future__ import annotations

from dataclasses import dataclass
from pathlib import Path
from typing import Callable

from .progress import LayoutProgress


@dataclass(frozen=True)
class TranslationRequest:
    input_path: Path
    output_dir: Path
    lang_in: str
    lang_out: str
    base_url: str
    api_key: str
    model: str
    qps: int


@dataclass(frozen=True)
class TranslationOutcome:
    """Where the translate step left the bilingual PDF."""

    dual_pdf_path: Path
    page_count: int | None = None


TranslateDocument = Callable[[TranslationRequest, LayoutProgress], TranslationOutcome]
