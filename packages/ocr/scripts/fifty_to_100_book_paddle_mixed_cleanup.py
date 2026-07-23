#!/usr/bin/env python3.11
"""Convert current 50-100MB Zotero book PDFs to PaddleOCR-VL Markdown.

Targets are seeded from the live Zotero local API on first run and then tracked
in SQLite so deleted source PDFs remain visible in the report.

Delete policy mirrors the >100MB cleanup:
- clear-law: keep original PDF
- adjacent-delete / nonlaw-delete: delete original PDF after Markdown upload
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
    ZoteroLocalClient,
    ZoteroWebClient,
    add_zotero_tag_argument,
    configure_logging,
    creator_name,
    get_config,
    item_year,
    process_attachment,
)


MIN_SIZE_MB = 50.0
MAX_SIZE_MB = 100.0
REPORT_PATH = APP_ROOT / "reports" / "fifty_to_100_book_pdf_paddle_mixed_cleanup.md"
TABLE_NAME = "fifty_to_100_book_pdf_cleanup_jobs"

CLEAR_LAW_KEYS = {
    "YQM8UV72",  # Courts, Privacy and Data Protection in the Digital Environment
    "9CSQMHEX",  # 读懂法理学
    "IVF844P8",  # 法学方法论
    "B8MJ4MRT",  # 解释与法律理论
    "5KM24QU5",  # 法律与分歧
    "BDCTTPTR",  # Advanced Introduction to Empirical Legal Research
    "96ZTQIWW",  # The Cultural Life of Intellectual Properties...
    "V8T5NF5G",  # 中国法学教育的“系统集成”改革
    "HKM4MIVL",  # 法哲学
    "6LK33WP4",  # 刑事法與憲法的對話
    "GGUINU3E",  # 法律东方主义
    "JCLUAS6Z",  # The Oxford Handbook of Jurisprudence and Philosophy of Law
    "U5TRMZJF",  # 法律、实用主义与民主
    "4LTDQH6W",  # Advanced Introduction to Legal Research Methods
}

ADJACENT_DELETE_KEYS = {
    "JJTB7TLL",  # 宗教与公共理性
    "RUZMNPCI",  # 传统中国日常生活中的协商——中古契约研究
    "6N2DS6DW",  # 大师学述 富勒
}

LAW_KEYWORDS = (
    "法律",
    "法学",
    "法理",
    "法哲",
    "法治",
    "法與",
    "法与",
    "刑事法",
    "憲法",
    "宪法",
    "知识产权",
    "著作权",
    "专利",
    "法科",
    "案例",
    "jurisprudence",
    "legal",
    "law",
    "courts",
    "data protection",
    "intellectual propert",
)


@dataclass(frozen=True)
class Target:
    pdf_key: str
    title: str
    classification: str
    delete_policy: str
    size_mb: float
    parent_key: str | None
    authors: str
    year: str
    source_path: str | None


@dataclass
class JobSnapshot:
    pdf_key: str
    title: str
    classification: str
    delete_policy: str
    size_mb: float | None
    status: str
    source_deleted: bool
    md_attachment_key: str | None
    md_path: str | None
    error: str | None
    parent_key: str | None
    authors: str | None
    year: str | None
    source_path: str | None
    updated_at: str | None


def now_utc() -> str:
    return dt.datetime.now(dt.UTC).replace(microsecond=0).isoformat()


def md_escape(value: object) -> str:
    return str(value or "").replace("|", "\\|").replace("\n", " ")


def author_summary(attachment: Attachment) -> str:
    names = [creator_name(creator) for creator in attachment.parent_creators]
    names = [name for name in names if name]
    return "; ".join(names) if names else "未知作者"


def classify_target(pdf_key: str, title: str) -> str:
    if pdf_key in CLEAR_LAW_KEYS:
        return "clear-law"
    if pdf_key in ADJACENT_DELETE_KEYS:
        return "adjacent-delete"
    lowered = title.casefold()
    if any(keyword.casefold() in lowered for keyword in LAW_KEYWORDS):
        return "clear-law"
    return "nonlaw-delete"


def delete_policy_for(classification: str) -> str:
    return "keep" if classification == "clear-law" else "delete"


def init_schema(conn: sqlite3.Connection) -> None:
    conn.executescript(
        f"""
        CREATE TABLE IF NOT EXISTS {TABLE_NAME} (
            pdf_key TEXT PRIMARY KEY,
            title TEXT NOT NULL,
            classification TEXT NOT NULL,
            delete_policy TEXT NOT NULL,
            size_mb REAL,
            parent_key TEXT,
            authors TEXT,
            year TEXT,
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
    classification: str,
    delete_policy: str,
    size_mb: float | None = None,
    parent_key: str | None = None,
    authors: str | None = None,
    year: str | None = None,
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
        f"""
        INSERT INTO {TABLE_NAME} (
            pdf_key, title, classification, delete_policy, size_mb, parent_key, authors, year,
            source_path, source_deleted, md_status, md_path, md_attachment_key, delete_status,
            error, created_at, updated_at
        ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
        ON CONFLICT(pdf_key) DO UPDATE SET
            title=excluded.title,
            classification=excluded.classification,
            delete_policy=excluded.delete_policy,
            size_mb=COALESCE(excluded.size_mb, size_mb),
            parent_key=COALESCE(excluded.parent_key, parent_key),
            authors=COALESCE(excluded.authors, authors),
            year=COALESCE(excluded.year, year),
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
            classification,
            delete_policy,
            size_mb,
            parent_key,
            authors,
            year,
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


def seed_live_targets(conn: sqlite3.Connection, local: ZoteroLocalClient) -> int:
    seeded = 0
    for attachment in local.iter_pdf_attachments():
        if attachment.parent_item_type != "book":
            continue
        size_mb = attachment.path.stat().st_size / (1024 * 1024)
        if not (MIN_SIZE_MB < size_mb <= MAX_SIZE_MB):
            continue
        title = attachment.parent_title or attachment.title or "未命名"
        classification = classify_target(attachment.key, title)
        upsert_job(
            conn,
            pdf_key=attachment.key,
            title=title,
            classification=classification,
            delete_policy=delete_policy_for(classification),
            size_mb=size_mb,
            parent_key=attachment.parent_key,
            authors=author_summary(attachment),
            year=item_year(attachment),
            source_path=str(attachment.path),
            error=None,
        )
        seeded += 1
    return seeded


def load_targets(conn: sqlite3.Connection) -> list[Target]:
    rows = conn.execute(
        f"""
        SELECT pdf_key, title, classification, delete_policy, size_mb, parent_key, authors, year, source_path
        FROM {TABLE_NAME}
        ORDER BY COALESCE(size_mb, 0) DESC, pdf_key
        """
    ).fetchall()
    return [
        Target(
            pdf_key=row["pdf_key"],
            title=row["title"],
            classification=row["classification"],
            delete_policy=row["delete_policy"],
            size_mb=float(row["size_mb"] or 0),
            parent_key=row["parent_key"],
            authors=row["authors"] or "",
            year=row["year"] or "",
            source_path=row["source_path"],
        )
        for row in rows
    ]


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
    target: Target,
    attachment: Attachment | None,
    row: sqlite3.Row,
) -> None:
    previous = state.conn.execute(
        f"SELECT source_deleted FROM {TABLE_NAME} WHERE pdf_key=?",
        (target.pdf_key,),
    ).fetchone()
    if previous and int(previous["source_deleted"] or 0) == 1:
        return
    ZoteroWebClient(config).delete_item(target.pdf_key)
    upsert_job(
        state.conn,
        pdf_key=target.pdf_key,
        title=target.title,
        classification=target.classification,
        delete_policy=target.delete_policy,
        size_mb=target.size_mb,
        parent_key=attachment.parent_key if attachment else row["parent_key"],
        source_path=str(attachment.path) if attachment else target.source_path,
        source_deleted=True,
        md_status="completed",
        md_path=row["output_path"],
        md_attachment_key=row["zotero_attachment_key"],
        delete_status="deleted_source_pdf",
        error=None,
    )


def read_snapshots(conn: sqlite3.Connection) -> list[JobSnapshot]:
    rows = conn.execute(
        f"""
        SELECT pdf_key, title, classification, delete_policy, size_mb, parent_key, authors, year,
               source_path, source_deleted, md_status, md_path, md_attachment_key, delete_status,
               error, updated_at
        FROM {TABLE_NAME}
        ORDER BY COALESCE(size_mb, 0) DESC, pdf_key
        """
    ).fetchall()
    out: list[JobSnapshot] = []
    for row in rows:
        status = row["delete_status"] or row["md_status"] or "pending"
        out.append(
            JobSnapshot(
                pdf_key=row["pdf_key"],
                title=row["title"],
                classification=row["classification"],
                delete_policy=row["delete_policy"],
                size_mb=row["size_mb"],
                status=status,
                source_deleted=bool(row["source_deleted"]),
                md_attachment_key=row["md_attachment_key"],
                md_path=row["md_path"],
                error=row["error"],
                parent_key=row["parent_key"],
                authors=row["authors"],
                year=row["year"],
                source_path=row["source_path"],
                updated_at=row["updated_at"],
            )
        )
    return out


def write_report(conn: sqlite3.Connection) -> None:
    REPORT_PATH.parent.mkdir(parents=True, exist_ok=True)
    snapshots = read_snapshots(conn)
    completed = sum(1 for item in snapshots if item.md_attachment_key)
    deleted = sum(1 for item in snapshots if item.source_deleted)
    kept = sum(1 for item in snapshots if item.status == "kept_source_pdf")
    failed = sum(1 for item in snapshots if item.error)
    clear_law = sum(1 for item in snapshots if item.classification == "clear-law")
    adjacent = sum(1 for item in snapshots if item.classification == "adjacent-delete")
    nonlaw = sum(1 for item in snapshots if item.classification == "nonlaw-delete")
    lines = [
        "# 50-100MB Book PDF PaddleOCR Mixed Cleanup",
        "",
        f"- Updated: `{dt.datetime.now().astimezone().isoformat(timespec='seconds')}`",
        "- Scope: current Zotero `book` PDF attachments with local size `>50MB` and `<=100MB` at enumeration time.",
        "- Rule: all targets are converted to PaddleOCR-VL Markdown and uploaded to Zotero.",
        "- Delete policy: `clear-law` keeps source PDF; `adjacent-delete` and `nonlaw-delete` delete source PDF after Markdown upload.",
        "",
        "## Summary",
        "",
        f"- Target PDFs: `{len(snapshots)}`",
        f"- clear-law / keep: `{clear_law}`",
        f"- adjacent-delete / delete: `{adjacent}`",
        f"- nonlaw-delete / delete: `{nonlaw}`",
        f"- Markdown attachments present: `{completed}`",
        f"- Source PDFs deleted: `{deleted}`",
        f"- Source PDFs kept: `{kept}`",
        f"- Failed/error: `{failed}`",
        "",
        "## Jobs",
        "",
        "| # | PDF key | Parent key | Size | Title | Author | Year | Classification | Delete policy | Status | Deleted source | Markdown attachment | Markdown path | Source path | Error |",
        "|---:|---|---|---:|---|---|---|---|---|---|---|---|---|---|---|",
    ]
    for index, item in enumerate(snapshots, 1):
        size = f"{item.size_mb:.1f} MB" if item.size_mb else ""
        lines.append(
            f"| {index} | {item.pdf_key} | {md_escape(item.parent_key)} | {size} | {md_escape(item.title)} | "
            f"{md_escape(item.authors)} | {md_escape(item.year)} | {item.classification} | {item.delete_policy} | "
            f"{md_escape(item.status)} | {'yes' if item.source_deleted else 'no'} | "
            f"{md_escape(item.md_attachment_key)} | {md_escape(item.md_path)} | {md_escape(item.source_path)} | "
            f"{md_escape(item.error)} |"
        )
    REPORT_PATH.write_text("\n".join(lines) + "\n", encoding="utf-8")


def target_done(conn: sqlite3.Connection, target: Target) -> bool:
    row = conn.execute(
        f"""
        SELECT source_deleted, md_attachment_key, delete_status, md_status
        FROM {TABLE_NAME}
        WHERE pdf_key=?
        """,
        (target.pdf_key,),
    ).fetchone()
    if not row:
        return False
    if row["md_status"] == "source_missing_before_completed_md":
        return True
    if target.delete_policy == "delete":
        return bool(row["source_deleted"])
    return bool(row["md_attachment_key"]) and row["delete_status"] == "kept_source_pdf"


def process_targets(args: argparse.Namespace) -> int:
    config = get_config(zotero_tags=args.zotero_tag)
    configure_logging(config.output_root)
    state = StateDB(config.output_root / ".state" / "zotero_llm.sqlite3")
    init_schema(state.conn)
    local = ZoteroLocalClient(config.zotero_local_api, config.zotero_storage, config.request_timeout)
    local.ping()
    seeded = seed_live_targets(state.conn, local)
    logging.info("Seeded %s live 50-100MB book PDF targets", seeded)
    write_report(state.conn)
    if args.report_only:
        return 0

    deadline = time.time() + args.max_runtime_minutes * 60
    ocr_pages_remaining = config.max_ocr_pages_per_run

    for target in load_targets(state.conn):
        write_report(state.conn)
        if time.time() + 30 > deadline:
            logging.info("Deadline reached before next target")
            break
        if target_done(state.conn, target):
            logging.info("SKIP finished target %s policy=%s", target.pdf_key, target.delete_policy)
            continue

        attachment: Attachment | None = None
        try:
            attachment = local.get_pdf_attachment(target.pdf_key)
            upsert_job(
                state.conn,
                pdf_key=target.pdf_key,
                title=target.title,
                classification=target.classification,
                delete_policy=target.delete_policy,
                size_mb=target.size_mb,
                parent_key=attachment.parent_key,
                authors=author_summary(attachment),
                year=item_year(attachment),
                source_path=str(attachment.path),
                error=None,
            )
        except Exception as exc:
            row = completed_paddle_markdown(state, target.pdf_key)
            if row:
                if target.delete_policy == "delete":
                    delete_source_pdf(config=config, state=state, target=target, attachment=None, row=row)
                else:
                    upsert_job(
                        state.conn,
                        pdf_key=target.pdf_key,
                        title=target.title,
                        classification=target.classification,
                        delete_policy=target.delete_policy,
                        size_mb=target.size_mb,
                        md_status="completed",
                        md_path=row["output_path"],
                        md_attachment_key=row["zotero_attachment_key"],
                        delete_status="kept_source_pdf",
                        error=None,
                    )
                continue
            upsert_job(
                state.conn,
                pdf_key=target.pdf_key,
                title=target.title,
                classification=target.classification,
                delete_policy=target.delete_policy,
                size_mb=target.size_mb,
                md_status="source_missing_before_completed_md",
                delete_status="not_deleted",
                error=f"Could not read source PDF attachment: {exc}",
            )
            logging.warning("Could not read source attachment %s: %s", target.pdf_key, exc)
            continue

        row = completed_paddle_markdown(state, target.pdf_key)
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
                upsert_job(
                    state.conn,
                    pdf_key=target.pdf_key,
                    title=target.title,
                    classification=target.classification,
                    delete_policy=target.delete_policy,
                    size_mb=target.size_mb,
                    md_status=status,
                    error=None,
                )
            except (QuotaExhaustedError, DeadlineReached) as exc:
                upsert_job(
                    state.conn,
                    pdf_key=target.pdf_key,
                    title=target.title,
                    classification=target.classification,
                    delete_policy=target.delete_policy,
                    size_mb=target.size_mb,
                    md_status="stopped",
                    error=str(exc),
                )
                logging.error("Stopping at %s: %s", target.pdf_key, exc)
                break
            except RetryableRemoteError as exc:
                upsert_job(
                    state.conn,
                    pdf_key=target.pdf_key,
                    title=target.title,
                    classification=target.classification,
                    delete_policy=target.delete_policy,
                    size_mb=target.size_mb,
                    md_status="retryable_error",
                    error=str(exc),
                )
                logging.warning("Retryable remote error for %s: %s", target.pdf_key, exc)
                continue
            except Exception as exc:
                upsert_job(
                    state.conn,
                    pdf_key=target.pdf_key,
                    title=target.title,
                    classification=target.classification,
                    delete_policy=target.delete_policy,
                    size_mb=target.size_mb,
                    md_status="failed",
                    error=str(exc),
                )
                logging.exception("Failed %s", target.pdf_key)
                continue
            row = completed_paddle_markdown(state, target.pdf_key)

        if row:
            upsert_job(
                state.conn,
                pdf_key=target.pdf_key,
                title=target.title,
                classification=target.classification,
                delete_policy=target.delete_policy,
                size_mb=target.size_mb,
                parent_key=attachment.parent_key,
                source_path=str(attachment.path),
                md_status="completed",
                md_path=row["output_path"],
                md_attachment_key=row["zotero_attachment_key"],
                error=None,
            )
            try:
                if target.delete_policy == "delete":
                    delete_source_pdf(config=config, state=state, target=target, attachment=attachment, row=row)
                    logging.info("Deleted source PDF %s after Markdown %s", target.pdf_key, row["zotero_attachment_key"])
                else:
                    upsert_job(
                        state.conn,
                        pdf_key=target.pdf_key,
                        title=target.title,
                        classification=target.classification,
                        delete_policy=target.delete_policy,
                        size_mb=target.size_mb,
                        delete_status="kept_source_pdf",
                        error=None,
                    )
                    logging.info("Kept source PDF %s after Markdown %s", target.pdf_key, row["zotero_attachment_key"])
            except Exception as exc:
                upsert_job(
                    state.conn,
                    pdf_key=target.pdf_key,
                    title=target.title,
                    classification=target.classification,
                    delete_policy=target.delete_policy,
                    size_mb=target.size_mb,
                    delete_status="delete_failed" if target.delete_policy == "delete" else "keep_mark_failed",
                    error=f"Markdown completed but post-process failed: {exc}",
                )
                logging.exception("Post-process failed %s", target.pdf_key)
        write_report(state.conn)
    write_report(state.conn)
    return 0


def main() -> int:
    parser = argparse.ArgumentParser(
        description="PaddleOCR current 50-100MB book PDFs, keeping clear-law sources and deleting adjacent/non-law sources."
    )
    parser.add_argument("--max-runtime-minutes", type=float, default=55.0)
    parser.add_argument("--report-only", action="store_true", help="Seed live targets and write the report without OCR.")
    add_zotero_tag_argument(parser)
    return process_targets(parser.parse_args())


if __name__ == "__main__":
    raise SystemExit(main())
