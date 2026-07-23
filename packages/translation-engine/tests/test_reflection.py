import json
from pathlib import Path
import tempfile
import unittest

from translation_engine.engine import run_manifest
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


class InterruptingReflectionProvider(ReflectionFakeProvider):
    def translate(self, request: TranslationRequest) -> str:
        if "Output only the suggestions" in request.system_instruction:
            self.reflection_requests.append(request)
            if len(self.reflection_requests) == 2:
                raise ProviderUnavailableError("simulated second-pass interruption")
            return f"critique-{len(self.reflection_requests)}"
        return super().translate(request)


class ReflectionSecondPassTests(unittest.TestCase):
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

    def test_invalid_revised_placeholders_keep_the_draft(self) -> None:
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
            draft_path = project_root / unit["secondPassArtifacts"]["draft"]["path"]
            revised_path = (
                project_root / unit["secondPassArtifacts"]["revised"]["path"]
            )
            self.assertEqual(unit["status"], "completed")
            self.assertEqual(
                revised_path.read_text(encoding="utf-8"),
                draft_path.read_text(encoding="utf-8"),
            )
            self.assertEqual(revised_path.read_text(encoding="utf-8"), "FIRST `keepCase`\n")

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
                draft_checkpoint["idempotencyKey"]["passId"], "translation-v1"
            )
            self.assertEqual(
                reflection_checkpoint["idempotencyKey"]["passId"],
                "reflection-v1",
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


if __name__ == "__main__":
    unittest.main()
