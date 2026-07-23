import json
from pathlib import Path
import tempfile
import unittest

from translation_engine.engine import run_manifest
from translation_engine.providers import ProviderUnavailableError, TranslationRequest
from tests.fixtures import build_run_fixture


class InterruptingReflectionProvider:
    profile_id = "fake-provider-profile"
    config_id = "fake-config-no-secrets"

    def __init__(self, *, label: str, fail_on_reflection: int | None = None) -> None:
        self.label = label
        self.fail_on_reflection = fail_on_reflection
        self.draft_requests = 0
        self.reflection_requests = 0
        self.improve_requests = 0

    def translate(self, request: TranslationRequest) -> str:
        if "Output only the suggestions" in request.system_instruction:
            self.reflection_requests += 1
            if self.reflection_requests == self.fail_on_reflection:
                raise ProviderUnavailableError("simulated reflection interruption")
            return f"{self.label}-critique-{self.reflection_requests}"
        if "translation editing" in request.system_instruction:
            self.improve_requests += 1
            return json.loads(request.text)["draft"]
        self.draft_requests += 1
        return request.text.upper()


class ReflectionCheckpointTests(unittest.TestCase):
    def test_interrupted_second_pass_resumes_with_a_pass_specific_checkpoint(self) -> None:
        with tempfile.TemporaryDirectory() as temporary_directory:
            project_root = Path(temporary_directory)
            manifest_path = build_run_fixture(
                project_root,
                source_text="aa\nbb\ncc\n",
                max_tokens=3,
                second_pass_enabled=True,
            )
            interrupted_provider = InterruptingReflectionProvider(
                label="old", fail_on_reflection=2
            )

            interrupted = run_manifest(
                manifest_path,
                provider_factory=lambda profile_id, *, config_id: interrupted_provider,
            )

            first_pass_path = (
                project_root
                / "chapters"
                / "translated"
                / ".partial"
                / "chapter_001.json"
            )
            second_pass_path = first_pass_path.parent / "reflection" / "chapter_001.json"
            self.assertEqual(interrupted["units"][0]["status"], "failed")
            self.assertTrue(first_pass_path.is_file())
            self.assertTrue(second_pass_path.is_file())
            first_checkpoint = json.loads(first_pass_path.read_text(encoding="utf-8"))
            second_checkpoint = json.loads(second_pass_path.read_text(encoding="utf-8"))
            self.assertNotEqual(
                first_checkpoint["idempotencyKey"]["passId"],
                second_checkpoint["idempotencyKey"]["passId"],
            )
            self.assertEqual(second_checkpoint["nextChunkIndex"], 1)

            resumed_provider = InterruptingReflectionProvider(label="new")
            resumed = run_manifest(
                manifest_path,
                provider_factory=lambda profile_id, *, config_id: resumed_provider,
            )

            unit = resumed["units"][0]
            self.assertEqual(unit["status"], "completed")
            self.assertEqual(unit["metrics"]["resumedChunkCount"], 3)
            self.assertEqual(unit["metrics"]["secondPassResumedChunkCount"], 1)
            self.assertEqual(resumed_provider.draft_requests, 0)
            self.assertEqual(resumed_provider.reflection_requests, 2)
            self.assertEqual(resumed_provider.improve_requests, 2)
            evidence_path = (
                project_root / unit["secondPassArtifacts"]["reflection"]["path"]
            )
            evidence = evidence_path.read_text(encoding="utf-8")
            self.assertIn("old-critique-1", evidence)
            self.assertIn("new-critique-1", evidence)
            self.assertFalse(first_pass_path.exists())
            self.assertFalse(second_pass_path.exists())


if __name__ == "__main__":
    unittest.main()
