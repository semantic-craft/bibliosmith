import json
from pathlib import Path
import tempfile
import unittest

from translation_engine.engine import run_manifest
from translation_engine.pipeline import (
    SecondPassRequest,
    WindowedReflectionSecondPass,
    run_second_pass_chunk,
)
from translation_engine.profiles import TargetLanguageProfile
from translation_engine.providers import ProviderUnavailableError, TranslationRequest
from tests.fixtures import build_run_fixture


class ReflectionFakeProvider:
    profile_id = "fake-provider-profile"
    config_id = "fake-config-no-secrets"

    def __init__(self) -> None:
        self.draft_requests: list[TranslationRequest] = []
        self.reflection_requests: list[TranslationRequest] = []
        self.improve_requests: list[TranslationRequest] = []

    def translate(self, request: TranslationRequest) -> str:
        if "Output only the suggestions" in request.system_instruction:
            self.reflection_requests.append(request)
            return f"critique-{len(self.reflection_requests)}"
        if "translation editing" in request.system_instruction:
            self.improve_requests.append(request)
            payload = json.loads(request.text)
            return payload["draft"].replace("FIRST", "REVISED")
        self.draft_requests.append(request)
        return request.text.upper()


class DroppingPlaceholderProvider(ReflectionFakeProvider):
    def translate(self, request: TranslationRequest) -> str:
        if "translation editing" in request.system_instruction:
            self.improve_requests.append(request)
            payload = json.loads(request.text)
            return payload["draft"].replace("⟦PH_000000⟧", "")
        return super().translate(request)


class DroppingParagraphPlaceholderProvider(ReflectionFakeProvider):
    def translate(self, request: TranslationRequest) -> str:
        if "translation editing" in request.system_instruction:
            self.improve_requests.append(request)
            payload = json.loads(request.text)
            return payload["draft"].replace("⟦PH_000000⟧", "")
        return super().translate(request)


class DuplicatingParagraphPlaceholderProvider(ReflectionFakeProvider):
    def translate(self, request: TranslationRequest) -> str:
        if "translation editing" in request.system_instruction:
            self.improve_requests.append(request)
            payload = json.loads(request.text)
            return payload["draft"].replace("⟦PH_000001⟧", "⟦PH_000000⟧")
        return super().translate(request)


class AddingStructureReflectionProvider(ReflectionFakeProvider):
    def translate(self, request: TranslationRequest) -> str:
        if "translation editing" in request.system_instruction:
            self.improve_requests.append(request)
            payload = json.loads(request.text)
            return (
                payload["draft"]
                .replace("HEADING", "# 标题\n\n")
                .replace("PARAGRAPH.", "第一段。")
            )
        return super().translate(request)


class SplittingParagraphReflectionProvider(ReflectionFakeProvider):
    def translate(self, request: TranslationRequest) -> str:
        if "translation editing" in request.system_instruction:
            self.improve_requests.append(request)
            draft = json.loads(request.text)["draft"]
            return draft.replace("MIDDLE", "MIDDLE\n\n")
        return super().translate(request)


class InterruptingReflectionProvider(ReflectionFakeProvider):
    def translate(self, request: TranslationRequest) -> str:
        if "Output only the suggestions" in request.system_instruction:
            self.reflection_requests.append(request)
            if len(self.reflection_requests) == 2:
                raise ProviderUnavailableError("simulated second-pass interruption")
            return f"critique-{len(self.reflection_requests)}"
        return super().translate(request)


class ReflectionPlaceholderLeakProvider:
    profile_id = "reflection-placeholder-leak-test"
    config_id = "fake-config-no-secrets"

    def __init__(self) -> None:
        self.improve_reflection = ""

    def translate(self, request: TranslationRequest) -> str:
        if "Output only the suggestions" in request.system_instruction:
            return "Keep the neighboring marker ⟦PH_000000⟧ unchanged."
        if "translation editing" in request.system_instruction:
            payload = json.loads(request.text)
            self.improve_reflection = payload["reflection"]
            leaked = (
                "⟦PH_000000⟧" if "⟦PH_000000⟧" in self.improve_reflection else ""
            )
            return f'{leaked}{payload["draft"]}'
        raise AssertionError("unexpected request")


class FlakyStructureReflectionProvider(ReflectionFakeProvider):
    def __init__(self) -> None:
        super().__init__()
        self.improve_attempts = 0

    def translate(self, request: TranslationRequest) -> str:
        if "translation editing" in request.system_instruction:
            self.improve_requests.append(request)
            self.improve_attempts += 1
            draft = json.loads(request.text)["draft"]
            if self.improve_attempts == 1:
                return f"⟦PH_999999⟧{draft}"
            return draft
        return super().translate(request)


class FlakyReflectionProvider(ReflectionFakeProvider):
    def __init__(self) -> None:
        super().__init__()
        self.reflection_attempts = 0

    def translate(self, request: TranslationRequest) -> str:
        if "Output only the suggestions" in request.system_instruction:
            self.reflection_requests.append(request)
            self.reflection_attempts += 1
            return "bad-structure-advice" if self.reflection_attempts == 1 else "good"
        if "translation editing" in request.system_instruction:
            self.improve_requests.append(request)
            payload = json.loads(request.text)
            leaked = (
                "⟦PH_999999⟧"
                if payload["reflection"] == "bad-structure-advice"
                else ""
            )
            return f'{leaked}{payload["draft"]}'
        return super().translate(request)


class RepeatingReflectionProvider(ReflectionFakeProvider):
    def __init__(self) -> None:
        super().__init__()
        self.improve_attempts = 0

    def translate(self, request: TranslationRequest) -> str:
        if "translation editing" in request.system_instruction:
            self.improve_requests.append(request)
            self.improve_attempts += 1
            draft = json.loads(request.text)["draft"]
            if self.improve_attempts == 1:
                repeated = "商业秘密的经济价值需要单独评估。"
                return f"{repeated}中间内容。{repeated}"
            return draft
        return super().translate(request)


class ReflectionSecondPassTests(unittest.TestCase):
    def test_reflection_placeholders_are_not_fed_back_into_the_revised_draft(
        self,
    ) -> None:
        provider = ReflectionPlaceholderLeakProvider()
        request = SecondPassRequest(
            source_text="current source ⟦PH_000001⟧",
            draft_text="current draft ⟦PH_000001⟧",
            previous_source_text="previous source ⟦PH_000000⟧",
            previous_draft_text="previous draft ⟦PH_000000⟧",
            next_source_text=None,
            next_draft_text=None,
            source_language="de",
            target_language="zh-Hans",
            terminology_criteria="none",
        )

        result = run_second_pass_chunk(
            WindowedReflectionSecondPass(provider),
            request,
        )

        self.assertEqual(result.revised_text, request.draft_text)
        self.assertIn("⟦PH_000000⟧", result.reflection_text)
        self.assertNotIn("⟦PH_000000⟧", provider.improve_reflection)

    def test_second_pass_retries_a_transient_structure_failure(self) -> None:
        with tempfile.TemporaryDirectory() as temporary_directory:
            project_root = Path(temporary_directory)
            manifest_path = build_run_fixture(
                project_root,
                source_text="first `keepCase`\n",
                max_tokens=40,
                second_pass_enabled=True,
            )
            provider = FlakyStructureReflectionProvider()

            report = run_manifest(
                manifest_path,
                provider_factory=lambda profile_id, *, config_id: provider,
            )

            self.assertEqual(report["units"][0]["status"], "completed")
            self.assertEqual(provider.improve_attempts, 2)

    def test_second_pass_retry_regenerates_the_reflection(self) -> None:
        with tempfile.TemporaryDirectory() as temporary_directory:
            project_root = Path(temporary_directory)
            manifest_path = build_run_fixture(
                project_root,
                source_text="first `keepCase`\n",
                max_tokens=40,
                second_pass_enabled=True,
            )
            provider = FlakyReflectionProvider()

            report = run_manifest(
                manifest_path,
                provider_factory=lambda profile_id, *, config_id: provider,
            )

            self.assertEqual(report["units"][0]["status"], "completed")
            self.assertEqual(provider.reflection_attempts, 2)
            self.assertEqual(len(provider.improve_requests), 2)

    def test_second_pass_retries_model_added_repetition(self) -> None:
        with tempfile.TemporaryDirectory() as temporary_directory:
            project_root = Path(temporary_directory)
            manifest_path = build_run_fixture(
                project_root,
                source_text="Trade secret value needs a separate assessment.\n",
                max_tokens=200,
                second_pass_enabled=True,
            )
            provider = RepeatingReflectionProvider()

            report = run_manifest(
                manifest_path,
                provider_factory=lambda profile_id, *, config_id: provider,
            )

            self.assertEqual(report["units"][0]["status"], "completed")
            self.assertEqual(provider.improve_attempts, 2)
            translated = (
                project_root / "chapters" / "translated" / "chapter_001.md"
            ).read_text(encoding="utf-8")
            self.assertEqual(
                translated,
                "TRADE SECRET VALUE NEEDS A SEPARATE ASSESSMENT.\n",
            )

    def test_enabled_second_pass_archives_draft_reflection_and_revised(self) -> None:
        with tempfile.TemporaryDirectory() as temporary_directory:
            project_root = Path(temporary_directory)
            manifest_path = build_run_fixture(
                project_root,
                source_text="first\n",
                max_tokens=20,
                second_pass_enabled=True,
            )
            provider = ReflectionFakeProvider()

            report = run_manifest(
                manifest_path,
                provider_factory=lambda profile_id, *, config_id: provider,
            )

            unit = report["units"][0]
            self.assertEqual(unit["status"], "completed")
            self.assertEqual(
                set(unit["secondPassArtifacts"]),
                {"draft", "reflection", "revised"},
            )
            paths = {
                name: project_root / artifact["path"]
                for name, artifact in unit["secondPassArtifacts"].items()
            }
            self.assertEqual(paths["draft"].read_text(encoding="utf-8"), "FIRST\n")
            self.assertIn(
                "critique-1", paths["reflection"].read_text(encoding="utf-8")
            )
            self.assertEqual(
                paths["revised"].read_text(encoding="utf-8"), "REVISED\n"
            )
            self.assertEqual(
                (
                    len(provider.draft_requests),
                    len(provider.reflection_requests),
                    len(provider.improve_requests),
                ),
                (1, 1, 1),
            )
            self.assertIn(
                "Translate each source segment exactly once",
                provider.improve_requests[0].system_instruction,
            )
            self.assertIn(
                "bibliography title text",
                provider.improve_requests[0].system_instruction,
            )

    def test_reflection_uses_only_neighbor_blocks_and_chunk_glossary_criteria(
        self,
    ) -> None:
        with tempfile.TemporaryDirectory() as temporary_directory:
            project_root = Path(temporary_directory)
            manifest_path = build_run_fixture(
                project_root,
                source_text="aa\nbb\ncc\ndd\n",
                max_tokens=3,
                second_pass_enabled=True,
            )
            provider = ReflectionFakeProvider()
            profile = TargetLanguageProfile(
                language="zh-Hans",
                system_instruction="translate",
                glossary_hook=lambda source, task: f"TERMS:{source.strip()}",
            )

            report = run_manifest(
                manifest_path,
                provider_factory=lambda profile_id, *, config_id: provider,
                target_profile_factory=lambda language: profile,
            )

            self.assertEqual(report["units"][0]["status"], "completed")
            windows = [json.loads(request.text) for request in provider.reflection_requests]
            self.assertEqual(
                windows,
                [
                    {
                        "previous": None,
                        "current": {"source": "aa\n", "draft": "AA\n"},
                        "next": {"source": "bb\n", "draft": "BB\n"},
                    },
                    {
                        "previous": {"source": "aa\n", "draft": "AA\n"},
                        "current": {"source": "bb\n", "draft": "BB\n"},
                        "next": {"source": "cc\n", "draft": "CC\n"},
                    },
                    {
                        "previous": {"source": "bb\n", "draft": "BB\n"},
                        "current": {"source": "cc\n", "draft": "CC\n"},
                        "next": {"source": "dd\n", "draft": "DD\n"},
                    },
                    {
                        "previous": {"source": "cc\n", "draft": "CC\n"},
                        "current": {"source": "dd\n", "draft": "DD\n"},
                        "next": None,
                    },
                ],
            )
            self.assertTrue(
                all(
                    f"TERMS:{source}" in request.system_instruction
                    for source, request in zip(
                        ("aa", "bb", "cc", "dd"),
                        provider.reflection_requests,
                        strict=True,
                    )
                )
            )

    def test_invalid_revised_structure_fails_instead_of_claiming_completion(self) -> None:
        with tempfile.TemporaryDirectory() as temporary_directory:
            project_root = Path(temporary_directory)
            manifest_path = build_run_fixture(
                project_root,
                source_text="first `keepCase`\n",
                max_tokens=40,
                second_pass_enabled=True,
            )
            provider = DroppingPlaceholderProvider()

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
            self.assertFalse(
                (project_root / "chapters" / "translated" / "chapter_001.md").exists()
            )

    def test_missing_paragraph_marker_keeps_the_validated_first_pass_draft(
        self,
    ) -> None:
        with tempfile.TemporaryDirectory() as temporary_directory:
            project_root = Path(temporary_directory)
            manifest_path = build_run_fixture(
                project_root,
                source_text="First paragraph.\n\nSecond paragraph.\n",
                max_tokens=100,
                second_pass_enabled=True,
            )
            provider = DroppingParagraphPlaceholderProvider()

            report = run_manifest(
                manifest_path,
                provider_factory=lambda profile_id, *, config_id: provider,
            )

            unit = report["units"][0]
            self.assertEqual(unit["status"], "completed")
            self.assertEqual(unit["metrics"]["secondPassDraftFallbackCount"], 1)
            translated = (
                project_root / "chapters" / "translated" / "chapter_001.md"
            ).read_text(encoding="utf-8")
            self.assertEqual(
                translated,
                "FIRST PARAGRAPH.\n\nSECOND PARAGRAPH.\n",
            )

    def test_repeated_paragraph_marker_keeps_the_validated_first_pass_draft(
        self,
    ) -> None:
        with tempfile.TemporaryDirectory() as temporary_directory:
            project_root = Path(temporary_directory)
            manifest_path = build_run_fixture(
                project_root,
                source_text="First.\n\nSecond.\n\nThird.\n",
                max_tokens=100,
                second_pass_enabled=True,
            )
            provider = DuplicatingParagraphPlaceholderProvider()

            report = run_manifest(
                manifest_path,
                provider_factory=lambda profile_id, *, config_id: provider,
            )

            unit = report["units"][0]
            self.assertEqual(unit["status"], "completed")
            self.assertEqual(unit["metrics"]["secondPassDraftFallbackCount"], 1)
            translated = (
                project_root / "chapters" / "translated" / "chapter_001.md"
            ).read_text(encoding="utf-8")
            self.assertEqual(translated, "FIRST.\n\nSECOND.\n\nTHIRD.\n")

    def test_model_added_break_keeps_the_validated_first_pass_draft(self) -> None:
        with tempfile.TemporaryDirectory() as temporary_directory:
            project_root = Path(temporary_directory)
            manifest_path = build_run_fixture(
                project_root,
                source_text="First middle last.\n",
                max_tokens=100,
                second_pass_enabled=True,
            )
            provider = SplittingParagraphReflectionProvider()

            report = run_manifest(
                manifest_path,
                provider_factory=lambda profile_id, *, config_id: provider,
            )

            unit = report["units"][0]
            self.assertEqual(unit["status"], "completed")
            self.assertEqual(unit["metrics"]["secondPassDraftFallbackCount"], 1)
            self.assertEqual(len(provider.improve_requests), 2)
            translated = (
                project_root / "chapters" / "translated" / "chapter_001.md"
            ).read_text(encoding="utf-8")
            self.assertEqual(translated, "FIRST MIDDLE LAST.\n")

    def test_second_pass_removes_model_added_block_structure(self) -> None:
        with tempfile.TemporaryDirectory() as temporary_directory:
            project_root = Path(temporary_directory)
            manifest_path = build_run_fixture(
                project_root,
                source_text="# heading\n\nparagraph.\n",
                max_tokens=80,
                second_pass_enabled=True,
            )
            provider = AddingStructureReflectionProvider()

            report = run_manifest(
                manifest_path,
                provider_factory=lambda profile_id, *, config_id: provider,
            )

            translated = (
                project_root / "chapters" / "translated" / "chapter_001.md"
            ).read_text(encoding="utf-8")

        self.assertEqual(report["units"][0]["status"], "completed")
        self.assertEqual(translated, "# 标题\n\n第一段。\n")

    def test_second_pass_resumes_by_chunk_with_a_distinct_idempotency_key(
        self,
    ) -> None:
        with tempfile.TemporaryDirectory() as temporary_directory:
            project_root = Path(temporary_directory)
            manifest_path = build_run_fixture(
                project_root,
                source_text="aa\nbb\ncc\n",
                max_tokens=3,
                second_pass_enabled=True,
            )
            interrupted_provider = InterruptingReflectionProvider()

            interrupted = run_manifest(
                manifest_path,
                provider_factory=lambda profile_id, *, config_id: interrupted_provider,
            )

            partial = project_root / "chapters" / "translated" / ".partial"
            draft_checkpoint_path = partial / "chapter_001.json"
            reflection_checkpoint_path = partial / "reflection" / "chapter_001.json"
            draft_checkpoint = json.loads(
                draft_checkpoint_path.read_text(encoding="utf-8")
            )
            reflection_checkpoint = json.loads(
                reflection_checkpoint_path.read_text(encoding="utf-8")
            )
            self.assertEqual(
                interrupted["units"][0]["error"],
                {"code": "provider_unavailable", "retryable": True},
            )
            self.assertEqual(
                draft_checkpoint["idempotencyKey"]["passId"],
                "translation-v1+chunking-policy-v5",
            )
            # The reflection key composes the first pass's id, because the
            # revisions it stores were computed from that pass's drafts.
            self.assertEqual(
                reflection_checkpoint["idempotencyKey"]["passId"],
                "reflection-v1+translation-v1+chunking-policy-v5",
            )
            self.assertEqual(reflection_checkpoint["nextChunkIndex"], 1)
            self.assertEqual(reflection_checkpoint["translatedChunks"], ["AA\n"])
            self.assertEqual(reflection_checkpoint["reflectionChunks"], ["critique-1"])

            resumed_provider = ReflectionFakeProvider()
            resumed = run_manifest(
                manifest_path,
                provider_factory=lambda profile_id, *, config_id: resumed_provider,
            )

            self.assertEqual(resumed["units"][0]["status"], "completed")
            self.assertEqual(
                resumed["units"][0]["metrics"]["secondPassResumedChunkCount"],
                1,
            )
            self.assertEqual(len(resumed_provider.draft_requests), 0)
            self.assertEqual(len(resumed_provider.reflection_requests), 2)
            self.assertEqual(len(resumed_provider.improve_requests), 2)
            self.assertFalse(draft_checkpoint_path.exists())
            self.assertFalse(reflection_checkpoint_path.exists())

    def test_a_text_cleanup_change_does_not_resume_a_stale_reflection(self) -> None:
        """An interrupted run whose text-cleanup setting then flips must redo the
        reflection rather than resume revisions computed from the old drafts.
        The reflection key used to omit text-cleanup entirely, so the first pass
        re-translated (its own key changed) while the reflection resumed against
        drafts that no longer existed."""
        with tempfile.TemporaryDirectory() as temporary_directory:
            project_root = Path(temporary_directory)
            interrupted_manifest = build_run_fixture(
                project_root,
                source_text="aa\nbb\ncc\n",
                max_tokens=3,
                second_pass_enabled=True,
                text_cleanup=False,
            )
            run_manifest(
                interrupted_manifest,
                provider_factory=lambda profile_id, *, config_id: (
                    InterruptingReflectionProvider()
                ),
            )
            reflection_checkpoint_path = (
                project_root
                / "chapters"
                / "translated"
                / ".partial"
                / "reflection"
                / "chapter_001.json"
            )
            self.assertTrue(reflection_checkpoint_path.exists())

            # Same project, same source, only the text-cleanup setting flips, so
            # rewrite the manifest rather than rebuilding the fixture.
            cleaned_manifest = project_root / "translation-run-cleanup.json"
            manifest = json.loads(interrupted_manifest.read_text(encoding="utf-8"))
            manifest["textCleanup"] = True
            cleaned_manifest.write_text(json.dumps(manifest), encoding="utf-8")
            resumed_provider = ReflectionFakeProvider()
            resumed = run_manifest(
                cleaned_manifest,
                provider_factory=lambda profile_id, *, config_id: resumed_provider,
            )

            self.assertEqual(resumed["units"][0]["status"], "completed")
            self.assertEqual(
                resumed["units"][0]["metrics"]["secondPassResumedChunkCount"],
                0,
                "a flipped text-cleanup setting must invalidate the reflection",
            )
            self.assertEqual(len(resumed_provider.reflection_requests), 3)


if __name__ == "__main__":
    unittest.main()
