import json
from pathlib import Path
import tempfile
import unittest

from translation_engine.engine import EngineError, run_manifest
from translation_engine.providers import TranslationRequest
from tests.fixtures import build_run_fixture


class ModelRecordingProvider:
    """Records the model in effect at translate time, which is what the override
    changes: the engine sets provider.model before any request goes out."""

    profile_id = "fake-provider-profile"
    config_id = "fake-config-no-secrets"
    model = "registry-default-model"

    def __init__(self) -> None:
        self.models: list[str] = []

    def translate(self, request: TranslationRequest) -> str:
        self.models.append(self.model)
        return request.text


def _with_model(manifest_path: Path, model: object) -> None:
    manifest = json.loads(manifest_path.read_text(encoding="utf-8"))
    manifest["model"] = model
    manifest_path.write_text(json.dumps(manifest, indent=2) + "\n", encoding="utf-8")


class ModelOverrideTests(unittest.TestCase):
    def test_a_manifest_model_replaces_the_registry_default(self) -> None:
        with tempfile.TemporaryDirectory() as temporary_directory:
            project_root = Path(temporary_directory)
            manifest_path = build_run_fixture(
                project_root, source_text="One sentence.\n", max_tokens=100
            )
            _with_model(manifest_path, "qwen3.6-flash")
            provider = ModelRecordingProvider()

            report = run_manifest(
                manifest_path,
                provider_factory=lambda profile_id, *, config_id: provider,
            )

            self.assertEqual(report["units"][0]["status"], "completed")
            self.assertTrue(provider.models)
            self.assertTrue(all(model == "qwen3.6-flash" for model in provider.models))

    def test_no_model_keeps_the_registry_default_untouched(self) -> None:
        with tempfile.TemporaryDirectory() as temporary_directory:
            project_root = Path(temporary_directory)
            manifest_path = build_run_fixture(
                project_root, source_text="One sentence.\n", max_tokens=100
            )
            provider = ModelRecordingProvider()

            run_manifest(
                manifest_path,
                provider_factory=lambda profile_id, *, config_id: provider,
            )

            self.assertTrue(all(m == "registry-default-model" for m in provider.models))

    def test_an_empty_or_non_string_model_is_rejected(self) -> None:
        for bad in ["", "   ", 7, {"name": "x"}]:
            with tempfile.TemporaryDirectory() as temporary_directory:
                project_root = Path(temporary_directory)
                manifest_path = build_run_fixture(
                    project_root, source_text="One sentence.\n", max_tokens=100
                )
                _with_model(manifest_path, bad)

                with self.assertRaisesRegex(EngineError, "invalid_model_override"):
                    run_manifest(
                        manifest_path,
                        provider_factory=lambda profile_id, *, config_id: (
                            ModelRecordingProvider()
                        ),
                    )


if __name__ == "__main__":
    unittest.main()
