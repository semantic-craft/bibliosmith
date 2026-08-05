from pathlib import Path
import json
import tempfile
import unittest

from translation_engine.engine import run_manifest
from translation_engine.profiles import ZH_HANS
from translation_engine.prompt_packs import (
    compile_translation_prompt,
    parse_prompt_pack_revision,
    revision_content_sha256,
)
from translation_engine.providers import TranslationRequest
from translation_engine.prompt_preview import run_prompt_preview_manifest
from tests.fixtures import FOUR_DIMENSION_PROMPT_PACK, build_run_fixture


STRUCTURE_FIDELITY_REVISION = {
    "schema": "translation-prompt-pack-revision-v1",
    "packId": "builtin.structure-fidelity",
    "revisionId": "2026.08.05-1",
    "displayName": "结构保真翻译",
    "executor": "programmatic",
    "sourceLanguage": "auto",
    "targetLanguage": "zh-Hans",
    "stages": [
        {
            "stageId": "translate",
            "template": "PACK_TEMPLATE_SENTINEL Translate faithfully into Simplified Chinese.",
        }
    ],
}
STRUCTURE_FIDELITY_REVISION["contentSha256"] = revision_content_sha256(
    STRUCTURE_FIDELITY_REVISION
)


class CapturingProvider:
    profile_id = "fake-provider-profile"
    config_id = "fake-config-no-secrets"

    def __init__(self) -> None:
        self.requests: list[TranslationRequest] = []

    def translate(self, request: TranslationRequest) -> str:
        self.requests.append(request)
        return request.text.upper()


class FourDimensionProvider:
    profile_id = "fake-provider-profile"
    config_id = "fake-config-no-secrets"

    def __init__(self) -> None:
        self.requests: list[TranslationRequest] = []

    def translate(self, request: TranslationRequest) -> str:
        self.requests.append(request)
        if "Output only the suggestions" in request.system_instruction:
            return "FOUR_DIMENSION_EVIDENCE"
        if "translation editing" in request.system_instruction:
            payload = json.loads(request.text)
            if payload["reflection"] != "FOUR_DIMENSION_EVIDENCE":
                raise AssertionError("reflection did not reach the revision request")
            return payload["draft"].replace("DRAFT", "FINAL")
        return "DRAFT"


class SchemeAwareProvider(CapturingProvider):
    def translate(self, request: TranslationRequest) -> str:
        self.requests.append(request)
        return (
            "本地方案真实结果"
            if "LOCAL_EDIT_SENTINEL" in request.system_instruction
            else "内置方案结果"
        )


class PromptPackRunTests(unittest.TestCase):
    def test_selected_structure_pack_drives_the_real_provider_request(self) -> None:
        with tempfile.TemporaryDirectory() as temporary_directory:
            manifest_path = build_run_fixture(
                Path(temporary_directory),
                source_text="A quiet village.\n",
                max_tokens=100,
                prompt_pack=STRUCTURE_FIDELITY_REVISION,
            )
            provider = CapturingProvider()

            report = run_manifest(
                manifest_path,
                provider_factory=lambda profile_id, *, config_id: provider,
            )

            self.assertEqual(report["summary"]["completed"], 1)
            self.assertEqual(len(provider.requests), 1)
            instruction = provider.requests[0].system_instruction
            self.assertIn("PACK_TEMPLATE_SENTINEL", instruction)
            self.assertIn("Preserve every protected placeholder exactly", instruction)

    def test_ephemeral_preview_is_the_exact_request_compiled_for_execution(self) -> None:
        revision = parse_prompt_pack_revision(
            STRUCTURE_FIDELITY_REVISION,
            executor="programmatic",
            source_language="auto",
            target_language="zh-Hans",
        )
        compiled = compile_translation_prompt(
            revision=revision,
            target_profile=ZH_HANS,
            source_text="Private source sample.",
            source_language="auto",
            target_language="zh-Hans",
            task_manifest={},
            previous_translation_tail="相邻译文",
        )

        preview = compiled.preview()

        actual_prompt = preview["actualPrompt"]
        self.assertEqual(
            actual_prompt["systemInstruction"], compiled.request.system_instruction
        )
        self.assertEqual(actual_prompt["currentSource"], compiled.request.text)
        self.assertEqual(
            preview["injections"],
            ["template", "current-source", "glossary", "executor-safety", "neighbor-context"],
        )

    def test_local_revision_template_changes_the_real_translation_result(self) -> None:
        local_revision = json.loads(json.dumps(STRUCTURE_FIDELITY_REVISION))
        local_revision.update(
            {
                "packId": "local.test-structure-style",
                "revisionId": "local-2",
                "displayName": "我的结构译法",
                "source": {
                    "kind": "local-copy",
                    "sourcePackId": "builtin.structure-fidelity",
                    "sourceRevisionId": "2026.08.05-1",
                },
            }
        )
        local_revision["stages"][0]["template"] = (
            "LOCAL_EDIT_SENTINEL 按本书语体完整翻译当前块。"
        )
        local_revision["contentSha256"] = revision_content_sha256(local_revision)

        with tempfile.TemporaryDirectory() as temporary_directory:
            root = Path(temporary_directory)
            manifest_path = build_run_fixture(
                root,
                source_text="Private source.\n",
                max_tokens=100,
                prompt_pack=local_revision,
            )
            provider = SchemeAwareProvider()

            run_manifest(
                manifest_path,
                provider_factory=lambda profile_id, *, config_id: provider,
            )

            self.assertIn("LOCAL_EDIT_SENTINEL", provider.requests[0].system_instruction)
            self.assertEqual(
                (root / "chapters" / "translated" / "chapter_001.md").read_text(
                    encoding="utf-8"
                ),
                "本地方案真实结果\n",
            )
    def test_four_dimension_pack_executes_translate_reflect_and_improve(self) -> None:
        with tempfile.TemporaryDirectory() as temporary_directory:
            manifest_path = build_run_fixture(
                Path(temporary_directory),
                source_text="Source.\n",
                max_tokens=100,
                second_pass_enabled=True,
                prompt_pack=FOUR_DIMENSION_PROMPT_PACK,
            )
            provider = FourDimensionProvider()

            report = run_manifest(
                manifest_path,
                provider_factory=lambda profile_id, *, config_id: provider,
            )

            self.assertEqual(report["summary"]["completed"], 1)
            self.assertEqual(len(provider.requests), 3)
            self.assertIn(
                "Create a faithful first translation.",
                provider.requests[0].system_instruction,
            )
            self.assertIn(
                "Reflect on accuracy, fluency, style, and terminology.",
                provider.requests[1].system_instruction,
            )
            self.assertIn(
                "Improve the draft using the four-dimension reflection.",
                provider.requests[2].system_instruction,
            )

    def test_preview_manifest_and_real_run_share_the_compiler(self) -> None:
        with tempfile.TemporaryDirectory() as temporary_directory:
            manifest_path = build_run_fixture(
                Path(temporary_directory),
                source_text="Private source sample.\n",
                max_tokens=100,
                prompt_pack=STRUCTURE_FIDELITY_REVISION,
            )
            provider = CapturingProvider()
            run_manifest(
                manifest_path,
                provider_factory=lambda profile_id, *, config_id: provider,
            )

            preview = run_prompt_preview_manifest(manifest_path)

            self.assertEqual(len(preview["stages"]), 1)
            actual = preview["stages"][0]["actualPrompt"]
            self.assertEqual(
                actual["systemInstruction"], provider.requests[0].system_instruction
            )
            self.assertEqual(actual["currentSource"], provider.requests[0].text)


if __name__ == "__main__":
    unittest.main()
