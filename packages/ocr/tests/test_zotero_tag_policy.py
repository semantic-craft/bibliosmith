from pathlib import Path
import json
import tempfile
import time
import sys
import unittest
from types import SimpleNamespace
from unittest.mock import patch


SCRIPT_DIR = Path(__file__).resolve().parents[1] / "scripts"
sys.path.insert(0, str(SCRIPT_DIR))

from zotero_llm_worker import (  # noqa: E402
    Attachment,
    StateDB,
    TextLayerSample,
    WORKER_EXTRACTION_CONTRACT_VERSION,
    attachment_provenance_note,
    attachment_tags,
    build_parser,
    emit_attachment_evidence,
    format_page_ranges,
    get_config,
    md5_file,
    process_mineru_route,
    route_attachment,
    source_key_from_provenance_note,
)


class ZoteroTagPolicyTests(unittest.TestCase):
    def fixture_attachment(self, path: Path) -> Attachment:
        return Attachment(
            key="KXPSMW4C",
            title="Example.pdf",
            path=path,
            parent_key="XEMDH9G8",
            parent_item_type="book",
            parent_title="Example",
            parent_creators=[],
            parent_date="2026",
            content_type="application/pdf",
        )

    def test_default_command_writes_no_tags(self) -> None:
        args = build_parser().parse_args([])
        config = get_config(zotero_tags=args.zotero_tag)

        self.assertEqual([], attachment_tags(config))

    def test_only_explicit_tag_names_are_written(self) -> None:
        args = build_parser().parse_args(
            ["--zotero-tag", "chosen", "--zotero-tag", " chosen ", "--zotero-tag", "second"]
        )
        config = get_config(zotero_tags=args.zotero_tag)

        self.assertEqual(["chosen", "second"], attachment_tags(config))

    def test_provenance_moves_to_note_and_round_trips_source_key(self) -> None:
        config = get_config()
        attachment = Attachment(
            key="KXPSMW4C",
            title="Example.pdf",
            path=Path("/tmp/Example.pdf"),
            parent_key="XEMDH9G8",
            parent_item_type="book",
            parent_title="Example",
            parent_creators=[],
            parent_date="2026",
            content_type="application/pdf",
        )

        note = attachment_provenance_note(attachment, "paddle-ocr", config)

        self.assertIn("Conversion Route: paddle-ocr", note)
        self.assertEqual("KXPSMW4C", source_key_from_provenance_note(note))

    def test_force_mineru_is_a_single_attachment_worker_route(self) -> None:
        args = build_parser().parse_args(
            ["--attachment-key", "KXPSMW4C", "--force-mineru", "--preserve-source"]
        )

        self.assertEqual("KXPSMW4C", args.attachment_key)
        self.assertTrue(args.force_mineru)
        self.assertTrue(args.preserve_source)
        self.assertEqual("1-3,5,7-8", format_page_ranges([1, 2, 3, 5, 7, 8]))

    def test_pipeline_route_selects_mineru_for_low_text_journal_only_when_available(self) -> None:
        attachment = self.fixture_attachment(Path("/tmp/Example.pdf"))
        attachment.parent_item_type = "journalArticle"
        config = get_config()
        config.baidu_token = "paddle-fixture"
        low_text = TextLayerSample(
            extractable=False,
            chars=0,
            sample_pages=[1, 2],
            degraded=False,
            reason="",
        )
        with patch("zotero_llm_worker.sample_text_layer", return_value=low_text):
            config.mineru_token_available = True
            self.assertEqual(
                "mineru",
                route_attachment(
                    attachment, attachment.path, 2, config, pipeline_route=True
                )[0],
            )
            config.mineru_token_available = False
            self.assertEqual(
                "paddle-ocr",
                route_attachment(
                    attachment, attachment.path, 2, config, pipeline_route=True
                )[0],
            )

    def test_pipeline_route_keeps_degraded_text_blocked_even_with_mineru_available(self) -> None:
        attachment = self.fixture_attachment(Path("/tmp/Example.pdf"))
        config = get_config()
        config.mineru_token_available = True
        degraded = TextLayerSample(
            extractable=True,
            chars=1600,
            sample_pages=[1, 2],
            degraded=True,
            reason="dirty text fixture",
        )
        with patch("zotero_llm_worker.sample_text_layer", return_value=degraded):
            route = route_attachment(
                attachment, attachment.path, 2, config, pipeline_route=True
            )[0]

        self.assertEqual("needs-mineru", route)

    def test_pipeline_route_blocks_paddle_before_extract_when_credential_is_missing(self) -> None:
        attachment = self.fixture_attachment(Path("/tmp/Example.pdf"))
        config = get_config()
        config.baidu_token = ""
        scanned = TextLayerSample(
            extractable=False,
            chars=0,
            sample_pages=[1, 2],
            degraded=False,
            reason="",
        )
        with patch("zotero_llm_worker.sample_text_layer", return_value=scanned):
            route = route_attachment(
                attachment, attachment.path, 2, config, pipeline_route=True
            )[0]

        self.assertEqual("missing-paddleocr-token", route)

    def test_mineru_adapter_returns_markdown_through_worker_state(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            source = root / "source.pdf"
            source.write_bytes(b"%PDF fixture\n")
            attachment = Attachment(
                key="KXPSMW4C",
                title="Example.pdf",
                path=source,
                parent_key="XEMDH9G8",
                parent_item_type="book",
                parent_title="Example",
                parent_creators=[],
                parent_date="2026",
                content_type="application/pdf",
            )
            config = get_config()
            config.output_root = root / "output"
            state = StateDB(root / "state.sqlite3")

            def fake_run(command: list[str], **_: object) -> SimpleNamespace:
                output_dir = Path(command[command.index("--output-dir") + 1])
                result = output_dir / "batch" / "document" / "extracted" / "full.md"
                result.parent.mkdir(parents=True)
                result.write_text("# MinerU result\n", encoding="utf-8")
                return SimpleNamespace(returncode=0)

            with patch("zotero_llm_worker.subprocess.run", side_effect=fake_run):
                status = process_mineru_route(
                    attachment=attachment,
                    config=config,
                    state=state,
                    source_md5="fixture-md5",
                    page_count=2,
                    pages=[1, 2],
                    route_reason="forced by test",
                    no_upload=True,
                    deadline=time.time() + 30,
                )

            self.assertEqual("local_complete", status)
            row = state.document(attachment.key, "fixture-md5")
            assert row is not None
            self.assertEqual("mineru", row["route"])
            markdown = Path(row["output_path"])
            self.assertIn('parent_item_key: "XEMDH9G8"', markdown.read_text(encoding="utf-8"))
            self.assertEqual([], list(config.output_root.rglob("full.md")))

    def test_completed_worker_evidence_binds_source_markdown_and_zotero_key(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            source = root / "source.pdf"
            source.write_bytes(b"%PDF fixture\n")
            markdown = root / "source.md"
            markdown.write_text("# Complete\n", encoding="utf-8")
            attachment = Attachment(
                key="KXPSMW4C",
                title="Example.pdf",
                path=source,
                parent_key="XEMDH9G8",
                parent_item_type="book",
                parent_title="Example",
                parent_creators=[],
                parent_date="2026",
                content_type="application/pdf",
            )
            state = StateDB(root / "state.sqlite3")
            source_md5 = md5_file(source)
            state.upsert_document(
                attachment=attachment,
                source_md5=source_md5,
                route="pdf-text",
                status="completed",
                page_count=1,
                output_path=markdown,
                zotero_attachment_key="MDKEY123",
            )

            with self.assertLogs(level="INFO") as logs:
                emit_attachment_evidence(
                    attachment=attachment,
                    state=state,
                    observed_status="skipped_completed",
                )

            line = next(line for line in logs.output if "BOOK_PIPELINE_ATTACHMENT_EVIDENCE " in line)
            payload = json.loads(line.split("BOOK_PIPELINE_ATTACHMENT_EVIDENCE ", 1)[1])
            self.assertEqual("already_completed", payload["status"])
            self.assertEqual("KXPSMW4C", payload["pdfAttachmentKey"])
            self.assertEqual("XEMDH9G8", payload["parentItemKey"])
            self.assertEqual("MDKEY123", payload["markdownAttachmentKey"])
            self.assertEqual(
                WORKER_EXTRACTION_CONTRACT_VERSION,
                payload["extractionContractVersion"],
            )
            self.assertEqual(64, len(payload["sourceSha256"]))
            self.assertEqual(64, len(payload["markdownSha256"]))

    def test_completed_worker_evidence_rejects_missing_extraction_contract(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            source = root / "source.pdf"
            source.write_bytes(b"%PDF fixture\n")
            markdown = root / "source.md"
            markdown.write_text("# Legacy complete\n", encoding="utf-8")
            attachment = Attachment(
                key="KXPSMW4C",
                title="Example.pdf",
                path=source,
                parent_key="XEMDH9G8",
                parent_item_type="book",
                parent_title="Example",
                parent_creators=[],
                parent_date="2026",
                content_type="application/pdf",
            )
            state = StateDB(root / "state.sqlite3")
            source_md5 = md5_file(source)
            state.upsert_document(
                attachment=attachment,
                source_md5=source_md5,
                route="pdf-text",
                status="completed",
                page_count=1,
                output_path=markdown,
                zotero_attachment_key="MDKEY123",
            )
            state.conn.execute(
                "UPDATE documents SET extraction_contract_version=NULL WHERE pdf_key=?",
                (attachment.key,),
            )
            state.conn.commit()

            with patch("zotero_llm_worker.logging.info") as info:
                emit_attachment_evidence(
                    attachment=attachment,
                    state=state,
                    observed_status="skipped_completed",
                )

            info.assert_not_called()


if __name__ == "__main__":
    unittest.main()
