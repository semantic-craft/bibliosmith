from __future__ import annotations

import hashlib
import json
from dataclasses import dataclass
from pathlib import Path
from typing import AbstractSet, Any, Callable, Mapping, Sequence

from .checkpoint import CheckpointStore, UnitCheckpoint, UnitIdempotencyKey
from .chunking import TokenChunker, Utf8ByteTokenCounter
from .files import atomic_write_text
from .glossary import (
    GlossaryEntry,
    bind_glossary_entries,
    find_glossary_violations,
    parse_glossary_csv,
)
from .pipeline import (
    SecondPass,
    SecondPassRequest,
    WindowedReflectionSecondPass,
    run_second_pass_chunk,
    translate_chunk_with_fallback,
)
from .placeholders import (
    ProtectedMarkdown,
    protect_chunk_structure,
    protect_markdown_for_chunking,
)
from .profiles import TargetLanguageProfile, get_target_profile
from .providers import (
    LLMProvider,
    ProviderError,
    ProviderUnavailableError,
    RateLimitError,
    TranslationRequest,
    create_provider,
)


class EngineError(Exception):
    def __init__(self, code: str) -> None:
        super().__init__(code)
        self.code = code


MAX_CUSTOM_INSTRUCTION_CHARACTERS = 2000


ProviderFactory = Callable[..., LLMProvider]
TargetProfileFactory = Callable[[str], TargetLanguageProfile]


@dataclass(frozen=True)
class TranslationUnitSource:
    unit_id: str
    source_text: str
    task_bytes: bytes
    profile_task: dict[str, Any]


def run_manifest(
    manifest_path: Path,
    *,
    provider_factory: ProviderFactory = create_provider,
    target_profile_factory: TargetProfileFactory = get_target_profile,
    second_pass: SecondPass | None = None,
) -> dict[str, Any]:
    manifest = _read_json(manifest_path)
    if manifest.get("schema") != "translation-engine-run-v1":
        raise EngineError("unsupported_manifest_schema")
    custom_translation, custom_reflection = _parse_custom_instructions(manifest)
    project_root = Path(_required_string(manifest, "projectRoot")).resolve()
    source_map_path = _project_path(
        project_root, _required_string(manifest, "sourceMapPath")
    )
    source_map = _read_json(source_map_path)
    if source_map.get("schema") != "local-reading-source-map-v1":
        raise EngineError("unsupported_source_map_schema")

    target_language = _required_string(manifest, "targetLanguage")
    try:
        profile = target_profile_factory(target_language)
        provider = provider_factory(
            _required_string(manifest, "providerProfileId"),
            config_id=_required_string(manifest, "providerConfigId"),
        )
    except ValueError as error:
        raise EngineError(str(error).replace(" ", "_")) from error
    apply_model_override(provider, manifest)

    second_pass_enabled = manifest.get("secondPassEnabled", False)
    if not isinstance(second_pass_enabled, bool):
        raise EngineError("invalid_second_pass_enabled")
    text_cleanup = _parse_text_cleanup(manifest)
    active_second_pass = None
    if second_pass_enabled:
        active_second_pass = second_pass or WindowedReflectionSecondPass(provider)

    units = manifest.get("units")
    if not isinstance(units, list) or not units:
        raise EngineError("empty_unit_list")
    max_tokens = manifest.get("maxTokens")
    if not isinstance(max_tokens, int) or isinstance(max_tokens, bool) or max_tokens < 1:
        raise EngineError("invalid_max_tokens")
    translation_policy_version = _required_string(
        manifest, "translationPolicyVersion"
    )
    placeholder_retries = manifest.get("placeholderRetries", 1)
    if (
        not isinstance(placeholder_retries, int)
        or isinstance(placeholder_retries, bool)
        or placeholder_retries < 0
    ):
        raise EngineError("invalid_placeholder_retries")

    unit_reports = []
    for position, unit in enumerate(units):
        try:
            unit_reports.append(
                _translate_unit(
                    project_root=project_root,
                    source_map=source_map,
                    task_path=_project_path(
                        project_root, _required_string(unit, "taskManifestPath")
                    ),
                    source_language=str(manifest.get("sourceLanguage", "auto")),
                    target_language=target_language,
                    provider=provider,
                    target_profile=profile,
                    max_tokens=max_tokens,
                    translation_policy_version=translation_policy_version,
                    placeholder_retries=placeholder_retries,
                    second_pass=active_second_pass,
                    text_cleanup=text_cleanup,
                    custom_translation=custom_translation,
                    custom_reflection=custom_reflection,
                )
            )
        except (
            OSError,
            json.JSONDecodeError,
            EngineError,
            ProviderError,
            ValueError,
        ) as error:
            code = getattr(error, "code", "unit_invalid")
            unit_reports.append(
                {
                    "unitId": _safe_unit_id(project_root, unit),
                    "status": "failed",
                    "error": {
                        "code": code,
                        "retryable": isinstance(error, ProviderUnavailableError),
                    },
                }
            )
            if isinstance(error, RateLimitError):
                # The provider pool is exhausted for longer than we are
                # willing to wait; attempting further units would only churn
                # the same throttle. Fail them retryable and stop.
                unit_reports.extend(
                    {
                        "unitId": _safe_unit_id(project_root, remaining),
                        "status": "failed",
                        "error": {"code": RateLimitError.code, "retryable": True},
                    }
                    for remaining in units[position + 1 :]
                )
                break

    completed = sum(report["status"] == "completed" for report in unit_reports)
    failed = len(unit_reports) - completed
    return {
        "schema": "translation-engine-report-v1",
        "summary": {"total": len(unit_reports), "completed": completed, "failed": failed},
        "units": unit_reports,
    }


def load_translation_unit(
    *,
    project_root: Path,
    source_map: dict[str, Any],
    task_path: Path,
    target_language: str,
) -> TranslationUnitSource:
    task_bytes = task_path.read_bytes()
    task = json.loads(task_bytes)
    if not isinstance(task, dict):
        raise EngineError("invalid_json_object")
    if task.get("schema") != "local-reading-translation-task-v1":
        raise EngineError("unsupported_task_schema")
    unit_id = _required_string(task, "chapterId")
    if task.get("targetLanguage") != target_language:
        raise EngineError("target_language_mismatch")

    chapter = next(
        (
            value
            for value in source_map.get("chapters", [])
            if isinstance(value, dict) and value.get("id") == unit_id
        ),
        None,
    )
    if chapter is None:
        raise EngineError("unit_not_in_source_map")

    source_relative = _required_string(task, "sourceChapterPath")
    if chapter.get("chapterSourcePath") != source_relative:
        raise EngineError("source_path_mismatch")
    source_path = _project_path(project_root, source_relative)
    source_text = source_path.read_text(encoding="utf-8")
    source_sha256 = _sha256(source_text.encode())
    if task.get("sourceChapterSha256") != source_sha256:
        raise EngineError("source_hash_mismatch")
    if chapter.get("chapterSourceSha256") != source_sha256:
        raise EngineError("source_map_hash_mismatch")

    glossary_path = _project_path(project_root, _required_string(task, "glossaryPath"))
    glossary_bytes = glossary_path.read_bytes()
    if task.get("glossarySha256") != _sha256(glossary_bytes):
        raise EngineError("glossary_hash_mismatch")
    try:
        glossary_entries = parse_glossary_csv(glossary_bytes.decode("utf-8-sig"))
    except (UnicodeDecodeError, ValueError) as error:
        raise EngineError("invalid_glossary") from error
    return TranslationUnitSource(
        unit_id=unit_id,
        source_text=source_text,
        task_bytes=task_bytes,
        profile_task=bind_glossary_entries(task, glossary_entries),
    )


def _translate_unit(
    *,
    project_root: Path,
    source_map: dict[str, Any],
    task_path: Path,
    source_language: str,
    target_language: str,
    provider: LLMProvider,
    target_profile: TargetLanguageProfile,
    max_tokens: int,
    translation_policy_version: str,
    placeholder_retries: int,
    second_pass: SecondPass | None,
    text_cleanup: bool,
    custom_translation: str | None,
    custom_reflection: str | None,
) -> dict[str, Any]:
    unit = load_translation_unit(
        project_root=project_root,
        source_map=source_map,
        task_path=task_path,
        target_language=target_language,
    )
    task_bytes = unit.task_bytes
    unit_id = unit.unit_id
    source_text = unit.source_text
    profile_task = unit.profile_task

    counter = Utf8ByteTokenCounter()
    chunk_safe_source = protect_markdown_for_chunking(source_text)
    chunk_safe_parts = TokenChunker(max_tokens=max_tokens, counter=counter).split(
        chunk_safe_source.text
    )
    protected_chunks = [
        protect_chunk_structure(chunk, chunk_safe_source) for chunk in chunk_safe_parts
    ]
    chunks = [chunk.text for chunk in protected_chunks]
    checkpoint_store = CheckpointStore(
        project_root / "chapters" / "translated" / ".partial"
    )
    translation_pass_id = _custom_instruction_pass_id(
        "translation-v1-text-cleanup" if text_cleanup else "translation-v1",
        custom_translation,
    )
    idempotency_key = UnitIdempotencyKey(
        task_manifest_sha256=_sha256(task_bytes),
        provider_profile_id=provider.profile_id,
        provider_config_id=provider.config_id,
        translation_policy_version=translation_policy_version,
        pass_id=translation_pass_id,
    )
    checkpoint = checkpoint_store.load(unit_id, idempotency_key)
    if checkpoint is not None and checkpoint.next_chunk_index > len(chunks):
        checkpoint_store.delete(unit_id)
        checkpoint = None
    resumed_chunk_count = checkpoint.next_chunk_index if checkpoint is not None else 0
    translated_chunks = list(checkpoint.translated_chunks) if checkpoint else []
    checkpoint_prefix_open = True
    aligned_fallback_count = 0
    source_fallback_count = 0
    provider_attempt_count = 0
    # A chunk that fell back to its source text is untranslated, so every term in
    # it would read as a glossary violation. That is already reported as
    # sourceFallbackCount; counting it twice would bury real terminology drift.
    # Resumed chunks need no equivalent set: the checkpoint prefix closes at the
    # first source fallback, so anything resumed is known to have translated.
    source_fallback_indices: set[int] = set()
    for index in range(resumed_chunk_count, len(chunks)):
        system_instruction = target_profile.build_system_instruction(
            source_text=chunks[index],
            task_manifest=profile_task,
            text_cleanup=text_cleanup,
            custom_instruction=custom_translation,
        )
        chunk_system_instruction = system_instruction
        if translated_chunks:
            previous_translation_tail = _translation_context_tail(
                translated_chunks[-1]
            )
            if previous_translation_tail:
                if custom_translation:
                    chunk_system_instruction = (
                        f"# CONTEXT\n{previous_translation_tail}\n\n{system_instruction}"
                    )
                else:
                    chunk_system_instruction = (
                        f"{system_instruction}\n\n# CONTEXT\n{previous_translation_tail}"
                    )
        result = translate_chunk_with_fallback(
            provider,
            TranslationRequest(
                text=chunks[index],
                source_language=source_language,
                target_language=target_language,
                system_instruction=chunk_system_instruction,
            ),
            placeholder_retries=placeholder_retries,
            candidate_validator=lambda candidate, protected=protected_chunks[index]: (
                _candidate_preserves_structure(protected, candidate)
            ),
        )
        translated_chunks.append(result.text)
        provider_attempt_count += result.provider_attempts
        if result.degradation == "aligned":
            aligned_fallback_count += 1
        elif result.degradation == "source":
            source_fallback_count += 1
            source_fallback_indices.add(index)
            checkpoint_prefix_open = False

        if checkpoint_prefix_open:
            checkpoint_store.save(
                unit_id,
                idempotency_key,
                UnitCheckpoint(
                    next_chunk_index=index + 1,
                    translated_chunks=tuple(translated_chunks),
                ),
            )
    second_pass_applied = second_pass is not None and source_fallback_count == 0
    second_pass_resumed_chunk_count = 0
    second_pass_checkpoint_store = None
    if second_pass_applied:
        draft_relative = f"qa/reflection/{unit_id}.draft.md"
        draft_restored = _restore_chunks(protected_chunks, translated_chunks)
        atomic_write_text(
            _project_path(project_root, draft_relative),
            draft_restored,
        )
        reflection_chunks: list[str] = []
        revised_chunks: list[str] = []
        second_pass_checkpoint_store = CheckpointStore(
            project_root
            / "chapters"
            / "translated"
            / ".partial"
            / "reflection"
        )
        # The reflection revises the first pass's drafts, so it inherits
        # everything the first pass depended on. Keyed only on "reflection-v1",
        # a resumed reflection could reuse revisions computed from drafts that a
        # different text-cleanup or custom-translation setting had since
        # replaced. Composing the first-pass id in makes the key describe what
        # the checkpoint actually depends on; a mismatch now redoes the
        # reflection instead of blending two passes.
        second_pass_idempotency_key = UnitIdempotencyKey(
            task_manifest_sha256=_sha256(task_bytes),
            provider_profile_id=provider.profile_id,
            provider_config_id=provider.config_id,
            translation_policy_version=translation_policy_version,
            pass_id=_custom_instruction_pass_id(
                f"reflection-v1+{translation_pass_id}", custom_reflection
            ),
        )
        second_pass_checkpoint = second_pass_checkpoint_store.load(
            unit_id,
            second_pass_idempotency_key,
        )
        if (
            second_pass_checkpoint is not None
            and (
                second_pass_checkpoint.next_chunk_index > len(chunks)
                or len(second_pass_checkpoint.reflection_chunks)
                != second_pass_checkpoint.next_chunk_index
            )
        ):
            second_pass_checkpoint_store.delete(unit_id)
            second_pass_checkpoint = None
        if second_pass_checkpoint is not None:
            second_pass_resumed_chunk_count = (
                second_pass_checkpoint.next_chunk_index
            )
            reflection_chunks = list(second_pass_checkpoint.reflection_chunks)
            revised_chunks = list(second_pass_checkpoint.translated_chunks)
        for index in range(second_pass_resumed_chunk_count, len(chunks)):
            source_chunk = chunks[index]
            result = run_second_pass_chunk(
                second_pass,
                SecondPassRequest(
                    source_text=source_chunk,
                    draft_text=translated_chunks[index],
                    previous_source_text=chunks[index - 1] if index > 0 else None,
                    previous_draft_text=(
                        translated_chunks[index - 1] if index > 0 else None
                    ),
                    next_source_text=(
                        chunks[index + 1] if index + 1 < len(chunks) else None
                    ),
                    next_draft_text=(
                        translated_chunks[index + 1]
                        if index + 1 < len(translated_chunks)
                        else None
                    ),
                    source_language=source_language,
                    target_language=target_language,
                    terminology_criteria=target_profile.build_system_instruction(
                        source_text=source_chunk,
                        task_manifest=profile_task,
                    ),
                    custom_instruction=custom_reflection,
                ),
                candidate_validator=lambda candidate, protected=protected_chunks[index]: (
                    _candidate_preserves_structure(protected, candidate)
                ),
            )
            reflection_chunks.append(result.reflection_text)
            revised_chunks.append(result.revised_text)
            second_pass_checkpoint_store.save(
                unit_id,
                second_pass_idempotency_key,
                UnitCheckpoint(
                    next_chunk_index=index + 1,
                    translated_chunks=tuple(revised_chunks),
                    reflection_chunks=tuple(reflection_chunks),
                ),
            )
        reflection_text = _render_reflection_evidence(reflection_chunks)
        reflection_relative = f"qa/reflection/{unit_id}.reflection.md"
        atomic_write_text(
            _project_path(project_root, reflection_relative),
            reflection_text,
        )
    output_chunks = revised_chunks if second_pass_applied else translated_chunks
    # Checked against what is actually delivered, so a reflection pass that
    # walks a term back is caught rather than hidden behind the draft.
    glossary_violations = _collect_glossary_violations(
        chunks=chunks,
        output_chunks=output_chunks,
        profile_task=profile_task,
        skip_indices=source_fallback_indices,
    )
    restored = _restore_chunks(protected_chunks, output_chunks)
    complete = source_fallback_count == 0
    output_relative = (
        f"chapters/translated/{unit_id}.md"
        if complete
        else f"chapters/translated/.partial/{unit_id}.degraded.md"
    )
    output_path = _project_path(project_root, output_relative)
    atomic_write_text(output_path, restored)
    if complete:
        checkpoint_store.delete(unit_id)
        if second_pass_checkpoint_store is not None:
            second_pass_checkpoint_store.delete(unit_id)
        _project_path(
            project_root,
            f"chapters/translated/.partial/{unit_id}.degraded.md",
        ).unlink(missing_ok=True)
    report = {
        "unitId": unit_id,
        "status": "completed" if complete else "failed",
        "metrics": {
            "alignedFallbackCount": aligned_fallback_count,
            "chunkCount": len(chunks),
            "glossaryViolationCount": len(glossary_violations),
            "providerAttemptCount": provider_attempt_count,
            "resumedChunkCount": resumed_chunk_count,
            "secondPassApplied": second_pass_applied,
            "secondPassResumedChunkCount": second_pass_resumed_chunk_count,
            "sourceFallbackCount": source_fallback_count,
            "tokenCounter": "utf8-byte-upper-bound-v1",
        },
        "artifact": {
            "kind": (
                "chapter_translation" if complete else "chapter_translation_degraded"
            ),
            "path": output_relative,
            "sha256": _sha256(restored.encode()),
            "complete": complete,
        },
    }
    if glossary_violations:
        # The count alone says a book drifted somewhere in 400 pages, which is
        # not something anyone can act on. The terms are what make it fixable.
        report["glossaryViolations"] = [
            {"source": entry.source, "translation": entry.translation}
            for entry in glossary_violations
        ]
    if second_pass_applied:
        second_pass_artifacts = {
            "draft": {
                "kind": "chapter_translation_draft",
                "path": draft_relative,
                "sha256": _sha256(draft_restored.encode()),
                "complete": True,
            },
            "reflection": {
                "kind": "chapter_translation_reflection",
                "path": reflection_relative,
                "sha256": _sha256(reflection_text.encode()),
                "complete": True,
            },
            "revised": report["artifact"],
        }
        report["secondPassArtifacts"] = second_pass_artifacts
    if not complete:
        report["error"] = {"code": "translation_incomplete", "retryable": True}
    return report


def _collect_glossary_violations(
    *,
    chunks: Sequence[str],
    output_chunks: Sequence[str],
    profile_task: Mapping[str, object],
    skip_indices: AbstractSet[int],
) -> tuple[GlossaryEntry, ...]:
    """Union the per-chunk violations, keeping first-seen order.

    Per chunk because that is the granularity the glossary is injected at: each
    chunk demands only the terms its own source text contains. Deduplicated
    across chunks because a term missing from nine chapters is one thing to fix,
    not nine, and a count that scaled with chapter length would say more about
    the book's structure than its terminology.
    """
    seen: dict[tuple[str, str], GlossaryEntry] = {}
    for index, source_chunk in enumerate(chunks):
        if index in skip_indices or index >= len(output_chunks):
            continue
        for entry in find_glossary_violations(
            source_chunk, output_chunks[index], profile_task
        ):
            seen.setdefault((entry.source, entry.translation), entry)
    return tuple(seen.values())


def _read_json(path: Path) -> dict[str, Any]:
    value = json.loads(path.read_text(encoding="utf-8"))
    if not isinstance(value, dict):
        raise EngineError("invalid_json_object")
    return value


def apply_model_override(provider: LLMProvider, manifest: Mapping[str, Any]) -> None:
    """Point the provider at a run-chosen model, keeping the registry default
    when the manifest names none. The model is part of the provider's identity
    (it goes into every request), so overriding it here means the sample and the
    full run agree without a second lookup path."""
    model = manifest.get("model")
    if model is None:
        return
    if not isinstance(model, str) or not model.strip():
        raise EngineError("invalid_model_override")
    provider.model = model


def _parse_text_cleanup(manifest: dict[str, Any]) -> bool:
    text_cleanup = manifest.get("textCleanup", False)
    if not isinstance(text_cleanup, bool):
        raise EngineError("invalid_text_cleanup")
    return text_cleanup


def _parse_custom_instructions(
    manifest: dict[str, Any],
) -> tuple[str | None, str | None]:
    if "customInstructions" not in manifest:
        return None, None
    value = manifest["customInstructions"]
    if not isinstance(value, dict) or any(
        key not in {"translation", "reflection"} for key in value
    ):
        raise EngineError("invalid_custom_instructions")

    parsed: list[str | None] = []
    for key in ("translation", "reflection"):
        if key not in value:
            parsed.append(None)
            continue
        instruction = value[key]
        if not isinstance(instruction, str):
            raise EngineError("invalid_custom_instructions")
        if len(instruction) > MAX_CUSTOM_INSTRUCTION_CHARACTERS:
            raise EngineError("custom_instructions_too_long")
        parsed.append(instruction or None)
    return parsed[0], parsed[1]


def _custom_instruction_pass_id(base: str, instruction: str | None) -> str:
    if not instruction:
        return base
    digest = _sha256(instruction.encode())[:16]
    return f"{base}-custom-{digest}"


def _restore_chunks(
    protected_chunks: list[ProtectedMarkdown], translated_chunks: list[str]
) -> str:
    if len(protected_chunks) != len(translated_chunks):
        raise ValueError("translated chunk count mismatch")
    return "".join(
        protected.restore(translated)
        for protected, translated in zip(
            protected_chunks, translated_chunks, strict=True
        )
    )


def _candidate_preserves_structure(
    protected: ProtectedMarkdown, candidate: str
) -> bool:
    try:
        source = protected.restore(protected.text)
        translated = protected.restore(candidate)
    except ValueError:
        return False
    return (
        _markdown_heading_shape(source) == _markdown_heading_shape(translated)
        and _markdown_content_block_count(source)
        == _markdown_content_block_count(translated)
    )


def _markdown_heading_shape(text: str) -> list[int]:
    levels: list[int] = []
    in_fence = False
    for line in text.splitlines():
        trimmed = line.lstrip()
        if trimmed.startswith("```") or trimmed.startswith("~~~"):
            in_fence = not in_fence
            continue
        if in_fence:
            continue
        hashes = len(trimmed) - len(trimmed.lstrip("#"))
        if 1 <= hashes <= 6 and (
            len(trimmed) == hashes or trimmed[hashes] in " \t"
        ):
            levels.append(hashes)
    return levels


def _markdown_content_block_count(text: str) -> int:
    count = 0
    in_block = False
    for line in text.splitlines():
        content = bool(line.strip()) and not _line_is_heading(line)
        if content and not in_block:
            count += 1
        in_block = content
    return count


def _line_is_heading(line: str) -> bool:
    trimmed = line.lstrip()
    hashes = len(trimmed) - len(trimmed.lstrip("#"))
    return 1 <= hashes <= 6 and (
        len(trimmed) == hashes or trimmed[hashes] in " \t"
    )


def _required_string(value: Any, key: str) -> str:
    if not isinstance(value, dict) or not isinstance(value.get(key), str) or not value[key]:
        raise EngineError("missing_required_field")
    return value[key]


def _project_path(project_root: Path, relative: str) -> Path:
    candidate = (project_root / relative).resolve()
    if candidate != project_root and project_root not in candidate.parents:
        raise EngineError("path_outside_project")
    return candidate


def _safe_unit_id(project_root: Path, unit: Any) -> str:
    try:
        task_path = _project_path(project_root, _required_string(unit, "taskManifestPath"))
        task = _read_json(task_path)
        return _required_string(task, "chapterId")
    except (OSError, json.JSONDecodeError, EngineError):
        return "unknown"


def _sha256(value: bytes) -> str:
    return hashlib.sha256(value).hexdigest()


def _translation_context_tail(text: str, word_limit: int = 25) -> str:
    word_starts: list[int] = []
    inside_non_cjk_word = False
    for index, character in enumerate(text):
        if _is_cjk_ideograph(character):
            word_starts.append(index)
            inside_non_cjk_word = False
        elif character.isalnum() or character == "_":
            if not inside_non_cjk_word:
                word_starts.append(index)
            inside_non_cjk_word = True
        else:
            inside_non_cjk_word = False
    if not word_starts:
        return ""
    tail_start = word_starts[-word_limit] if len(word_starts) >= word_limit else 0
    return text[tail_start:].strip()


def _is_cjk_ideograph(character: str) -> bool:
    codepoint = ord(character)
    return (
        0x3400 <= codepoint <= 0x4DBF
        or 0x4E00 <= codepoint <= 0x9FFF
        or 0xF900 <= codepoint <= 0xFAFF
    )


def _render_reflection_evidence(reflections: list[str]) -> str:
    sections = ["# Windowed reflection QA evidence\n\n"]
    for index, reflection in enumerate(reflections, start=1):
        sections.append(f"## Chunk {index}\n\n{reflection.rstrip()}\n\n")
    return "".join(sections)
