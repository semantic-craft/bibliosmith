import json
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest

from tests.fixtures import build_run_fixture, build_sample_fixture
from translation_engine.engine import EngineError, run_manifest
from translation_engine.providers import TranslationRequest
from translation_engine.sample import run_sample_manifest


class CapturingProvider:
    profile_id = "fake-provider-profile"
    config_id = "fake-config-no-secrets"

    def __init__(self) -> None:
        self.instructions: list[str] = []

    def translate(self, request: TranslationRequest) -> str:
        self.instructions.append(request.system_instruction)
        return request.text


# Three chapters and sample_count=1 puts exactly one block in the sample: the
# selector treats the first and last as front and back matter.
SAMPLE_TEXTS = ["Front matter.", "Body sentence.", "Back matter."]
RUN_TEXT = "Body sentence.\n"


class SampleTranslationTests(unittest.TestCase):
    def test_fake_provider_returns_stable_comparison_report_without_translation_artifacts(self) -> None:
        with tempfile.TemporaryDirectory() as temporary_directory:
            project_root = Path(temporary_directory)
            manifest_path = build_sample_fixture(
                project_root,
                source_texts=[
                    "Front matter.",
                    "# Second\n\nKeep `literal` here. This sentence stays out.",
                    "Third chapter.",
                    "Fourth sample sentence. This sentence stays out.",
                    "Back matter.",
                ],
                sample_count=2,
                character_budget=18,
            )
            files_before = {
                path.relative_to(project_root): path.read_bytes()
                for path in project_root.rglob("*")
                if path.is_file()
            }

            report = run_sample_manifest(manifest_path)

            self.assertEqual(set(report), {"schema", "samples"})
            self.assertEqual(report["schema"], "translation-engine-sample-report-v1")
            self.assertEqual(len(report["samples"]), 2)
            self.assertEqual(
                [sample["chunkRef"] for sample in report["samples"]],
                ["chapter_002", "chapter_004"],
            )
            self.assertEqual(
                set(report["samples"][0]),
                {"chunkRef", "sourceExcerpt", "translatedExcerpt", "degradation"},
            )
            self.assertEqual(
                report["samples"][0]["sourceExcerpt"],
                "# Second\n\nKeep `literal` here.",
            )
            self.assertEqual(
                report["samples"][0]["translatedExcerpt"],
                "# SECOND\n\nKEEP `literal` HERE.",
            )
            self.assertEqual(report["samples"][0]["degradation"], "none")
            self.assertFalse((project_root / "chapters" / "translated").exists())
            self.assertEqual(
                {
                    path.relative_to(project_root): path.read_bytes()
                    for path in project_root.rglob("*")
                    if path.is_file()
                },
                files_before,
            )

    def test_sample_cli_prints_only_the_json_report(self) -> None:
        with tempfile.TemporaryDirectory() as temporary_directory:
            project_root = Path(temporary_directory)
            manifest_path = build_sample_fixture(
                project_root,
                source_texts=[
                    "Front matter.",
                    "Sample sentence.",
                    "Back matter.",
                ],
                sample_count=1,
                character_budget=5,
            )

            completed = subprocess.run(
                [
                    sys.executable,
                    "-m",
                    "translation_engine.sample_cli",
                    "--manifest",
                    str(manifest_path),
                ],
                check=False,
                capture_output=True,
                text=True,
            )

            self.assertEqual(completed.returncode, 0, completed.stderr)
            report = json.loads(completed.stdout)
            self.assertEqual(report["schema"], "translation-engine-sample-report-v1")
            self.assertEqual(report["samples"][0]["chunkRef"], "chapter_002")
            self.assertEqual(completed.stderr, "")


class SamplePromptFidelityTests(unittest.TestCase):
    """The preview exists to be trusted, so it has to use the real run's prompt.

    Before this, the sample carried neither textCleanup nor customInstructions,
    so anyone who had set either was approving a full run on the strength of a
    translation produced under different instructions.
    """

    def _sample_instruction(self, **fixture_kwargs: object) -> str:
        with tempfile.TemporaryDirectory() as temporary_directory:
            project_root = Path(temporary_directory)
            manifest_path = build_sample_fixture(
                project_root,
                source_texts=SAMPLE_TEXTS,
                sample_count=1,
                character_budget=64,
                **fixture_kwargs,  # type: ignore[arg-type]
            )
            provider = CapturingProvider()

            run_sample_manifest(
                manifest_path,
                provider_factory=lambda profile_id, *, config_id: provider,
            )

            self.assertFalse((project_root / "chapters" / "translated").exists())
            self.assertEqual(len(provider.instructions), 1)
            return provider.instructions[0]

    def _run_instruction(self, **fixture_kwargs: object) -> str:
        with tempfile.TemporaryDirectory() as temporary_directory:
            project_root = Path(temporary_directory)
            manifest_path = build_run_fixture(
                project_root,
                source_text=RUN_TEXT,
                max_tokens=100,
                **fixture_kwargs,  # type: ignore[arg-type]
            )
            provider = CapturingProvider()

            run_manifest(
                manifest_path,
                provider_factory=lambda profile_id, *, config_id: provider,
            )

            return provider.instructions[0]

    def test_text_cleanup_reaches_the_sample_prompt(self) -> None:
        without = self._sample_instruction()
        with_cleanup = self._sample_instruction(text_cleanup=True)

        self.assertNotIn("# TEXT CLEANUP", without)
        self.assertIn("# TEXT CLEANUP - WITHIN PARAGRAPHS ONLY", with_cleanup)

    def test_custom_translation_directive_precedes_structure_protection(self) -> None:
        instruction = self._sample_instruction(
            custom_instructions={"translation": "Use restrained literary Chinese."}
        )

        self.assertIn("# USER STYLE DIRECTIVES", instruction)
        self.assertIn("Use restrained literary Chinese.", instruction)
        # The protection block has to stay last, or a style directive could talk
        # the model out of preserving placeholders and headings.
        self.assertLess(
            instruction.index("Use restrained literary Chinese."),
            instruction.index("# MANDATORY STRUCTURE PROTECTION"),
        )

    def test_sample_and_run_build_byte_identical_instructions(self) -> None:
        """The point of the ticket: same book, same settings, same prompt.

        Both fixtures default to an empty glossary, so the glossary block is
        absent from each and the comparison covers the whole instruction rather
        than only the two sections in question.
        """
        settings: dict[str, object] = {
            "text_cleanup": True,
            "custom_instructions": {"translation": "Use restrained literary Chinese."},
        }

        self.assertEqual(
            self._sample_instruction(**settings),
            self._run_instruction(**settings),
        )

    def test_reflection_directive_is_validated_but_not_applied(self) -> None:
        """The sample runs one pass, so a reflection directive has no pass to join.

        It is still parsed, so a malformed one is rejected here exactly as the
        full run would reject it rather than surviving review and failing later.
        """
        instruction = self._sample_instruction(
            custom_instructions={
                "translation": "Use restrained literary Chinese.",
                "reflection": "Critique anachronistic wording.",
            }
        )

        self.assertIn("Use restrained literary Chinese.", instruction)
        self.assertNotIn("Critique anachronistic wording.", instruction)
        self.assertNotIn("# USER REFLECTION DIRECTIVES", instruction)

        with self.assertRaisesRegex(EngineError, "invalid_custom_instructions"):
            self._sample_instruction(custom_instructions={"translation": 5})

    def test_non_boolean_text_cleanup_is_rejected(self) -> None:
        with self.assertRaisesRegex(EngineError, "invalid_text_cleanup"):
            self._sample_instruction(text_cleanup="true")


if __name__ == "__main__":
    unittest.main()
