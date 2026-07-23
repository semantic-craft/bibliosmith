#!/usr/bin/env python3
"""Inventory >100MB Zotero PDFs for PaddleOCR-VL Markdown.

The workflow is conservative:
- enumerate all local Zotero PDF attachments larger than 100 MiB;
- write a report before processing;
- process only PDFs without an extractable text layer;
- never delete or overwrite Zotero attachments;
- skip OCR when this PDF key already has a completed PaddleOCR Markdown row;
- use the existing shared OCR worker so large PDFs are split into safe
  chunks without degrading scans via pre-compression.
"""

from __future__ import annotations

import argparse
import datetime as dt
import hashlib
import logging
import os
import re
import shutil
import sqlite3
import subprocess
import sys
import time
from dataclasses import dataclass, replace
from pathlib import Path
from typing import Any

SCRIPT_DIR = Path(__file__).resolve().parent
APP_ROOT = SCRIPT_DIR.parent
sys.path.insert(0, str(SCRIPT_DIR))

from searchable_pdf_inventory import (  # noqa: E402
    PageSample,
    author_summary,
    classify_samples,
    item_title,
    pdf_page_count,
    sample_pages,
    sample_text_with_pdftotext,
    size_mb,
)
from zotero_llm_worker import (  # noqa: E402
    Attachment,
    DeadlineReached,
    QuotaExhaustedError,
    RetryableRemoteError,
    StateDB,
    WorkerError,
    ZoteroLocalClient,
    add_zotero_tag_argument,
    configure_logging,
    get_config,
    item_year,
    md5_file,
    pdf_page_count as worker_pdf_page_count,
    process_attachment,
    safe_slug,
)


MIN_SIZE_MB = 100.0
REPORT_PATH = APP_ROOT / "reports" / "giant_pdf_over_100mb.md"
PENDING_ROUTE = "paddleocr-vl-1.5-md"


@dataclass
class GiantPDF:
    attachment: Attachment
    size_bytes: int
    page_count: int | None
    page_error: str | None
    samples: list[PageSample]
    text_extractable: bool
    sample_note: str
    suggested_route: str
    existing_md: sqlite3.Row | None


def safe_pdf_stem(attachment: Attachment) -> str:
    return safe_slug(f"{author_summary(attachment)}_{item_year(attachment)}_{item_title(attachment)}", max_len=150)


def sha256_file(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as fh:
        for chunk in iter(lambda: fh.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def init_job_table(conn: sqlite3.Connection) -> None:
    conn.executescript(
        """
        CREATE TABLE IF NOT EXISTS giant_pdf_md_jobs (
            pdf_key TEXT PRIMARY KEY,
            parent_key TEXT,
            original_path TEXT NOT NULL,
            compressed_path TEXT,
            original_md5 TEXT,
            compressed_md5 TEXT,
            original_bytes INTEGER,
            compressed_bytes INTEGER,
            page_count INTEGER,
            compression_method TEXT,
            compression_status TEXT,
            md_status TEXT,
            zotero_attachment_key TEXT,
            error TEXT,
            created_at TEXT NOT NULL,
            updated_at TEXT NOT NULL
        );
        """
    )
    conn.commit()


def now_utc() -> str:
    return dt.datetime.now(dt.UTC).replace(microsecond=0).isoformat()


def existing_completed_md(state: StateDB, pdf_key: str) -> sqlite3.Row | None:
    rows = state.conn.execute(
        """
        SELECT * FROM documents
        WHERE pdf_key=? AND route='paddle-ocr' AND status='completed'
        ORDER BY updated_at DESC
        """,
        (pdf_key,),
    ).fetchall()
    for row in rows:
        output_path = row["output_path"]
        if output_path and Path(output_path).exists():
            return row
    return None


def inspect_attachment(attachment: Attachment, state: StateDB) -> GiantPDF | None:
    try:
        size_bytes = attachment.path.stat().st_size
    except OSError:
        return None
    if size_bytes <= int(MIN_SIZE_MB * 1024 * 1024):
        return None
    pages, page_error = pdf_page_count(attachment.path)
    samples: list[PageSample] = []
    if pages:
        for page in sample_pages(pages):
            samples.append(sample_text_with_pdftotext(attachment.path, page))
    text_extractable, _, note = classify_samples(samples)
    if page_error:
        route = "inspect-error"
    elif text_extractable:
        route = "pdf-text-or-skip"
    else:
        route = PENDING_ROUTE
    return GiantPDF(
        attachment=attachment,
        size_bytes=size_bytes,
        page_count=pages,
        page_error=page_error,
        samples=samples,
        text_extractable=text_extractable,
        sample_note=note,
        suggested_route=route,
        existing_md=existing_completed_md(state, attachment.key),
    )


def sample_summary(item: GiantPDF) -> str:
    parts: list[str] = []
    for sample in item.samples:
        if sample.chars is None:
            parts.append(f"p{sample.page}=ERR")
        else:
            parts.append(f"p{sample.page}={sample.chars}")
    return ", ".join(parts) if parts else "no samples"


def md_escape(value: object) -> str:
    return str(value if value is not None else "").replace("|", "\\|").replace("\n", " ")


def render_report(items: list[GiantPDF], queue: list[GiantPDF]) -> str:
    generated = dt.datetime.now().astimezone().isoformat(timespec="seconds")
    lines = [
        "# Giant PDF Over 100MB",
        "",
        f"- Generated: `{generated}`",
        f"- Filter: local Zotero PDF attachments larger than `{MIN_SIZE_MB:g} MB`",
        "- Text-layer test: `pdftotext` on pages `1, 2, 3, middle, last`.",
        "- OCR queue: only files without an extractable text layer.",
        "- OCR route: existing PaddleOCR-VL-1.6 worker, with chunking; no pre-compression.",
        "- Safety: this workflow does not delete or overwrite original Zotero attachments.",
        "",
        "## Summary",
        "",
        f"- Total >100MB PDFs: `{len(items)}`",
        f"- Without extractable text layer: `{sum(1 for item in items if not item.text_extractable and not item.page_error)}`",
        f"- With extractable text layer: `{sum(1 for item in items if item.text_extractable)}`",
        f"- Already completed PaddleOCR Markdown in state DB: `{sum(1 for item in queue if item.existing_md is not None)}`",
        f"- Pending PaddleOCR-VL Markdown: `{sum(1 for item in queue if item.existing_md is None)}`",
        "",
        "## PaddleOCR-VL Markdown Queue",
        "",
    ]
    if queue:
        lines.extend(
            [
                "| # | Size | PDF key | Parent | Pages | Existing MD | 作者 | 年份 | 标题 |",
                "|---:|---:|---|---|---:|---|---|---|---|",
            ]
        )
        for i, item in enumerate(queue, 1):
            a = item.attachment
            existing = item.existing_md["zotero_attachment_key"] if item.existing_md else ""
            lines.append(
                f"| {i} | {size_mb(item.size_bytes)} | {a.key} | {a.parent_key or ''} | "
                f"{item.page_count or 'ERROR'} | {existing} | {md_escape(author_summary(a))} | "
                f"{item_year(a)} | {md_escape(item_title(a))} |"
            )
    else:
        lines.append("No items require PaddleOCR.")
    lines.extend(["", "## Full >100MB Inventory", ""])
    lines.extend(
        [
            "| # | Size | PDF key | Parent | Layer | Route | Pages | Samples | 作者 | 年份 | 标题 |",
            "|---:|---:|---|---|---|---|---:|---|---|---|---|",
        ]
    )
    for i, item in enumerate(items, 1):
        a = item.attachment
        layer = "yes" if item.text_extractable else "no"
        lines.append(
            f"| {i} | {size_mb(item.size_bytes)} | {a.key} | {a.parent_key or ''} | "
            f"{layer} | {item.suggested_route} | {item.page_count or 'ERROR'} | "
            f"{md_escape(sample_summary(item))} | {md_escape(author_summary(a))} | "
            f"{item_year(a)} | {md_escape(item_title(a))} |"
        )
    return "\n".join(lines).rstrip() + "\n"


def enumerate_giant_pdfs(local: ZoteroLocalClient, state: StateDB) -> list[GiantPDF]:
    items: list[GiantPDF] = []
    for attachment in local.iter_pdf_attachments():
        item = inspect_attachment(attachment, state)
        if item:
            items.append(item)
    items.sort(key=lambda item: item.size_bytes, reverse=True)
    return items


def record_job(
    state: StateDB,
    *,
    item: GiantPDF,
    compressed_path: Path | None,
    original_md5: str | None,
    compressed_md5: str | None,
    compressed_bytes: int | None,
    compression_status: str,
    md_status: str | None = None,
    zotero_attachment_key: str | None = None,
    error: str | None = None,
) -> None:
    init_job_table(state.conn)
    ts = now_utc()
    state.conn.execute(
        """
        INSERT INTO giant_pdf_md_jobs (
            pdf_key, parent_key, original_path, compressed_path, original_md5,
            compressed_md5, original_bytes, compressed_bytes, page_count,
            compression_method, compression_status, md_status,
            zotero_attachment_key, error, created_at, updated_at
        ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
        ON CONFLICT(pdf_key) DO UPDATE SET
            parent_key=excluded.parent_key,
            original_path=excluded.original_path,
            compressed_path=excluded.compressed_path,
            original_md5=excluded.original_md5,
            compressed_md5=excluded.compressed_md5,
            original_bytes=excluded.original_bytes,
            compressed_bytes=excluded.compressed_bytes,
            page_count=excluded.page_count,
            compression_method=excluded.compression_method,
            compression_status=excluded.compression_status,
            md_status=excluded.md_status,
            zotero_attachment_key=excluded.zotero_attachment_key,
            error=excluded.error,
            updated_at=excluded.updated_at
        """,
        (
            item.attachment.key,
            item.attachment.parent_key,
            str(item.attachment.path),
            str(compressed_path) if compressed_path else None,
            original_md5,
            compressed_md5,
            item.size_bytes,
            compressed_bytes,
            item.page_count,
            "ghostscript:220dpi-color-gray-300dpi-mono",
            compression_status,
            md_status,
            zotero_attachment_key,
            error,
            ts,
            ts,
        ),
    )
    state.conn.commit()


def process_queue(items: list[GiantPDF], args: argparse.Namespace) -> None:
    config = get_config(zotero_tags=args.zotero_tag)
    configure_logging(config.output_root)
    state = StateDB(config.output_root / ".state" / "zotero_llm.sqlite3")
    init_job_table(state.conn)
    deadline = time.time() + args.max_runtime_minutes * 60
    remaining = config.max_ocr_pages_per_run
    processed = 0
    for item in items:
        if args.limit and processed >= args.limit:
            break
        if item.existing_md is not None:
            logging.info("Skip %s: existing completed Markdown %s", item.attachment.key, item.existing_md["output_path"])
            record_job(
                state,
                item=item,
                compressed_path=None,
                original_md5=md5_file(item.attachment.path),
                compressed_md5=None,
                compressed_bytes=None,
                compression_status="skipped_existing_md",
                md_status="skipped_existing_md",
                zotero_attachment_key=item.existing_md["zotero_attachment_key"],
            )
            continue
        try:
            record_job(
                state,
                item=item,
                compressed_path=None,
                original_md5=md5_file(item.attachment.path),
                compressed_md5=None,
                compressed_bytes=None,
                compression_status="not_applicable_preserve_scan_quality",
            )
            status, pages_used = process_attachment(
                attachment=item.attachment,
                config=config,
                state=state,
                page_spec=None,
                no_upload=args.no_upload,
                dry_run=False,
                force_route="paddle-ocr",
                deadline=deadline,
                ocr_pages_remaining=remaining,
            )
            remaining -= pages_used
            row = state.conn.execute(
                """
                SELECT zotero_attachment_key FROM documents
                WHERE pdf_key=? AND route='paddle-ocr'
                ORDER BY updated_at DESC LIMIT 1
                """,
                (item.attachment.key,),
            ).fetchone()
            record_job(
                state,
                item=item,
                compressed_path=None,
                original_md5=md5_file(item.attachment.path),
                compressed_md5=None,
                compressed_bytes=None,
                compression_status="not_applicable_preserve_scan_quality",
                md_status=status,
                zotero_attachment_key=row["zotero_attachment_key"] if row else None,
            )
            processed += 1
            if status == "partial_budget_exhausted":
                logging.info("Stopping after OCR page budget exhausted")
                break
        except (QuotaExhaustedError, DeadlineReached) as exc:
            logging.error("Stopping batch at %s: %s", item.attachment.key, exc)
            record_job(
                state,
                item=item,
                compressed_path=None,
                original_md5=md5_file(item.attachment.path),
                compressed_md5=None,
                compressed_bytes=None,
                compression_status="not_applicable_preserve_scan_quality",
                md_status="stopped",
                error=str(exc),
            )
            break
        except RetryableRemoteError as exc:
            logging.warning("Retryable remote error for %s: %s", item.attachment.key, exc)
            record_job(
                state,
                item=item,
                compressed_path=None,
                original_md5=md5_file(item.attachment.path),
                compressed_md5=None,
                compressed_bytes=None,
                compression_status="not_applicable_preserve_scan_quality",
                md_status="retryable_error",
                error=str(exc),
            )
            processed += 1
            continue
        except Exception as exc:
            logging.exception("Failed %s", item.attachment.key)
            record_job(
                state,
                item=item,
                compressed_path=None,
                original_md5=md5_file(item.attachment.path),
                compressed_md5=None,
                compressed_bytes=None,
                compression_status="not_applicable_preserve_scan_quality",
                md_status="failed",
                error=str(exc),
            )
            processed += 1
            continue


def main() -> int:
    parser = argparse.ArgumentParser(description="Process giant Zotero PDFs into PaddleOCR Markdown.")
    parser.add_argument("--list-only", action="store_true", help="Only write the report.")
    parser.add_argument("--no-upload", action="store_true", help="Generate Markdown locally without Zotero upload.")
    parser.add_argument("--limit", type=int, help="Limit pending OCR items processed.")
    parser.add_argument("--max-runtime-minutes", type=float, default=55.0)
    add_zotero_tag_argument(parser)
    args = parser.parse_args()

    config = get_config(zotero_tags=args.zotero_tag)
    state = StateDB(config.output_root / ".state" / "zotero_llm.sqlite3")
    init_job_table(state.conn)
    local = ZoteroLocalClient(config.zotero_local_api, config.zotero_storage, config.request_timeout)
    local.ping()
    items = enumerate_giant_pdfs(local, state)
    queue = [item for item in items if item.suggested_route == PENDING_ROUTE]
    REPORT_PATH.parent.mkdir(parents=True, exist_ok=True)
    REPORT_PATH.write_text(render_report(items, queue), encoding="utf-8")
    print(f"Wrote {REPORT_PATH}")
    print(
        "summary "
        f"total_gt_100mb={len(items)} "
        f"no_text={len(queue)} "
        f"existing_md={sum(1 for item in queue if item.existing_md is not None)} "
        f"pending={sum(1 for item in queue if item.existing_md is None)}"
    )
    if args.list_only:
        return 0
    process_queue(queue, args)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
