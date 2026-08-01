"""Offline coverage for the dual-engine OCR sampling command.

Both engines are hosted APIs, so every test here injects fake runners instead of
calling them. What is under test is everything around the network: which pages
get sampled, that both engines see the same extracted PDF, the report shape, and
that one engine failing still yields a usable comparison.
"""

from __future__ import annotations

import importlib.util
import json
import os
from pathlib import Path
import sys
import unittest
from unittest import mock

from pypdf import PdfReader, PdfWriter


PACKAGE_ROOT = Path(__file__).resolve().parents[1]
# sample_compare imports its sibling engine clients by bare name, exactly as it
# does when the launcher runs it as a script from the package root.
sys.path.insert(0, str(PACKAGE_ROOT))


def load_sample_compare():  # type: ignore[no-untyped-def]
    module_name = "ocr_sample_compare_test"
    spec = importlib.util.spec_from_file_location(
        module_name, PACKAGE_ROOT / "sample_compare.py"
    )
    if spec is None or spec.loader is None:
        raise RuntimeError("Cannot import the OCR sample compare module")
    module = importlib.util.module_from_spec(spec)
    sys.modules[module_name] = module
    spec.loader.exec_module(module)
    return module


sample_compare = load_sample_compare()


def write_pdf(path: Path, pages: int) -> Path:
    """A PDF whose pages are told apart by size.

    Identical blank pages would make the extraction step untestable: a
    1-based/0-based slip would send the wrong pages to both paid APIs while the
    page *count* stayed right, so every assertion about counts would still pass
    and the user would compare engines on pages the report does not name.
    Width encodes the 1-based page number.
    """
    writer = PdfWriter()
    for number in range(1, pages + 1):
        writer.add_blank_page(width=100 + number, height=200)
    with path.open("wb") as handle:
        writer.write(handle)
    return path


def page_numbers_of(pdf: Path) -> list[int]:
    """Recover the original page numbers write_pdf encoded into each page."""
    return [int(round(float(page.mediabox.width))) - 100 for page in PdfReader(str(pdf)).pages]


class RecordingRunner:
    """A stand-in engine that records what it was handed."""

    def __init__(self, markdown: str, *, failure: Exception | None = None) -> None:
        self.markdown = markdown
        self.failure = failure
        self.calls: list[tuple[Path, Path]] = []
        self.received_pages: list[list[int]] = []

    def __call__(self, sample_pdf: Path, work_dir: Path):  # type: ignore[no-untyped-def]
        self.calls.append((sample_pdf, work_dir))
        # Read while the scratch PDF still exists; the run deletes it on the
        # way out, so a test that looks afterwards sees nothing.
        self.received_pages.append(page_numbers_of(sample_pdf))
        if self.failure is not None:
            raise self.failure
        return sample_compare.EngineOutcome(
            markdown=self.markdown, page_count=len(PdfReader(str(sample_pdf)).pages)
        )


class SelectInternalPagesTests(unittest.TestCase):
    def test_endpoints_are_never_sampled(self) -> None:
        for total in range(3, 40):
            pages = sample_compare.select_internal_pages(total, 3)
            self.assertTrue(pages, f"{total}-page PDF produced no sample")
            self.assertNotIn(1, pages)
            self.assertNotIn(total, pages)

    def test_pages_are_spread_and_ordered(self) -> None:
        self.assertEqual(sample_compare.select_internal_pages(200, 3), [51, 101, 150])
        self.assertEqual(sample_compare.select_internal_pages(11, 4), [3, 5, 7, 9])

    def test_short_books_fall_back_to_every_interior_page(self) -> None:
        self.assertEqual(sample_compare.select_internal_pages(5, 3), [2, 3, 4])
        self.assertEqual(sample_compare.select_internal_pages(3, 3), [2])

    def test_books_without_an_interior_select_nothing(self) -> None:
        self.assertEqual(sample_compare.select_internal_pages(2, 3), [])
        self.assertEqual(sample_compare.select_internal_pages(1, 3), [])

    def test_invalid_counts_are_rejected(self) -> None:
        with self.assertRaises(sample_compare.SampleCompareError):
            sample_compare.select_internal_pages(10, 0)
        with self.assertRaises(sample_compare.SampleCompareError):
            sample_compare.select_internal_pages(0, 3)


class ManifestFixture:
    def __init__(self, root: Path, *, pages: int = 40, **overrides: object) -> None:
        self.project_root = root / "project"
        self.project_root.mkdir(parents=True, exist_ok=True)
        self.source_pdf = write_pdf(root / "book.pdf", pages)
        self.report_path = self.project_root / "qa" / "ocr-sample" / "report.json"
        manifest = {
            "schema": sample_compare.MANIFEST_SCHEMA,
            "projectRoot": str(self.project_root),
            "sourcePdfPath": str(self.source_pdf),
            "reportPath": "qa/ocr-sample/report.json",
            "workDir": "qa/ocr-sample/work",
            "samplePages": 3,
            "characterBudget": 40,
            "engines": ["paddleocr", "mineru"],
        }
        manifest.update(overrides)
        self.path = root / "manifest.json"
        self.path.write_text(json.dumps(manifest), encoding="utf-8")


class RunSampleManifestTests(unittest.TestCase):
    def setUp(self) -> None:
        import tempfile

        self._tmp = tempfile.TemporaryDirectory(prefix="ocr-sample-compare-tests-")
        self.root = Path(self._tmp.name)
        self.addCleanup(self._tmp.cleanup)

    def test_both_engines_see_the_same_extracted_pages(self) -> None:
        fixture = ManifestFixture(self.root, pages=40)
        paddle_runner = RecordingRunner("# Paddle")
        mineru_runner = RecordingRunner("# MinerU")

        report = sample_compare.run_sample_manifest(
            fixture.path,
            engine_runners={"paddleocr": paddle_runner, "mineru": mineru_runner},
        )

        self.assertEqual(report["schema"], sample_compare.REPORT_SCHEMA)
        self.assertEqual(report["totalPages"], 40)
        self.assertEqual(report["sampledPages"], [11, 21, 30])
        self.assertEqual(len(paddle_runner.calls), 1)
        self.assertEqual(len(mineru_runner.calls), 1)
        # The same file, so neither engine is judged on different pages. The
        # page count is read through the report because the runners saw the
        # scratch PDF while it existed and this assertion runs after cleanup.
        self.assertEqual(paddle_runner.calls[0][0], mineru_runner.calls[0][0])
        self.assertEqual(
            [entry["pageCount"] for entry in report["engines"]], [3, 3]
        )
        # The bytes each engine received are the pages the report names. A page
        # count alone would not catch an extraction that is off by one, which
        # would bill both APIs for pages the comparison misattributes.
        self.assertEqual(paddle_runner.received_pages[0], report["sampledPages"])
        self.assertEqual(mineru_runner.received_pages[0], report["sampledPages"])
        # Separate scratch directories, so one engine cannot overwrite the other.
        self.assertNotEqual(paddle_runner.calls[0][1], mineru_runner.calls[0][1])

    def test_the_report_lands_on_disk_and_matches_the_return_value(self) -> None:
        fixture = ManifestFixture(self.root)
        report = sample_compare.run_sample_manifest(
            fixture.path,
            engine_runners={
                "paddleocr": RecordingRunner("# Paddle"),
                "mineru": RecordingRunner("# MinerU"),
            },
        )

        self.assertTrue(fixture.report_path.is_file())
        self.assertEqual(
            json.loads(fixture.report_path.read_text(encoding="utf-8")), report
        )
        engines = {entry["engine"]: entry for entry in report["engines"]}
        self.assertEqual(sorted(engines), ["mineru", "paddleocr"])
        self.assertEqual(engines["paddleocr"]["status"], "ok")
        self.assertEqual(engines["paddleocr"]["markdownExcerpt"], "# Paddle")
        self.assertEqual(engines["paddleocr"]["characterCount"], len("# Paddle"))
        self.assertEqual(engines["paddleocr"]["pageCount"], 3)
        self.assertIsNone(engines["paddleocr"]["error"])
        self.assertIsInstance(engines["paddleocr"]["elapsedMs"], int)

    def test_the_report_records_the_source_digest(self) -> None:
        fixture = ManifestFixture(self.root)
        report = sample_compare.run_sample_manifest(
            fixture.path,
            engine_runners={
                "paddleocr": RecordingRunner("a"),
                "mineru": RecordingRunner("b"),
            },
        )
        self.assertEqual(
            report["sourcePdfSha256"], sample_compare.sha256_file(fixture.source_pdf)
        )

    def test_excerpts_honour_the_character_budget(self) -> None:
        fixture = ManifestFixture(self.root, characterBudget=5)
        report = sample_compare.run_sample_manifest(
            fixture.path,
            engine_runners={
                "paddleocr": RecordingRunner("0123456789"),
                "mineru": RecordingRunner("abc"),
            },
        )
        engines = {entry["engine"]: entry for entry in report["engines"]}
        self.assertEqual(engines["paddleocr"]["markdownExcerpt"], "01234")
        # The full length is still reported, so a truncated panel does not read
        # as a short extraction.
        self.assertEqual(engines["paddleocr"]["characterCount"], 10)
        self.assertEqual(engines["mineru"]["markdownExcerpt"], "abc")

    def test_the_budget_counts_characters_not_bytes(self) -> None:
        # Chinese scans are this project's main workload: 4000 characters is
        # 12000 bytes, so a byte-based budget on either side would truncate
        # every real sample to a third and the launcher would reject it for
        # exceeding its own budget.
        chinese = "第一章 绪论。" * 20
        fixture = ManifestFixture(self.root, characterBudget=10)
        report = sample_compare.run_sample_manifest(
            fixture.path,
            engine_runners={
                "paddleocr": RecordingRunner(chinese),
                "mineru": RecordingRunner("abc"),
            },
        )
        excerpt = report["engines"][0]["markdownExcerpt"]
        self.assertEqual(excerpt, chinese[:10])
        self.assertEqual(len(excerpt), 10)
        self.assertGreater(len(excerpt.encode("utf-8")), 10)
        self.assertEqual(report["engines"][0]["characterCount"], len(chinese))

    def test_one_failing_engine_still_produces_a_comparison(self) -> None:
        fixture = ManifestFixture(self.root)
        report = sample_compare.run_sample_manifest(
            fixture.path,
            engine_runners={
                "paddleocr": RecordingRunner("", failure=RuntimeError("token missing")),
                "mineru": RecordingRunner("# MinerU"),
            },
        )
        engines = {entry["engine"]: entry for entry in report["engines"]}
        self.assertEqual(engines["paddleocr"]["status"], "failed")
        self.assertIn("token missing", engines["paddleocr"]["error"])
        self.assertEqual(engines["paddleocr"]["markdownExcerpt"], "")
        self.assertEqual(engines["mineru"]["status"], "ok")
        self.assertTrue(fixture.report_path.is_file())

    def test_a_single_engine_may_be_requested(self) -> None:
        fixture = ManifestFixture(self.root, engines=["mineru"])
        report = sample_compare.run_sample_manifest(
            fixture.path, engine_runners={"mineru": RecordingRunner("# MinerU")}
        )
        self.assertEqual([entry["engine"] for entry in report["engines"]], ["mineru"])

    def test_a_book_without_an_interior_is_refused(self) -> None:
        fixture = ManifestFixture(self.root, pages=2)
        with self.assertRaises(sample_compare.SampleCompareError) as caught:
            sample_compare.run_sample_manifest(
                fixture.path, engine_runners={"paddleocr": RecordingRunner("x")}
            )
        self.assertIn("pdf_too_short_to_sample", str(caught.exception))
        self.assertFalse(fixture.report_path.exists())

    def test_manifest_validation_rejects_bad_input(self) -> None:
        cases = {
            "unsupported_manifest_schema": {"schema": "ocr-sample-compare-v0"},
            "invalid_samplePages": {"samplePages": 0},
            "invalid_engines": {"engines": []},
            "unsupported_engine:tesseract": {"engines": ["tesseract"]},
            "duplicate_engine:mineru": {"engines": ["mineru", "mineru"]},
            "missing_source_pdf": {"sourcePdfPath": str(self.root / "absent.pdf")},
        }
        for expected, override in cases.items():
            with self.subTest(expected=expected):
                root = self.root / expected.replace(":", "-")
                root.mkdir(parents=True, exist_ok=True)
                fixture = ManifestFixture(root, **override)
                with self.assertRaises(sample_compare.SampleCompareError) as caught:
                    sample_compare.run_sample_manifest(
                        fixture.path,
                        engine_runners={
                            "paddleocr": RecordingRunner("x"),
                            "mineru": RecordingRunner("y"),
                        },
                    )
                self.assertIn(expected, str(caught.exception))

    def test_sample_page_count_is_capped(self) -> None:
        fixture = ManifestFixture(
            self.root, samplePages=sample_compare.MAX_SAMPLE_PAGES + 1
        )
        with self.assertRaises(sample_compare.SampleCompareError) as caught:
            sample_compare.run_sample_manifest(
                fixture.path, engine_runners={"paddleocr": RecordingRunner("x")}
            )
        self.assertIn("invalid_samplePages", str(caught.exception))

    def test_report_paths_cannot_escape_the_project_root(self) -> None:
        fixture = ManifestFixture(self.root, reportPath="../escaped/report.json")
        with self.assertRaises(sample_compare.SampleCompareError) as caught:
            sample_compare.run_sample_manifest(
                fixture.path, engine_runners={"paddleocr": RecordingRunner("x")}
            )
        self.assertIn("path_outside_project_root", str(caught.exception))

    def test_scratch_pdfs_do_not_outlive_the_run(self) -> None:
        fixture = ManifestFixture(self.root)
        runner = RecordingRunner("# Paddle")
        sample_compare.run_sample_manifest(
            fixture.path,
            engine_runners={"paddleocr": runner, "mineru": RecordingRunner("# MinerU")},
        )
        self.assertFalse(runner.calls[0][0].exists())


class EngineErrorRedactionTests(unittest.TestCase):
    """The error text lands in a file, so it gets the log-line treatment."""

    def test_a_signed_result_url_loses_its_query_string(self) -> None:
        redacted = sample_compare.redact_engine_error(
            "HTTPError: 403 for url: https://paddle.example/result.jsonl"
            "?X-Amz-Signature=deadbeef&X-Amz-Expires=900"
        )
        self.assertNotIn("deadbeef", redacted)
        self.assertNotIn("X-Amz-Signature", redacted)
        # The host and path survive, so the failure is still diagnosable.
        self.assertIn("https://paddle.example/result.jsonl", redacted)

    def test_an_echoed_auth_header_is_redacted(self) -> None:
        redacted = sample_compare.redact_engine_error(
            "PaddleOCRError: submit failed: Authorization: bearer sk-live-abc123"
        )
        self.assertNotIn("sk-live-abc123", redacted)

    def test_a_token_assignment_is_redacted(self) -> None:
        redacted = sample_compare.redact_engine_error("MINERU_API_TOKEN=tok-abc123")
        self.assertNotIn("tok-abc123", redacted)

    def test_a_missing_key_message_stays_legible(self) -> None:
        # Over-redaction here would hide which credential to go configure.
        message = "SampleCompareError: MINERU_API_TOKEN is not configured"
        self.assertEqual(sample_compare.redact_engine_error(message), message)

    def test_a_plain_url_without_a_query_is_untouched(self) -> None:
        message = "ConnectionError: https://mineru.net/api/v4/extract/task unreachable"
        self.assertEqual(sample_compare.redact_engine_error(message), message)

    def test_the_report_carries_the_redacted_text(self) -> None:
        import tempfile

        with tempfile.TemporaryDirectory(prefix="ocr-sample-redact-") as tmp:
            fixture = ManifestFixture(Path(tmp))
            report = sample_compare.run_sample_manifest(
                fixture.path,
                engine_runners={
                    "paddleocr": RecordingRunner(
                        "",
                        failure=RuntimeError(
                            "403 https://paddle.example/r.jsonl?sig=deadbeef"
                        ),
                    ),
                    "mineru": RecordingRunner("# MinerU"),
                },
            )
        self.assertNotIn("deadbeef", json.dumps(report))


class ReportContractTests(unittest.TestCase):
    """Pin the field set the launcher deserializes.

    BookPipelineOcrSampleReport in book_pipeline.rs is `deny_unknown_fields`,
    so an added key here fails the launcher at run time rather than at compile
    time. These two assertions are the cheap half of that contract; the other
    half is a_real_python_sample_report_deserializes_and_validates on the Rust
    side, which parses this writer's actual output.
    """

    def setUp(self) -> None:
        import tempfile

        self._tmp = tempfile.TemporaryDirectory(prefix="ocr-sample-contract-")
        self.addCleanup(self._tmp.cleanup)
        fixture = ManifestFixture(Path(self._tmp.name))
        # One engine succeeds and one fails, because _run_one_engine builds two
        # independent dict literals and only exercising the success branch
        # would let the failure branch drift away from the Rust struct.
        self.report = sample_compare.run_sample_manifest(
            fixture.path,
            engine_runners={
                "paddleocr": RecordingRunner("# Paddle"),
                "mineru": RecordingRunner("", failure=RuntimeError("no token")),
            },
        )

    def test_top_level_fields_match_the_rust_struct(self) -> None:
        self.assertEqual(
            sorted(self.report),
            [
                "characterBudget",
                "engines",
                "sampledPages",
                "schema",
                "sourcePdfSha256",
                "totalPages",
            ],
        )

    def test_top_level_types_match_the_rust_struct(self) -> None:
        # Rust types: String, String, u32, Vec<u32>, usize, Vec<_>. A float or
        # a stringified number here deserializes nowhere.
        self.assertIsInstance(self.report["schema"], str)
        self.assertIsInstance(self.report["sourcePdfSha256"], str)
        self.assertIsInstance(self.report["totalPages"], int)
        self.assertNotIsInstance(self.report["totalPages"], bool)
        self.assertTrue(all(isinstance(page, int) for page in self.report["sampledPages"]))
        self.assertIsInstance(self.report["characterBudget"], int)

    def test_engine_fields_match_the_rust_struct(self) -> None:
        statuses = {entry["status"] for entry in self.report["engines"]}
        self.assertEqual(statuses, {"ok", "failed"}, "both branches must be covered")
        for entry in self.report["engines"]:
            self.assertEqual(
                sorted(entry),
                [
                    "characterCount",
                    "elapsedMs",
                    "engine",
                    "error",
                    "markdownExcerpt",
                    "pageCount",
                    "status",
                ],
            )

    def test_engine_types_match_the_rust_struct(self) -> None:
        for entry in self.report["engines"]:
            with self.subTest(status=entry["status"]):
                self.assertIsInstance(entry["engine"], str)
                self.assertIsInstance(entry["status"], str)
                self.assertIsInstance(entry["markdownExcerpt"], str)
                self.assertIsInstance(entry["characterCount"], int)
                # Rust reads elapsedMs as u64: a float would not deserialize.
                self.assertIsInstance(entry["elapsedMs"], int)
                self.assertNotIsInstance(entry["elapsedMs"], bool)
                self.assertGreaterEqual(entry["elapsedMs"], 0)
                # Option<u32> / Option<String>: None, never 0 or "".
                self.assertIn(
                    type(entry["pageCount"]), (int, type(None)), entry["pageCount"]
                )
                if entry["status"] == "ok":
                    self.assertIsNone(entry["error"])
                else:
                    self.assertIsInstance(entry["error"], str)
                    self.assertIsNone(entry["pageCount"])


class EngineRunnerWiringTests(unittest.TestCase):
    """Drive the real runners with the network primitives stubbed out.

    Both runners build an argparse namespace through the engine CLI's own
    parser and hand it to functions that read attributes off it. A missing or
    misnamed attribute raises nowhere until a real book is sampled against a
    live API, which is the one place it must not first be discovered.
    """

    def setUp(self) -> None:
        import tempfile

        self._tmp = tempfile.TemporaryDirectory(prefix="ocr-sample-runners-")
        self.root = Path(self._tmp.name)
        self.addCleanup(self._tmp.cleanup)
        self.sample_pdf = write_pdf(self.root / "sample.pdf", 3)
        self.work_dir = self.root / "work"

    def test_paddleocr_runner_drives_submit_poll_and_download(self) -> None:
        seen: dict[str, object] = {}

        def submit_job(args, headers, optional_payload):  # type: ignore[no-untyped-def]
            seen["input"] = args.input
            seen["model"] = args.model
            seen["job_url"] = args.job_url
            seen["timeout"] = args.timeout_seconds
            seen["headers"] = headers
            seen["optional_payload"] = optional_payload
            return "job-1"

        def poll_json_url(args, headers, job_id):  # type: ignore[no-untyped-def]
            seen["job_id"] = job_id
            seen["poll_seconds"] = args.poll_seconds
            seen["max_runtime"] = args.max_runtime_seconds
            return "https://example.invalid/result.jsonl"

        def download_jsonl(json_url, timeout_seconds):  # type: ignore[no-untyped-def]
            seen["json_url"] = json_url
            return json.dumps(
                {"result": {"layoutParsingResults": [{"markdown": {"text": "page body"}}]}}
            )

        with mock.patch.dict(os.environ, {"BAIDU_PADDLEOCR_TOKEN": "tok-123"}, clear=False), (
            mock.patch.object(sample_compare.paddle, "submit_job", submit_job)
        ), mock.patch.object(
            sample_compare.paddle, "poll_json_url", poll_json_url
        ), mock.patch.object(sample_compare.paddle, "download_jsonl", download_jsonl):
            outcome = sample_compare.run_paddleocr_sample(self.sample_pdf, self.work_dir)

        self.assertEqual(outcome.markdown, "page body")
        self.assertEqual(outcome.page_count, 1)
        self.assertEqual(seen["input"], str(self.sample_pdf))
        self.assertEqual(seen["model"], sample_compare.paddle.DEFAULT_MODEL)
        self.assertEqual(seen["job_url"], sample_compare.paddle.DEFAULT_JOB_URL)
        self.assertEqual(seen["headers"], {"Authorization": "bearer tok-123"})
        self.assertEqual(
            seen["optional_payload"], sample_compare.paddle.DEFAULT_OPTIONAL_PAYLOAD
        )
        self.assertEqual(seen["job_id"], "job-1")
        # The raw engine response is kept beside the report for diagnosis.
        self.assertTrue((self.work_dir / "paddleocr.jsonl").is_file())

    def test_paddleocr_runner_refuses_without_a_token(self) -> None:
        with mock.patch.dict(os.environ, {"BAIDU_PADDLEOCR_TOKEN": ""}, clear=False):
            with self.assertRaises(sample_compare.SampleCompareError) as caught:
                sample_compare.run_paddleocr_sample(self.sample_pdf, self.work_dir)
        self.assertIn("BAIDU_PADDLEOCR_TOKEN", str(caught.exception))

    def test_mineru_runner_drives_submit_poll_and_download(self) -> None:
        seen: dict[str, object] = {}

        def submit_local_batch(session, args, token, batch):  # type: ignore[no-untyped-def]
            seen["token"] = token
            seen["model_version"] = args.model_version
            seen["language"] = args.language
            seen["is_ocr"] = args.is_ocr
            seen["enable_table"] = args.enable_table
            seen["enable_formula"] = args.enable_formula
            seen["upload_timeout"] = args.upload_timeout_seconds
            seen["names"] = [item.name for item in batch]
            seen["local_paths"] = [item.local_path for item in batch]
            # The payload builders read the same namespace; calling them here
            # is what proves every attribute they need is present.
            seen["payload"] = mineru_module.common_payload(
                args, args.model_version, include_cache=False
            )
            seen["entry"] = mineru_module.file_entry(
                batch[0], args, include_url=False, model_version=args.model_version
            )
            return "batch-1"

        def poll_batch(session, args, token, batch_id, batch):  # type: ignore[no-untyped-def]
            seen["batch_id"] = batch_id
            seen["poll_seconds"] = args.poll_seconds
            seen["max_runtime"] = args.max_runtime_seconds
            return [{"data_id": batch[0].data_id, "state": "done"}]

        markdown_path = self.root / "part.md"
        markdown_path.write_text("# MinerU body", encoding="utf-8")

        def download_results(args, batch_id, results, batch):  # type: ignore[no-untyped-def]
            seen["output_dir"] = args.output_dir
            seen["no_download"] = args.no_download
            return [
                mineru_module.DownloadedPart(item=batch[0], markdown_path=markdown_path)
            ]

        mineru_module = sample_compare.mineru
        with mock.patch.dict(
            os.environ, {"MINERU_API_TOKEN": "mineru-tok"}, clear=False
        ), mock.patch.object(
            mineru_module, "submit_local_batch", submit_local_batch
        ), mock.patch.object(
            mineru_module, "poll_batch", poll_batch
        ), mock.patch.object(mineru_module, "download_results", download_results):
            outcome = sample_compare.run_mineru_sample(self.sample_pdf, self.work_dir)

        self.assertEqual(outcome.markdown, "# MinerU body")
        self.assertEqual(outcome.page_count, 3)
        self.assertEqual(seen["token"], "mineru-tok")
        # vlm is what the launcher's local-PDF MinerU route already runs.
        self.assertEqual(seen["model_version"], "vlm")
        self.assertEqual(seen["payload"]["model_version"], "vlm")
        self.assertEqual(seen["entry"]["name"], "sample.pdf")
        self.assertEqual(seen["batch_id"], "batch-1")
        self.assertEqual(seen["output_dir"], str(self.work_dir))
        self.assertFalse(seen["no_download"])

    def test_mineru_runner_accepts_either_token_spelling(self) -> None:
        for name in ("MINERU_API_TOKEN", "MINERU_TOKEN"):
            with self.subTest(name=name):
                env = {"MINERU_API_TOKEN": "", "MINERU_TOKEN": "", name: "tok"}
                with mock.patch.dict(os.environ, env, clear=False), mock.patch.object(
                    sample_compare.mineru,
                    "submit_local_batch",
                    lambda *_args, **_kwargs: (_ for _ in ()).throw(
                        RuntimeError("reached submit")
                    ),
                ):
                    with self.assertRaises(RuntimeError) as caught:
                        sample_compare.run_mineru_sample(self.sample_pdf, self.work_dir)
                self.assertIn("reached submit", str(caught.exception))

    def test_mineru_runner_refuses_without_a_token(self) -> None:
        env = {"MINERU_API_TOKEN": "", "MINERU_TOKEN": ""}
        with mock.patch.dict(os.environ, env, clear=False):
            with self.assertRaises(sample_compare.SampleCompareError) as caught:
                sample_compare.run_mineru_sample(self.sample_pdf, self.work_dir)
        self.assertIn("MINERU_API_TOKEN", str(caught.exception))

    def test_the_default_runner_table_names_both_engines(self) -> None:
        self.assertEqual(
            sorted(sample_compare.DEFAULT_ENGINE_RUNNERS), ["mineru", "paddleocr"]
        )
        self.assertIs(
            sample_compare.DEFAULT_ENGINE_RUNNERS["paddleocr"],
            sample_compare.run_paddleocr_sample,
        )
        self.assertIs(
            sample_compare.DEFAULT_ENGINE_RUNNERS["mineru"],
            sample_compare.run_mineru_sample,
        )


class PaddleMarkdownTests(unittest.TestCase):
    def test_per_page_markdown_is_concatenated(self) -> None:
        jsonl = "\n".join(
            json.dumps(payload)
            for payload in [
                {
                    "result": {
                        "layoutParsingResults": [
                            {"markdown": {"text": "page one"}},
                            {"markdown": {"text": "page two"}},
                        ]
                    }
                },
                {"result": {"layoutParsingResults": [{"markdown": {"text": "page three"}}]}},
            ]
        )
        markdown, pages = sample_compare.paddle_markdown_from_jsonl(jsonl)
        self.assertEqual(markdown, "page one\n\npage two\n\npage three")
        self.assertEqual(pages, 3)

    def test_blank_lines_and_missing_markdown_are_tolerated(self) -> None:
        jsonl = '\n{"result": {"layoutParsingResults": [{}]}}\n\n'
        markdown, pages = sample_compare.paddle_markdown_from_jsonl(jsonl)
        self.assertEqual(markdown, "")
        self.assertEqual(pages, 1)

    def test_invalid_jsonl_is_reported_as_a_sample_error(self) -> None:
        with self.assertRaises(sample_compare.SampleCompareError):
            sample_compare.paddle_markdown_from_jsonl("{not json}")


if __name__ == "__main__":
    unittest.main()
