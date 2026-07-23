from __future__ import annotations

import json
from pathlib import Path
import re
from typing import Any, Callable

from .files import atomic_write_text
from .prompts import NER_SYSTEM_INSTRUCTION
from .providers import LLMProvider, TranslationRequest, create_provider


NER_SAMPLE_CHARACTERS = 6000
NER_CATEGORIES = (
    "character",
    "location",
    "organization",
    "item",
    "title",
    "other",
)
DEFAULT_CANDIDATE_PATH = Path("glossary/ner-candidates.json")

ProviderFactory = Callable[..., LLMProvider]


def extract_ner_candidates(
    project_root: Path,
    *,
    provider_profile_id: str,
    provider_config_id: str,
    provider_factory: ProviderFactory = create_provider,
) -> dict[str, Any]:
    root = project_root.resolve()
    source_path = _project_path(root, Path("source/source.md"))
    source_text = source_path.read_text(encoding="utf-8")
    sample = source_text[:NER_SAMPLE_CHARACTERS]
    provider = provider_factory(provider_profile_id, config_id=provider_config_id)
    response = provider.translate(
        TranslationRequest(
            text=sample,
            source_language="auto",
            target_language="zh-Hans",
            system_instruction=NER_SYSTEM_INSTRUCTION,
        )
    )
    if not isinstance(response, str):
        raise ValueError("NER provider returned a non-text response")
    candidates = parse_ner_response(response)

    candidate_path = _project_path(root, DEFAULT_CANDIDATE_PATH)
    relative_output = candidate_path.relative_to(root).as_posix()
    document = {
        "schema": "translation-engine-ner-candidates-v1",
        "source": {
            "path": "source/source.md",
            "sampledCharacters": len(sample),
            "truncated": len(source_text) > len(sample),
        },
        "categories": list(NER_CATEGORIES),
        "reviewStatus": "pending",
        "candidates": candidates,
        "notice": (
            "Review candidates manually. This command does not modify glossary/terms.csv."
        ),
    }
    atomic_write_text(
        candidate_path,
        json.dumps(document, ensure_ascii=False, indent=2) + "\n",
    )
    return {
        "schema": "translation-engine-ner-report-v1",
        "candidateCount": len(candidates),
        "artifact": {"path": relative_output},
    }


def parse_ner_response(response: str) -> list[dict[str, str]]:
    without_thinking = re.sub(
        r"<think\b[^>]*>.*?</think>", "", response, flags=re.IGNORECASE | re.DOTALL
    )
    tagged = re.search(
        r"<NER_JSON\b[^>]*>(.*?)</NER_JSON>",
        without_thinking,
        flags=re.IGNORECASE | re.DOTALL,
    )
    candidate_text = tagged.group(1) if tagged else without_thinking
    candidate_text = re.sub(
        r"```(?:json)?\s*|```", "", candidate_text, flags=re.IGNORECASE
    )
    value = _first_json_value(candidate_text)
    values = _candidate_values(value)

    normalized: list[dict[str, str]] = []
    seen_sources: set[str] = set()
    for candidate in values:
        if not isinstance(candidate, dict):
            continue
        source = _first_string(candidate, "source", "entity", "term", "name")
        translation = _first_string(
            candidate,
            "translation",
            "target",
            "suggested_translation",
            "target_term",
        )
        if not source or not translation:
            continue
        category = _first_string(candidate, "category", "type", "label").lower()
        if category not in NER_CATEGORIES:
            category = "other"
        source_key = source.casefold()
        if source_key in seen_sources:
            continue
        seen_sources.add(source_key)
        normalized.append(
            {"source": source, "translation": translation, "category": category}
        )
    return normalized


def _first_json_value(text: str) -> Any:
    starts = sorted(
        index for index, character in enumerate(text) if character in "[{"
    )
    for start in starts:
        balanced = _balanced_value(text, start)
        if balanced is None:
            continue
        for candidate in (balanced, re.sub(r",\s*([}\]])", r"\1", balanced)):
            try:
                return json.loads(candidate)
            except json.JSONDecodeError:
                continue
    raise ValueError("NER response does not contain valid JSON")


def _balanced_value(text: str, start: int) -> str | None:
    opening = text[start]
    if opening not in "[{":
        return None
    pairs = {"[": "]", "{": "}"}
    stack = [pairs[opening]]
    in_string = False
    escaped = False
    for index in range(start + 1, len(text)):
        character = text[index]
        if in_string:
            if escaped:
                escaped = False
            elif character == "\\":
                escaped = True
            elif character == '"':
                in_string = False
            continue
        if character == '"':
            in_string = True
        elif character in pairs:
            stack.append(pairs[character])
        elif character in "]}":
            if not stack or character != stack.pop():
                return None
            if not stack:
                return text[start : index + 1]
    return None


def _candidate_values(value: Any) -> list[Any]:
    if isinstance(value, list):
        return value
    if isinstance(value, dict):
        for key in ("entities", "terms", "candidates", "items", "results"):
            nested = value.get(key)
            if isinstance(nested, list):
                return nested
        return [value]
    raise ValueError("NER JSON must be an array or object")


def _first_string(value: dict[str, Any], *keys: str) -> str:
    for key in keys:
        candidate = value.get(key)
        if isinstance(candidate, str) and candidate.strip():
            return candidate.strip()
    return ""


def _project_path(project_root: Path, path: Path) -> Path:
    candidate = path.resolve() if path.is_absolute() else (project_root / path).resolve()
    if candidate != project_root and project_root not in candidate.parents:
        raise ValueError("path outside project")
    return candidate
