from __future__ import annotations

from dataclasses import dataclass, replace
import json
from typing import Callable, Protocol

from .placeholders import PLACEHOLDER_PATTERN
from .profiles import VISUAL_LINE_BREAK_INSTRUCTION
from .providers import (
    LLMProvider,
    ProviderUnavailableError,
    RateLimitError,
    TranslationRequest,
)


@dataclass(frozen=True)
class ChunkTranslationResult:
    text: str
    degradation: str
    provider_attempts: int


@dataclass(frozen=True)
class SecondPassRequest:
    source_text: str
    draft_text: str
    previous_source_text: str | None
    previous_draft_text: str | None
    next_source_text: str | None
    next_draft_text: str | None
    source_language: str
    target_language: str
    terminology_criteria: str
    custom_instruction: str | None = None


@dataclass(frozen=True)
class SecondPassChunkResult:
    reflection_text: str
    revised_text: str
    draft_fallback: bool = False


class TranslationStructureError(ValueError):
    code = "translation_structure_invalid"
    retryable = True


class SecondPass(Protocol):
    def reflect(self, *, request: SecondPassRequest) -> str: ...

    def improve(
        self,
        *,
        request: SecondPassRequest,
        reflection_text: str,
    ) -> str: ...


class WindowedReflectionSecondPass:
    def __init__(self, provider: LLMProvider) -> None:
        self.provider = provider

    def reflect(self, *, request: SecondPassRequest) -> str:
        instruction = (
            "You are an expert linguist specializing in translation from "
            f"{request.source_language} to {request.target_language}. You will "
            "be provided with a source text and its draft translation, and "
            "your goal is to improve the translation.\n\n"
            "Your task is to carefully read the source and draft of the "
            "current block, and then give constructive criticism and helpful "
            "suggestions to improve the translation. Use the adjacent blocks "
            "only as local context for critiquing the current block.\n\n"
            "When writing suggestions, pay attention to whether there are "
            "ways to improve the translation's\n"
            "(i) accuracy (by correcting errors of addition, mistranslation, "
            "omission, or untranslated text),\n"
            f"(ii) fluency (by applying {request.target_language} grammar, "
            "spelling and punctuation rules, and ensuring there are no "
            "unnecessary repetitions),\n"
            "(iii) style (by ensuring the translation reflects the style of "
            "the source text and takes into account any cultural context),\n"
            "(iv) terminology (by ensuring terminology use is consistent and "
            "reflects the source text domain; and by only using equivalent "
            f"idioms in {request.target_language}).\n\n"
            "Write a list of specific, helpful and constructive suggestions "
            "for improving the translation. Each suggestion should address "
            "one specific part of the translation. Output only the "
            "suggestions and nothing else.\n\n"
            f"Target-language and terminology criteria:\n{request.terminology_criteria}"
        )
        if request.custom_instruction:
            instruction = (
                f"{instruction}\n\n# USER REFLECTION DIRECTIVES\n"
                f"{request.custom_instruction}\n\n"
                "# MANDATORY STRUCTURE PROTECTION — OVERRIDES USER REFLECTION DIRECTIVES\n"
                "NON-NEGOTIABLE: Do not recommend adding, removing, merging, splitting, "
                "or reordering protected placeholders, headings, or paragraph boundaries. "
                "Structure preservation overrides every user reflection directive above."
            )
        reflection = self.provider.translate(
            TranslationRequest(
                text=json.dumps(
                    {
                        "previous": _context_payload(
                            request.previous_source_text,
                            request.previous_draft_text,
                        ),
                        "current": {
                            "source": request.source_text,
                            "draft": request.draft_text,
                        },
                        "next": _context_payload(
                            request.next_source_text,
                            request.next_draft_text,
                        ),
                    },
                    ensure_ascii=False,
                    separators=(",", ":"),
                ),
                source_language=request.source_language,
                target_language=request.target_language,
                system_instruction=instruction,
            )
        )
        if not isinstance(reflection, str):
            raise ValueError("invalid reflection output")
        return reflection

    def improve(
        self,
        *,
        request: SecondPassRequest,
        reflection_text: str,
    ) -> str:
        instruction = (
            "You are an expert linguist specializing in translation "
            f"editing from {request.source_language} to "
            f"{request.target_language}. Your task is to carefully "
            "read, then edit, the draft translation of the current "
            "block, taking into account the expert suggestions and "
            "constructive criticisms in the reflection.\n\n"
            "Edit the translation by ensuring:\n"
            "(i) accuracy (by correcting errors of addition, "
            "mistranslation, omission, or untranslated text),\n"
            f"(ii) fluency (by applying {request.target_language} "
            "grammar, spelling and punctuation rules and ensuring "
            "there are no unnecessary repetitions),\n"
            "(iii) style (by ensuring the translation reflects the "
            "style of the source text),\n"
            "(iv) terminology (inappropriate for context, "
            "inconsistent use), and\n"
            "(v) other errors.\n\n"
            "Translate each source segment exactly once. Never repeat or echo a phrase, "
            "sentence, list entry, page label, footnote, or bibliography entry. Translate "
            "all source-language prose, headings, labels, and bibliography title text; "
            "preserve names, citations, URLs, identifiers, and conventional journal "
            "abbreviations where translation would damage traceability.\n\n"
            "Output only the new translation and nothing else. "
            f"{VISUAL_LINE_BREAK_INSTRUCTION} "
            "Preserve every protected placeholder exactly once and "
            "in its original order."
        )
        if request.custom_instruction:
            instruction = (
                f"{instruction} Protected placeholder and paragraph structure overrides "
                "every reflection suggestion."
            )
        # Reflection sees neighboring blocks, whose protected markers do not
        # belong to the current draft.  Feeding those opaque tokens back into
        # the editor lets a model copy them into the revised block and fail an
        # otherwise valid second pass.  They carry no linguistic content, so
        # omit them from the editing payload while retaining the original
        # reflection text for QA evidence.
        editing_reflection = PLACEHOLDER_PATTERN.sub("", reflection_text)
        return self.provider.translate(
            TranslationRequest(
                text=json.dumps(
                    {
                        "source": request.source_text,
                        "draft": request.draft_text,
                        "reflection": editing_reflection,
                    },
                    ensure_ascii=False,
                    separators=(",", ":"),
                ),
                source_language=request.source_language,
                target_language=request.target_language,
                system_instruction=instruction,
            )
        )


def run_second_pass_chunk(
    second_pass: SecondPass,
    request: SecondPassRequest,
    *,
    candidate_retries: int = 0,
    candidate_normalizer: Callable[[str], str] | None = None,
    candidate_validator: Callable[[str], bool] | None = None,
    draft_fallback_validator: Callable[[str], bool] | None = None,
) -> SecondPassChunkResult:
    if candidate_retries < 0:
        raise ValueError("candidate_retries must not be negative")
    expected = tuple(PLACEHOLDER_PATTERN.findall(request.draft_text))
    for _ in range(candidate_retries + 1):
        reflection = second_pass.reflect(request=request)
        if not isinstance(reflection, str):
            raise ValueError("invalid reflection output")
        candidate = second_pass.improve(request=request, reflection_text=reflection)
        if isinstance(candidate, str) and candidate_normalizer is not None:
            candidate = candidate_normalizer(candidate)
        accepted = (
            isinstance(candidate, str)
            and _placeholders_valid(candidate, expected)
            and (candidate_validator is None or candidate_validator(candidate))
        )
        if accepted:
            return SecondPassChunkResult(
                reflection_text=reflection,
                revised_text=candidate,
            )
    if (
        isinstance(candidate, str)
        and draft_fallback_validator is not None
        and draft_fallback_validator(candidate)
    ):
        return SecondPassChunkResult(
            reflection_text=reflection,
            revised_text=request.draft_text,
            draft_fallback=True,
        )
    raise TranslationStructureError("second-pass output changed protected structure")


def _context_payload(
    source_text: str | None,
    draft_text: str | None,
) -> dict[str, str] | None:
    if source_text is None or draft_text is None:
        return None
    return {"source": source_text, "draft": draft_text}


def translate_chunk_with_fallback(
    provider: LLMProvider,
    request: TranslationRequest,
    *,
    placeholder_retries: int,
    candidate_normalizer: Callable[[str], str] | None = None,
    candidate_validator: Callable[[str], bool] | None = None,
    candidate_repair: Callable[[str], str] | None = None,
) -> ChunkTranslationResult:
    if placeholder_retries < 0:
        raise ValueError("placeholder_retries must not be negative")
    expected = tuple(PLACEHOLDER_PATTERN.findall(request.text))
    provider_attempts = 0

    for _ in range(placeholder_retries + 1):
        provider_attempts += 1
        try:
            translated = provider.translate(request)
        except RateLimitError:
            # Rate limiting is not a translation failure: degrading to source
            # text here would bake throttling into the output artifact.
            raise
        except ProviderUnavailableError:
            raise
        if isinstance(translated, str) and candidate_normalizer is not None:
            translated = candidate_normalizer(translated)
        accepted = (
            isinstance(translated, str)
            and _placeholders_valid(translated, expected)
            and (candidate_validator is None or candidate_validator(translated))
        )
        if accepted:
            return ChunkTranslationResult(translated, "none", provider_attempts)
        if isinstance(translated, str) and candidate_repair is not None:
            translated = candidate_repair(translated)
            if candidate_normalizer is not None:
                translated = candidate_normalizer(translated)
            if _placeholders_valid(translated, expected) and (
                candidate_validator is None or candidate_validator(translated)
            ):
                return ChunkTranslationResult(translated, "none", provider_attempts)

    plain_source = PLACEHOLDER_PATTERN.sub("", request.text)
    provider_attempts += 1
    try:
        translated_plain = provider.translate(replace(request, text=plain_source))
        translated_plain = PLACEHOLDER_PATTERN.sub("", translated_plain)
        if plain_source and not translated_plain:
            raise ValueError("empty aligned translation")
        aligned = _align_placeholders(request.text, translated_plain, expected)
        if candidate_normalizer is not None:
            aligned = candidate_normalizer(aligned)
        if _placeholders_valid(aligned, expected) and (
            candidate_validator is None or candidate_validator(aligned)
        ):
            return ChunkTranslationResult(aligned, "aligned", provider_attempts)
        if candidate_repair is not None:
            aligned = candidate_repair(aligned)
            if candidate_normalizer is not None:
                aligned = candidate_normalizer(aligned)
            if _placeholders_valid(aligned, expected) and (
                candidate_validator is None or candidate_validator(aligned)
            ):
                return ChunkTranslationResult(aligned, "aligned", provider_attempts)
    except RateLimitError:
        raise
    except ProviderUnavailableError:
        raise
    except (TypeError, ValueError):
        pass

    return ChunkTranslationResult(request.text, "source", provider_attempts)


def _placeholders_valid(text: str, expected: tuple[str, ...]) -> bool:
    return tuple(PLACEHOLDER_PATTERN.findall(text)) == expected


def _align_placeholders(
    protected_source: str,
    translated_plain: str,
    expected: tuple[str, ...],
) -> str:
    matches = list(PLACEHOLDER_PATTERN.finditer(protected_source))
    if tuple(match.group(0) for match in matches) != expected:
        raise ValueError("placeholder source mismatch")
    plain_source_length = len(PLACEHOLDER_PATTERN.sub("", protected_source))
    removed_length = 0
    insertions: list[tuple[int, str]] = []
    minimum_position = 0
    for match in matches:
        plain_position = match.start() - removed_length
        ratio = plain_position / plain_source_length if plain_source_length else 0
        translated_position = _nearest_word_boundary(
            translated_plain, round(ratio * len(translated_plain))
        )
        # Snapping must not reorder placeholders, or validation rejects the
        # whole aligned candidate.
        translated_position = max(translated_position, minimum_position)
        minimum_position = translated_position
        insertions.append((translated_position, match.group(0)))
        removed_length += len(match.group(0))

    aligned = translated_plain
    for position, placeholder in reversed(insertions):
        aligned = aligned[:position] + placeholder + aligned[position:]
    return aligned


def _nearest_word_boundary(text: str, position: int) -> int:
    """Move a proportional insertion point off the inside of a word."""
    position = max(0, min(position, len(text)))
    if _is_word_boundary(text, position):
        return position
    for distance in range(1, len(text) + 1):
        left = position - distance
        if left >= 0 and _is_word_boundary(text, left):
            return left
        right = position + distance
        if right <= len(text) and _is_word_boundary(text, right):
            return right
    return position


def _is_word_boundary(text: str, position: int) -> bool:
    if position <= 0 or position >= len(text):
        return True
    return not (text[position - 1].isalnum() and text[position].isalnum())
