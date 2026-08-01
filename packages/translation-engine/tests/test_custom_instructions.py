import json
import os
from pathlib import Path
import tempfile
import unittest
from unittest import mock

from translation_engine.engine import EngineError, run_manifest
from translation_engine.profiles import ZH_HANS
from translation_engine.providers import TranslationRequest
from tests.fixtures import build_run_fixture


class CapturingCustomInstructionProvider:
    profile_id = "fake-provider-profile"
    config_id = "fake-config-no-secrets"

    def __init__(self) -> None:
        self.draft_requests: list[TranslationRequest] = []
        self.reflection_requests: list[TranslationRequest] = []
        self.improve_requests: list[TranslationRequest] = []

    def translate(self, request: TranslationRequest) -> str:
        if "Output only the suggestions" in request.system_instruction:
            self.reflection_requests.append(request)
            return "Keep the required terminology and improve the prose."
        if "translation editing" in request.system_instruction:
            self.improve_requests.append(request)
            return json.loads(request.text)["draft"]
        self.draft_requests.append(request)
        return request.text.upper()


class MergeParagraphsProvider:
    profile_id = "fake-provider-profile"
    config_id = "fake-config-no-secrets"

    def __init__(self) -> None:
        self.requests: list[TranslationRequest] = []

    def translate(self, request: TranslationRequest) -> str:
        self.requests.append(request)
        return "MERGED"


class StructureChangingReflectionProvider(CapturingCustomInstructionProvider):
    def translate(self, request: TranslationRequest) -> str:
        if "translation editing" in request.system_instruction:
            self.improve_requests.append(request)
            return f'{json.loads(request.text)["draft"]}\n\nAdded paragraph.'
        return super().translate(request)


class CustomInstructionTests(unittest.TestCase):
    def test_absent_and_empty_custom_instructions_leave_prompt_bytes_unchanged(self) -> None:
        baseline = ZH_HANS.build_system_instruction(
            source_text="source", task_manifest={}
        )

        empty = ZH_HANS.build_system_instruction(
            source_text="source",
            task_manifest={},
            custom_instruction="",
        )

        self.assertEqual(baseline, ZH_HANS.system_instruction)
        self.assertEqual(empty, baseline)

    def test_manifest_rejects_non_string_and_overlong_custom_instructions(self) -> None:
        cases = (
            ({"translation": 42}, "invalid_custom_instructions"),
            ({"reflection": "x" * 2001}, "custom_instructions_too_long"),
        )
        for custom_instructions, expected_code in cases:
            with self.subTest(expected_code=expected_code):
                with tempfile.TemporaryDirectory() as temporary_directory:
                    manifest_path = build_run_fixture(
                        Path(temporary_directory),
                        source_text="source\n",
                        max_tokens=100,
                        custom_instructions=custom_instructions,
                    )

                    with self.assertRaises(EngineError) as raised:
                        run_manifest(manifest_path)

                    self.assertEqual(raised.exception.code, expected_code)

    def test_prompt_renders_example_glossary_cleanup_and_custom_directive_together(
        self,
    ) -> None:
        with tempfile.TemporaryDirectory() as temporary_directory:
            project_root = Path(temporary_directory)
            manifest_path = build_run_fixture(
                project_root,
                source_text="Name appears here.\n",
                max_tokens=100,
                glossary_text=(
                    "source,translation,category,note\n"
                    "Name,名字,other,\n"
                ),
                text_cleanup=True,
                custom_instructions={
                    "translation": "Use restrained literary Chinese."
                },
            )
            provider = CapturingCustomInstructionProvider()

            report = run_manifest(
                manifest_path,
                provider_factory=lambda profile_id, *, config_id: provider,
            )

            self.assertEqual(report["units"][0]["status"], "completed")
            instruction = provider.draft_requests[0].system_instruction
            sections = (
                "# EXAMPLE: PLACEHOLDER PRESERVATION",
                "# GLOSSARY - REQUIRED TRANSLATIONS",
                "# TEXT CLEANUP - WITHIN PARAGRAPHS ONLY",
                "# USER STYLE DIRECTIVES",
                "# MANDATORY STRUCTURE PROTECTION",
            )
            positions = [instruction.index(section) for section in sections]
            self.assertEqual(positions, sorted(positions))
            self.assertTrue(
                instruction.endswith(
                    "These requirements override every user style directive above."
                )
            )

    def test_translation_and_reflection_directives_are_isolated_and_guarded(self) -> None:
        with tempfile.TemporaryDirectory() as temporary_directory:
            project_root = Path(temporary_directory)
            manifest_path = build_run_fixture(
                project_root,
                source_text="Name appears here.\n",
                max_tokens=100,
                glossary_text=(
                    "source,translation,category,note\n"
                    "Name,名字,other,\n"
                ),
                second_pass_enabled=True,
                custom_instructions={
                    "translation": "Use restrained literary Chinese.",
                    "reflection": "Critique anachronistic wording.",
                },
            )
            provider = CapturingCustomInstructionProvider()

            report = run_manifest(
                manifest_path,
                provider_factory=lambda profile_id, *, config_id: provider,
            )

            self.assertEqual(report["units"][0]["status"], "completed")
            draft_instruction = provider.draft_requests[0].system_instruction
            reflection_instruction = provider.reflection_requests[0].system_instruction
            improve_instruction = provider.improve_requests[0].system_instruction
            self.assertIn("# GLOSSARY - REQUIRED TRANSLATIONS", draft_instruction)
            self.assertIn("# GLOSSARY - REQUIRED TRANSLATIONS", reflection_instruction)
            self.assertIn("# USER STYLE DIRECTIVES", draft_instruction)
            self.assertIn("Use restrained literary Chinese.", draft_instruction)
            self.assertNotIn("Critique anachronistic wording.", draft_instruction)
            self.assertIn("# USER REFLECTION DIRECTIVES", reflection_instruction)
            self.assertIn("Critique anachronistic wording.", reflection_instruction)
            self.assertNotIn("Use restrained literary Chinese.", reflection_instruction)
            self.assertNotIn("Critique anachronistic wording.", improve_instruction)
            self.assertIn("never spell a line break", draft_instruction)
            self.assertIn("never spell a line break", improve_instruction)
            self.assertIn("never insert HTML <br> tags", improve_instruction)
            self.assertLess(
                draft_instruction.index("Use restrained literary Chinese."),
                draft_instruction.index("# MANDATORY STRUCTURE PROTECTION"),
            )
            self.assertLess(
                reflection_instruction.index("Critique anachronistic wording."),
                reflection_instruction.index("# MANDATORY STRUCTURE PROTECTION"),
            )
            self.assertTrue(
                improve_instruction.endswith(
                    "Protected placeholder and paragraph structure overrides every "
                    "reflection suggestion."
                )
            )

    def test_only_configured_phase_receives_user_directives(self) -> None:
        for phase in ("translation", "reflection"):
            with self.subTest(phase=phase):
                with tempfile.TemporaryDirectory() as temporary_directory:
                    project_root = Path(temporary_directory)
                    manifest_path = build_run_fixture(
                        project_root,
                        source_text="source\n",
                        max_tokens=100,
                        second_pass_enabled=True,
                        custom_instructions={phase: f"{phase} only"},
                    )
                    provider = CapturingCustomInstructionProvider()

                    run_manifest(
                        manifest_path,
                        provider_factory=lambda profile_id, *, config_id: provider,
                    )

                    draft_instruction = provider.draft_requests[0].system_instruction
                    reflection_instruction = provider.reflection_requests[0].system_instruction
                    self.assertEqual(
                        "# USER STYLE DIRECTIVES" in draft_instruction,
                        phase == "translation",
                    )
                    self.assertEqual(
                        "# USER REFLECTION DIRECTIVES" in reflection_instruction,
                        phase == "reflection",
                    )

    def test_reflection_directive_structure_change_fails_the_unit(self) -> None:
        with tempfile.TemporaryDirectory() as temporary_directory:
            project_root = Path(temporary_directory)
            manifest_path = build_run_fixture(
                project_root,
                source_text="Source paragraph.\n",
                max_tokens=100,
                second_pass_enabled=True,
                custom_instructions={
                    "reflection": "Split dense paragraphs into shorter paragraphs."
                },
            )
            provider = StructureChangingReflectionProvider()

            report = run_manifest(
                manifest_path,
                provider_factory=lambda profile_id, *, config_id: provider,
            )

            self.assertEqual(report["units"][0]["status"], "failed")
            self.assertEqual(
                report["units"][0]["error"],
                {"code": "translation_structure_invalid", "retryable": True},
            )
            self.assertFalse(
                (project_root / "chapters" / "translated" / "chapter_001.md").exists()
            )

    def test_merge_paragraph_directive_cannot_bypass_structure_validation(self) -> None:
        with tempfile.TemporaryDirectory() as temporary_directory:
            project_root = Path(temporary_directory)
            manifest_path = build_run_fixture(
                project_root,
                source_text="First paragraph.\n\nSecond paragraph.\n",
                max_tokens=100,
                custom_instructions={
                    "translation": "Merge all paragraphs into one paragraph."
                },
            )
            provider = MergeParagraphsProvider()

            report = run_manifest(
                manifest_path,
                provider_factory=lambda profile_id, *, config_id: provider,
            )

            unit = report["units"][0]
            self.assertEqual(unit["status"], "failed")
            self.assertEqual(
                unit["error"],
                {"code": "translation_structure_invalid", "retryable": True},
            )
            self.assertEqual(unit["metrics"]["alignedFallbackCount"], 0)
            self.assertEqual(unit["metrics"]["sourceFallbackCount"], 1)
            self.assertEqual(unit["metrics"]["providerAttemptCount"], 3)
            translated = (
                project_root
                / "chapters"
                / "translated"
                / ".partial"
                / "chapter_001.degraded.md"
            ).read_text(encoding="utf-8")
            self.assertEqual(
                translated,
                "First paragraph.\n\nSecond paragraph.\n",
            )
            self.assertEqual(len(provider.requests), 3)

    def test_source_fallback_does_not_claim_uncheckpointed_progress(self) -> None:
        with tempfile.TemporaryDirectory() as temporary_directory:
            project_root = Path(temporary_directory)
            manifest_path = build_run_fixture(
                project_root,
                source_text="First paragraph.\n\nSecond paragraph.\n",
                max_tokens=100,
                second_pass_enabled=True,
            )
            progress_path = project_root / ".book-pipeline-progress"
            provider = MergeParagraphsProvider()

            with mock.patch.dict(
                os.environ,
                {"BIBLIOSMITH_PROGRESS_PATH": str(progress_path)},
            ):
                report = run_manifest(
                    manifest_path,
                    provider_factory=lambda profile_id, *, config_id: provider,
                )

            metrics = report["units"][0]["metrics"]
            progress = json.loads(progress_path.read_text(encoding="utf-8"))
            self.assertEqual(report["units"][0]["status"], "failed")
            self.assertFalse(metrics["secondPassApplied"])
            self.assertEqual(progress["total"], metrics["chunkCount"] * 2)
            self.assertEqual(progress["completed"], 0)
            self.assertEqual(progress["phase"], "translating")


if __name__ == "__main__":
    unittest.main()
