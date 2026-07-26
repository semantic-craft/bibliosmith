"""Units overlap; chunks inside a unit do not.

Every provider double here is written so a serialized engine fails rather than
quietly passes: the barriers only release once the expected number of units is
inside `translate` at the same time, and they time out instead of hanging.
"""

import json
from pathlib import Path
import tempfile
import threading
import unittest

from translation_engine.engine import run_manifest
from translation_engine.providers import (
    FakeProvider,
    ProviderUnavailableError,
    RateLimitError,
    TranslationRequest,
)
from tests.fixtures import build_multi_unit_run_fixture


# Long enough that a loaded machine does not trip it, short enough that a serial
# regression reports a failure instead of stalling the suite.
BARRIER_TIMEOUT_SECONDS = 5.0

# Each unit's first chunk carries its marker, so a request can be traced back to
# the unit that issued it. Same byte length, so every unit splits identically.
MARKERS = ("alpha", "bravo", "delta", "gamma")
# Two chunks at maxTokens=16: "<marker> one two " then "three four.\n".
TWO_CHUNK_TOKENS = 16
ONE_CHUNK_TOKENS = 200


def source_text(marker: str) -> str:
    return f"{marker} one two three four.\n"


def request_marker(request: TranslationRequest) -> str:
    return request.text.split()[0]


class ConcurrencyProbeProvider:
    """Records the true parallelism the engine reached, and insists on it.

    The barrier is the positive half of the proof: it releases only when
    `concurrency_limit` units are simultaneously inside `translate`. The in-flight
    counter is the negative half, catching an engine that overlaps more units than
    the provider allows.
    """

    profile_id = "fake-provider-profile"
    config_id = "fake-config-no-secrets"

    def __init__(self, *, concurrency_limit: int) -> None:
        self.concurrency_limit = concurrency_limit
        self.max_in_flight = 0
        self.requests: list[TranslationRequest] = []
        self._fake = FakeProvider(config_id=self.config_id)
        self._lock = threading.Lock()
        self._in_flight = 0
        self._barrier = threading.Barrier(concurrency_limit)
        self._barrier_tripped = False

    def translate(self, request: TranslationRequest) -> str:
        with self._lock:
            self._in_flight += 1
            self.max_in_flight = max(self.max_in_flight, self._in_flight)
            self.requests.append(request)
            wait_for_peers = not self._barrier_tripped
        try:
            if wait_for_peers:
                try:
                    self._barrier.wait(timeout=BARRIER_TIMEOUT_SECONDS)
                except threading.BrokenBarrierError:
                    pass
                with self._lock:
                    self._barrier_tripped = True
            return self._fake.translate(request)
        finally:
            with self._lock:
                self._in_flight -= 1


class HeldUnitProvider:
    """Holds one unit until every other unit has finished, inverting the order."""

    profile_id = "fake-provider-profile"
    config_id = "fake-config-no-secrets"
    concurrency_limit = 3

    def __init__(self, *, held_marker: str, release_after: int) -> None:
        self.completion_order: list[str] = []
        self._held_marker = held_marker
        self._release_after = release_after
        self._fake = FakeProvider(config_id=self.config_id)
        self._lock = threading.Lock()
        self._released = threading.Event()

    def translate(self, request: TranslationRequest) -> str:
        marker = request_marker(request)
        if marker == self._held_marker:
            self._released.wait(timeout=BARRIER_TIMEOUT_SECONDS)
        translated = self._fake.translate(request)
        with self._lock:
            self.completion_order.append(marker)
            if len(self.completion_order) >= self._release_after:
                self._released.set()
        return translated


class ThrottleWithPeerInFlightProvider:
    """Throttles one unit only once a second unit is demonstrably mid-request."""

    profile_id = "fake-provider-profile"
    config_id = "fake-config-no-secrets"
    concurrency_limit = 2

    def __init__(self, *, throttled_marker: str) -> None:
        self.markers: list[str] = []
        self._throttled_marker = throttled_marker
        self._fake = FakeProvider(config_id=self.config_id)
        self._lock = threading.Lock()
        self._both_started = threading.Barrier(2)

    def translate(self, request: TranslationRequest) -> str:
        marker = request_marker(request)
        with self._lock:
            self.markers.append(marker)
        try:
            self._both_started.wait(timeout=BARRIER_TIMEOUT_SECONDS)
        except threading.BrokenBarrierError:
            pass
        if marker == self._throttled_marker:
            raise RateLimitError(retry_after_seconds=600.0)
        return self._fake.translate(request)


class FailChunksContainingProvider:
    """Fails every chunk carrying `needle`, whichever unit it belongs to."""

    profile_id = "fake-provider-profile"
    config_id = "fake-config-no-secrets"
    concurrency_limit = 2

    def __init__(self, *, needle: str) -> None:
        self._needle = needle
        self._fake = FakeProvider(config_id=self.config_id)

    def translate(self, request: TranslationRequest) -> str:
        if self._needle in request.text:
            raise ProviderUnavailableError("simulated interruption")
        return self._fake.translate(request)


class UnitConcurrencyTests(unittest.TestCase):
    def test_units_overlap_up_to_the_provider_limit_with_chunks_still_serial(
        self,
    ) -> None:
        provider = ConcurrencyProbeProvider(concurrency_limit=3)
        with tempfile.TemporaryDirectory() as temporary_directory:
            manifest_path = build_multi_unit_run_fixture(
                Path(temporary_directory),
                source_texts=[source_text(marker) for marker in MARKERS],
                max_tokens=TWO_CHUNK_TOKENS,
            )

            report = run_manifest(
                manifest_path,
                provider_factory=lambda profile_id, *, config_id: provider,
            )

        self.assertEqual(
            report["summary"], {"total": 4, "completed": 4, "failed": 0}
        )
        # Exactly the declared limit: the barrier proves three units were inside
        # translate together, the counter proves a fourth never joined them.
        self.assertEqual(provider.max_in_flight, 3)
        self.assertEqual(len(provider.requests), len(MARKERS) * 2)
        for marker in MARKERS:
            self.assertEqual(
                sum(
                    f"# CONTEXT\n{marker.upper()}" in request.system_instruction
                    for request in provider.requests
                ),
                1,
                f"{marker} lost its within-unit chunk chain",
            )
            self.assertEqual(
                sum(
                    request_marker(request) == marker
                    for request in provider.requests
                ),
                1,
            )

    def test_report_order_follows_the_manifest_not_the_finishing_order(self) -> None:
        provider = HeldUnitProvider(held_marker=MARKERS[0], release_after=3)
        with tempfile.TemporaryDirectory() as temporary_directory:
            manifest_path = build_multi_unit_run_fixture(
                Path(temporary_directory),
                source_texts=[source_text(marker) for marker in MARKERS],
                max_tokens=ONE_CHUNK_TOKENS,
            )

            report = run_manifest(
                manifest_path,
                provider_factory=lambda profile_id, *, config_id: provider,
            )

        # The first unit finished last, and still leads the report.
        self.assertEqual(provider.completion_order[-1], MARKERS[0])
        self.assertEqual(set(provider.completion_order[:3]), set(MARKERS[1:]))
        self.assertEqual(
            [unit["unitId"] for unit in report["units"]],
            [f"chapter_{index:03d}" for index in range(1, len(MARKERS) + 1)],
        )
        self.assertEqual(
            report["summary"], {"total": 4, "completed": 4, "failed": 0}
        )

    def test_rate_limit_stops_dispatch_without_abandoning_a_unit_in_flight(
        self,
    ) -> None:
        """The concurrent replacement for "fail every remaining unit and stop".

        A throttle now latches dispatch: units that have not begun are failed
        retryable without a request, and a unit already mid-flight is left to
        report its own real outcome rather than being written off for a limit it
        may never hit.
        """
        provider = ThrottleWithPeerInFlightProvider(throttled_marker=MARKERS[0])
        with tempfile.TemporaryDirectory() as temporary_directory:
            project_root = Path(temporary_directory)
            manifest_path = build_multi_unit_run_fixture(
                project_root,
                source_texts=[source_text(marker) for marker in MARKERS],
                max_tokens=ONE_CHUNK_TOKENS,
            )

            report = run_manifest(
                manifest_path,
                provider_factory=lambda profile_id, *, config_id: provider,
            )

            translated = project_root / "chapters" / "translated"
            self.assertTrue((translated / "chapter_002.md").is_file())
            self.assertFalse((translated / "chapter_001.md").exists())

        self.assertEqual(
            report["summary"], {"total": 4, "completed": 1, "failed": 3}
        )
        self.assertEqual(
            [unit["unitId"] for unit in report["units"]],
            [f"chapter_{index:03d}" for index in range(1, len(MARKERS) + 1)],
        )
        self.assertEqual(report["units"][1]["status"], "completed")
        for position in (0, 2, 3):
            unit = report["units"][position]
            self.assertEqual(unit["status"], "failed")
            self.assertEqual(unit["error"]["code"], "provider_rate_limited")
            self.assertTrue(unit["error"]["retryable"])
        # Only the two units that had already started ever reached the provider.
        self.assertEqual(sorted(provider.markers), sorted(MARKERS[:2]))

    def test_each_unit_resumes_from_its_own_checkpoint(self) -> None:
        markers = MARKERS[:2]
        with tempfile.TemporaryDirectory() as temporary_directory:
            project_root = Path(temporary_directory)
            manifest_path = build_multi_unit_run_fixture(
                project_root,
                source_texts=[source_text(marker) for marker in markers],
                max_tokens=TWO_CHUNK_TOKENS,
            )

            interrupted = run_manifest(
                manifest_path,
                provider_factory=lambda profile_id, *, config_id: (
                    FailChunksContainingProvider(needle="three")
                ),
            )

            partial = project_root / "chapters" / "translated" / ".partial"
            for index, marker in enumerate(markers, start=1):
                self.assertEqual(interrupted["units"][index - 1]["status"], "failed")
                checkpoint = json.loads(
                    (partial / f"chapter_{index:03d}.json").read_text(encoding="utf-8")
                )
                self.assertEqual(checkpoint["nextChunkIndex"], 1)
                self.assertEqual(
                    checkpoint["translatedChunks"],
                    [f"{marker.upper()} ONE TWO "],
                )

            resume_provider = ConcurrencyProbeProvider(concurrency_limit=2)
            resumed = run_manifest(
                manifest_path,
                provider_factory=lambda profile_id, *, config_id: resume_provider,
            )

            for index, marker in enumerate(markers, start=1):
                unit = resumed["units"][index - 1]
                self.assertEqual(unit["status"], "completed")
                self.assertEqual(unit["metrics"]["resumedChunkCount"], 1)
                self.assertFalse((partial / f"chapter_{index:03d}.json").exists())
                self.assertEqual(
                    (
                        project_root
                        / "chapters"
                        / "translated"
                        / f"chapter_{index:03d}.md"
                    ).read_text(encoding="utf-8"),
                    f"{marker.upper()} ONE TWO THREE FOUR.\n",
                )

        # Both units resumed at once, each from its own checkpoint prefix.
        self.assertEqual(resume_provider.max_in_flight, 2)
        self.assertEqual(len(resume_provider.requests), len(markers))


if __name__ == "__main__":
    unittest.main()
