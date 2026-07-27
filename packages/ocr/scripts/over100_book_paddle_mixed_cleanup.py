#!/usr/bin/env python3.11
"""Convert current >100MB Zotero book PDFs to PaddleOCR-VL Markdown.

All targets are OCR'd to Markdown and uploaded as Zotero child attachments.
Source PDF deletion is policy-driven:
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

from cleanup_approval import refuse_unapproved_delete  # noqa: E402
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
    get_config,
    process_attachment,
)


@dataclass(frozen=True)
class Target:
    pdf_key: str
    title: str
    classification: str
    delete_policy: str


TARGETS: list[Target] = [
    Target("T4FBENY6", "Research Handbook on EU Data Protection Law", "clear-law", "keep"),
    Target("SHFFDZGZ", "超越法律", "clear-law", "keep"),
    Target("EKZU6TCE", "法理学 从古希腊到后现代", "clear-law", "keep"),
    Target("NKMLNKRY", "Trusted Data, revised and expanded edition: A New Framework for Identity and Data Sharing", "nonlaw-delete", "delete"),
    Target("H33B3PEY", "法理学", "clear-law", "keep"),
    Target("WDT47IM6", "宗教与公共理性", "adjacent-delete", "delete"),
    Target("RN4C8H4Z", "法律哲学", "clear-law", "keep"),
    Target("ZBE34PFH", "法律人的智能体：ChatGPT与DeepSeek", "clear-law", "keep"),
    Target("2XJ6LKXL", "International Copyright and Neighbouring Rights (2 Volumes): The Berne Convention and Beyond 2", "clear-law", "keep"),
    Target("R27YDYDH", "法理学的政治分析", "clear-law", "keep"),
    Target("TTMW7SSZ", "從法律規範性到法理學方法論", "clear-law", "keep"),
    Target("UWUBDP32", "中华帝国的法律", "clear-law", "keep"),
    Target("2LGXT37S", "以自由看待发展", "nonlaw-delete", "delete"),
    Target("MGDC4Q77", "逃避人性", "nonlaw-delete", "delete"),
    Target("N6N3U82I", "Collected Papers", "nonlaw-delete", "delete"),
    Target("WQYMW6FX", "牛津法理学与法哲学手册(上册)", "clear-law", "keep"),
    Target("FZZ76QFC", "列宁、黑格尔和西方马克思主义-一种批判性研究", "nonlaw-delete", "delete"),
    Target("V9SV8BL3", "法律、实用主义与民主", "clear-law", "keep"),
    Target("G3EFQHK5", "法科学生必修课", "clear-law", "keep"),
    Target("UXWGI8F3", "中国的现代化", "nonlaw-delete", "delete"),
    Target("EQM6AL5A", "Legal Values in Western Society", "clear-law", "keep"),
    Target("7G9VXXGF", "牛津法理学与法哲学手册(上册)", "clear-law", "keep"),
    Target("YY87AZVF", "近代中国与新世界 ：康有为变法与大同思想研究", "nonlaw-delete", "delete"),
    Target("MPVPGYRZ", "批评性语篇分析:经典阅读", "nonlaw-delete", "delete"),
    Target("TXW9R9E4", "实用主义政治哲学", "nonlaw-delete", "delete"),
    Target("QLADNT7K", "法哲學：自然法研究", "clear-law", "keep"),
    Target("C8VDJSDV", "民事指导性案例研究：一个方法论的视角", "clear-law", "keep"),
    Target("6F9LXQNR", "知识产权正当性解释", "clear-law", "keep"),
    Target("5ZLFNRNI", "中国政治", "nonlaw-delete", "delete"),
    Target("ZWM5RY6V", "用扣子（Coze）搭建AI Agent （零基础，实战版）—给普通人的智能体入门书", "nonlaw-delete", "delete"),
    Target("4ENGJK5E", "法律、立法与自由", "clear-law", "keep"),
    Target("SAPQ5I33", "比较法总论", "clear-law", "keep"),
    Target("FPWQUYQR", "转变的中国——历史变迁与欧洲经验的局限", "nonlaw-delete", "delete"),
    Target("VTCLMFXC", "合法性", "clear-law", "keep"),
    Target("4W6H3C8F", "法律中的模糊性", "clear-law", "keep"),
    Target("6HB86MHF", "法律人的Python课", "clear-law", "keep"),
    Target("DY5M2EXA", "原则的实践", "adjacent-delete", "delete"),
    Target("ZW4D7X7M", "The Future of the Internet--And How to Stop It", "nonlaw-delete", "delete"),
    Target("GVU6ZPTP", "法律论证原理-司法裁决之证立理论概览", "clear-law", "keep"),
    Target("5TUC2QXH", "专利危机与应对之道", "clear-law", "keep"),
    Target("R7XKKGEX", "Key Directions in Legal Education: National and International Perspectives", "clear-law", "keep"),
    Target("MISPVHJC", "法哲学", "clear-law", "keep"),
    Target("HN9GA3CE", "读懂法理学", "clear-law", "keep"),
]

REPORT_PATH = APP_ROOT / "reports" / "over100_book_pdf_paddle_mixed_cleanup.md"
TABLE_NAME = "over100_book_pdf_cleanup_jobs"


@dataclass
class JobSnapshot:
    pdf_key: str
    title: str
    classification: str
    delete_policy: str
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
        f"""
        CREATE TABLE IF NOT EXISTS {TABLE_NAME} (
            pdf_key TEXT PRIMARY KEY,
            title TEXT NOT NULL,
            classification TEXT NOT NULL,
            delete_policy TEXT NOT NULL,
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
    classification: str,
    delete_policy: str,
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
        f"""
        INSERT INTO {TABLE_NAME} (
            pdf_key, title, classification, delete_policy, parent_key, source_path, source_deleted, md_status,
            md_path, md_attachment_key, delete_status, error, created_at, updated_at
        ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
        ON CONFLICT(pdf_key) DO UPDATE SET
            title=excluded.title,
            classification=excluded.classification,
            delete_policy=excluded.delete_policy,
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
            classification,
            delete_policy,
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
    # The launcher records a source-cleanup approval bound to the built
    # reading artifacts. This is the only place a source PDF is actually
    # deleted, so it is where that record has to be honoured.
    if not refuse_unapproved_delete(target.pdf_key, logger=logging.getLogger(__name__)):
        return
    ZoteroWebClient(config).delete_item(target.pdf_key)
    upsert_job(
        state.conn,
        pdf_key=target.pdf_key,
        title=target.title,
        classification=target.classification,
        delete_policy=target.delete_policy,
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
        f"""
        SELECT pdf_key, title, classification, delete_policy, source_deleted, md_status, md_path,
               md_attachment_key, delete_status, error, updated_at
        FROM {TABLE_NAME}
        ORDER BY instr(?, pdf_key)
        """,
        (",".join(item.pdf_key for item in TARGETS),),
    ).fetchall()
    by_key = {row["pdf_key"]: row for row in rows}
    out: list[JobSnapshot] = []
    for item in TARGETS:
        row = by_key.get(item.pdf_key)
        if not row:
            out.append(
                JobSnapshot(
                    item.pdf_key,
                    item.title,
                    item.classification,
                    item.delete_policy,
                    "pending",
                    False,
                    None,
                    None,
                    None,
                    None,
                )
            )
            continue
        status = row["delete_status"] or row["md_status"] or "pending"
        out.append(
            JobSnapshot(
                pdf_key=item.pdf_key,
                title=row["title"],
                classification=row["classification"],
                delete_policy=row["delete_policy"],
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
    kept = sum(1 for item in snapshots if item.status == "kept_source_pdf")
    failed = sum(1 for item in snapshots if item.error)
    lines = [
        "# Over-100MB Book PDF PaddleOCR Mixed Cleanup",
        "",
        f"- Updated: `{dt.datetime.now().astimezone().isoformat(timespec='seconds')}`",
        "- Scope: current Zotero `book` PDF attachments larger than 100MB at enumeration time.",
        "- Rule: all targets are converted to PaddleOCR-VL Markdown and uploaded to Zotero.",
        "- Delete policy: `clear-law` keeps source PDF; `adjacent-delete` and `nonlaw-delete` delete source PDF after Markdown upload.",
        "",
        "## Summary",
        "",
        f"- Target PDFs: `{len(TARGETS)}`",
        f"- Markdown attachments present: `{completed}`",
        f"- Source PDFs deleted: `{deleted}`",
        f"- Source PDFs kept: `{kept}`",
        f"- Failed/error: `{failed}`",
        "",
        "## Jobs",
        "",
        "| # | PDF key | Title | Classification | Delete policy | Status | Deleted source | Markdown attachment | Markdown path | Error |",
        "|---:|---|---|---|---|---|---|---|---|---|",
    ]
    for index, item in enumerate(snapshots, 1):
        def esc(value: object) -> str:
            return str(value or "").replace("|", "\\|").replace("\n", " ")

        lines.append(
            f"| {index} | {item.pdf_key} | {esc(item.title)} | {item.classification} | {item.delete_policy} | "
            f"{esc(item.status)} | {'yes' if item.source_deleted else 'no'} | {esc(item.md_attachment_key)} | "
            f"{esc(item.md_path)} | {esc(item.error)} |"
        )
    REPORT_PATH.write_text("\n".join(lines) + "\n", encoding="utf-8")


def target_done(conn: sqlite3.Connection, target: Target) -> bool:
    row = conn.execute(
        f"""
        SELECT source_deleted, md_attachment_key, delete_status
        FROM {TABLE_NAME}
        WHERE pdf_key=?
        """,
        (target.pdf_key,),
    ).fetchone()
    if not row:
        return False
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
    deadline = time.time() + args.max_runtime_minutes * 60
    ocr_pages_remaining = config.max_ocr_pages_per_run

    for target in TARGETS:
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
                parent_key=attachment.parent_key,
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
                    delete_status="delete_failed" if target.delete_policy == "delete" else "keep_mark_failed",
                    error=f"Markdown completed but post-process failed: {exc}",
                )
                logging.exception("Post-process failed %s", target.pdf_key)
        write_report(state.conn)
    write_report(state.conn)
    return 0


def main() -> int:
    parser = argparse.ArgumentParser(
        description="PaddleOCR current >100MB book PDFs, keeping clear-law sources and deleting adjacent/non-law sources."
    )
    parser.add_argument("--max-runtime-minutes", type=float, default=55.0)
    add_zotero_tag_argument(parser)
    return process_targets(parser.parse_args())


if __name__ == "__main__":
    raise SystemExit(main())
