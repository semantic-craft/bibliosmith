#!/usr/bin/env python3.11
"""Convert selected non-law large PDFs to PaddleOCR-VL Markdown, then delete source PDFs.

Destructive step is gated: a source PDF attachment is deleted only after a
completed PaddleOCR Markdown row exists, the local Markdown file exists, and
the generated Markdown has a Zotero attachment key.
"""

from __future__ import annotations

import argparse
import datetime as dt
import logging
import sqlite3
import sys
import time
from dataclasses import dataclass
from pathlib import Path
from typing import Any

SCRIPT_DIR = Path(__file__).resolve().parent
APP_ROOT = SCRIPT_DIR.parent
sys.path.insert(0, str(SCRIPT_DIR))

from zotero_llm_worker import (  # noqa: E402
    Attachment,
    DeadlineReached,
    QuotaExhaustedError,
    RetryableRemoteError,
    StateDB,
    WorkerError,
    ZoteroLocalClient,
    ZoteroWebClient,
    add_zotero_tag_argument,
    configure_logging,
    get_config,
    process_attachment,
)


TARGETS: list[tuple[str, str]] = [
    ("PSMYUFKY", "蒯因著作集-第6卷"),
    ("SS73B9E8", "简明逻辑学导论 第10版"),
    ("UW2LAXRW", "简明逻辑学导论 第10版"),
    ("IY6KVVQ4", "蒯因著作集-第4卷"),
    ("C2QS8KF3", "蒯因著作集-第5卷"),
    ("GMGAVTK3", "蒯因著作集-第3卷"),
    ("FVNGYXSH", "认识与谬误"),
    ("GTC9FGFF", "形而上学"),
    ("SNR3WW9I", "蒯因著作集-第2卷"),
    ("X3KBP8VB", "中世纪哲学"),
    ("88RAIGNF", "当代知识论"),
    ("PYW8X63W", "蒯因著作集-第1卷"),
    ("BQYDHP5V", "当代语言哲学导论"),
    ("NX4NZ458", "逻辑与哲学"),
    ("YW9RJ3T7", "现代哲学简史"),
    ("TQXG6DLW", "逻辑与哲学"),
    ("MJYKP8BD", "中世纪哲学"),
    ("BURUE7PK", "文化哲學講演錄"),
]

REPORT_PATH = APP_ROOT / "reports" / "nonlaw_large_pdf_paddle_cleanup.md"


@dataclass
class JobSnapshot:
    pdf_key: str
    title: str
    status: str
    source_deleted: bool
    md_attachment_key: str | None
    md_path: str | None
    error: str | None
    updated_at: str | None


def now_utc() -> str:
    return dt.datetime.now(dt.UTC).replace(microsecond=0).isoformat()


def init_schema(conn: sqlite3.Connection) -> None:
    conn.executescript(
        """
        CREATE TABLE IF NOT EXISTS nonlaw_large_pdf_cleanup_jobs (
            pdf_key TEXT PRIMARY KEY,
            title TEXT NOT NULL,
            parent_key TEXT,
            source_path TEXT,
            source_deleted INTEGER NOT NULL DEFAULT 0,
            md_status TEXT,
            md_path TEXT,
            md_attachment_key TEXT,
            delete_status TEXT,
            error TEXT,
            created_at TEXT NOT NULL,
            updated_at TEXT NOT NULL
        );
        """
    )
    conn.commit()


def upsert_job(
    conn: sqlite3.Connection,
    *,
    pdf_key: str,
    title: str,
    parent_key: str | None = None,
    source_path: str | None = None,
    source_deleted: bool | None = None,
    md_status: str | None = None,
    md_path: str | None = None,
    md_attachment_key: str | None = None,
    delete_status: str | None = None,
    error: str | None = None,
) -> None:
    ts = now_utc()
    conn.execute(
        """
        INSERT INTO nonlaw_large_pdf_cleanup_jobs (
            pdf_key, title, parent_key, source_path, source_deleted, md_status,
            md_path, md_attachment_key, delete_status, error, created_at, updated_at
        ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
        ON CONFLICT(pdf_key) DO UPDATE SET
            title=excluded.title,
            parent_key=COALESCE(excluded.parent_key, parent_key),
            source_path=COALESCE(excluded.source_path, source_path),
            source_deleted=CASE
                WHEN excluded.source_deleted = 1 THEN 1
                ELSE source_deleted
            END,
            md_status=COALESCE(excluded.md_status, md_status),
            md_path=COALESCE(excluded.md_path, md_path),
            md_attachment_key=COALESCE(excluded.md_attachment_key, md_attachment_key),
            delete_status=COALESCE(excluded.delete_status, delete_status),
            error=excluded.error,
            updated_at=excluded.updated_at
        """,
        (
            pdf_key,
            title,
            parent_key,
            source_path,
            1 if source_deleted else 0,
            md_status,
            md_path,
            md_attachment_key,
            delete_status,
            error,
            ts,
            ts,
        ),
    )
    conn.commit()


def completed_paddle_markdown(state: StateDB, pdf_key: str) -> sqlite3.Row | None:
    rows = state.conn.execute(
        """
        SELECT *
        FROM documents
        WHERE pdf_key=? AND route='paddle-ocr' AND status='completed'
        ORDER BY updated_at DESC
        """,
        (pdf_key,),
    ).fetchall()
    for row in rows:
        output_path = row["output_path"]
        if output_path and Path(output_path).exists() and row["zotero_attachment_key"]:
            if Path(output_path).stat().st_size > 0:
                return row
    return None


def delete_source_pdf(
    *,
    config: Any,
    state: StateDB,
    pdf_key: str,
    title: str,
    attachment: Attachment | None,
    row: sqlite3.Row,
) -> None:
    previous = state.conn.execute(
        "SELECT source_deleted FROM nonlaw_large_pdf_cleanup_jobs WHERE pdf_key=?",
        (pdf_key,),
    ).fetchone()
    if previous and int(previous["source_deleted"] or 0) == 1:
        return
    ZoteroWebClient(config).delete_item(pdf_key)
    upsert_job(
        state.conn,
        pdf_key=pdf_key,
        title=title,
        parent_key=attachment.parent_key if attachment else row["parent_key"],
        source_path=str(attachment.path) if attachment else None,
        source_deleted=True,
        md_status="completed",
        md_path=row["output_path"],
        md_attachment_key=row["zotero_attachment_key"],
        delete_status="deleted_source_pdf",
        error=None,
    )


def read_snapshots(conn: sqlite3.Connection) -> list[JobSnapshot]:
    rows = conn.execute(
        """
        SELECT pdf_key, title, source_deleted, md_status, md_path,
               md_attachment_key, delete_status, error, updated_at
        FROM nonlaw_large_pdf_cleanup_jobs
        ORDER BY instr(?, pdf_key)
        """,
        (",".join(key for key, _ in TARGETS),),
    ).fetchall()
    by_key = {row["pdf_key"]: row for row in rows}
    out: list[JobSnapshot] = []
    for key, title in TARGETS:
        row = by_key.get(key)
        if not row:
            out.append(JobSnapshot(key, title, "pending", False, None, None, None, None))
            continue
        status = row["delete_status"] or row["md_status"] or "pending"
        out.append(
            JobSnapshot(
                pdf_key=key,
                title=title,
                status=status,
                source_deleted=bool(row["source_deleted"]),
                md_attachment_key=row["md_attachment_key"],
                md_path=row["md_path"],
                error=row["error"],
                updated_at=row["updated_at"],
            )
        )
    return out


def write_report(conn: sqlite3.Connection) -> None:
    REPORT_PATH.parent.mkdir(parents=True, exist_ok=True)
    snapshots = read_snapshots(conn)
    deleted = sum(1 for item in snapshots if item.source_deleted)
    completed = sum(1 for item in snapshots if item.md_attachment_key)
    failed = sum(1 for item in snapshots if item.error)
    lines = [
        "# Non-Law Large PDF PaddleOCR Cleanup",
        "",
        f"- Updated: `{dt.datetime.now().astimezone().isoformat(timespec='seconds')}`",
        "- Scope: 18 selected >100MB non-law PDFs, excluding `ZWM5RY6V`.",
        "- Rule: delete source PDF attachment only after PaddleOCR-VL Markdown is completed and uploaded.",
        "",
        "## Summary",
        "",
        f"- Target PDFs: `{len(TARGETS)}`",
        f"- Markdown attachments present: `{completed}`",
        f"- Source PDFs deleted: `{deleted}`",
        f"- Failed/error: `{failed}`",
        "",
        "## Jobs",
        "",
        "| # | PDF key | Title | Status | Deleted source | Markdown attachment | Markdown path | Error |",
        "|---:|---|---|---|---|---|---|---|",
    ]
    for index, item in enumerate(snapshots, 1):
        def esc(value: object) -> str:
            return str(value or "").replace("|", "\\|").replace("\n", " ")

        lines.append(
            f"| {index} | {item.pdf_key} | {esc(item.title)} | {esc(item.status)} | "
            f"{'yes' if item.source_deleted else 'no'} | {esc(item.md_attachment_key)} | "
            f"{esc(item.md_path)} | {esc(item.error)} |"
        )
    REPORT_PATH.write_text("\n".join(lines) + "\n", encoding="utf-8")


def process_targets(args: argparse.Namespace) -> int:
    config = get_config(zotero_tags=args.zotero_tag)
    configure_logging(config.output_root)
    state = StateDB(config.output_root / ".state" / "zotero_llm.sqlite3")
    init_schema(state.conn)
    local = ZoteroLocalClient(config.zotero_local_api, config.zotero_storage, config.request_timeout)
    local.ping()
    deadline = time.time() + args.max_runtime_minutes * 60
    ocr_pages_remaining = config.max_ocr_pages_per_run

    for key, title in TARGETS:
        write_report(state.conn)
        if time.time() + 30 > deadline:
            logging.info("Deadline reached before next target")
            break
        existing = state.conn.execute(
            "SELECT source_deleted FROM nonlaw_large_pdf_cleanup_jobs WHERE pdf_key=?",
            (key,),
        ).fetchone()
        if existing and int(existing["source_deleted"] or 0) == 1:
            logging.info("SKIP deleted source PDF %s", key)
            continue
        attachment: Attachment | None = None
        try:
            attachment = local.get_pdf_attachment(key)
            upsert_job(
                state.conn,
                pdf_key=key,
                title=title,
                parent_key=attachment.parent_key,
                source_path=str(attachment.path),
                error=None,
            )
        except Exception as exc:
            row = completed_paddle_markdown(state, key)
            if row:
                delete_source_pdf(config=config, state=state, pdf_key=key, title=title, attachment=None, row=row)
                continue
            upsert_job(
                state.conn,
                pdf_key=key,
                title=title,
                md_status="source_missing_before_completed_md",
                delete_status="not_deleted",
                error=f"Could not read source PDF attachment: {exc}",
            )
            logging.warning("Could not read source attachment %s: %s", key, exc)
            continue

        row = completed_paddle_markdown(state, key)
        if not row:
            try:
                status, pages_used = process_attachment(
                    attachment=attachment,
                    config=config,
                    state=state,
                    page_spec=None,
                    no_upload=False,
                    dry_run=False,
                    force_route="paddle-ocr",
                    deadline=deadline,
                    ocr_pages_remaining=ocr_pages_remaining,
                )
                ocr_pages_remaining -= pages_used
                upsert_job(state.conn, pdf_key=key, title=title, md_status=status, error=None)
            except (QuotaExhaustedError, DeadlineReached) as exc:
                upsert_job(state.conn, pdf_key=key, title=title, md_status="stopped", error=str(exc))
                logging.error("Stopping at %s: %s", key, exc)
                break
            except RetryableRemoteError as exc:
                upsert_job(state.conn, pdf_key=key, title=title, md_status="retryable_error", error=str(exc))
                logging.warning("Retryable remote error for %s: %s", key, exc)
                continue
            except Exception as exc:
                upsert_job(state.conn, pdf_key=key, title=title, md_status="failed", error=str(exc))
                logging.exception("Failed %s", key)
                continue
            row = completed_paddle_markdown(state, key)

        if row:
            upsert_job(
                state.conn,
                pdf_key=key,
                title=title,
                parent_key=attachment.parent_key,
                source_path=str(attachment.path),
                md_status="completed",
                md_path=row["output_path"],
                md_attachment_key=row["zotero_attachment_key"],
                error=None,
            )
            try:
                delete_source_pdf(config=config, state=state, pdf_key=key, title=title, attachment=attachment, row=row)
                logging.info("Deleted source PDF %s after Markdown %s", key, row["zotero_attachment_key"])
            except Exception as exc:
                upsert_job(
                    state.conn,
                    pdf_key=key,
                    title=title,
                    delete_status="delete_failed",
                    error=f"Markdown completed but source delete failed: {exc}",
                )
                logging.exception("Delete failed %s", key)
        write_report(state.conn)
    write_report(state.conn)
    return 0


def main() -> int:
    parser = argparse.ArgumentParser(description="PaddleOCR selected non-law large PDFs, then delete source PDFs.")
    parser.add_argument("--max-runtime-minutes", type=float, default=55.0)
    add_zotero_tag_argument(parser)
    return process_targets(parser.parse_args())


if __name__ == "__main__":
    raise SystemExit(main())
