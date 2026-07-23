from pathlib import Path
import tempfile
import unittest

from translation_engine.engine import run_manifest
from translation_engine.pipeline import SecondPass, SecondPassRequest
from translation_engine.profiles import TargetLanguageProfile
from translation_engine.providers import TranslationRequest
from tests.fixtures import build_run_fixture


class CapturingProvider:
    profile_id = "fake-provider-profile"
    config_id = "fake-config-no-secrets"

    def __init__(self) -> None:
        self.instructions: list[str] = []

    def translate(self, request: TranslationRequest) -> str:
        self.instructions.append(request.system_instruction)
        return request.text.upper()


class ReplacingSecondPass(SecondPass):
    def __init__(self) -> None:
        self.called = False

    def reflect(
        self,
        *,
        request: SecondPassRequest,
    ) -> str:
        self.called = True
        return "replace FIRST with SECOND"

    def improve(
        self,
        *,
        request: SecondPassRequest,
        reflection_text: str,
    ) -> str:
        return request.draft_text.replace("FIRST", "SECOND")


class PipelineExtensionTests(unittest.TestCase):
    def test_profile_glossary_hook_and_second_pass_are_usable_pipeline_seams(self) -> None:
        with tempfile.TemporaryDirectory() as temporary_directory:
            project_root = Path(temporary_directory)
            manifest_path = build_run_fixture(
                project_root,
                source_text="first\n",
                max_tokens=20,
                second_pass_enabled=True,
            )
            provider = CapturingProvider()
            second_pass = ReplacingSecondPass()
            glossary_calls: list[str] = []

            def glossary_hook(source_text: str, task_manifest: dict[str, object]) -> str:
                glossary_calls.append(source_text)
                return "GLOSSARY SLOT"

            profile = TargetLanguageProfile(
                language="zh-Hans",
                system_instruction="translate",
                glossary_hook=glossary_hook,
            )

            report = run_manifest(
                manifest_path,
                provider_factory=lambda profile_id, *, config_id: provider,
                target_profile_factory=lambda language: profile,
                second_pass=second_pass,
            )

            self.assertEqual(report["units"][0]["status"], "completed")
            self.assertEqual(glossary_calls, ["first\n", "first\n"])
            self.assertTrue(all("GLOSSARY SLOT" in value for value in provider.instructions))
            self.assertTrue(second_pass.called)
            output = (
                project_root / "chapters" / "translated" / "chapter_001.md"
            ).read_text(encoding="utf-8")
            self.assertEqual(output, "SECOND\n")

    def test_second_pass_is_disabled_by_default(self) -> None:
        with tempfile.TemporaryDirectory() as temporary_directory:
            project_root = Path(temporary_directory)
            manifest_path = build_run_fixture(
                project_root, source_text="first\n", max_tokens=20
            )
            second_pass = ReplacingSecondPass()

            report = run_manifest(manifest_path, second_pass=second_pass)

            self.assertEqual(report["units"][0]["status"], "completed")
            self.assertFalse(second_pass.called)
            output = (
                project_root / "chapters" / "translated" / "chapter_001.md"
            ).read_text(encoding="utf-8")
            self.assertEqual(output, "FIRST\n")

if __name__ == "__main__":
    unittest.main()
