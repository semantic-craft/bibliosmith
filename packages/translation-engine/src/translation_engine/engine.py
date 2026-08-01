from __future__ import annotations

from concurrent.futures import ThreadPoolExecutor
from copy import deepcopy
import hashlib
import json
import re
import threading
from dataclasses import dataclass
from pathlib import Path
from typing import AbstractSet, Any, Callable, Mapping, Sequence

from .checkpoint import (
    CheckpointStore,
    CompletionStore,
    UnitCheckpoint,
    UnitCompletion,
    UnitIdempotencyKey,
)
from .chunking import TokenChunker, Utf8ByteTokenCounter
from .files import atomic_write_text
from .glossary import (
    GlossaryEntry,
    bind_glossary_entries,
    find_glossary_violations,
    find_variant_spans,
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
    PLACEHOLDER_PATTERN,
    ProtectedMarkdown,
    protect_chunk_structure,
    protect_markdown_for_chunking,
)
from .profiles import TargetLanguageProfile, get_target_profile
from .progress import OperationProgress
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

# Enough places to see a pattern, few enough that a book-length report stays
# readable and small on the way through stdout into the job's state.
MAX_VIOLATION_OCCURRENCES = 2
VIOLATION_EXCERPT_CHARACTERS = 200
ADJACENT_REPEAT_PATTERN = re.compile(r"(?P<unit>.{4,15})(?P=unit)", re.DOTALL)
PRESERVED_LITERAL_PATTERN = re.compile(
    r"(?:https?://|www\.)\S+"
    r"|[\w.+-]+@[\w.-]+\.[A-Za-z]{2,}"
    r"|10\.\d{4,9}/\S+"
    r"|\b\d{10,}\b"
    r"|[A-Za-z0-9._~:/?#\[\]@!$&'()*+,;=%-]{80,}",
    re.IGNORECASE,
)


ProviderFactory = Callable[..., LLMProvider]
TargetProfileFactory = Callable[[str], TargetLanguageProfile]


@dataclass(frozen=True)
class TranslationUnitSource:
    unit_id: str
    source_text: str
    task_bytes: bytes
    profile_task: dict[str, Any]


@dataclass(frozen=True)
class TranslationWorkPlan:
    total: int
    resumed_by_unit: Mapping[str, int]


@dataclass(frozen=True)
class GlossaryViolationOccurrence:
    """One place a demanded term appeared, next to what came back instead."""

    chunk_index: int
    source_excerpt: str
    translated_excerpt: str


@dataclass(frozen=True)
class GlossaryViolation:
    entry: GlossaryEntry
    occurrences: tuple[GlossaryViolationOccurrence, ...]


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
    planned_work = _planned_translation_work(
        project_root=project_root,
        source_map=source_map,
        units=units,
        target_language=target_language,
        max_tokens=max_tokens,
        pass_count=2 if second_pass_enabled else 1,
        provider=provider,
        translation_policy_version=translation_policy_version,
        text_cleanup=text_cleanup,
        custom_translation=custom_translation,
        custom_reflection=custom_reflection,
    )
    operation_progress = OperationProgress.from_environment(
        stage_id="translate", unit_kind="chunks", total=planned_work.total
    )
    operation_progress.start("translating")
    for unit_id, completed in planned_work.resumed_by_unit.items():
        operation_progress.update_item(unit_id, completed, "translating")
    placeholder_retries = manifest.get("placeholderRetries", 1)
    if (
        not isinstance(placeholder_retries, int)
        or isinstance(placeholder_retries, bool)
        or placeholder_retries < 0
    ):
        raise EngineError("invalid_placeholder_retries")

    # Latched once by whichever unit exhausts the provider's throttle budget, and
    # then read by every unit that has not started yet. It stops dispatch; it
    # never interrupts a unit already in flight. See _rate_limited_report.
    dispatch_stopped = threading.Event()

    def translate_one(unit: Any) -> dict[str, Any]:
        try:
            if dispatch_stopped.is_set():
                return _rate_limited_report(project_root, unit)
            operation_progress.touch("translating")
            return _translate_unit(
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
                progress_callback=operation_progress.update_item,
            )
        except (
            OSError,
            json.JSONDecodeError,
            EngineError,
            ProviderError,
            ValueError,
        ) as error:
            if isinstance(error, RateLimitError):
                dispatch_stopped.set()
            return {
                "unitId": _safe_unit_id(project_root, unit),
                "status": "failed",
                "error": {
                    "code": getattr(error, "code", "unit_invalid"),
                    "retryable": getattr(
                        error,
                        "retryable",
                        isinstance(error, ProviderUnavailableError),
                    ),
                },
            }
    # Units overlap up to the provider's declared limit. Chunks inside a unit
    # stay strictly serial -- each chunk's prompt carries the tail of the previous
    # chunk's translation -- and every unit writes only paths derived from its own
    # unit id, so two units never contend for a checkpoint or an output file.
    with ThreadPoolExecutor(
        max_workers=_unit_concurrency(provider, len(units)),
        thread_name_prefix="translation-unit",
    ) as pool:
        futures = [pool.submit(translate_one, unit) for unit in units]
        try:
            # Indexed by submission order, so the report follows the manifest's
            # unit list and does not drift with whichever unit finished first.
            unit_reports = [future.result() for future in futures]
        except BaseException:
            # A worker died in a way no per-unit handler covers: a kill, a signal,
            # a bug. Drop the units that have not started rather than translating
            # more of a run that is already over; the ones in flight cannot be
            # interrupted and resume from their own checkpoints.
            for future in futures:
                future.cancel()
            raise

    completed = sum(report["status"] == "completed" for report in unit_reports)
    failed = len(unit_reports) - completed
    return {
        "schema": "translation-engine-report-v1",
        "summary": {"total": len(unit_reports), "completed": completed, "failed": failed},
        "units": unit_reports,
    }


def _planned_translation_work(
    *,
    project_root: Path,
    source_map: Mapping[str, Any],
    units: Sequence[Any],
    target_language: str,
    max_tokens: int,
    pass_count: int,
    provider: LLMProvider,
    translation_policy_version: str,
    text_cleanup: bool,
    custom_translation: str | None,
    custom_reflection: str | None,
) -> TranslationWorkPlan:
    total = 0
    resumed_by_unit: dict[str, int] = {}
    checkpoint_store = CheckpointStore(
        project_root / "chapters" / "translated" / ".partial"
    )
    reflection_store = CheckpointStore(
        project_root
        / "chapters"
        / "translated"
        / ".partial"
        / "reflection"
    )
    completion_store = CompletionStore(
        project_root / "chapters" / "translated" / ".completed"
    )
    translation_pass_id = _custom_instruction_pass_id(
        "translation-v1-text-cleanup" if text_cleanup else "translation-v1",
        custom_translation,
    )
    for unit in units:
        try:
            task_path = _project_path(
                project_root, _required_string(unit, "taskManifestPath")
            )
            loaded = load_translation_unit(
                project_root=project_root,
                source_map=source_map,
                task_path=task_path,
                target_language=target_language,
            )
            protected = protect_markdown_for_chunking(loaded.source_text)
            chunk_count = len(
                TokenChunker(max_tokens=max_tokens, counter=Utf8ByteTokenCounter()).split(
                    protected.text
                )
            )
            total += chunk_count * pass_count
            completion_key = _completion_key(
                task_bytes=loaded.task_bytes,
                provider=provider,
                translation_policy_version=translation_policy_version,
                translation_pass_id=translation_pass_id,
                max_tokens=max_tokens,
                second_pass_enabled=pass_count == 2,
                custom_reflection=custom_reflection,
            )
            completion = completion_store.load(loaded.unit_id, completion_key)
            if completion is not None:
                if _completion_is_valid(
                    project_root=project_root,
                    unit_id=loaded.unit_id,
                    chunk_count=chunk_count,
                    second_pass_enabled=pass_count == 2,
                    completion=completion,
                ):
                    resumed_by_unit[loaded.unit_id] = chunk_count * pass_count
                    continue
                completion_store.delete(loaded.unit_id)
            first_pass_key = UnitIdempotencyKey(
                task_manifest_sha256=_sha256(loaded.task_bytes),
                provider_profile_id=provider.profile_id,
                provider_config_id=provider.config_id,
                translation_policy_version=translation_policy_version,
                pass_id=translation_pass_id,
            )
            checkpoint = checkpoint_store.load(loaded.unit_id, first_pass_key)
            if checkpoint is not None and checkpoint.next_chunk_index > chunk_count:
                checkpoint_store.delete(loaded.unit_id)
                checkpoint = None
            completed = checkpoint.next_chunk_index if checkpoint is not None else 0

            if pass_count == 2:
                if completed == chunk_count:
                    reflection_key = UnitIdempotencyKey(
                        task_manifest_sha256=_sha256(loaded.task_bytes),
                        provider_profile_id=provider.profile_id,
                        provider_config_id=provider.config_id,
                        translation_policy_version=translation_policy_version,
                        pass_id=_custom_instruction_pass_id(
                            f"reflection-v1+{translation_pass_id}", custom_reflection
                        ),
                    )
                    reflection = reflection_store.load(loaded.unit_id, reflection_key)
                    if reflection is not None and (
                        reflection.next_chunk_index > chunk_count
                        or len(reflection.reflection_chunks)
                        != reflection.next_chunk_index
                    ):
                        reflection_store.delete(loaded.unit_id)
                        reflection = None
                    if reflection is not None:
                        completed += reflection.next_chunk_index
            if completed > 0:
                resumed_by_unit[loaded.unit_id] = completed
        except (OSError, json.JSONDecodeError, EngineError, ValueError):
            total += pass_count
    return TranslationWorkPlan(max(1, total), resumed_by_unit)


def _completion_key(
    *,
    task_bytes: bytes,
    provider: LLMProvider,
    translation_policy_version: str,
    translation_pass_id: str,
    max_tokens: int,
    second_pass_enabled: bool,
    custom_reflection: str | None,
) -> UnitIdempotencyKey:
    model = getattr(provider, "model", "")
    model_suffix = f"+model-{_sha256(model.encode())[:16]}" if model else ""
    completed_pass_id = (
        f"completion-v1+{translation_pass_id}+max-{max_tokens}{model_suffix}"
    )
    if second_pass_enabled:
        reflection_pass_id = _custom_instruction_pass_id(
            f"reflection-v1+{translation_pass_id}", custom_reflection
        )
        completed_pass_id = (
            f"completion-v1+{reflection_pass_id}+max-{max_tokens}{model_suffix}"
        )
    return UnitIdempotencyKey(
        task_manifest_sha256=_sha256(task_bytes),
        provider_profile_id=provider.profile_id,
        provider_config_id=provider.config_id,
        translation_policy_version=translation_policy_version,
        pass_id=completed_pass_id,
    )


def _completion_is_valid(
    *,
    project_root: Path,
    unit_id: str,
    chunk_count: int,
    second_pass_enabled: bool,
    completion: UnitCompletion,
) -> bool:
    report = completion.report
    metrics = report.get("metrics")
    artifact = report.get("artifact")
    if (
        completion.chunk_count != chunk_count
        or report.get("unitId") != unit_id
        or report.get("status") != "completed"
        or not isinstance(metrics, dict)
        or metrics.get("chunkCount") != chunk_count
        or metrics.get("secondPassApplied") is not second_pass_enabled
        or not _completion_artifact_matches(
            project_root,
            artifact,
            expected_path=f"chapters/translated/{unit_id}.md",
        )
    ):
        return False
    second_pass_artifacts = report.get("secondPassArtifacts")
    if not second_pass_enabled:
        return second_pass_artifacts is None
    if not isinstance(second_pass_artifacts, dict):
        return False
    return (
        _completion_artifact_matches(
            project_root,
            second_pass_artifacts.get("draft"),
            expected_path=f"qa/reflection/{unit_id}.draft.md",
        )
        and _completion_artifact_matches(
            project_root,
            second_pass_artifacts.get("reflection"),
            expected_path=f"qa/reflection/{unit_id}.reflection.md",
        )
        and second_pass_artifacts.get("revised") == artifact
    )


def _completion_artifact_matches(
    project_root: Path,
    artifact: Any,
    *,
    expected_path: str,
) -> bool:
    if (
        not isinstance(artifact, dict)
        or artifact.get("path") != expected_path
        or artifact.get("complete") is not True
        or not isinstance(artifact.get("sha256"), str)
    ):
        return False
    try:
        path = _project_path(project_root, expected_path)
        return path.is_file() and _sha256(path.read_bytes()) == artifact["sha256"]
    except OSError:
        return False


def _reused_completion_report(completion: UnitCompletion) -> dict[str, Any]:
    report = deepcopy(completion.report)
    metrics = report["metrics"]
    metrics["completionReused"] = True
    metrics["providerAttemptCount"] = 0
    metrics["resumedChunkCount"] = completion.chunk_count
    metrics["secondPassResumedChunkCount"] = (
        completion.chunk_count if metrics.get("secondPassApplied") else 0
    )
    return report


def _unit_concurrency(provider: LLMProvider, unit_count: int) -> int:
    """How many units may translate at once against this provider.

    The limit lives on the provider because it describes what the remote service
    tolerates, not something a run gets to choose. A provider that declares none
    -- the offline fake, or any client written before the limit existed -- runs one
    unit at a time, which is what the engine did before units could overlap.
    """
    return max(1, min(getattr(provider, "concurrency_limit", 1), unit_count))


def _rate_limited_report(project_root: Path, unit: Any) -> dict[str, Any]:
    """A unit that was never attempted because the run is already throttled.

    Once every credential is exhausted for longer than the provider is willing to
    wait, starting another unit would only churn the same throttle, so units that
    have not begun are failed retryable without a single request. Units already in
    flight are deliberately left alone: one of them may hold a credential that is
    still good, the throttle may lift before its next chunk, and killing it would
    throw away a call that has already been paid for along with the checkpoint
    prefix it was about to write. Each of those reports its own real outcome.
    """
    return {
        "unitId": _safe_unit_id(project_root, unit),
        "status": "failed",
        "error": {"code": RateLimitError.code, "retryable": True},
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
    progress_callback: Callable[[str, int, str], None] | None = None,
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
    completion_store = CompletionStore(
        project_root / "chapters" / "translated" / ".completed"
    )
    completion_key = _completion_key(
        task_bytes=task_bytes,
        provider=provider,
        translation_policy_version=translation_policy_version,
        translation_pass_id=translation_pass_id,
        max_tokens=max_tokens,
        second_pass_enabled=second_pass is not None,
        custom_reflection=custom_reflection,
    )
    completion = completion_store.load(unit_id, completion_key)
    if completion is not None:
        if _completion_is_valid(
            project_root=project_root,
            unit_id=unit_id,
            chunk_count=len(chunks),
            second_pass_enabled=second_pass is not None,
            completion=completion,
        ):
            checkpoint_store.delete(unit_id)
            CheckpointStore(
                project_root
                / "chapters"
                / "translated"
                / ".partial"
                / "reflection"
            ).delete(unit_id)
            _project_path(
                project_root,
                f"chapters/translated/.partial/{unit_id}.degraded.md",
            ).unlink(missing_ok=True)
            if progress_callback is not None:
                progress_callback(
                    unit_id,
                    len(chunks) * (2 if second_pass is not None else 1),
                    "completed",
                )
            return _reused_completion_report(completion)
        completion_store.delete(unit_id)
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
    if progress_callback is not None:
        progress_callback(unit_id, resumed_chunk_count, "translating")
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
    persisted_chunk_count = resumed_chunk_count
    for index in range(resumed_chunk_count, len(chunks)):
        if progress_callback is not None:
            progress_callback(unit_id, persisted_chunk_count, "translating")
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
                context_block = (
                    "# PREVIOUS TRANSLATION CONTEXT — REFERENCE ONLY\n"
                    "This text is not part of the current source segment. Use it only "
                    "for terminology and continuity. Do not reproduce or continue it "
                    "as a passage. Reuse individual terms only when the current source "
                    "independently requires them.\n"
                    f"{previous_translation_tail}\n\n"
                    "# CURRENT SEGMENT ONLY\n"
                    "Translate only the source segment in the user message. Do not "
                    "reproduce the reference context as part of the answer."
                )
                if custom_translation:
                    chunk_system_instruction = (
                        f"{context_block}\n\n{system_instruction}"
                    )
                else:
                    chunk_system_instruction = (
                        f"{system_instruction}\n\n{context_block}"
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
            candidate_normalizer=lambda candidate, protected=protected_chunks[index]: (
                _normalize_candidate_structure(protected, candidate)
            ),
            candidate_validator=lambda candidate, protected=protected_chunks[index]: (
                _candidate_is_acceptable(
                    protected,
                    candidate,
                    target_language=target_profile.language,
                )
                and _candidate_avoids_previous_echo(
                    previous_source=chunks[index - 1] if index > 0 else None,
                    current_source=chunks[index],
                    previous_candidate=(
                        translated_chunks[-1] if translated_chunks else None
                    ),
                    candidate=candidate,
                )
            ),
            candidate_repair=(
                None
                if text_cleanup or custom_translation
                else lambda candidate, protected=protected_chunks[index]: (
                    _remove_unprotected_paragraph_breaks(protected, candidate)
                )
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
            persisted_chunk_count = index + 1
        if progress_callback is not None:
            progress_callback(unit_id, persisted_chunk_count, "translating")
    second_pass_applied = second_pass is not None and source_fallback_count == 0
    second_pass_resumed_chunk_count = 0
    second_pass_draft_fallbacks: list[bool] = []
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
            second_pass_draft_fallbacks = list(
                second_pass_checkpoint.second_pass_draft_fallbacks
            ) or [False] * second_pass_checkpoint.next_chunk_index
        if progress_callback is not None:
            progress_callback(
                unit_id,
                len(chunks) + second_pass_resumed_chunk_count,
                "reviewing",
            )
        for index in range(second_pass_resumed_chunk_count, len(chunks)):
            if progress_callback is not None:
                progress_callback(unit_id, len(chunks) + index, "reviewing")
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
                candidate_retries=placeholder_retries,
                candidate_normalizer=lambda candidate, protected=protected_chunks[index]: (
                    _normalize_candidate_structure(protected, candidate)
                ),
                candidate_validator=lambda candidate, protected=protected_chunks[index]: (
                    _candidate_is_acceptable(
                        protected,
                        candidate,
                        target_language=target_profile.language,
                    )
                    and _candidate_avoids_previous_echo(
                        previous_source=chunks[index - 1] if index > 0 else None,
                        current_source=chunks[index],
                        previous_candidate=(
                            revised_chunks[-1] if revised_chunks else None
                        ),
                        candidate=candidate,
                    )
                ),
                draft_fallback_validator=(
                    None
                    if custom_reflection
                    else lambda candidate, protected=protected_chunks[index]: (
                        _candidate_allows_validated_draft_fallback(
                            protected, candidate
                        )
                    )
                ),
            )
            reflection_chunks.append(result.reflection_text)
            revised_chunks.append(result.revised_text)
            second_pass_draft_fallbacks.append(result.draft_fallback)
            second_pass_checkpoint_store.save(
                unit_id,
                second_pass_idempotency_key,
                UnitCheckpoint(
                    next_chunk_index=index + 1,
                    translated_chunks=tuple(revised_chunks),
                    reflection_chunks=tuple(reflection_chunks),
                    second_pass_draft_fallbacks=tuple(
                        second_pass_draft_fallbacks
                    ),
                ),
            )
            if progress_callback is not None:
                progress_callback(unit_id, len(chunks) + index + 1, "reviewing")
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
    report = {
        "unitId": unit_id,
        "status": "completed" if complete else "failed",
        "metrics": {
            "alignedFallbackCount": aligned_fallback_count,
            "chunkCount": len(chunks),
            "completionReused": False,
            "glossaryViolationCount": len(glossary_violations),
            "providerAttemptCount": provider_attempt_count,
            "resumedChunkCount": resumed_chunk_count,
            "secondPassApplied": second_pass_applied,
            "secondPassDraftFallbackCount": sum(second_pass_draft_fallbacks),
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
        # not something anyone can act on. A structured finding is: the term, the
        # translation it was supposed to get, the chunk it happened in, and the
        # source and output text of the stretch it happened in. Reported as
        # evidence and never as a gate -- see the note on find_glossary_violations
        # for why a signal with known false positives must not block a chapter.
        report["glossaryViolations"] = [
            {
                "source": violation.entry.source,
                "translation": violation.entry.translation,
                "occurrences": [
                    {
                        "chunkIndex": occurrence.chunk_index,
                        "sourceExcerpt": occurrence.source_excerpt,
                        "translatedExcerpt": occurrence.translated_excerpt,
                    }
                    for occurrence in violation.occurrences
                ],
            }
            for violation in glossary_violations
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
        report["error"] = {
            "code": "translation_structure_invalid",
            "retryable": True,
        }
    else:
        completion_store.save(
            unit_id,
            completion_key,
            UnitCompletion(chunk_count=len(chunks), report=report),
        )
        checkpoint_store.delete(unit_id)
        if second_pass_checkpoint_store is not None:
            second_pass_checkpoint_store.delete(unit_id)
        _project_path(
            project_root,
            f"chapters/translated/.partial/{unit_id}.degraded.md",
        ).unlink(missing_ok=True)
    return report


def _collect_glossary_violations(
    *,
    chunks: Sequence[str],
    output_chunks: Sequence[str],
    profile_task: Mapping[str, object],
    skip_indices: AbstractSet[int],
) -> tuple[GlossaryViolation, ...]:
    """Union the per-chunk violations, keeping first-seen order.

    Per chunk because that is the granularity the glossary is injected at: each
    chunk demands only the terms its own source text contains. Deduplicated
    across chunks because a term missing from nine chapters is one thing to fix,
    not nine, and a count that scaled with chapter length would say more about
    the book's structure than its terminology.
    """
    entries: dict[tuple[str, str], GlossaryEntry] = {}
    occurrences: dict[tuple[str, str], list[GlossaryViolationOccurrence]] = {}
    for index, source_chunk in enumerate(chunks):
        if index in skip_indices or index >= len(output_chunks):
            continue
        for entry in find_glossary_violations(
            source_chunk, output_chunks[index], profile_task
        ):
            key = (entry.source, entry.translation)
            entries.setdefault(key, entry)
            found = occurrences.setdefault(key, [])
            budget = MAX_VIOLATION_OCCURRENCES - len(found)
            if budget > 0:
                found.extend(
                    _violation_occurrences(
                        chunk_index=index,
                        source_chunk=source_chunk,
                        output_chunk=output_chunks[index],
                        entry=entry,
                    )[:budget]
                )
    return tuple(
        GlossaryViolation(entry=entry, occurrences=tuple(occurrences[key]))
        for key, entry in entries.items()
    )


def _violation_occurrences(
    *,
    chunk_index: int,
    source_chunk: str,
    output_chunk: str,
    entry: GlossaryEntry,
) -> list[GlossaryViolationOccurrence]:
    """Pair each source occurrence of the term with what the model wrote there.

    A term and an expected translation say a book drifted; they do not say what
    it drifted to, and nobody can act on that without opening the chapter. The
    pairing is exact rather than approximate: a candidate is only accepted when
    it carries the chunk's protected placeholders once each and in order, and
    paragraph breaks and heading prefixes are themselves placeholders, so
    splitting source and output on the placeholder pattern yields the same number
    of segments in the same order. Segment *i* of the output is therefore the
    model's rendering of segment *i* of the source.
    """
    source_segments = _placeholder_segments(source_chunk)
    output_segments = _placeholder_segments(output_chunk)
    if len(source_segments) != len(output_segments):
        # Unreachable while the validators hold; reporting the term without a
        # location beats an IndexError in a run that otherwise succeeded.
        return []
    found: list[GlossaryViolationOccurrence] = []
    reported: set[int] = set()
    for start, end in find_variant_spans(source_chunk, entry):
        position = _segment_index(source_segments, start)
        # One entry per segment, not per match: a term used three times in the
        # same stretch of text is one place to look, and repeating the excerpt
        # would spend the occurrence budget saying the same thing.
        if position in reported:
            continue
        reported.add(position)
        segment_start, segment_text = source_segments[position]
        found.append(
            GlossaryViolationOccurrence(
                chunk_index=chunk_index,
                source_excerpt=_excerpt(
                    segment_text, start - segment_start, end - segment_start
                ),
                translated_excerpt=_excerpt(output_segments[position][1], 0, 0),
            )
        )
    return found


def _placeholder_segments(text: str) -> list[tuple[int, str]]:
    """The (offset, text) runs between protected placeholders, including empties."""
    segments: list[tuple[int, str]] = []
    cursor = 0
    for placeholder in PLACEHOLDER_PATTERN.finditer(text):
        segments.append((cursor, text[cursor : placeholder.start()]))
        cursor = placeholder.end()
    segments.append((cursor, text[cursor:]))
    return segments


def _segment_index(segments: Sequence[tuple[int, str]], position: int) -> int:
    last = 0
    for index, (start, _) in enumerate(segments):
        if start > position:
            break
        last = index
    return last


def _excerpt(text: str, start: int, end: int) -> str:
    """A window of at most VIOLATION_EXCERPT_CHARACTERS centred on [start, end).

    Bounded because this report is JSON on stdout that the launcher folds into a
    job's state: a four-hundred-page book must stay something a person can read.
    """
    if len(text) <= VIOLATION_EXCERPT_CHARACTERS:
        return text.strip()
    # Offsets are the caller's, measured against this exact string, so the window
    # is computed before anything is trimmed and only the result is stripped.
    span = max(end - start, 0)
    margin = max((VIOLATION_EXCERPT_CHARACTERS - span) // 2, 0)
    window_start = max(min(start - margin, len(text) - VIOLATION_EXCERPT_CHARACTERS), 0)
    window_end = window_start + VIOLATION_EXCERPT_CHARACTERS
    prefix = "…" if window_start > 0 else ""
    suffix = "…" if window_end < len(text) else ""
    return f"{prefix}{text[window_start:window_end].strip()}{suffix}"


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
    source_chunks = [
        protected.restore(protected.text) for protected in protected_chunks
    ]
    output_chunks = [
        protected.restore(translated)
        for protected, translated in zip(
            protected_chunks, translated_chunks, strict=True
        )
    ]

    def edge_whitespace(source_edge: str, output_edge: str) -> str:
        if "\n" in source_edge or "\r" in source_edge:
            return source_edge
        if "\n" in output_edge or "\r" in output_edge:
            return ""
        return output_edge

    rebuilt_chunks: list[str] = []
    for source, output in zip(source_chunks, output_chunks, strict=True):
        if not source or source.isspace():
            rebuilt_chunks.append(edge_whitespace(source, output))
            continue
        source_leading = re.match(r"\s*", source).group(0)
        source_trailing = re.search(r"\s*$", source).group(0)
        output_leading = re.match(r"\s*", output).group(0)
        output_trailing = re.search(r"\s*$", output).group(0)
        output_core = output[len(output_leading) :]
        if output_trailing:
            output_core = output_core[: -len(output_trailing)]
        rebuilt_chunks.append(
            edge_whitespace(source_leading, output_leading)
            + output_core
            + edge_whitespace(source_trailing, output_trailing)
        )
    return "".join(rebuilt_chunks)


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
        and all(
            translated_count <= source_count
            for translated_count, source_count in zip(
                _visual_break_token_count(translated),
                _visual_break_token_count(source),
                strict=True,
            )
        )
    )


def _candidate_is_acceptable(
    protected: ProtectedMarkdown,
    candidate: str,
    *,
    target_language: str = "zh-Hans",
) -> bool:
    if not _candidate_preserves_structure(protected, candidate):
        return False
    source = protected.restore(protected.text)
    translated = protected.restore(candidate)
    return not _model_added_repetition(
        source, translated
    ) and not _copies_long_source_span(
        protected.text,
        candidate,
        target_language=target_language,
    )


def _candidate_avoids_previous_echo(
    *,
    previous_source: str | None,
    current_source: str,
    previous_candidate: str | None,
    candidate: str,
) -> bool:
    if previous_source is None or previous_candidate is None:
        return True
    source_overlap = _boundary_overlap_text(previous_source, current_source)
    candidate_overlap = _boundary_overlap_text(previous_candidate, candidate)
    if len(candidate_overlap) < 16:
        return True
    if len(source_overlap) < 16:
        return False
    source_literals = _preserved_literals(source_overlap)
    candidate_literals = _preserved_literals(candidate_overlap)
    shared_literals = source_literals & candidate_literals
    if not shared_literals:
        return False
    source_residual = _boundary_residual(source_overlap, shared_literals)
    candidate_residual = _boundary_residual(candidate_overlap, shared_literals)
    if len(candidate_residual) < 16:
        return True
    return (
        len(source_residual) >= 16
        and len(candidate_residual) <= len(source_residual) + 8
        and len(candidate_residual) * 4 >= len(source_residual)
    )


def _boundary_overlap_size(previous: str, current: str) -> int:
    return len(_boundary_overlap_text(previous, current))


def _boundary_overlap_text(previous: str, current: str) -> str:
    previous_text = _normalize_repetition_text(
        PLACEHOLDER_PATTERN.sub("", previous)
    )[-800:]
    current_text = _normalize_repetition_text(
        PLACEHOLDER_PATTERN.sub("", current)
    )[:800]
    if not previous_text or not current_text:
        return ""
    # Prefix-function on `current + sentinel + previous` gives the exact
    # suffix(previous) -> prefix(current) overlap. A common phrase elsewhere in
    # either window is not a chunk-boundary echo.
    combined = f"{current_text}\0{previous_text}"
    prefix_lengths = [0] * len(combined)
    for index in range(1, len(combined)):
        matched = prefix_lengths[index - 1]
        while matched and combined[index] != combined[matched]:
            matched = prefix_lengths[matched - 1]
        if combined[index] == combined[matched]:
            matched += 1
        prefix_lengths[index] = matched
    overlap_size = min(prefix_lengths[-1], len(current_text), len(previous_text))
    return current_text[:overlap_size]


def _preserved_literals(text: str) -> set[str]:
    return {
        match.group(0).casefold()
        for match in PRESERVED_LITERAL_PATTERN.finditer(text)
    }


def _boundary_residual(text: str, shared_literals: set[str]) -> str:
    residual = text
    for literal in sorted(shared_literals, key=len, reverse=True):
        residual = re.sub(re.escape(literal), "", residual, flags=re.IGNORECASE)
    return re.sub(r"[\W_]+", "", residual, flags=re.UNICODE)


def _model_added_repetition(source: str, candidate: str) -> bool:
    return not _repetition_regions_align(
        source_regions=_repetition_profile(source),
        candidate_regions=_repetition_profile(candidate),
    )


def _repetition_regions_align(
    *,
    source_regions: Sequence[float],
    candidate_regions: Sequence[float],
    tolerance: float = 0.25,
) -> bool:
    source_index = 0
    for candidate_region in candidate_regions:
        while (
            source_index < len(source_regions)
            and source_regions[source_index] < candidate_region - tolerance
        ):
            source_index += 1
        if (
            source_index >= len(source_regions)
            or source_regions[source_index] > candidate_region + tolerance
        ):
            return False
        source_index += 1
    return True


def _copies_long_source_span(
    source: str,
    candidate: str,
    *,
    target_language: str = "zh-Hans",
) -> bool:
    source_text = _normalize_repetition_text(_strip_preserved_literals(source))
    candidate_text = _normalize_repetition_text(_strip_preserved_literals(candidate))
    if _probably_target_language(source_text, target_language):
        return False
    gram_length = 160
    if len(source_text) < gram_length or len(candidate_text) < gram_length:
        return False
    source_grams = {
        source_text[start : start + gram_length]
        for start in range(0, len(source_text) - gram_length + 1)
        if _meaningful_repeat_unit(source_text[start : start + gram_length])
    }
    return any(
        candidate_text[start : start + gram_length] in source_grams
        for start in range(0, len(candidate_text) - gram_length + 1)
    )


def _strip_preserved_literals(text: str) -> str:
    without_placeholders = PLACEHOLDER_PATTERN.sub("", text)
    return PRESERVED_LITERAL_PATTERN.sub("", without_placeholders)


def _probably_target_language(text: str, target_language: str) -> bool:
    if target_language != "zh-Hans":
        return False
    han = sum("\u3400" <= character <= "\u9fff" for character in text)
    kana = sum(
        "\u3040" <= character <= "\u30ff" or "\u31f0" <= character <= "\u31ff"
        for character in text
    )
    cyrillic = sum("\u0400" <= character <= "\u052f" for character in text)
    latin = sum(
        ("A" <= character <= "Z") or ("a" <= character <= "z")
        for character in text
    )
    return han >= 120 and kana == 0 and cyrillic == 0 and latin * 3 < han


def _repetition_profile(text: str) -> tuple[float, ...]:
    normalized = _normalize_repetition_text(text)
    if not normalized:
        return ()
    positions = [
        (match.start() + (len(match.group(0)) / 2)) / len(normalized)
        for match in ADJACENT_REPEAT_PATTERN.finditer(normalized)
        if _meaningful_repeat_unit(match.group("unit"))
    ]
    gram_length = 16
    maximum_distance = 600
    previous_positions: dict[str, int] = {}
    previous_pair: tuple[int, int] | None = None
    for start in range(0, len(normalized) - gram_length + 1):
        gram = normalized[start : start + gram_length]
        if not _meaningful_repeat_unit(gram):
            continue
        previous = previous_positions.get(gram)
        if (
            previous is not None
            and start - previous >= gram_length
            and start - previous <= maximum_distance
        ):
            pair = (previous, start)
            if previous_pair != (previous - 1, start - 1):
                positions.append(
                    (previous + start + gram_length) / (2 * len(normalized))
                )
            previous_pair = pair
        else:
            previous_pair = None
        previous_positions[gram] = start
    positions.sort()
    distinct_positions: list[float] = []
    for position in positions:
        if not distinct_positions or position - distinct_positions[-1] > 0.03:
            distinct_positions.append(position)
    return tuple(distinct_positions)


def _normalize_repetition_text(text: str) -> str:
    return re.sub(r"\s+", " ", text).strip()


def _meaningful_repeat_unit(unit: str) -> bool:
    return sum(character.isalnum() for character in unit) >= 4


def _visual_break_token_count(text: str) -> tuple[int, int]:
    return (
        text.count(r"\n"),
        len(re.findall(r"<br\s*/?>", text, flags=re.IGNORECASE)),
    )


def _candidate_only_drops_block_placeholders(
    protected: ProtectedMarkdown,
    candidate: str,
) -> bool:
    expected = protected.placeholders
    actual = tuple(PLACEHOLDER_PATTERN.findall(candidate))
    if len(actual) >= len(expected):
        return False
    missing: list[str] = []
    expected_index = 0
    for placeholder in actual:
        while (
            expected_index < len(expected)
            and expected[expected_index] != placeholder
        ):
            missing.append(expected[expected_index])
            expected_index += 1
        if expected_index == len(expected):
            return False
        expected_index += 1
    missing.extend(expected[expected_index:])
    replacements = dict(protected.replacements)
    return bool(missing) and all(
        _STRUCTURE_PARAGRAPH_BREAK.fullmatch(replacements[placeholder])
        or _line_is_heading(replacements[placeholder])
        for placeholder in missing
    )


def _candidate_allows_validated_draft_fallback(
    protected: ProtectedMarkdown,
    candidate: str,
) -> bool:
    if tuple(PLACEHOLDER_PATTERN.findall(candidate)) == protected.placeholders:
        return True
    replacements = dict(protected.replacements)
    if all(
        _STRUCTURE_PARAGRAPH_BREAK.fullmatch(replacements[placeholder])
        or _line_is_heading(replacements[placeholder])
        for placeholder in protected.placeholders
    ):
        return True
    return _candidate_only_drops_block_placeholders(protected, candidate)


def _remove_unprotected_paragraph_breaks(
    protected: ProtectedMarkdown,
    candidate: str,
) -> str:
    if tuple(PLACEHOLDER_PATTERN.findall(candidate)) != protected.placeholders:
        return candidate
    tokens = re.split(f"({PLACEHOLDER_PATTERN.pattern})", candidate)
    for index in range(0, len(tokens), 2):
        tokens[index] = _STRUCTURE_PARAGRAPH_BREAK.sub(" ", tokens[index])
    return "".join(tokens)


_CANDIDATE_HEADING_BEFORE_PLACEHOLDER = re.compile(
    r"(?m)^[ \t]{0,3}#{1,6}[ \t]*\Z"
)
_CANDIDATE_HEADING_AFTER_PLACEHOLDER = re.compile(r"\A[ \t]*#{1,6}[ \t]+")
_STRUCTURE_PARAGRAPH_BREAK = re.compile(r"\n[ \t]*\n(?:[ \t]*\n)*")


def _normalize_candidate_structure(
    protected: ProtectedMarkdown, candidate: str
) -> str:
    """Keep block structure owned by placeholders, never by model formatting."""
    if tuple(PLACEHOLDER_PATTERN.findall(candidate)) != protected.placeholders:
        return candidate
    tokens = re.split(f"({PLACEHOLDER_PATTERN.pattern})", candidate)
    replacements = dict(protected.replacements)
    for index in range(1, len(tokens), 2):
        original = replacements[tokens[index]]
        if _line_is_heading(original):
            tokens[index - 1] = _CANDIDATE_HEADING_BEFORE_PLACEHOLDER.sub(
                "", tokens[index - 1]
            )
            tokens[index + 1] = _CANDIDATE_HEADING_AFTER_PLACEHOLDER.sub(
                "", tokens[index + 1]
            )
        elif _STRUCTURE_PARAGRAPH_BREAK.fullmatch(original):
            tokens[index - 1] = tokens[index - 1].rstrip(" \t\r\n")
            tokens[index + 1] = tokens[index + 1].lstrip(" \t\r\n")
    return "".join(tokens)


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
    text = PLACEHOLDER_PATTERN.sub(" ", text)
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
