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
from .pipeline import SecondPassRequest, WindowedReflectionSecondPass
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
    stages = [compiled.preview()]
    if prompt_pack.uses_reflection:
        recorder = _PromptRecorder()
        second_pass = WindowedReflectionSecondPass(recorder)
        second_pass_request = SecondPassRequest(
            source_text=protected.text,
            draft_text="${INITIAL_TRANSLATION_RESULT}",
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
        )
        reflection = second_pass.reflect(request=second_pass_request)
        second_pass.improve(
            request=second_pass_request,
            reflection_text=reflection,
        )
        stages.extend(
            [
                _request_preview(
                    stage_id="reflect",
                    template=prompt_pack.template_for("reflect"),
                    request=recorder.requests[0],
                    injections=[
                        "template",
                        "current-source",
                        "initial-translation:runtime-result",
                        "glossary",
                        "executor-safety",
                        "neighbor-context:none-for-preview-sample",
                    ],
                ),
                _request_preview(
                    stage_id="improve",
                    template=prompt_pack.template_for("improve"),
                    request=recorder.requests[1],
                    injections=[
                        "template",
                        "current-source",
                        "initial-translation:runtime-result",
                        "reflection:runtime-result",
                        "glossary",
                        "executor-safety",
                    ],
                ),
            ]
        )
    return {
        "schema": "translation-engine-prompt-preview-report-v1",
        "promptPackReference": {
            "packId": prompt_pack.pack_id,
            "revisionId": prompt_pack.revision_id,
            "contentSha256": prompt_pack.content_sha256,
        },
        "stages": stages,
    }


class _PromptRecorder:
    def __init__(self) -> None:
        self.requests: list[Any] = []

    def translate(self, request: Any) -> str:
        self.requests.append(request)
        return "${FOUR_DIMENSION_REFLECTION_RESULT}"


def _request_preview(
    *, stage_id: str, template: str, request: Any, injections: list[str]
) -> dict[str, object]:
    return {
        "stageId": stage_id,
        "template": template,
        "actualPrompt": {
            "systemInstruction": request.system_instruction,
            "currentSource": request.text,
        },
        "injections": injections,
    }
