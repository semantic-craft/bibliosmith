import json
import hashlib
import os
from pathlib import Path
import tempfile
import unittest
from unittest import mock

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
from tests.fixtures import build_multi_unit_run_fixture, build_run_fixture


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


class ProgressCapturingProvider(RecordingFakeProvider):
    concurrency_limit = 1

    def __init__(self, progress_path: Path) -> None:
        super().__init__()
        self.progress_path = progress_path
        self.first_request_progress: dict[str, object] | None = None

    def translate(self, request: TranslationRequest) -> str:
        if self.first_request_progress is None:
            self.first_request_progress = json.loads(
                self.progress_path.read_text(encoding="utf-8")
            )
        return super().translate(request)


class CheckpointStoreTests(unittest.TestCase):
    def test_queued_resume_checkpoints_are_counted_before_dispatch(self) -> None:
        with tempfile.TemporaryDirectory() as temporary_directory:
            project_root = Path(temporary_directory)
            manifest_path = build_multi_unit_run_fixture(
                project_root,
                source_texts=["aa bb", "cc dd"],
                max_tokens=3,
            )
            second_task = project_root / "qa" / "tasks" / "chapter_002.json"
            checkpoint_key = UnitIdempotencyKey(
                task_manifest_sha256=hashlib.sha256(second_task.read_bytes()).hexdigest(),
                provider_profile_id="fake-provider-profile",
                provider_config_id="fake-config-no-secrets",
                translation_policy_version="translation-policy-v1",
            )
            CheckpointStore(
                project_root / "chapters" / "translated" / ".partial"
            ).save(
                "chapter_002",
                checkpoint_key,
                UnitCheckpoint(next_chunk_index=1, translated_chunks=("CC ",)),
            )
            progress_path = project_root / ".book-pipeline-progress"
            provider = ProgressCapturingProvider(progress_path)

            with mock.patch.dict(
                os.environ,
                {"BIBLIOSMITH_PROGRESS_PATH": str(progress_path)},
            ):
                report = run_manifest(
                    manifest_path,
                    provider_factory=lambda profile_id, *, config_id: provider,
                )

            self.assertEqual(report["summary"]["completed"], 2)
            self.assertIsNotNone(provider.first_request_progress)
            self.assertEqual(provider.first_request_progress["completed"], 1)

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
                interrupted["units"][0]["error"],
                {"code": "provider_unavailable", "retryable": True},
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
            self.assertFalse(degraded_output_path.exists())

            resumed = run_manifest(manifest_path)

            self.assertEqual(resumed["units"][0]["status"], "completed")
            self.assertEqual(resumed["units"][0]["metrics"]["resumedChunkCount"], 1)
            self.assertFalse(checkpoint_path.exists())
            self.assertFalse(degraded_output_path.exists())
            output = final_output_path.read_text(encoding="utf-8")
            self.assertEqual(output, "ONE TWO THREE FOUR FIVE SIX.\n")

if __name__ == "__main__":
    unittest.main()
