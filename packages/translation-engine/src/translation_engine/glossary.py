from __future__ import annotations

import csv
from dataclasses import dataclass
from io import StringIO
import re
from typing import Mapping, Sequence


MAX_GLOSSARY_ENTRIES_PER_CHUNK = 50
_BOUND_GLOSSARY_KEY = "_translation_engine_glossary_entries"
_CJK_PATTERN = re.compile(
    "[\u3400-\u4dbf\u4e00-\u9fff\uf900-\ufaff\u3040-\u30ff\uac00-\ud7af]"
)


@dataclass(frozen=True)
class GlossaryEntry:
    source: str
    translation: str
    category: str = ""

    @property
    def variants(self) -> tuple[str, ...]:
        return tuple(value.strip() for value in self.source.split("|") if value.strip())

    @property
    def longest_variant_length(self) -> int:
        return max((len(value) for value in self.variants), default=0)


def parse_glossary_csv(text: str) -> tuple[GlossaryEntry, ...]:
    reader = csv.DictReader(StringIO(text), strict=True)
    if reader.fieldnames is None or not {"source", "translation"}.issubset(
        reader.fieldnames
    ):
        raise ValueError("glossary must contain source and translation columns")

    entries: list[GlossaryEntry] = []
    try:
        for row in reader:
            source = (row.get("source") or "").strip()
            translation = (row.get("translation") or "").strip()
            if not source or not translation:
                continue
            entries.append(
                GlossaryEntry(
                    source=source,
                    translation=translation,
                    category=(row.get("category") or "").strip(),
                )
            )
    except csv.Error as error:
        raise ValueError("invalid glossary CSV") from error
    return tuple(entries)


def bind_glossary_entries(
    task_manifest: Mapping[str, object], entries: Sequence[GlossaryEntry]
) -> dict[str, object]:
    bound = dict(task_manifest)
    bound[_BOUND_GLOSSARY_KEY] = tuple(entries)
    return bound


def select_glossary_entries(
    source_text: str, task_manifest: Mapping[str, object]
) -> tuple[GlossaryEntry, ...]:
    """The entries this chunk's prompt actually carries, in prompt order.

    Shared by the block builder and the output check so the two cannot disagree
    about which terms were demanded. Checking a term the model was never shown --
    one dropped by the per-chunk cap -- would be a violation of nothing.
    """
    value = task_manifest.get(_BOUND_GLOSSARY_KEY, ())
    if not isinstance(value, tuple) or not all(
        isinstance(entry, GlossaryEntry) for entry in value
    ):
        return ()

    matches: list[tuple[int, int, GlossaryEntry]] = []
    for index, entry in enumerate(value):
        frequency = sum(_variant_frequency(source_text, variant) for variant in entry.variants)
        if frequency:
            matches.append((frequency, index, entry))

    if not matches:
        return ()
    selected = sorted(
        matches,
        key=lambda match: (
            -match[0],
            -match[2].longest_variant_length,
            match[1],
        ),
    )[:MAX_GLOSSARY_ENTRIES_PER_CHUNK]
    selected.sort(key=lambda match: (-match[2].longest_variant_length, match[1]))
    return tuple(entry for _, _, entry in selected)


def find_glossary_violations(
    source_text: str, translated_text: str, task_manifest: Mapping[str, object]
) -> tuple[GlossaryEntry, ...]:
    """Demanded terms whose required translation is absent from the output.

    Deliberately a signal rather than a verdict. Chinese word formation can put a
    required form inside a longer compound, and a term can be legitimately absent
    where the source form was part of a larger name, so this is reported and
    never used to fail or degrade a chunk. Matching mirrors the source-side rule
    in `_variant_frequency`: substring for CJK, word boundaries for Latin, which
    is what makes a required translation left in Latin script behave sensibly.
    """
    return tuple(
        entry
        for entry in select_glossary_entries(source_text, task_manifest)
        if not _variant_frequency(translated_text, entry.translation)
    )


def build_mandatory_glossary_block(
    source_text: str, task_manifest: Mapping[str, object]
) -> str:
    selected = select_glossary_entries(source_text, task_manifest)
    if not selected:
        return ""

    lines = [
        "# GLOSSARY - REQUIRED TRANSLATIONS",
        (
            "MANDATORY: Whenever a listed source form appears in this chunk, use "
            "exactly its required translation. Do not substitute synonyms or alternate spellings."
        ),
    ]
    for entry in selected:
        category = f" [{entry.category}]" if entry.category else ""
        lines.append(f"- {entry.source} -> {entry.translation}{category}")
    return "\n".join(lines)


def _variant_frequency(source_text: str, variant: str) -> int:
    if _CJK_PATTERN.search(variant):
        return source_text.count(variant)
    return len(
        re.findall(
            rf"\b{re.escape(variant)}\b",
            source_text,
            flags=re.IGNORECASE,
        )
    )
