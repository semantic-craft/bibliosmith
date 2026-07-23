import json
from pathlib import Path
import tempfile
import unittest

from translation_engine.engine import run_manifest
from translation_engine.providers import (
    FatalError,
    RateLimitError,
    TranslationRequest,
)
from tests.fixtures import build_run_fixture


class FatalProvider:
    profile_id = "fatal-provider"
    config_id = "fatal-config"

    def translate(self, request: TranslationRequest) -> str:
        raise FatalError("fake invalid request")


class RateLimitedProvider:
    profile_id = "rate-limited-provider"
    config_id = "rate-limited-config"

    def __init__(self) -> None:
        self.calls = 0

    def translate(self, request: TranslationRequest) -> str:
        self.calls += 1
        raise RateLimitError(retry_after_seconds=600.0)


class ProviderErrorReportingTests(unittest.TestCase):
    def test_fatal_provider_error_fails_the_unit_without_marking_it_retryable(
        self,
    ) -> None:
        with tempfile.TemporaryDirectory() as temporary_directory:
            manifest_path = build_run_fixture(
                Path(temporary_directory), source_text="chapter\n", max_tokens=20
            )

            report = run_manifest(
                manifest_path,
                provider_factory=lambda profile_id, *, config_id: FatalProvider(),
            )

        self.assertEqual(report["summary"], {"total": 1, "completed": 0, "failed": 1})
        self.assertEqual(report["units"][0]["error"]["code"], "provider_fatal_error")
        self.assertFalse(report["units"][0]["error"]["retryable"])

    def test_rate_limit_stops_remaining_units_as_retryable_failures(self) -> None:
        provider = RateLimitedProvider()
        with tempfile.TemporaryDirectory() as temporary_directory:
            manifest_path = build_run_fixture(
                Path(temporary_directory), source_text="chapter\n", max_tokens=20
            )
            manifest = json.loads(manifest_path.read_text(encoding="utf-8"))
            manifest["units"] = manifest["units"] * 2
            manifest_path.write_text(
                json.dumps(manifest, indent=2) + "\n", encoding="utf-8"
            )

            report = run_manifest(
                manifest_path,
                provider_factory=lambda profile_id, *, config_id: provider,
            )

        self.assertEqual(report["summary"], {"total": 2, "completed": 0, "failed": 2})
        for unit in report["units"]:
            self.assertEqual(unit["error"]["code"], "provider_rate_limited")
            self.assertTrue(unit["error"]["retryable"])
        self.assertEqual(provider.calls, 1)


if __name__ == "__main__":
    unittest.main()
