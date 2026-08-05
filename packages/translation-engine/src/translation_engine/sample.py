from __future__ import annotations

from pathlib import Path
from typing import Any

from .engine import (
    EngineError,
    ProviderFactory,
    TargetProfileFactory,
    _candidate_preserves_structure,
    _parse_custom_instructions,
    _parse_text_cleanup,
    _project_path,
    _read_json,
    _required_string,
    apply_model_override,
    load_translation_unit,
)
from .pipeline import translate_chunk_with_fallback
from .placeholders import protect_chunk_structure, protect_markdown_for_chunking
from .profiles import get_target_profile
from .providers import TranslationRequest, create_provider
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

    # Both parsed with the engine's own helpers rather than re-read here, so the
    # sample cannot drift from the full run it is meant to preview.
    text_cleanup = _parse_text_cleanup(manifest)
    custom_translation, custom_reflection = _parse_custom_instructions(manifest)
    # custom_reflection is parsed for validation and then deliberately unused:
    # the sample path runs one translation pass and no reflection pass, so there
    # is nothing for a reflection instruction to apply to. Parsing it anyway
    # means a malformed instruction is rejected here exactly as the full run
    # would reject it, instead of passing review and failing later.
    del custom_reflection

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
            TranslationRequest(
                text=protected.text,
                source_language=str(manifest.get("sourceLanguage", "auto")),
                target_language=target_language,
                system_instruction=target_profile.build_system_instruction(
                    source_text=protected.text,
                    task_manifest=unit.profile_task,
                    text_cleanup=text_cleanup,
                    custom_instruction=custom_translation,
                ),
            ),
            placeholder_retries=placeholder_retries,
            candidate_validator=lambda candidate, current=protected: (
                _candidate_preserves_structure(current, candidate)
            ),
        )
        samples.append(
            {
                "chunkRef": unit.unit_id,
                "sourceExcerpt": source_excerpt,
                "translatedExcerpt": protected.restore(result.text),
                "degradation": result.degradation,
            }
        )
    return {"schema": "translation-engine-sample-report-v1", "samples": samples}


def _positive_integer(manifest: dict[str, Any], key: str, error_code: str) -> int:
    value = manifest.get(key)
    if not isinstance(value, int) or isinstance(value, bool) or value < 1:
        raise EngineError(error_code)
    return value
