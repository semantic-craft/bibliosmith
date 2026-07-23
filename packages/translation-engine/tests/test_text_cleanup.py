from pathlib import Path
import json
import tempfile
import unittest

from translation_engine.engine import EngineError, run_manifest
from translation_engine.providers import ProviderUnavailableError, TranslationRequest
from tests.fixtures import build_run_fixture


class CapturingProvider:
    profile_id = "fake-provider-profile"
    config_id = "fake-config-no-secrets"

    def __init__(self) -> None:
        self.instructions: list[str] = []

    def translate(self, request: TranslationRequest) -> str:
        self.instructions.append(request.system_instruction)
        return request.text


class ParagraphChangingProvider(CapturingProvider):
    def translate(self, request: TranslationRequest) -> str:
        self.instructions.append(request.system_instruction)
        if len(self.instructions) == 1:
            return f"{request.text}\n\nAdded paragraph."
        return request.text


class InterruptingProvider(CapturingProvider):
    def translate(self, request: TranslationRequest) -> str:
        if self.instructions:
            raise ProviderUnavailableError("simulated interruption")
        return super().translate(request)


class TextCleanupTests(unittest.TestCase):
    def test_manifest_rejects_non_boolean_text_cleanup(self) -> None:
        with tempfile.TemporaryDirectory() as temporary_directory:
            project_root = Path(temporary_directory)
            manifest_path = build_run_fixture(
                project_root,
                source_text="Source text.\n",
                max_tokens=100,
            )
            manifest = json.loads(manifest_path.read_text(encoding="utf-8"))
            manifest["textCleanup"] = "true"
            manifest_path.write_text(
                json.dumps(manifest, indent=2) + "\n",
                encoding="utf-8",
            )

            with self.assertRaisesRegex(EngineError, "invalid_text_cleanup"):
                run_manifest(manifest_path)

    def test_omitted_and_false_text_cleanup_keep_instruction_byte_identical(
        self,
    ) -> None:
        with (
            tempfile.TemporaryDirectory() as omitted_directory,
            tempfile.TemporaryDirectory() as false_directory,
        ):
            omitted_provider = CapturingProvider()
            false_provider = CapturingProvider()
            omitted_manifest = build_run_fixture(
                Path(omitted_directory),
                source_text="Source text.\n",
                max_tokens=100,
            )
            false_manifest = build_run_fixture(
                Path(false_directory),
                source_text="Source text.\n",
                max_tokens=100,
                text_cleanup=False,
            )

            run_manifest(
                omitted_manifest,
                provider_factory=lambda profile_id, *, config_id: omitted_provider,
            )
            run_manifest(
                false_manifest,
                provider_factory=lambda profile_id, *, config_id: false_provider,
            )

            self.assertEqual(false_provider.instructions, omitted_provider.instructions)
            self.assertNotIn("# TEXT CLEANUP", omitted_provider.instructions[0])

    def test_enabled_text_cleanup_appends_paragraph_internal_rules_after_glossary(
        self,
    ) -> None:
        with tempfile.TemporaryDirectory() as temporary_directory:
            project_root = Path(temporary_directory)
            manifest_path = build_run_fixture(
                project_root,
                source_text="Secret trans-\nlation.\n",
                max_tokens=100,
                glossary_text=(
                    "source,translation,category,note\n"
                    "Secret,秘密,other,\n"
                ),
                text_cleanup=True,
            )
            provider = CapturingProvider()

            report = run_manifest(
                manifest_path,
                provider_factory=lambda profile_id, *, config_id: provider,
            )

            self.assertEqual(report["units"][0]["status"], "completed")
            instruction = provider.instructions[0]
            self.assertIn("# GLOSSARY - REQUIRED TRANSLATIONS", instruction)
            self.assertIn("- Secret -> 秘密 [other]", instruction)
            self.assertIn("# TEXT CLEANUP - WITHIN PARAGRAPHS ONLY", instruction)
            self.assertIn("Rejoin words split by line-break hyphenation", instruction)
            self.assertIn("Fix extra or missing spaces", instruction)
            self.assertIn("Fix clearly incorrect punctuation", instruction)
            self.assertIn("Never add or remove content", instruction)
            self.assertIn("never merge or split paragraphs", instruction)
            self.assertIn("never add or remove headings", instruction)
            self.assertIn("never rewrite the author's style", instruction)
            self.assertLess(
                instruction.index("# GLOSSARY - REQUIRED TRANSLATIONS"),
                instruction.index("# TEXT CLEANUP - WITHIN PARAGRAPHS ONLY"),
            )

    def test_enabled_text_cleanup_still_rejects_changed_paragraph_count(self) -> None:
        with tempfile.TemporaryDirectory() as temporary_directory:
            project_root = Path(temporary_directory)
            manifest_path = build_run_fixture(
                project_root,
                source_text="Source paragraph.\n",
                max_tokens=100,
                text_cleanup=True,
            )
            provider = ParagraphChangingProvider()

            report = run_manifest(
                manifest_path,
                provider_factory=lambda profile_id, *, config_id: provider,
            )

            self.assertEqual(report["units"][0]["status"], "completed")
            self.assertEqual(report["units"][0]["metrics"]["providerAttemptCount"], 2)
            self.assertEqual(len(provider.instructions), 2)
            self.assertTrue(
                all("# TEXT CLEANUP" in instruction for instruction in provider.instructions)
            )
            translated = (
                project_root / "chapters" / "translated" / "chapter_001.md"
            ).read_text(encoding="utf-8")
            self.assertEqual(translated, "Source paragraph.\n")

    def test_enabled_text_cleanup_uses_a_distinct_checkpoint_pass(self) -> None:
        with tempfile.TemporaryDirectory() as temporary_directory:
            project_root = Path(temporary_directory)
            manifest_path = build_run_fixture(
                project_root,
                source_text="aa\nbb\n",
                max_tokens=3,
                text_cleanup=True,
            )

            report = run_manifest(
                manifest_path,
                provider_factory=lambda profile_id, *, config_id: InterruptingProvider(),
            )

            self.assertEqual(report["units"][0]["status"], "failed")
            checkpoint = json.loads(
                (
                    project_root
                    / "chapters"
                    / "translated"
                    / ".partial"
                    / "chapter_001.json"
                ).read_text(encoding="utf-8")
            )
            self.assertEqual(
                checkpoint["idempotencyKey"]["passId"],
                "translation-v1-text-cleanup",
            )


if __name__ == "__main__":
    unittest.main()
