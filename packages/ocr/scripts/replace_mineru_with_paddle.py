#!/usr/bin/env python3.11
"""Replace legacy MinerU Markdown attachments with PaddleOCR-VL Markdown.

The migration is intentionally conservative:
- discover Markdown attachments that were produced by MinerU;
- recover the source PDF from `ocr-source:<pdf_key>`, old MinerU job tables, or
  the sole sibling PDF under the same parent item;
- generate/upload PaddleOCR Markdown through the shared OCR worker;
- delete the old MinerU attachment only after a completed Paddle row exists.
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
    ROUTE_NEEDS_MINERU,
    add_zotero_tag_argument,
    configure_logging,
    get_config,
    install_dependencies_check,
    md5_file,
    pdf_page_count,
    process_attachment,
    reconcile_staged_conversion,
    route_attachment,
    source_key_from_provenance_note,
)


REPORT_PATH = APP_ROOT / "reports" / "mineru_to_paddle_replacement.md"
TABLE_NAME = "mineru_to_paddle_jobs"


@dataclass(frozen=True)
class MineruCandidate:
    md_key: str
    parent_key: str | None
    title: str
    filename: str
    source_pdf_key: str | None
    detection_reason: str


@dataclass(frozen=True)
class Job:
    md_key: str
    source_pdf_key: str | None
    parent_key: str | None
    title: str
    filename: str
    status: str


def now_utc() -> str:
    return dt.datetime.now(dt.UTC).replace(microsecond=0).isoformat()


def md_escape(value: object) -> str:
    return str(value or "").replace("|", "\\|").replace("\n", " ")


def tag_values(data: dict[str, Any]) -> list[str]:
    values: list[str] = []
    for tag in data.get("tags") or []:
        value = tag.get("tag") if isinstance(tag, dict) else tag
        if value:
            values.append(str(value))
    return values


def is_markdown_attachment(data: dict[str, Any]) -> bool:
    filename = str(data.get("filename") or "")
    return data.get("contentType") == "text/markdown" or filename.lower().endswith(".md")


def is_pdf_attachment(data: dict[str, Any]) -> bool:
    return data.get("contentType") == "application/pdf"


def source_from_tags(tags: list[str]) -> str | None:
    for tag in tags:
        if tag.startswith("ocr-source:"):
            value = tag.split(":", 1)[1].strip()
            if value:
                return value
    return None


def fetch_attachment_items(local: ZoteroLocalClient) -> list[dict[str, Any]]:
    items: list[dict[str, Any]] = []
    start = 0
    while True:
        batch = local.get("items", itemType="attachment", limit=100, start=start, format="json")
        if not batch:
            break
        items.extend(batch)
        start += len(batch)
        if len(batch) < 100:
            break
    return items


def read_old_mineru_maps(conn: sqlite3.Connection) -> tuple[dict[str, str], dict[str, str]]:
    md_to_pdf: dict[str, str] = {}
    reason_by_md: dict[str, str] = {}
    tables = {
        row["name"]
        for row in conn.execute(
            "SELECT name FROM sqlite_master WHERE type='table'"
        ).fetchall()
    }
    if "mineru_law_politics_jobs" in tables:
        rows = conn.execute(
            """
            SELECT md_attachment_key, pdf_key
            FROM mineru_law_politics_jobs
            WHERE md_attachment_key IS NOT NULL AND pdf_key IS NOT NULL
            """
        ).fetchall()
        for row in rows:
            md_to_pdf[row["md_attachment_key"]] = row["pdf_key"]
            reason_by_md[row["md_attachment_key"]] = "mineru_law_politics_jobs"
    if "documents" in tables:
        rows = conn.execute(
            """
            SELECT zotero_attachment_key, pdf_key
            FROM documents
            WHERE route='mineru-open-api'
              AND zotero_attachment_key IS NOT NULL
              AND pdf_key IS NOT NULL
            """
        ).fetchall()
        for row in rows:
            md_to_pdf.setdefault(row["zotero_attachment_key"], row["pdf_key"])
            reason_by_md.setdefault(row["zotero_attachment_key"], "documents.route=mineru-open-api")
    return md_to_pdf, reason_by_md


def collect_mineru_candidates(
    local: ZoteroLocalClient,
    conn: sqlite3.Connection,
) -> list[MineruCandidate]:
    md_to_pdf, reason_by_md = read_old_mineru_maps(conn)
    attachment_items = fetch_attachment_items(local)
    pdfs_by_parent: dict[str | None, list[str]] = {}
    markdown_candidates: list[MineruCandidate] = []

    for item in attachment_items:
        data = item.get("data", {})
        parent_key = data.get("parentItem")
        if is_pdf_attachment(data):
            pdfs_by_parent.setdefault(parent_key, []).append(data.get("key"))

    for item in attachment_items:
        data = item.get("data", {})
        if not is_markdown_attachment(data):
            continue
        md_key = data.get("key")
        if not md_key:
            continue
        tags = tag_values(data)
        lower_tags = " ".join(tags).casefold()
        reason_parts: list[str] = []
        if "mineru" in lower_tags:
            reason_parts.append("mineru tag")
        if md_key in md_to_pdf:
            reason_parts.append(reason_by_md.get(md_key, "old mineru job table"))
        title = str(data.get("title") or "")
        filename = str(data.get("filename") or "")
        if "mineru" in f"{title} {filename}".casefold():
            reason_parts.append("mineru title/filename")
        if not reason_parts:
            continue

        source_pdf_key = source_key_from_provenance_note(data.get("note")) or source_from_tags(tags) or md_to_pdf.get(md_key)
        parent_key = data.get("parentItem")
        if not source_pdf_key:
            siblings = pdfs_by_parent.get(parent_key, [])
            if len(siblings) == 1:
                source_pdf_key = siblings[0]
                reason_parts.append("sole sibling PDF fallback")
        markdown_candidates.append(
            MineruCandidate(
                md_key=md_key,
                parent_key=parent_key,
                title=title or filename or md_key,
                filename=filename,
                source_pdf_key=source_pdf_key,
                detection_reason=", ".join(reason_parts),
            )
        )

    markdown_candidates.sort(key=lambda item: (item.source_pdf_key or "", item.title.casefold(), item.md_key))
    return markdown_candidates


def init_schema(conn: sqlite3.Connection) -> None:
    conn.executescript(
        f"""
        CREATE TABLE IF NOT EXISTS {TABLE_NAME} (
            md_key TEXT PRIMARY KEY,
            source_pdf_key TEXT,
            parent_key TEXT,
            title TEXT,
            filename TEXT,
            detection_reason TEXT,
            source_parent_key TEXT,
            source_path TEXT,
            source_md5 TEXT,
            page_count INTEGER,
            status TEXT NOT NULL DEFAULT 'pending',
            paddle_attachment_key TEXT,
            paddle_output_path TEXT,
            pages_used INTEGER NOT NULL DEFAULT 0,
            error TEXT,
            created_at TEXT NOT NULL,
            updated_at TEXT NOT NULL
        );
        """
    )
    conn.commit()


def seed_candidates(conn: sqlite3.Connection, candidates: list[MineruCandidate]) -> None:
    ts = now_utc()
    for candidate in candidates:
        existing = conn.execute(
            f"SELECT status FROM {TABLE_NAME} WHERE md_key=?",
            (candidate.md_key,),
        ).fetchone()
        existing_status = str(existing["status"] or "") if existing else ""
        keep_status = existing and (
            existing_status.startswith("replaced")
            or existing_status.startswith("blocked")
            or existing_status == "in_progress"
        )
        status_expr = "status" if keep_status else "'pending'"
        conn.execute(
            f"""
            INSERT INTO {TABLE_NAME} (
                md_key, source_pdf_key, parent_key, title, filename, detection_reason,
                status, created_at, updated_at
            ) VALUES (?, ?, ?, ?, ?, ?, 'pending', ?, ?)
            ON CONFLICT(md_key) DO UPDATE SET
                source_pdf_key=excluded.source_pdf_key,
                parent_key=excluded.parent_key,
                title=excluded.title,
                filename=excluded.filename,
                detection_reason=excluded.detection_reason,
                status={status_expr},
                error=NULL,
                updated_at=excluded.updated_at
            """,
            (
                candidate.md_key,
                candidate.source_pdf_key,
                candidate.parent_key,
                candidate.title,
                candidate.filename,
                candidate.detection_reason,
                ts,
                ts,
            ),
        )
    conn.commit()


def claim_job(conn: sqlite3.Connection, md_key: str, *, stale_minutes: float) -> bool:
    cutoff = (
        dt.datetime.now(dt.UTC) - dt.timedelta(minutes=stale_minutes)
    ).replace(microsecond=0).isoformat()
    conn.execute("BEGIN IMMEDIATE")
    try:
        row = conn.execute(
            f"SELECT status, updated_at FROM {TABLE_NAME} WHERE md_key=?",
            (md_key,),
        ).fetchone()
        if not row:
            conn.rollback()
            return False
        status = str(row["status"] or "")
        updated_at = str(row["updated_at"] or "")
        if status.startswith("replaced") or status.startswith("blocked"):
            conn.rollback()
            return False
        if status == "in_progress" and updated_at >= cutoff:
            conn.rollback()
            return False
        conn.execute(
            f"""
            UPDATE {TABLE_NAME}
            SET status='in_progress',
                error=NULL,
                updated_at=?
            WHERE md_key=?
            """,
            (now_utc(), md_key),
        )
        conn.commit()
        return True
    except Exception:
        conn.rollback()
        raise


def update_job(
    conn: sqlite3.Connection,
    md_key: str,
    *,
    status: str,
    source_parent_key: str | None = None,
    source_path: Path | None = None,
    source_md5: str | None = None,
    page_count: int | None = None,
    paddle_attachment_key: str | None = None,
    paddle_output_path: str | Path | None = None,
    pages_used: int | None = None,
    error: str | None = None,
) -> None:
    conn.execute(
        f"""
        UPDATE {TABLE_NAME}
        SET status=?,
            source_parent_key=COALESCE(?, source_parent_key),
            source_path=COALESCE(?, source_path),
            source_md5=COALESCE(?, source_md5),
            page_count=COALESCE(?, page_count),
            paddle_attachment_key=COALESCE(?, paddle_attachment_key),
            paddle_output_path=COALESCE(?, paddle_output_path),
            pages_used=COALESCE(?, pages_used),
            error=?,
            updated_at=?
        WHERE md_key=?
        """,
        (
            status,
            source_parent_key,
            str(source_path) if source_path else None,
            source_md5,
            page_count,
            paddle_attachment_key,
            str(paddle_output_path) if paddle_output_path else None,
            pages_used,
            error,
            now_utc(),
            md_key,
        ),
    )
    conn.commit()


def load_jobs(conn: sqlite3.Connection, *, include_done: bool = False) -> list[Job]:
    status_filter = "" if include_done else "WHERE status NOT LIKE 'replaced%'"
    rows = conn.execute(
        f"""
        SELECT md_key, source_pdf_key, parent_key, title, filename, status
        FROM {TABLE_NAME}
        {status_filter}
        ORDER BY COALESCE(page_count, 100000), title COLLATE NOCASE, md_key
        """
    ).fetchall()
    return [
        Job(
            md_key=row["md_key"],
            source_pdf_key=row["source_pdf_key"],
            parent_key=row["parent_key"],
            title=row["title"] or row["filename"] or row["md_key"],
            filename=row["filename"] or "",
            status=row["status"],
        )
        for row in rows
    ]


def paddle_row_for(
    state: StateDB,
    *,
    attachment: Attachment,
    page_count: int,
    zotero: ZoteroWebClient | None = None,
) -> sqlite3.Row | None:
    source_md5 = md5_file(attachment.path)
    row = state.completed(attachment.key, source_md5)
    outcome = reconcile_staged_conversion(
        attachment=attachment,
        state=state,
        page_count=page_count,
    )
    if (
        row is None
        or not outcome.accepted
        or outcome.evidence is None
        or outcome.evidence.route != "paddle-ocr"
        or not outcome.evidence.markdown_attachment_key
    ):
        return None
    key = outcome.evidence.markdown_attachment_key
    if zotero is not None and not web_item_exists(zotero, key):
        return None
    return row


def web_item_exists(zotero: ZoteroWebClient, item_key: str) -> bool:
    last_error: Exception | None = None
    for attempt in range(4):
        try:
            response = zotero.session.get(f"{zotero.base_url}/items/{item_key}", timeout=zotero.timeout)
        except Exception as exc:
            last_error = exc
            time.sleep(min(2**attempt, 8))
            continue
        if response.status_code == 200:
            return True
        if response.status_code == 404:
            return False
        if response.status_code in {429, 500, 502, 503, 504}:
            last_error = WorkerError(f"Zotero item lookup retryable: {response.status_code} {response.text}")
            time.sleep(min(2**attempt, 8))
            continue
        raise WorkerError(f"Zotero item lookup failed: {response.status_code} {response.text}")
    raise RetryableRemoteError(f"Zotero item lookup retry failed for {item_key}: {last_error}")


def delete_old_mineru(zotero: ZoteroWebClient, old_md_key: str) -> str:
    existed_before = web_item_exists(zotero, old_md_key)
    if existed_before:
        zotero.delete_item(old_md_key)
    return "deleted_old_mineru" if existed_before else "old_mineru_already_absent"


def read_snapshots(conn: sqlite3.Connection) -> list[sqlite3.Row]:
    return conn.execute(
        f"""
        SELECT *
        FROM {TABLE_NAME}
        ORDER BY
            CASE
                WHEN status LIKE 'replaced%' THEN 4
                WHEN status LIKE 'blocked%' THEN 3
                WHEN status IN ('failed', 'retryable_error') THEN 2
                ELSE 1
            END,
            COALESCE(page_count, 100000),
            title COLLATE NOCASE,
            md_key
        """
    ).fetchall()


def write_report(conn: sqlite3.Connection) -> None:
    REPORT_PATH.parent.mkdir(parents=True, exist_ok=True)
    rows = read_snapshots(conn)
    status_counts: dict[str, int] = {}
    total_pages = 0
    replaced_pages = 0
    for row in rows:
        status = row["status"]
        status_counts[status] = status_counts.get(status, 0) + 1
        pages = int(row["page_count"] or 0)
        total_pages += pages
        if str(status).startswith("replaced"):
            replaced_pages += pages

    lines = [
        "# MinerU to PaddleOCR-VL Replacement",
        "",
        f"- Updated: `{dt.datetime.now().astimezone().isoformat(timespec='seconds')}`",
        "- Scope: Zotero Markdown attachments detected as MinerU output.",
        "- Rule: old MinerU Markdown is deleted only after completed PaddleOCR-VL Markdown exists for the same source PDF.",
        "",
        "## Summary",
        "",
        f"- MinerU Markdown candidates: `{len(rows)}`",
        f"- Source pages recorded: `{total_pages}`",
        f"- Replaced pages: `{replaced_pages}`",
    ]
    for status, count in sorted(status_counts.items()):
        lines.append(f"- `{status}`: `{count}`")
    lines.extend(
        [
            "",
            "## Jobs",
            "",
            "| # | MinerU MD | Source PDF | Parent | Pages | Status | Paddle MD | Title | Source path | Error |",
            "|---:|---|---|---|---:|---|---|---|---|---|",
        ]
    )
    for index, row in enumerate(rows, 1):
        lines.append(
            "| "
            + " | ".join(
                [
                    str(index),
                    md_escape(row["md_key"]),
                    md_escape(row["source_pdf_key"]),
                    md_escape(row["parent_key"]),
                    md_escape(row["page_count"]),
                    md_escape(row["status"]),
                    md_escape(row["paddle_attachment_key"]),
                    md_escape(row["title"] or row["filename"]),
                    md_escape(row["source_path"]),
                    md_escape(row["error"]),
                ]
            )
            + " |"
        )
    REPORT_PATH.write_text("\n".join(lines) + "\n", encoding="utf-8")


def process_jobs(args: argparse.Namespace) -> int:
    config = get_config(zotero_tags=args.zotero_tag)
    config.output_root.mkdir(parents=True, exist_ok=True)
    (config.output_root / ".state").mkdir(parents=True, exist_ok=True)
    configure_logging(config.output_root)
    install_dependencies_check()

    state = StateDB(config.output_root / ".state" / "zotero_llm.sqlite3")
    state.conn.execute("PRAGMA busy_timeout=10000")
    init_schema(state.conn)
    local = ZoteroLocalClient(config.zotero_local_api, config.zotero_storage, config.request_timeout)
    local.ping()
    zotero = ZoteroWebClient(config)

    candidates = collect_mineru_candidates(local, state.conn)
    seed_candidates(state.conn, candidates)
    write_report(state.conn)
    logging.info("Discovered %s MinerU Markdown candidates", len(candidates))
    if args.report_only or args.dry_run:
        return 0

    deadline = time.time() + args.max_runtime_minutes * 60
    ocr_pages_remaining = config.max_ocr_pages_per_run
    processed = 0

    for job in load_jobs(state.conn):
        if args.limit and processed >= args.limit:
            break
        write_report(state.conn)
        if time.time() + 30 > deadline:
            logging.info("Deadline reached before next MinerU replacement job")
            break
        if not claim_job(state.conn, job.md_key, stale_minutes=args.stale_in_progress_minutes):
            continue
        if not job.source_pdf_key:
            update_job(
                state.conn,
                job.md_key,
                status="blocked_missing_source_pdf",
                error="Could not infer source PDF key",
            )
            continue

        try:
            attachment = local.get_pdf_attachment(job.source_pdf_key)
            source_md5 = md5_file(attachment.path)
            page_count = pdf_page_count(attachment.path)
            update_job(
                state.conn,
                job.md_key,
                status="in_progress",
                source_parent_key=attachment.parent_key,
                source_path=attachment.path,
                source_md5=source_md5,
                page_count=page_count,
                error=None,
            )
        except Exception as exc:
            update_job(
                state.conn,
                job.md_key,
                status="blocked_missing_source_pdf",
                error=f"Could not read source PDF {job.source_pdf_key}: {exc}",
            )
            logging.warning("Missing source PDF for MinerU MD %s source=%s: %s", job.md_key, job.source_pdf_key, exc)
            continue

        if job.parent_key and attachment.parent_key and job.parent_key != attachment.parent_key:
            update_job(
                state.conn,
                job.md_key,
                status="blocked_parent_mismatch",
                error=f"MinerU parent {job.parent_key} != source PDF parent {attachment.parent_key}",
            )
            logging.warning("Parent mismatch for %s source=%s", job.md_key, job.source_pdf_key)
            continue

        natural_route, natural_reason, _ = route_attachment(attachment, attachment.path, page_count, config)
        if natural_route == ROUTE_NEEDS_MINERU:
            update_job(
                state.conn,
                job.md_key,
                status="blocked_dirty_text_layer",
                page_count=page_count,
                error=f"Keeping MinerU output; source PDF needs MinerU/quality review: {natural_reason}",
            )
            logging.warning("Keeping MinerU MD %s because source route is %s: %s", job.md_key, natural_route, natural_reason)
            continue

        try:
            row = paddle_row_for(
                state,
                attachment=attachment,
                page_count=page_count,
                zotero=zotero,
            )
        except Exception as exc:
            update_job(
                state.conn,
                job.md_key,
                status="retryable_error",
                error=f"Could not verify existing Paddle row: {exc}",
            )
            logging.warning("Could not verify existing Paddle row for %s: %s", job.md_key, exc)
            continue
        if row is not None:
            try:
                delete_status = delete_old_mineru(zotero, job.md_key)
            except Exception as exc:
                update_job(
                    state.conn,
                    job.md_key,
                    status="paddle_completed_delete_pending",
                    paddle_attachment_key=row["zotero_attachment_key"],
                    paddle_output_path=row["output_path"],
                    page_count=row["page_count"],
                    pages_used=0,
                    error=f"Paddle exists but old MinerU delete/check failed: {exc}",
                )
                logging.warning("Paddle exists but delete/check failed for %s: %s", job.md_key, exc)
                continue
            update_job(
                state.conn,
                job.md_key,
                status=f"replaced_existing_paddle:{delete_status}",
                paddle_attachment_key=row["zotero_attachment_key"],
                paddle_output_path=row["output_path"],
                page_count=row["page_count"],
                pages_used=0,
                error=None,
            )
            logging.info(
                "Replaced old MinerU %s using existing Paddle %s",
                job.md_key,
                row["zotero_attachment_key"],
            )
            processed += 1
            continue

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
        except (QuotaExhaustedError, DeadlineReached) as exc:
            update_job(
                state.conn,
                job.md_key,
                status="stopped_quota_or_deadline",
                error=str(exc),
            )
            logging.error("Stopping replacement batch at %s: %s", job.md_key, exc)
            break
        except RetryableRemoteError as exc:
            update_job(
                state.conn,
                job.md_key,
                status="retryable_error",
                error=str(exc),
            )
            logging.warning("Retryable remote error for %s: %s", job.md_key, exc)
            continue
        except Exception as exc:
            update_job(
                state.conn,
                job.md_key,
                status="failed",
                error=str(exc)[:2000],
            )
            logging.exception("Failed to replace MinerU MD %s", job.md_key)
            continue

        try:
            row = paddle_row_for(
                state,
                attachment=attachment,
                page_count=page_count,
                zotero=zotero,
            )
        except Exception as exc:
            update_job(
                state.conn,
                job.md_key,
                status="paddle_completed_delete_pending",
                page_count=page_count,
                pages_used=pages_used,
                error=f"Paddle worker returned {status}; Zotero verification/delete deferred: {exc}",
            )
            logging.warning("Deferred Zotero verification/delete for %s: %s", job.md_key, exc)
            continue
        if row is None:
            update_job(
                state.conn,
                job.md_key,
                status="failed",
                pages_used=pages_used,
                error=f"Paddle worker returned {status} but no completed Paddle row was found",
            )
            logging.error("No completed Paddle row after processing source %s", attachment.key)
            continue

        try:
            delete_status = delete_old_mineru(zotero, job.md_key)
        except Exception as exc:
            update_job(
                state.conn,
                job.md_key,
                status="paddle_completed_delete_pending",
                paddle_attachment_key=row["zotero_attachment_key"],
                paddle_output_path=row["output_path"],
                page_count=row["page_count"],
                pages_used=pages_used,
                error=f"Paddle completed but old MinerU delete/check failed: {exc}",
            )
            logging.warning("Paddle completed but delete/check failed for %s: %s", job.md_key, exc)
            continue
        update_job(
            state.conn,
            job.md_key,
            status=f"replaced:{delete_status}",
            paddle_attachment_key=row["zotero_attachment_key"],
            paddle_output_path=row["output_path"],
            page_count=row["page_count"],
            pages_used=pages_used,
            error=None,
        )
        processed += 1
        logging.info(
            "Replaced old MinerU %s with Paddle %s pages_used=%s",
            job.md_key,
            row["zotero_attachment_key"],
            pages_used,
        )
        if status == "partial_budget_exhausted":
            logging.info("Stopping after OCR page budget was exhausted")
            break

    write_report(state.conn)
    return 0


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(description="Replace MinerU Markdown attachments with PaddleOCR-VL Markdown.")
    parser.add_argument("--dry-run", action="store_true", help="Alias for --report-only.")
    parser.add_argument("--report-only", action="store_true", help="Discover candidates and write report without OCR/deletion.")
    parser.add_argument("--limit", type=int, help="Maximum number of MinerU Markdown attachments to replace in this run.")
    parser.add_argument("--max-runtime-minutes", type=float, default=55.0)
    parser.add_argument("--stale-in-progress-minutes", type=float, default=30.0)
    add_zotero_tag_argument(parser)
    return parser


def main() -> int:
    return process_jobs(build_parser().parse_args())


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except KeyboardInterrupt:
        raise SystemExit(130)
    except Exception as exc:
        logging.exception("Fatal error: %s", exc)
        raise SystemExit(1)
