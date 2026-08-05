import json
from copy import deepcopy
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest

from tests.fixtures import (
    FOUR_DIMENSION_PROMPT_PACK,
    STRUCTURE_FIDELITY_PROMPT_PACK,
    build_run_fixture,
    build_sample_fixture,
)
from translation_engine.engine import EngineError, run_manifest
from translation_engine.prompt_packs import revision_content_sha256
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


class FourDimensionSampleProvider(CapturingProvider):
    def translate(self, request: TranslationRequest) -> str:
        self.instructions.append(request.system_instruction)
        if "Output only the suggestions" in request.system_instruction:
            return "SAMPLE_REFLECTION_EVIDENCE"
        if "translation editing" in request.system_instruction:
            payload = json.loads(request.text)
            if payload["reflection"] != "SAMPLE_REFLECTION_EVIDENCE":
                raise AssertionError("sample reflection did not reach improvement")
            return payload["draft"].replace("DRAFT", "FINAL")
        return "DRAFT"


# Three chapters and sample_count=1 puts exactly one block in the sample: the
# selector treats the first and last as front and back matter.
SAMPLE_TEXTS = ["Front matter.", "Body sentence.", "Back matter."]
RUN_TEXT = "Body sentence.\n"


class SampleTranslationTests(unittest.TestCase):
    def test_four_dimension_sample_runs_translate_reflect_and_improve(self) -> None:
        with tempfile.TemporaryDirectory() as temporary_directory:
            manifest_path = build_sample_fixture(
                Path(temporary_directory),
                source_texts=SAMPLE_TEXTS,
                sample_count=1,
                character_budget=64,
                prompt_pack=FOUR_DIMENSION_PROMPT_PACK,
            )
            provider = FourDimensionSampleProvider()

            report = run_sample_manifest(
                manifest_path,
                provider_factory=lambda profile_id, *, config_id: provider,
            )

            self.assertEqual(len(provider.instructions), 3)
            self.assertEqual(report["samples"][0]["translatedExcerpt"], "FINAL")

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

    def test_the_spec_the_caller_passes_governs_both_how_many_and_how_much(
        self,
    ) -> None:
        """N and the character budget are inputs, not constants in here.

        The same five chapters sampled two ways: the counts, the chapters chosen,
        and where each excerpt stops all follow the manifest.
        """
        source_texts = [
            "Front matter.",
            "Second chapter opens. It also continues past the budget.",
            "Third chapter opens. It also continues past the budget.",
            "Fourth chapter opens. It also continues past the budget.",
            "Back matter.",
        ]

        def sample(*, sample_count: int, character_budget: int) -> list[dict]:
            with tempfile.TemporaryDirectory() as temporary_directory:
                manifest_path = build_sample_fixture(
                    Path(temporary_directory),
                    source_texts=source_texts,
                    sample_count=sample_count,
                    character_budget=character_budget,
                )
                return run_sample_manifest(manifest_path)["samples"]

        narrow = sample(sample_count=1, character_budget=20)
        wide = sample(sample_count=3, character_budget=200)

        self.assertEqual([entry["chunkRef"] for entry in narrow], ["chapter_003"])
        self.assertEqual(
            [entry["chunkRef"] for entry in wide],
            ["chapter_002", "chapter_003", "chapter_004"],
        )
        self.assertEqual(narrow[0]["sourceExcerpt"], "Third chapter opens.")
        self.assertEqual(wide[1]["sourceExcerpt"], source_texts[2])

    def test_two_chapters_or_fewer_yield_an_empty_report_at_no_cost(self) -> None:
        """Nothing sits between the excluded endpoints, so there is nothing to
        preview. It is a defined outcome, not a failure: the command succeeds,
        the provider is never called, and the caller has to explain the emptiness
        rather than render a blank panel."""
        for chapter_count in (1, 2):
            with self.subTest(chapters=chapter_count):
                provider = CapturingProvider()
                with tempfile.TemporaryDirectory() as temporary_directory:
                    project_root = Path(temporary_directory)
                    manifest_path = build_sample_fixture(
                        project_root,
                        source_texts=[
                            f"Chapter {index}." for index in range(chapter_count)
                        ],
                        sample_count=3,
                        character_budget=800,
                    )

                    report = run_sample_manifest(
                        manifest_path,
                        provider_factory=lambda profile_id, *, config_id: provider,
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

                self.assertEqual(
                    report,
                    {
                        "schema": "translation-engine-sample-report-v1",
                        "samples": [],
                    },
                )
                self.assertEqual(provider.instructions, [])
                self.assertEqual(completed.returncode, 0, completed.stderr)
                self.assertEqual(
                    json.loads(completed.stdout).get("samples"), []
                )
                self.assertNotIn("error", json.loads(completed.stdout))

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

    The sample and full run must compile the same selected immutable revision,
    or approval would be based on a request the real run never sends.
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

    def test_selected_template_precedes_structure_protection(self) -> None:
        prompt_pack = deepcopy(STRUCTURE_FIDELITY_PROMPT_PACK)
        prompt_pack["stages"][0]["template"] = "Use restrained literary Chinese."
        prompt_pack["contentSha256"] = revision_content_sha256(prompt_pack)
        instruction = self._sample_instruction(prompt_pack=prompt_pack)

        self.assertIn("Use restrained literary Chinese.", instruction)
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
        prompt_pack = deepcopy(STRUCTURE_FIDELITY_PROMPT_PACK)
        prompt_pack["stages"][0]["template"] = "Use restrained literary Chinese."
        prompt_pack["contentSha256"] = revision_content_sha256(prompt_pack)
        settings: dict[str, object] = {"text_cleanup": True, "prompt_pack": prompt_pack}

        self.assertEqual(
            self._sample_instruction(**settings),
            self._run_instruction(**settings),
        )

    def test_sample_rejects_a_tampered_prompt_pack_revision(self) -> None:
        prompt_pack = deepcopy(STRUCTURE_FIDELITY_PROMPT_PACK)
        prompt_pack["stages"][0]["template"] = "Tampered without a new hash."
        with self.assertRaisesRegex(EngineError, "prompt_pack_content_hash_mismatch"):
            self._sample_instruction(prompt_pack=prompt_pack)

    def test_non_boolean_text_cleanup_is_rejected(self) -> None:
        with self.assertRaisesRegex(EngineError, "invalid_text_cleanup"):
            self._sample_instruction(text_cleanup="true")


if __name__ == "__main__":
    unittest.main()
