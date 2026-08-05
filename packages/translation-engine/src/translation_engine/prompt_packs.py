from __future__ import annotations

from dataclasses import dataclass
import hashlib
import json
from typing import Any, Mapping

from .profiles import TargetLanguageProfile
from .providers import TranslationRequest


class PromptPackError(ValueError):
    """A prompt-pack snapshot cannot be executed as declared."""


@dataclass(frozen=True)
class PromptStageTemplate:
    stage_id: str
    template: str


@dataclass(frozen=True)
class PromptPackRevision:
    pack_id: str
    revision_id: str
    content_sha256: str
    display_name: str
    executor: str
    source_language: str
    target_language: str
    parameters: Mapping[str, str]
    stages: tuple[PromptStageTemplate, ...]

    def template_for(self, stage_id: str) -> str:
        for stage in self.stages:
            if stage.stage_id == stage_id:
                return stage.template
        raise PromptPackError(f"missing_prompt_stage:{stage_id}")

    def has_stage(self, stage_id: str) -> bool:
        return any(stage.stage_id == stage_id for stage in self.stages)

    def compiled_template_for(self, stage_id: str) -> str:
        template = self.template_for(stage_id)
        if not self.parameters:
            return template
        parameter_block = "\n".join(
            f"- {key}: {value}" for key, value in sorted(self.parameters.items())
        )
        return f"{template}\n\n# OPEN SCHEME PARAMETERS\n{parameter_block}"

    @property
    def uses_reflection(self) -> bool:
        return self.has_stage("reflect") and self.has_stage("improve")


@dataclass(frozen=True)
class CompiledPrompt:
    stage_id: str
    template: str
    request: TranslationRequest
    injections: tuple[str, ...]

    def preview(self) -> dict[str, object]:
        """Ephemeral view; callers must not persist the returned private text."""

        return {
            "stageId": self.stage_id,
            "template": self.template,
            "actualPrompt": {
                "systemInstruction": self.request.system_instruction,
                "currentSource": self.request.text,
            },
            "injections": list(self.injections),
        }


def compile_translation_prompt(
    *,
    revision: PromptPackRevision,
    target_profile: TargetLanguageProfile,
    source_text: str,
    source_language: str,
    target_language: str,
    task_manifest: Mapping[str, object],
    text_cleanup: bool = False,
    previous_translation_tail: str | None = None,
) -> CompiledPrompt:
    template = revision.compiled_template_for("translate")
    system_instruction = target_profile.build_system_instruction(
        source_text=source_text,
        task_manifest=task_manifest,
        text_cleanup=text_cleanup,
        prompt_template=template,
    )
    injections = ["template", "current-source", "glossary", "executor-safety"]
    if text_cleanup:
        injections.append("text-cleanup")
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
        system_instruction = f"{system_instruction}\n\n{context_block}"
        injections.append("neighbor-context")
    else:
        injections.append("neighbor-context:none-for-first-segment")
    return CompiledPrompt(
        stage_id="translate",
        template=template,
        request=TranslationRequest(
            text=source_text,
            source_language=source_language,
            target_language=target_language,
            system_instruction=system_instruction,
        ),
        injections=tuple(injections),
    )


def revision_content_sha256(snapshot: Mapping[str, Any]) -> str:
    """Hash the immutable revision content, excluding the hash field itself."""

    content = {key: value for key, value in snapshot.items() if key != "contentSha256"}
    encoded = json.dumps(
        content,
        ensure_ascii=False,
        separators=(",", ":"),
        sort_keys=True,
    ).encode("utf-8")
    return hashlib.sha256(encoded).hexdigest()


def parse_prompt_pack_revision(
    value: object,
    *,
    executor: str,
    source_language: str,
    target_language: str,
) -> PromptPackRevision:
    if not isinstance(value, dict):
        raise PromptPackError("invalid_prompt_pack")
    if value.get("schema") != "translation-prompt-pack-revision-v1":
        raise PromptPackError("unsupported_prompt_pack_schema")

    pack_id = _required_string(value, "packId")
    revision_id = _required_string(value, "revisionId")
    content_sha256 = _required_string(value, "contentSha256")
    display_name = _required_string(value, "displayName")
    declared_executor = _required_string(value, "executor")
    declared_source = _required_string(value, "sourceLanguage")
    declared_target = _required_string(value, "targetLanguage")
    if declared_executor != executor:
        raise PromptPackError("prompt_pack_executor_mismatch")
    if declared_source not in {"auto", source_language}:
        raise PromptPackError("prompt_pack_source_language_mismatch")
    if declared_target != target_language:
        raise PromptPackError("prompt_pack_target_language_mismatch")
    if content_sha256 != revision_content_sha256(value):
        raise PromptPackError("prompt_pack_content_hash_mismatch")

    raw_stages = value.get("stages")
    if not isinstance(raw_stages, list) or not raw_stages:
        raise PromptPackError("invalid_prompt_pack_stages")
    stages: list[PromptStageTemplate] = []
    stage_ids: set[str] = set()
    for raw_stage in raw_stages:
        if not isinstance(raw_stage, dict):
            raise PromptPackError("invalid_prompt_pack_stages")
        stage_id = _required_string(raw_stage, "stageId")
        template = _required_string(raw_stage, "template")
        if stage_id in stage_ids:
            raise PromptPackError("duplicate_prompt_pack_stage")
        stage_ids.add(stage_id)
        stages.append(PromptStageTemplate(stage_id, template))

    raw_parameters = value.get("parameters", {})
    if not isinstance(raw_parameters, dict) or any(
        key not in {"qualityFocus", "styleGuidance"}
        or not isinstance(parameter, str)
        or not parameter.strip()
        or len(parameter) > 2_000
        for key, parameter in raw_parameters.items()
    ):
        raise PromptPackError("invalid_prompt_pack_parameters")

    return PromptPackRevision(
        pack_id=pack_id,
        revision_id=revision_id,
        content_sha256=content_sha256,
        display_name=display_name,
        executor=declared_executor,
        source_language=declared_source,
        target_language=declared_target,
        parameters=dict(raw_parameters),
        stages=tuple(stages),
    )


def _required_string(value: Mapping[str, Any], key: str) -> str:
    item = value.get(key)
    if not isinstance(item, str) or not item:
        raise PromptPackError("invalid_prompt_pack")
    return item
