from __future__ import annotations

from pathlib import Path
from typing import Any

from .engine import (
    EngineError,
    ProviderFactory,
    TargetProfileFactory,
    _candidate_preserves_structure,
    _parse_text_cleanup,
    _project_path,
    _read_json,
    _required_string,
    apply_model_override,
    load_translation_unit,
)
from .pipeline import (
    SecondPassRequest,
    WindowedReflectionSecondPass,
    run_second_pass_chunk,
    translate_chunk_with_fallback,
)
from .placeholders import protect_chunk_structure, protect_markdown_for_chunking
from .profiles import get_target_profile
from .prompt_packs import (
    PromptPackError,
    compile_translation_prompt,
    parse_prompt_pack_revision,
)
from .providers import create_provider
from .sampling import select_internal_blocks, truncate_at_sentence_boundary


def run_sample_manifest(
    manifest_path: Path,
    *,
    provider_factory: ProviderFactory = create_provider,
    target_profile_factory: TargetProfileFactory = get_target_profile,
) -> dict[str, Any]:
    manifest = _read_json(manifest_path)
    if manifest.get("schema") != "translation-engine-sample-v1":
        raise EngineError("unsupported_manifest_schema")
    project_root = Path(_required_string(manifest, "projectRoot")).resolve()
    source_map = _read_json(
        _project_path(project_root, _required_string(manifest, "sourceMapPath"))
    )
    if source_map.get("schema") != "local-reading-source-map-v2":
        raise EngineError("unsupported_source_map_schema")

    target_language = _required_string(manifest, "targetLanguage")
    source_language = str(manifest.get("sourceLanguage", "auto"))
    if "customInstructions" in manifest:
        raise EngineError("unsupported_manifest_field:customInstructions")
    try:
        prompt_pack = parse_prompt_pack_revision(
            manifest.get("promptPack"),
            executor="programmatic",
            source_language=source_language,
            target_language=target_language,
        )
        prompt_pack.template_for("translate")
    except PromptPackError as error:
        raise EngineError(str(error)) from error
    try:
        target_profile = target_profile_factory(target_language)
        provider = provider_factory(
            _required_string(manifest, "providerProfileId"),
            config_id=_required_string(manifest, "providerConfigId"),
        )
    except ValueError as error:
        raise EngineError(str(error).replace(" ", "_")) from error
    apply_model_override(provider, manifest)

    units = manifest.get("units")
    if not isinstance(units, list) or not units:
        raise EngineError("empty_unit_list")
    sample_count = _positive_integer(manifest, "sampleCount", "invalid_sample_count")
    character_budget = _positive_integer(
        manifest, "characterBudget", "invalid_character_budget"
    )
    placeholder_retries = manifest.get("placeholderRetries", 1)
    if (
        not isinstance(placeholder_retries, int)
        or isinstance(placeholder_retries, bool)
        or placeholder_retries < 0
    ):
        raise EngineError("invalid_placeholder_retries")

    text_cleanup = _parse_text_cleanup(manifest)

    # Two units or fewer leaves nothing between the excluded endpoints, so the
    # report comes back empty and the provider is never called. That is the
    # defined outcome rather than a failure -- the endpoints are where title,
    # copyright, and trailing metadata live, and previewing those would show the
    # least representative pages in the book. Callers have to say so out loud;
    # an empty panel reads as a broken preview.
    samples = []
    for unit_entry in select_internal_blocks(units, sample_count):
        task_path = _project_path(
            project_root, _required_string(unit_entry, "taskManifestPath")
        )
        unit = load_translation_unit(
            project_root=project_root,
            source_map=source_map,
            task_path=task_path,
            target_language=target_language,
        )
        source_excerpt = truncate_at_sentence_boundary(
            unit.source_text, character_budget
        )
        inline_protection = protect_markdown_for_chunking(source_excerpt)
        protected = protect_chunk_structure(
            inline_protection.text, inline_protection
        )
        result = translate_chunk_with_fallback(
            provider,
            compile_translation_prompt(
                revision=prompt_pack,
                target_profile=target_profile,
                source_text=protected.text,
                source_language=source_language,
                target_language=target_language,
                task_manifest=unit.profile_task,
                text_cleanup=text_cleanup,
            ).request,
            placeholder_retries=placeholder_retries,
            candidate_validator=lambda candidate, current=protected: (
                _candidate_preserves_structure(current, candidate)
            ),
        )
        translated_text = result.text
        if prompt_pack.uses_reflection and result.degradation != "source":
            second_pass_result = run_second_pass_chunk(
                WindowedReflectionSecondPass(provider),
                SecondPassRequest(
                    source_text=protected.text,
                    draft_text=result.text,
                    previous_source_text=None,
                    previous_draft_text=None,
                    next_source_text=None,
                    next_draft_text=None,
                    source_language=source_language,
                    target_language=target_language,
                    terminology_criteria=target_profile.build_system_instruction(
                        source_text=protected.text,
                        task_manifest=unit.profile_task,
                    ),
                    reflection_template=prompt_pack.compiled_template_for("reflect"),
                    improve_template=prompt_pack.compiled_template_for("improve"),
                ),
                candidate_retries=placeholder_retries,
                candidate_validator=lambda candidate, current=protected: (
                    _candidate_preserves_structure(current, candidate)
                ),
                draft_fallback_validator=lambda candidate, current=protected: (
                    _candidate_preserves_structure(current, candidate)
                ),
            )
            translated_text = second_pass_result.revised_text
        samples.append(
            {
                "chunkRef": unit.unit_id,
                "sourceExcerpt": source_excerpt,
                "translatedExcerpt": protected.restore(translated_text),
                "degradation": result.degradation,
            }
        )
    return {"schema": "translation-engine-sample-report-v1", "samples": samples}


def _positive_integer(manifest: dict[str, Any], key: str, error_code: str) -> int:
    value = manifest.get(key)
    if not isinstance(value, int) or isinstance(value, bool) or value < 1:
        raise EngineError(error_code)
    return value
