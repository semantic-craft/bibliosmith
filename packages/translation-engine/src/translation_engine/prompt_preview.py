from __future__ import annotations

from pathlib import Path
from typing import Any

from .chunking import TokenChunker, Utf8ByteTokenCounter
from .engine import (
    EngineError,
    _parse_text_cleanup,
    _project_path,
    _read_json,
    _required_string,
    load_translation_unit,
)
from .placeholders import protect_chunk_structure, protect_markdown_for_chunking
from .profiles import get_target_profile
from .prompt_packs import (
    PromptPackError,
    compile_translation_prompt,
    parse_prompt_pack_revision,
)


def run_prompt_preview_manifest(manifest_path: Path) -> dict[str, Any]:
    """Compile a private prompt preview in memory without calling a provider."""

    manifest = _read_json(manifest_path)
    if manifest.get("schema") not in {
        "translation-engine-run-v1",
        "translation-engine-prompt-preview-v1",
    }:
        raise EngineError("unsupported_manifest_schema")
    if "customInstructions" in manifest:
        raise EngineError("unsupported_manifest_field:customInstructions")

    project_root = Path(_required_string(manifest, "projectRoot")).resolve()
    source_map = _read_json(
        _project_path(project_root, _required_string(manifest, "sourceMapPath"))
    )
    target_language = _required_string(manifest, "targetLanguage")
    source_language = str(manifest.get("sourceLanguage", "auto"))
    try:
        target_profile = get_target_profile(target_language)
        prompt_pack = parse_prompt_pack_revision(
            manifest.get("promptPack"),
            executor="programmatic",
            source_language=source_language,
            target_language=target_language,
        )
    except (ValueError, PromptPackError) as error:
        raise EngineError(str(error).replace(" ", "_")) from error

    units = manifest.get("units")
    if not isinstance(units, list) or not units:
        raise EngineError("empty_unit_list")
    unit = load_translation_unit(
        project_root=project_root,
        source_map=source_map,
        task_path=_project_path(
            project_root, _required_string(units[0], "taskManifestPath")
        ),
        target_language=target_language,
    )
    max_tokens = manifest.get("maxTokens", 600)
    if not isinstance(max_tokens, int) or isinstance(max_tokens, bool) or max_tokens < 1:
        raise EngineError("invalid_max_tokens")
    protected_source = protect_markdown_for_chunking(unit.source_text)
    first_chunk = TokenChunker(
        max_tokens=max_tokens, counter=Utf8ByteTokenCounter()
    ).split(protected_source.text)[0]
    protected = protect_chunk_structure(first_chunk, protected_source)
    compiled = compile_translation_prompt(
        revision=prompt_pack,
        target_profile=target_profile,
        source_text=protected.text,
        source_language=source_language,
        target_language=target_language,
        task_manifest=unit.profile_task,
        text_cleanup=_parse_text_cleanup(manifest),
    )
    return {
        "schema": "translation-engine-prompt-preview-report-v1",
        "promptPackReference": {
            "packId": prompt_pack.pack_id,
            "revisionId": prompt_pack.revision_id,
            "contentSha256": prompt_pack.content_sha256,
        },
        "stages": [compiled.preview()],
    }
