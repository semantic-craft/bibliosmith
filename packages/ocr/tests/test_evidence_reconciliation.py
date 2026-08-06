from __future__ import annotations

import dataclasses
import hashlib
import io
import json
from pathlib import Path
import sys
import tempfile
import time
import unittest
from types import SimpleNamespace
from unittest.mock import patch

import fitz


SCRIPT_DIR = Path(__file__).resolve().parents[1] / "scripts"
sys.path.insert(0, str(SCRIPT_DIR))

from zotero_llm_worker import (  # noqa: E402
    Attachment,
    ReconciliationBlocked,
    StateDB,
    ZoteroWebClient,
    cli,
    emit_attachment_evidence,
    get_config,
    md5_file,
    process_attachment,
    process_mineru_route,
    process_ocr_route,
    process_text_route,
    reconcile_staged_conversion,
    upload_test,
)


def write_pdf(path: Path, page_texts: list[str]) -> Path:
    document = fitz.open()
    for text in page_texts:
        page = document.new_page()
        page.insert_text((72, 300), text, fontsize=16)
    document.save(path)
    document.close()
    return path


class PdfTextEvidenceReconciliationTests(unittest.TestCase):
    def setUp(self) -> None:
        self.root = Path(self.enterContext(tempfile.TemporaryDirectory()))
        self.pdf = write_pdf(
            self.root / "Example.pdf",
            ["First page with enough text.", "Second page with enough text."],
        )
        self.attachment = Attachment(
            key="KXPSMW4C",
            title="Example.pdf",
            path=self.pdf,
            parent_key="XEMDH9G8",
            parent_item_type="book",
            parent_title="Example",
            parent_creators=[],
            parent_date="2026",
            content_type="application/pdf",
        )
        self.config = dataclasses.replace(get_config(), output_root=self.root / "output")
        self.state = StateDB(self.root / "state.sqlite3")

    def convert_without_upload(self) -> None:
        status = process_text_route(
            attachment=self.attachment,
            config=self.config,
            state=self.state,
            source_md5=md5_file(self.pdf),
            page_count=2,
            pages=[1, 2],
            route_reason="test",
            no_upload=True,
        )
        self.assertEqual("local_complete", status)

    def commit_route_evidence(self, route: str, pages: list[int]) -> None:
        source_md5 = md5_file(self.pdf)
        if route == "pdf-text":
            process_text_route(
                attachment=self.attachment,
                config=self.config,
                state=self.state,
                source_md5=source_md5,
                page_count=2,
                pages=pages,
                route_reason="conformance",
                no_upload=True,
            )
            return
        if route == "paddle-ocr":
            config = dataclasses.replace(
                self.config,
                baidu_token="fixture-token",
                baidu_model="PP-OCRv5",
                max_ocr_pages_per_job=2,
                baidu_max_upload_mb=64,
            )

            class ConformanceBaiduClient:
                def __init__(self, _: object) -> None:
                    pass

                def submit_job(self, pdf_path: Path, batch_id: str) -> str:
                    del batch_id
                    return pdf_path.stem

                def poll_json_url(
                    self,
                    job_id: str,
                    deadline: float,
                    on_progress=None,
                ) -> str:  # type: ignore[no-untyped-def]
                    del deadline, on_progress
                    return job_id

                def download_jsonl(self, url: str) -> str:
                    _, start, end = url.split("-")
                    results = [
                        {"prunedResult": {"rec_texts": [f"page {page}"]}}
                        for page in range(int(start), int(end) + 1)
                    ]
                    return json.dumps({"result": {"ocrResults": results}}) + "\n"

            with patch("zotero_llm_worker.BaiduOCRClient", ConformanceBaiduClient):
                process_ocr_route(
                    attachment=self.attachment,
                    config=config,
                    state=self.state,
                    source_md5=source_md5,
                    page_count=2,
                    pages=pages,
                    route_reason="conformance",
                    no_upload=True,
                    deadline=time.time() + 60,
                    ocr_pages_remaining=len(pages),
                )
            return
        if route == "mineru":
            def fake_run(command: list[str], **_: object) -> SimpleNamespace:
                output_dir = Path(command[command.index("--output-dir") + 1])
                result = output_dir / "document" / "full.md"
                result.parent.mkdir(parents=True)
                result.write_text("# MinerU fixture\n", encoding="utf-8")
                lines = result.read_text(encoding="utf-8").splitlines()
                result.with_suffix(".publication.json").write_text(
                    json.dumps(
                        {
                            "schema": "publication-extraction-evidence-v2",
                            "sourceFormat": "mineru",
                            "sourceDocuments": [
                                {
                                    "path": result.name,
                                    "startLine": 1,
                                    "endLine": len(lines),
                                    "pages": pages,
                                    "kind": "mineru_part_markdown",
                                    "sha256": hashlib.sha256(result.read_bytes()).hexdigest(),
                                }
                            ],
                            "sections": [],
                        }
                    )
                    + "\n",
                    encoding="utf-8",
                )
                (result.parent / "mineru_manifest.json").write_text(
                    '{"model_version":"fixture"}\n',
                    encoding="utf-8",
                )
                return SimpleNamespace(returncode=0)

            with patch("zotero_llm_worker.subprocess.run", side_effect=fake_run):
                process_mineru_route(
                    attachment=self.attachment,
                    config=self.config,
                    state=self.state,
                    source_md5=source_md5,
                    page_count=2,
                    pages=pages,
                    route_reason="conformance",
                    no_upload=True,
                    deadline=time.time() + 60,
                )
            return
        self.fail(f"unsupported conformance route: {route}")

    def replace_evidence(self, evidence: dict[str, object]) -> None:
        source_md5 = md5_file(self.pdf)
        raw = json.dumps(evidence, ensure_ascii=False, separators=(",", ":"))
        record = self.state.conversion_evidence_record(self.attachment.key, source_md5)
        self.assertIsNotNone(record)
        evidence_path = self.state.artifact_root / record[1]
        evidence_path.write_text(raw + "\n", encoding="utf-8")
        self.state.conn.execute(
            "UPDATE conversion_evidence SET evidence_json=? WHERE pdf_key=? AND source_md5=?",
            (raw, self.attachment.key, source_md5),
        )
        self.state.conn.commit()

    def test_pdf_text_commit_reconciles_exact_bytes_before_recovery_upload(self) -> None:
        self.convert_without_upload()

        outcome = reconcile_staged_conversion(
            attachment=self.attachment,
            state=self.state,
            page_count=2,
        )

        self.assertTrue(outcome.accepted)
        self.assertEqual("pdf-text", outcome.route)
        self.assertEqual("local_complete", outcome.status)
        self.assertEqual((1, 2), outcome.selected_pages)
        self.assertEqual(
            {"markdown", "publication-evidence", "route-sidecar"},
            {artifact.kind for artifact in outcome.evidence.artifacts},
        )

        outcome.evidence.markdown_path.write_text("tampered", encoding="utf-8")
        local = unittest.mock.Mock()
        local.get_pdf_attachment.return_value = self.attachment
        with patch("zotero_llm_worker.ZoteroWebClient") as web_client_class:
            with self.assertRaisesRegex(Exception, "artifact_drift"):
                upload_test(self.config, self.state, local, self.attachment.key)

        web_client_class.assert_not_called()

    def test_recovery_upload_binds_once_and_retry_reuses_the_same_attachment(self) -> None:
        self.convert_without_upload()
        local = unittest.mock.Mock()
        local.get_pdf_attachment.return_value = self.attachment

        with (
            patch("zotero_llm_worker.pdf_page_count", return_value=2),
            patch("zotero_llm_worker.ZoteroWebClient") as web_client_class,
        ):
            web_client = web_client_class.return_value
            web_client.create_markdown_attachment_item.return_value = "MDKEY123"
            web_client.markdown_attachment_matches.return_value = True

            upload_test(self.config, self.state, local, self.attachment.key)
            upload_test(self.config, self.state, local, self.attachment.key)

        web_client.create_markdown_attachment_item.assert_called_once()
        row = self.state.completed(self.attachment.key, md5_file(self.pdf))
        self.assertEqual("MDKEY123", row["zotero_attachment_key"] if row else None)
        outcome = reconcile_staged_conversion(
            attachment=self.attachment,
            state=self.state,
            page_count=2,
        )
        self.assertEqual("MDKEY123", outcome.evidence.markdown_attachment_key)

    def test_bound_remote_markdown_must_match_the_evidence_digest(self) -> None:
        client = ZoteroWebClient.__new__(ZoteroWebClient)
        client.base_url = "https://example.test"
        client.timeout = 1
        metadata_response = unittest.mock.Mock(
            status_code=200,
        )
        metadata_response.json.return_value = {
            "data": {
                "itemType": "attachment",
                "contentType": "text/markdown",
                "parentItem": self.attachment.parent_key,
                "filename": "Example.md",
                "note": f"OCR Source Key: {self.attachment.key}",
            }
        }
        file_response = unittest.mock.Mock(status_code=200, content=b"exact markdown")
        client.session = unittest.mock.Mock()
        client.session.get.side_effect = [metadata_response, file_response]

        self.assertTrue(
            client.markdown_attachment_matches(
                "MDKEY123",
                parent_key=self.attachment.parent_key,
                filename="Example.md",
                source_pdf_key=self.attachment.key,
                markdown_sha256=hashlib.sha256(b"exact markdown").hexdigest(),
            )
        )

        client.session.get.side_effect = [metadata_response, file_response]
        self.assertFalse(
            client.markdown_attachment_matches(
                "MDKEY123",
                parent_key=self.attachment.parent_key,
                filename="Example.md",
                source_pdf_key=self.attachment.key,
                markdown_sha256=hashlib.sha256(b"different markdown").hexdigest(),
            )
        )

    def test_provenance_recovery_paginates_all_remote_children(self) -> None:
        client = ZoteroWebClient.__new__(ZoteroWebClient)
        client.base_url = "https://example.test"
        client.timeout = 1
        first_page = unittest.mock.Mock(status_code=200)
        first_page.json.return_value = [
            {"data": {"key": f"OTHER{index:03}", "itemType": "note"}}
            for index in range(100)
        ]
        second_page = unittest.mock.Mock(status_code=200)
        second_page.json.return_value = [
            {
                "data": {
                    "key": "RECOVER1",
                    "itemType": "attachment",
                    "contentType": "text/markdown",
                    "parentItem": self.attachment.parent_key,
                    "filename": "Example.md",
                    "note": f"OCR Source Key: {self.attachment.key}",
                }
            }
        ]
        client.session = unittest.mock.Mock()
        client.session.get.side_effect = [first_page, second_page]

        key = client.find_markdown_attachment_by_provenance(
            parent_key=self.attachment.parent_key,
            filename="Example.md",
            source_pdf_key=self.attachment.key,
        )

        self.assertEqual("RECOVER1", key)
        self.assertEqual(100, client.session.get.call_args_list[1].kwargs["params"]["start"])

    def test_near_concurrent_upload_is_blocked_before_any_remote_write(self) -> None:
        self.convert_without_upload()
        source_md5 = md5_file(self.pdf)
        outcome = reconcile_staged_conversion(
            attachment=self.attachment,
            state=self.state,
            page_count=2,
        )
        self.assertIsNotNone(outcome.evidence)
        self.state.claim_upload(
            pdf_key=self.attachment.key,
            source_md5=source_md5,
            evidence=outcome.evidence,
        )
        local = unittest.mock.Mock()
        local.get_pdf_attachment.return_value = self.attachment

        with (
            patch("zotero_llm_worker.pdf_page_count", return_value=2),
            patch("zotero_llm_worker.ZoteroWebClient") as web_client_class,
            self.assertRaisesRegex(Exception, "upload_in_progress"),
        ):
            upload_test(self.config, self.state, local, self.attachment.key)

        web_client = web_client_class.return_value
        web_client.create_markdown_attachment_item.assert_not_called()
        web_client.upload_file.assert_not_called()

    def test_invalid_coverage_blocks_before_zotero(self) -> None:
        self.convert_without_upload()
        source_md5 = md5_file(self.pdf)
        raw = self.state.conversion_evidence_json(self.attachment.key, source_md5)
        evidence = json.loads(raw or "{}")
        evidence["selectedPages"] = [1, 1]
        self.replace_evidence(evidence)
        local = unittest.mock.Mock()
        local.get_pdf_attachment.return_value = self.attachment

        with (
            patch("zotero_llm_worker.pdf_page_count", return_value=2),
            patch("zotero_llm_worker.ZoteroWebClient") as web_client_class,
            self.assertRaisesRegex(Exception, "invalid_coverage"),
        ):
            upload_test(self.config, self.state, local, self.attachment.key)

        web_client_class.assert_not_called()

    def test_every_malformed_coverage_shape_is_rejected_at_the_public_seam(self) -> None:
        self.convert_without_upload()
        source_md5 = md5_file(self.pdf)
        original = json.loads(
            self.state.conversion_evidence_json(self.attachment.key, source_md5) or "{}"
        )

        for pages in ([2, 1], [1, "2"], [0], [-1], [3], []):
            with self.subTest(pages=pages):
                evidence = dict(original)
                evidence["selectedPages"] = pages
                self.replace_evidence(evidence)

                outcome = reconcile_staged_conversion(
                    attachment=self.attachment,
                    state=self.state,
                    page_count=2,
                )

                self.assertFalse(outcome.accepted)
                self.assertEqual("invalid_coverage", outcome.error_code)

    def test_reconciliation_rejects_an_unknown_route(self) -> None:
        self.convert_without_upload()
        source_md5 = md5_file(self.pdf)
        evidence = json.loads(
            self.state.conversion_evidence_json(self.attachment.key, source_md5) or "{}"
        )
        evidence["route"] = "legacy-inferred"
        self.replace_evidence(evidence)

        outcome = reconcile_staged_conversion(
            attachment=self.attachment,
            state=self.state,
            page_count=2,
        )

        self.assertFalse(outcome.accepted)
        self.assertEqual("unsupported_route", outcome.error_code)

    def test_source_and_page_count_drift_have_distinct_safe_outcomes(self) -> None:
        self.convert_without_upload()

        page_drift = reconcile_staged_conversion(
            attachment=self.attachment,
            state=self.state,
            page_count=3,
        )
        self.assertEqual("page_count_drift", page_drift.error_code)

        self.pdf.write_bytes(self.pdf.read_bytes() + b"changed")
        source_drift = reconcile_staged_conversion(
            attachment=self.attachment,
            state=self.state,
            page_count=2,
        )
        self.assertEqual("source_drift", source_drift.error_code)

    def test_each_referenced_file_is_hash_bound(self) -> None:
        for kind in ("markdown", "route-sidecar", "publication-evidence"):
            with self.subTest(kind=kind):
                self.convert_without_upload()
                outcome = reconcile_staged_conversion(
                    attachment=self.attachment,
                    state=self.state,
                    page_count=2,
                )
                artifact = next(
                    artifact for artifact in outcome.evidence.artifacts if artifact.kind == kind
                )
                artifact.path.write_bytes(artifact.path.read_bytes() + b"drift")

                drifted = reconcile_staged_conversion(
                    attachment=self.attachment,
                    state=self.state,
                    page_count=2,
                )

                self.assertEqual("artifact_drift", drifted.error_code)

    def test_legacy_staging_without_evidence_is_blocked_with_rerun_guidance(self) -> None:
        staging = self.config.output_root / ".state" / "staging" / self.attachment.key
        staging.mkdir(parents=True)
        (staging / "legacy.md").write_text("private legacy text", encoding="utf-8")
        local = unittest.mock.Mock()
        local.get_pdf_attachment.return_value = self.attachment

        with (
            patch("zotero_llm_worker.pdf_page_count", return_value=2),
            patch("zotero_llm_worker.ZoteroWebClient") as web_client_class,
            self.assertRaisesRegex(Exception, "missing_evidence.*Rerun conversion"),
        ):
            upload_test(self.config, self.state, local, self.attachment.key)

        web_client_class.assert_not_called()

    def test_state_evidence_uses_worker_relative_references_and_no_content(self) -> None:
        self.convert_without_upload()
        raw = self.state.conversion_evidence_json(
            self.attachment.key,
            md5_file(self.pdf),
        )

        self.assertIsNotNone(raw)
        self.assertNotIn(str(self.root), raw or "")
        self.assertNotIn("First page with enough text", raw or "")
        self.assertNotIn("token", raw or "")
        evidence = json.loads(raw or "{}")
        self.assertTrue(
            all(not Path(artifact["reference"]).is_absolute() for artifact in evidence["artifacts"])
        )

    def test_db_and_committed_evidence_reference_must_remain_identical(self) -> None:
        self.convert_without_upload()
        source_md5 = md5_file(self.pdf)
        raw = self.state.conversion_evidence_json(self.attachment.key, source_md5)
        evidence = json.loads(raw or "{}")
        evidence["route"] = "mineru"
        self.state.conn.execute(
            "UPDATE conversion_evidence SET evidence_json=? WHERE pdf_key=? AND source_md5=?",
            (json.dumps(evidence), self.attachment.key, source_md5),
        )
        self.state.conn.commit()

        outcome = reconcile_staged_conversion(
            attachment=self.attachment,
            state=self.state,
            page_count=2,
        )

        self.assertFalse(outcome.accepted)
        self.assertEqual("evidence_reference_drift", outcome.error_code)

    def test_committed_evidence_reference_rejects_trailing_byte_drift(self) -> None:
        self.convert_without_upload()
        source_md5 = md5_file(self.pdf)
        record = self.state.conversion_evidence_record(self.attachment.key, source_md5)
        self.assertIsNotNone(record)
        evidence_path = self.state.artifact_root / record[1]
        with evidence_path.open("ab") as handle:
            handle.write(b"   \n")

        outcome = reconcile_staged_conversion(
            attachment=self.attachment,
            state=self.state,
            page_count=2,
        )

        self.assertFalse(outcome.accepted)
        self.assertEqual("evidence_reference_drift", outcome.error_code)

    def test_validation_cli_emits_safe_machine_readable_failures_without_tracebacks(self) -> None:
        cases = [
            (
                ReconciliationBlocked("attachment_mismatch", "private path /tmp/book.md"),
                2,
                "BOOK_PIPELINE_EVIDENCE_MISMATCH attachment_mismatch\n",
            ),
            (
                RuntimeError("transport failed at /opt/private/book.md"),
                75,
                "BOOK_PIPELINE_EVIDENCE_RETRYABLE remote_validation_unavailable\n",
            ),
        ]
        for error, expected_code, expected_stderr in cases:
            with self.subTest(error=type(error).__name__):
                stderr = io.StringIO()
                with (
                    patch("zotero_llm_worker.main", side_effect=error),
                    patch("sys.stderr", stderr),
                ):
                    code = cli(["--verify-uploaded-evidence", "--attachment-key", "KXPSMW4C"])
                self.assertEqual(expected_code, code)
                self.assertEqual(expected_stderr, stderr.getvalue())
                self.assertNotIn("Traceback", stderr.getvalue())
                self.assertNotIn("/opt/private", stderr.getvalue())

    def test_active_upload_lease_cannot_be_revoked_by_another_conversion_commit(self) -> None:
        self.convert_without_upload()
        source_md5 = md5_file(self.pdf)
        outcome = reconcile_staged_conversion(
            attachment=self.attachment,
            state=self.state,
            page_count=2,
        )
        owner = self.state.claim_upload(
            pdf_key=self.attachment.key,
            source_md5=source_md5,
            evidence=outcome.evidence,
        )
        other = StateDB(self.root / "state.sqlite3")
        markdown = outcome.evidence.markdown_path
        artifacts = {artifact.kind: artifact.path for artifact in outcome.evidence.artifacts}

        with self.assertRaisesRegex(Exception, "upload_in_progress"):
            other.commit_conversion_evidence(
                attachment=self.attachment,
                source_md5=source_md5,
                route="pdf-text",
                page_count=2,
                selected_pages=[1, 2],
                markdown_path=markdown,
                sidecar_path=artifacts["route-sidecar"],
                publication_evidence_path=artifacts["publication-evidence"],
            )

        row = self.state.conn.execute(
            "SELECT upload_state, upload_owner_token FROM conversion_evidence"
        ).fetchone()
        self.assertEqual(("uploading", owner), tuple(row))

    def test_expired_owner_cannot_rewrite_mirror_after_lease_handoff(self) -> None:
        self.convert_without_upload()
        source_md5 = md5_file(self.pdf)
        outcome = reconcile_staged_conversion(
            attachment=self.attachment,
            state=self.state,
            page_count=2,
        )
        owner_a = self.state.claim_upload(
            pdf_key=self.attachment.key,
            source_md5=source_md5,
            evidence=outcome.evidence,
        )
        self.state.conn.execute(
            "UPDATE conversion_evidence SET upload_lease_expires_at='2000-01-01T00:00:00+00:00'"
        )
        self.state.conn.commit()
        other = StateDB(self.root / "state.sqlite3")
        owner_b = other.claim_upload(
            pdf_key=self.attachment.key,
            source_md5=source_md5,
            evidence=outcome.evidence,
        )
        record = self.state.conversion_evidence_record(self.attachment.key, source_md5)
        mirror = self.state.artifact_root / record[1]
        before = mirror.read_bytes()

        with self.assertRaisesRegex(Exception, "upload_lease_lost"):
            self.state.bind_markdown_attachment(
                attachment=self.attachment,
                source_md5=source_md5,
                evidence=outcome.evidence,
                markdown_attachment_key="STALE001",
                status="completed",
                upload_owner_token=owner_a,
            )

        self.assertEqual(before, mirror.read_bytes())
        still_valid = reconcile_staged_conversion(
            attachment=self.attachment,
            state=self.state,
            page_count=2,
        )
        self.assertTrue(still_valid.accepted)
        other.bind_markdown_attachment(
            attachment=self.attachment,
            source_md5=source_md5,
            evidence=outcome.evidence,
            markdown_attachment_key="CURRENT1",
            status="completed",
            upload_owner_token=owner_b,
        )
        rebound = reconcile_staged_conversion(
            attachment=self.attachment,
            state=self.state,
            page_count=2,
        )
        self.assertTrue(rebound.accepted)
        self.assertEqual("CURRENT1", rebound.evidence.markdown_attachment_key)

    def test_reconciliation_rejects_a_symlink_escape_from_the_artifact_root(self) -> None:
        self.convert_without_upload()
        source_md5 = md5_file(self.pdf)
        evidence = json.loads(
            self.state.conversion_evidence_json(self.attachment.key, source_md5) or "{}"
        )
        external = self.root.parent / f"{self.root.name}-outside.md"
        external.write_text("external private text", encoding="utf-8")
        self.addCleanup(external.unlink, missing_ok=True)
        link = self.state.artifact_root / "escaped.md"
        link.symlink_to(external)
        markdown = next(
            artifact for artifact in evidence["artifacts"] if artifact["kind"] == "markdown"
        )
        markdown["reference"] = "escaped.md"
        markdown["sha256"] = hashlib.sha256(external.read_bytes()).hexdigest()
        self.replace_evidence(evidence)

        outcome = reconcile_staged_conversion(
            attachment=self.attachment,
            state=self.state,
            page_count=2,
        )

        self.assertFalse(outcome.accepted)
        self.assertEqual("unsupported_evidence", outcome.error_code)

    def test_reconciliation_rejects_an_empty_artifact_reference(self) -> None:
        self.convert_without_upload()
        source_md5 = md5_file(self.pdf)
        evidence = json.loads(
            self.state.conversion_evidence_json(self.attachment.key, source_md5) or "{}"
        )
        next(
            artifact for artifact in evidence["artifacts"] if artifact["kind"] == "markdown"
        )["reference"] = ""
        self.replace_evidence(evidence)

        outcome = reconcile_staged_conversion(
            attachment=self.attachment,
            state=self.state,
            page_count=2,
        )

        self.assertFalse(outcome.accepted)
        self.assertEqual("unsupported_evidence", outcome.error_code)

    def test_worker_event_uses_only_relative_artifact_references(self) -> None:
        self.convert_without_upload()
        local = unittest.mock.Mock()
        local.get_pdf_attachment.return_value = self.attachment
        with (
            patch("zotero_llm_worker.pdf_page_count", return_value=2),
            patch("zotero_llm_worker.ZoteroWebClient") as web_client_class,
        ):
            web_client_class.return_value.create_markdown_attachment_item.return_value = (
                "MDKEY123"
            )
            upload_test(self.config, self.state, local, self.attachment.key)
        with self.assertLogs(level="INFO") as logs:
            emit_attachment_evidence(
                attachment=self.attachment,
                state=self.state,
                observed_status="skipped_completed",
            )
        line = next(
            line for line in logs.output if "BOOK_PIPELINE_ATTACHMENT_EVIDENCE " in line
        )
        payload = json.loads(line.split("BOOK_PIPELINE_ATTACHMENT_EVIDENCE ", 1)[1])

        self.assertNotIn("markdownPath", payload)
        for field in (
            "conversionEvidenceReference",
            "markdownReference",
            "routeSidecarReference",
            "publicationEvidenceReference",
        ):
            self.assertFalse(Path(payload[field]).is_absolute(), field)
        self.assertNotIn(str(self.root), json.dumps(payload))

    def test_upload_failure_preserves_conversion_evidence_for_retry(self) -> None:
        self.convert_without_upload()
        source_md5 = md5_file(self.pdf)
        original = self.state.conversion_evidence_json(self.attachment.key, source_md5)
        local = unittest.mock.Mock()
        local.get_pdf_attachment.return_value = self.attachment

        with (
            patch("zotero_llm_worker.pdf_page_count", return_value=2),
            patch("zotero_llm_worker.ZoteroWebClient") as web_client_class,
        ):
            web_client = web_client_class.return_value
            web_client.create_markdown_attachment_item.return_value = "MDKEY123"
            web_client.upload_file.side_effect = RuntimeError(
                "transport unavailable with private details"
            )
            with self.assertRaisesRegex(Exception, "upload_failure") as raised:
                upload_test(self.config, self.state, local, self.attachment.key)

        self.assertNotIn("private details", str(raised.exception))

        self.assertEqual(
            original,
            self.state.conversion_evidence_json(self.attachment.key, source_md5),
        )
        row = self.state.document(self.attachment.key, source_md5)
        self.assertEqual("local_complete", row["status"] if row else None)
        delivery = self.state.conn.execute(
            "SELECT upload_state, delivery_error_code FROM conversion_evidence"
        ).fetchone()
        self.assertEqual(("retryable", "upload_failure"), tuple(delivery))

        with (
            patch("zotero_llm_worker.pdf_page_count", return_value=2),
            patch("zotero_llm_worker.ZoteroWebClient") as retry_client_class,
        ):
            retry_client = retry_client_class.return_value
            retry_client.markdown_attachment_matches.return_value = True
            upload_test(self.config, self.state, local, self.attachment.key)

        retry_client.create_markdown_attachment_item.assert_not_called()
        retry_client.upload_file.assert_called_once_with(
            "MDKEY123",
            unittest.mock.ANY,
        )
        completed = self.state.completed(self.attachment.key, source_md5)
        self.assertEqual("MDKEY123", completed["zotero_attachment_key"] if completed else None)

    def test_bound_mismatch_blocks_before_source_normalization_or_remote_write(self) -> None:
        self.convert_without_upload()
        local = unittest.mock.Mock()
        local.get_pdf_attachment.return_value = self.attachment
        with (
            patch("zotero_llm_worker.pdf_page_count", return_value=2),
            patch("zotero_llm_worker.ZoteroWebClient") as initial_client_class,
        ):
            initial_client_class.return_value.create_markdown_attachment_item.return_value = (
                "OLDKEY1"
            )
            upload_test(self.config, self.state, local, self.attachment.key)

        with (
            patch("zotero_llm_worker.ZoteroWebClient") as recovery_client_class,
            patch("zotero_llm_worker.normalize_source_pdf_attachment_name") as normalize,
            self.assertRaisesRegex(Exception, "attachment_mismatch"),
        ):
            recovery_client = recovery_client_class.return_value
            recovery_client.markdown_attachment_matches.return_value = False
            process_attachment(
                attachment=self.attachment,
                config=self.config,
                state=self.state,
                page_spec=None,
                no_upload=False,
                dry_run=False,
                force_route=None,
                deadline=float("inf"),
                ocr_pages_remaining=100,
            )

        normalize.assert_not_called()
        recovery_client.create_markdown_attachment_item.assert_not_called()
        recovery_client.upload_file.assert_not_called()

    def test_stale_pending_child_is_cleared_before_a_fresh_retry(self) -> None:
        self.convert_without_upload()
        source_md5 = md5_file(self.pdf)
        local = unittest.mock.Mock()
        local.get_pdf_attachment.return_value = self.attachment
        with (
            patch("zotero_llm_worker.pdf_page_count", return_value=2),
            patch("zotero_llm_worker.ZoteroWebClient") as initial_client_class,
        ):
            initial_client = initial_client_class.return_value
            initial_client.create_markdown_attachment_item.return_value = "STALE001"
            initial_client.upload_file.side_effect = RuntimeError("transport failed")
            with self.assertRaisesRegex(Exception, "upload_failure"):
                upload_test(self.config, self.state, local, self.attachment.key)

        with (
            patch("zotero_llm_worker.pdf_page_count", return_value=2),
            patch("zotero_llm_worker.ZoteroWebClient") as stale_client_class,
        ):
            stale_client = stale_client_class.return_value
            stale_client.markdown_attachment_matches.return_value = False
            with self.assertRaisesRegex(Exception, "attachment_mismatch"):
                upload_test(self.config, self.state, local, self.attachment.key)

        pending = self.state.conn.execute(
            "SELECT pending_attachment_key FROM conversion_evidence"
        ).fetchone()
        self.assertIsNone(pending["pending_attachment_key"] if pending else "missing")

        with (
            patch("zotero_llm_worker.pdf_page_count", return_value=2),
            patch("zotero_llm_worker.ZoteroWebClient") as retry_client_class,
        ):
            retry_client = retry_client_class.return_value
            retry_client.find_markdown_attachment_by_provenance.return_value = None
            retry_client.create_markdown_attachment_item.return_value = "FRESH001"
            upload_test(self.config, self.state, local, self.attachment.key)

        retry_client.create_markdown_attachment_item.assert_called_once()
        completed = self.state.completed(self.attachment.key, source_md5)
        self.assertEqual("FRESH001", completed["zotero_attachment_key"] if completed else None)

    def test_expired_upload_lease_recovers_the_remote_child_created_before_crash(self) -> None:
        self.convert_without_upload()
        source_md5 = md5_file(self.pdf)
        outcome = reconcile_staged_conversion(
            attachment=self.attachment,
            state=self.state,
            page_count=2,
        )
        self.state.claim_upload(
            pdf_key=self.attachment.key,
            source_md5=source_md5,
            evidence=outcome.evidence,
        )
        self.state.conn.execute(
            "UPDATE conversion_evidence SET upload_lease_expires_at='2000-01-01T00:00:00+00:00'"
        )
        self.state.conn.commit()
        local = unittest.mock.Mock()
        local.get_pdf_attachment.return_value = self.attachment

        with (
            patch("zotero_llm_worker.pdf_page_count", return_value=2),
            patch("zotero_llm_worker.ZoteroWebClient") as web_client_class,
        ):
            web_client = web_client_class.return_value
            web_client.find_markdown_attachment_by_provenance.return_value = "RECOVER1"
            upload_test(self.config, self.state, local, self.attachment.key)

        web_client.create_markdown_attachment_item.assert_not_called()
        web_client.upload_file.assert_called_once_with("RECOVER1", unittest.mock.ANY)
        completed = self.state.completed(self.attachment.key, source_md5)
        self.assertEqual("RECOVER1", completed["zotero_attachment_key"] if completed else None)

    def test_missing_bound_remote_child_blocks_without_replacement_side_effects(self) -> None:
        self.convert_without_upload()
        local = unittest.mock.Mock()
        local.get_pdf_attachment.return_value = self.attachment
        with (
            patch("zotero_llm_worker.pdf_page_count", return_value=2),
            patch("zotero_llm_worker.ZoteroWebClient") as initial_client_class,
        ):
            initial_client = initial_client_class.return_value
            initial_client.create_markdown_attachment_item.return_value = "OLDKEY1"
            upload_test(self.config, self.state, local, self.attachment.key)

        with (
            patch("zotero_llm_worker.pdf_page_count", return_value=2),
            patch("zotero_llm_worker.ZoteroWebClient") as recovery_client_class,
            self.assertRaisesRegex(Exception, "attachment_mismatch"),
        ):
            recovery_client = recovery_client_class.return_value
            recovery_client.markdown_attachment_matches.return_value = False
            upload_test(self.config, self.state, local, self.attachment.key)

        recovery_client.find_markdown_attachment_by_provenance.assert_not_called()
        recovery_client.create_markdown_attachment_item.assert_not_called()
        recovery_client.upload_file.assert_not_called()
        completed = self.state.completed(self.attachment.key, md5_file(self.pdf))
        self.assertEqual("OLDKEY1", completed["zotero_attachment_key"] if completed else None)

    def test_all_routes_share_success_partial_prewrite_retry_and_bound_outcomes(self) -> None:
        local = unittest.mock.Mock()
        local.get_pdf_attachment.return_value = self.attachment
        for route in ("pdf-text", "paddle-ocr", "mineru"):
            with self.subTest(route=route, outcome="success"):
                self.commit_route_evidence(route, [1, 2])
                with (
                    patch("zotero_llm_worker.pdf_page_count", return_value=2),
                    patch("zotero_llm_worker.ZoteroWebClient") as client_class,
                ):
                    client_class.return_value.create_markdown_attachment_item.return_value = (
                        f"{route[:2].upper()}FULL01"
                    )
                    upload_test(self.config, self.state, local, self.attachment.key)
                completed = self.state.completed(self.attachment.key, md5_file(self.pdf))
                self.assertEqual("completed", completed["status"] if completed else None)

            with self.subTest(route=route, outcome="partial"):
                self.commit_route_evidence(route, [1])
                with (
                    patch("zotero_llm_worker.pdf_page_count", return_value=2),
                    patch("zotero_llm_worker.ZoteroWebClient") as client_class,
                ):
                    client_class.return_value.create_markdown_attachment_item.return_value = (
                        f"{route[:2].upper()}PART01"
                    )
                    upload_test(self.config, self.state, local, self.attachment.key)
                row = self.state.document(self.attachment.key, md5_file(self.pdf))
                self.assertEqual("uploaded_partial", row["status"] if row else None)

            with self.subTest(route=route, outcome="prewrite"):
                self.commit_route_evidence(route, [1, 2])
                outcome = reconcile_staged_conversion(
                    attachment=self.attachment,
                    state=self.state,
                    page_count=2,
                )
                outcome.evidence.markdown_path.write_text("drift", encoding="utf-8")
                with (
                    patch("zotero_llm_worker.pdf_page_count", return_value=2),
                    patch("zotero_llm_worker.ZoteroWebClient") as client_class,
                    self.assertRaisesRegex(Exception, "artifact_drift"),
                ):
                    upload_test(self.config, self.state, local, self.attachment.key)
                client_class.assert_not_called()

            with self.subTest(route=route, outcome="retry_and_bound"):
                self.commit_route_evidence(route, [1, 2])
                pending_key = f"{route[:2].upper()}RETRY1"
                with (
                    patch("zotero_llm_worker.pdf_page_count", return_value=2),
                    patch("zotero_llm_worker.ZoteroWebClient") as failed_client_class,
                ):
                    failed_client = failed_client_class.return_value
                    failed_client.create_markdown_attachment_item.return_value = pending_key
                    failed_client.upload_file.side_effect = RuntimeError("private transport")
                    with self.assertRaisesRegex(Exception, "upload_failure"):
                        upload_test(self.config, self.state, local, self.attachment.key)
                with (
                    patch("zotero_llm_worker.pdf_page_count", return_value=2),
                    patch("zotero_llm_worker.ZoteroWebClient") as retry_client_class,
                ):
                    retry_client = retry_client_class.return_value
                    retry_client.markdown_attachment_matches.return_value = True
                    upload_test(self.config, self.state, local, self.attachment.key)
                    upload_test(self.config, self.state, local, self.attachment.key)
                retry_client.create_markdown_attachment_item.assert_not_called()
                retry_client.upload_file.assert_called_once_with(
                    pending_key,
                    unittest.mock.ANY,
                )


if __name__ == "__main__":
    unittest.main()
