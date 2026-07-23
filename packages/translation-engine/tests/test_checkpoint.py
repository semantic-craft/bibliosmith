import json
from pathlib import Path
import tempfile
import unittest

from translation_engine.checkpoint import (
    CheckpointStore,
    UnitCheckpoint,
    UnitIdempotencyKey,
)
from translation_engine.engine import run_manifest
from translation_engine.profiles import TargetLanguageProfile
from translation_engine.providers import (
    FakeProvider,
    ProviderUnavailableError,
    TranslationRequest,
)
from tests.fixtures import build_run_fixture


class FailAfterOneChunkProvider:
    profile_id = "fake-provider-profile"
    config_id = "fake-config-no-secrets"

    def __init__(self) -> None:
        self.calls = 0
        self.fake = FakeProvider(config_id=self.config_id)

    def translate(self, request: TranslationRequest) -> str:
        self.calls += 1
        if self.calls >= 2:
            raise ProviderUnavailableError("simulated interruption")
        return self.fake.translate(request)


class ContextCapturingProvider:
    profile_id = "fake-provider-profile"
    config_id = "fake-config-no-secrets"
    first_translation = "一二三四五六七八九十甲乙丙丁戊己庚辛壬癸子丑寅卯辰巳午未申酉"

    def __init__(self) -> None:
        self.requests: list[TranslationRequest] = []

    def translate(self, request: TranslationRequest) -> str:
        self.requests.append(request)
        if len(self.requests) == 1:
            return self.first_translation
        return request.text.upper()


class SimulatedKill(BaseException):
    pass


class KillAfterOneChunkProvider:
    profile_id = "fake-provider-profile"
    config_id = "fake-config-no-secrets"

    def __init__(self, credential: str) -> None:
        self.credential = credential
        self.calls = 0
        self.fake = FakeProvider(config_id=self.config_id)

    def translate(self, request: TranslationRequest) -> str:
        self.calls += 1
        if self.calls > 1:
            raise SimulatedKill
        return self.fake.translate(request)


class RecordingFakeProvider:
    profile_id = "fake-provider-profile"
    config_id = "fake-config-no-secrets"

    def __init__(self) -> None:
        self.requests: list[TranslationRequest] = []
        self.fake = FakeProvider(config_id=self.config_id)

    def translate(self, request: TranslationRequest) -> str:
        self.requests.append(request)
        return self.fake.translate(request)


class CheckpointStoreTests(unittest.TestCase):
    def test_later_chunk_request_includes_previous_translation_tail_of_25_words(
        self,
    ) -> None:
        with tempfile.TemporaryDirectory() as temporary_directory:
            project_root = Path(temporary_directory)
            manifest_path = build_run_fixture(
                project_root,
                source_text="aa bb",
                max_tokens=3,
            )
            provider = ContextCapturingProvider()
            profile = TargetLanguageProfile(
                language="zh-Hans",
                system_instruction="translate",
            )

            report = run_manifest(
                manifest_path,
                provider_factory=lambda profile_id, *, config_id: provider,
                target_profile_factory=lambda language: profile,
            )

            expected_tail = provider.first_translation[-25:]
            self.assertEqual(report["units"][0]["status"], "completed")
            self.assertEqual(len(provider.requests), 2)
            self.assertEqual(provider.requests[0].system_instruction, "translate")
            self.assertEqual(
                provider.requests[1].system_instruction,
                f"translate\n\n# CONTEXT\n{expected_tail}",
            )
            self.assertNotIn("一二三四五", provider.requests[1].system_instruction)

    def test_killed_run_resumes_to_uninterrupted_output_without_persisting_secrets(
        self,
    ) -> None:
        with tempfile.TemporaryDirectory() as temporary_directory:
            root = Path(temporary_directory)
            source_text = "one two three four five six.\n"
            baseline_root = root / "baseline"
            resumed_root = root / "resumed"
            baseline_manifest = build_run_fixture(
                baseline_root,
                source_text=source_text,
                max_tokens=4,
            )
            resumed_manifest = build_run_fixture(
                resumed_root,
                source_text=source_text,
                max_tokens=4,
            )

            baseline_report = run_manifest(baseline_manifest)
            credential = "credential-must-never-be-persisted"
            interrupted_provider = KillAfterOneChunkProvider(credential)
            with self.assertRaises(SimulatedKill):
                run_manifest(
                    resumed_manifest,
                    provider_factory=lambda profile_id, *, config_id: (
                        interrupted_provider
                    ),
                )

            checkpoint_path = (
                resumed_root
                / "chapters"
                / "translated"
                / ".partial"
                / "chapter_001.json"
            )
            checkpoint_text = checkpoint_path.read_text(encoding="utf-8")
            checkpoint_document = json.loads(checkpoint_text)
            self.assertEqual(checkpoint_document["nextChunkIndex"], 1)
            self.assertEqual(checkpoint_document["translatedChunks"], ["ONE "])
            self.assertEqual(
                set(checkpoint_document),
                {
                    "schema",
                    "unitId",
                    "idempotencyKey",
                    "nextChunkIndex",
                    "translatedChunks",
                },
            )
            self.assertNotIn(credential, checkpoint_text)
            self.assertNotIn(source_text, checkpoint_text)

            resumed_provider = RecordingFakeProvider()
            resumed_report = run_manifest(
                resumed_manifest,
                provider_factory=lambda profile_id, *, config_id: resumed_provider,
            )

            baseline_output = (
                baseline_root / "chapters" / "translated" / "chapter_001.md"
            ).read_text(encoding="utf-8")
            resumed_output = (
                resumed_root / "chapters" / "translated" / "chapter_001.md"
            ).read_text(encoding="utf-8")
            self.assertEqual(baseline_report["units"][0]["status"], "completed")
            self.assertEqual(resumed_report["units"][0]["status"], "completed")
            self.assertEqual(
                resumed_report["units"][0]["metrics"]["resumedChunkCount"],
                1,
            )
            self.assertEqual(resumed_output, baseline_output)
            self.assertIn("# CONTEXT\nONE", resumed_provider.requests[0].system_instruction)
            self.assertFalse(checkpoint_path.exists())

    def test_changed_idempotency_key_invalidates_private_resume_cache(self) -> None:
        with tempfile.TemporaryDirectory() as temporary_directory:
            store = CheckpointStore(Path(temporary_directory))
            original_key = UnitIdempotencyKey(
                task_manifest_sha256="a" * 64,
                provider_profile_id="fake-provider-profile",
                provider_config_id="fake-config-v1",
                translation_policy_version="translation-policy-v1",
            )
            checkpoint = UnitCheckpoint(
                next_chunk_index=1,
                translated_chunks=("translated private chunk",),
            )
            store.save("chapter_001", original_key, checkpoint)

            self.assertEqual(store.load("chapter_001", original_key), checkpoint)

            changed_key = UnitIdempotencyKey(
                task_manifest_sha256="b" * 64,
                provider_profile_id="fake-provider-profile",
                provider_config_id="fake-config-v1",
                translation_policy_version="translation-policy-v1",
            )
            self.assertIsNone(store.load("chapter_001", changed_key))
            self.assertFalse(store.path_for("chapter_001").exists())

    def test_interrupted_unit_resumes_then_deletes_checkpoint_after_output(self) -> None:
        with tempfile.TemporaryDirectory() as temporary_directory:
            project_root = Path(temporary_directory)
            manifest_path = build_run_fixture(
                project_root,
                source_text="one two three four five six.\n",
                max_tokens=4,
            )
            flaky = FailAfterOneChunkProvider()

            interrupted = run_manifest(
                manifest_path,
                provider_factory=lambda profile_id, *, config_id: flaky,
            )

            checkpoint_path = (
                project_root
                / "chapters"
                / "translated"
                / ".partial"
                / "chapter_001.json"
            )
            self.assertEqual(interrupted["units"][0]["status"], "failed")
            self.assertEqual(
                interrupted["units"][0]["error"]["code"], "translation_incomplete"
            )
            self.assertTrue(checkpoint_path.is_file())
            final_output_path = (
                project_root / "chapters" / "translated" / "chapter_001.md"
            )
            degraded_output_path = (
                project_root
                / "chapters"
                / "translated"
                / ".partial"
                / "chapter_001.degraded.md"
            )
            self.assertFalse(final_output_path.exists())
            degraded_output = degraded_output_path.read_text(encoding="utf-8")
            self.assertEqual(degraded_output, "ONE two three four five six.\n")

            resumed = run_manifest(manifest_path)

            self.assertEqual(resumed["units"][0]["status"], "completed")
            self.assertEqual(resumed["units"][0]["metrics"]["resumedChunkCount"], 1)
            self.assertFalse(checkpoint_path.exists())
            self.assertFalse(degraded_output_path.exists())
            output = final_output_path.read_text(encoding="utf-8")
            self.assertEqual(output, "ONE TWO THREE FOUR FIVE SIX.\n")

if __name__ == "__main__":
    unittest.main()
