#!/usr/bin/env python3.11
"""Convert unprocessed law-theory / politics Zotero PDFs with MinerU Open API.

Rules:
- skip a parent item once it already has any Markdown child attachment;
- use MinerU Open API only for PDF-to-Markdown conversion;
- keep source PDFs and upload Markdown as sibling child attachments;
- filename/title format: 作者_年份_题名.md.
"""

from __future__ import annotations

import argparse
import datetime as dt
import json
import logging
import re
import shutil
import sqlite3
import subprocess
import sys
import time
from dataclasses import dataclass
from pathlib import Path
from typing import Any

SCRIPT_DIR = Path(__file__).resolve().parent
APP_ROOT = SCRIPT_DIR.parent
sys.path.insert(0, str(SCRIPT_DIR))

from ten_to_50_pdf_paddle_inventory import matched_keywords, read_parent_data  # noqa: E402
from zotero_llm_worker import (  # noqa: E402
    Attachment,
    StateDB,
    WorkerError,
    ZoteroLocalClient,
    ZoteroWebClient,
    add_zotero_tag_argument,
    attachment_provenance_note,
    configure_logging,
    creator_name,
    get_config,
    item_year,
    markdown_filename,
    md5_file,
    pdf_page_count,
    source_key_from_provenance_note,
)


MAX_SIZE_MB = 200.0
MAX_PAGES = 200
REPORT_PATH = APP_ROOT / "reports" / "mineru_law_politics_markdown.md"
TABLE_NAME = "mineru_law_politics_jobs"
TARGET_COLLECTIONS = {
    "9D8QWEMZ",  # 80_宽泛 / 法理学（宽泛）
    "ZX4JS3K7",  # 80_宽泛 / 儒家与法思想（宽泛）
}

LAW_THEORY_KEYWORDS = (
    "法理",
    "法哲",
    "法律哲学",
    "法学方法",
    "法律方法",
    "法律理论",
    "法律规范",
    "规范性",
    "合法性",
    "法律论证",
    "法律推理",
    "司法裁判",
    "法解释",
    "法解釋",
    "法教义",
    "法社会",
    "法社會",
    "比较法",
    "自然法",
    "实证主义法学",
    "法思想",
    "法治理论",
    "jurisprudence",
    "legal theory",
    "philosophy of law",
    "legal philosophy",
    "legal reasoning",
    "legal argument",
    "legal method",
    "legal methods",
    "legal normativity",
    "rule of law",
    "legality",
    "natural law",
    "legal positivism",
    "sociology of law",
    "comparative law theory",
    "critical legal",
    "law and economics",
    "law and society",
    "hart",
    "dworkin",
    "raz",
    "fuller",
    "finnis",
    "posner",
    "tamanaha",
    "cotterrell",
    "maccormick",
    "waldron",
)

POLITICS_KEYWORDS = (
    "政治学",
    "政治理论",
    "政治哲学",
    "政治思想",
    "政治自由主义",
    "公共理性",
    "民主理论",
    "宪政",
    "治理理论",
    "国家理论",
    "公民身份",
    "自由主义",
    "共和主义",
    "正义论",
    "社会契约",
    "集体主义",
    "意识形态",
    "中国政治",
    "political theory",
    "political philosophy",
    "political thought",
    "political liberalism",
    "public reason",
    "democratic theory",
    "constitutionalism",
    "liberalism",
    "republicanism",
    "justice theory",
    "social contract",
    "civil society",
    "citizenship",
    "ideology",
    "rawls",
    "nussbaum",
    "sen",
    "strauss",
    "foucault",
    "habermas",
    "cohen",
    "nozick",
    "hayek",
)

PHILOSOPHY_PRIORITY_KEYWORDS = (
    "哲学",
    "哲學",
    "法哲",
    "法理",
    "政治哲学",
    "政治哲學",
    "思想",
    "伦理",
    "倫理",
    "形而上学",
    "主体",
    "自主",
    "自由",
    "正义",
    "德性",
    "philosophy",
    "philosophical",
    "jurisprudence",
    "theory",
    "theoretical",
    "morality",
    "moral",
    "ethics",
    "autonomy",
    "freedom",
    "justice",
    "metaphysics",
    "hart",
    "dworkin",
    "raz",
    "fuller",
    "finnis",
    "rawls",
    "foucault",
    "habermas",
    "cohen",
    "nozick",
    "hayek",
    "nussbaum",
    "sen",
)


@dataclass(frozen=True)
class Target:
    pdf_key: str
    parent_key: str | None
    title: str
    authors: str
    year: str
    source_path: Path
    source_md5: str
    size_mb: float
    page_count: int
    classification: str
    basis: str
    priority: int


def now_utc() -> str:
    return dt.datetime.now(dt.UTC).replace(microsecond=0).isoformat()


def md_escape(value: object) -> str:
    text = "" if value is None else str(value)
    return text.replace("|", "\\|").replace("\n", " ")


def author_summary(attachment: Attachment) -> str:
    names = [creator_name(creator) for creator in attachment.parent_creators]
    names = [name for name in names if name]
    return "; ".join(names) if names else "未知作者"


def field_text(attachment: Attachment, parent_data: dict[str, Any]) -> str:
    fields = [
        attachment.parent_title or "",
        attachment.title or "",
        attachment.path.name,
        parent_data.get("abstractNote") or "",
        parent_data.get("extra") or "",
        parent_data.get("seriesTitle") or "",
        parent_data.get("shortTitle") or "",
    ]
    # Deliberately exclude publisher to avoid false positives such as
    # "Hart Publishing" being treated as an H.L.A. Hart jurisprudence match.
    for creator in parent_data.get("creators") or []:
        name = creator_name(creator)
        if name:
            fields.append(name)
    for tag in parent_data.get("tags") or []:
        value = tag.get("tag") if isinstance(tag, dict) else tag
        if value:
            fields.append(str(value))
    return " ".join(fields)


def classify_target(
    attachment: Attachment,
    parent_data: dict[str, Any],
) -> tuple[str | None, str, int]:
    text = field_text(attachment, parent_data)
    collections = set(parent_data.get("collections") or [])
    if collections & TARGET_COLLECTIONS:
        classification = "collection"
        basis = "collection: " + ", ".join(sorted(collections & TARGET_COLLECTIONS))
    else:
        law_matches = matched_keywords(text, LAW_THEORY_KEYWORDS)
        if law_matches:
            classification = "law-theory"
            basis = "law-theory keyword: " + ", ".join(law_matches[:5])
        else:
            politics_matches = matched_keywords(text, POLITICS_KEYWORDS)
            if not politics_matches:
                return None, "no target keyword/collection matched", 99
            classification = "politics"
            basis = "politics keyword: " + ", ".join(politics_matches[:5])

    priority_matches = matched_keywords(text, PHILOSOPHY_PRIORITY_KEYWORDS)
    if priority_matches:
        return classification, basis + "; priority: " + ", ".join(priority_matches[:5]), 0
    if classification in {"collection", "law-theory"}:
        return classification, basis, 1
    return classification, basis, 2


def init_schema(conn: sqlite3.Connection) -> None:
    conn.executescript(
        f"""
        CREATE TABLE IF NOT EXISTS {TABLE_NAME} (
            pdf_key TEXT PRIMARY KEY,
            parent_key TEXT,
            title TEXT NOT NULL,
            authors TEXT,
            year TEXT,
            source_path TEXT,
            source_md5 TEXT,
            size_mb REAL,
            page_count INTEGER,
            classification TEXT,
            basis TEXT,
            priority INTEGER,
            status TEXT NOT NULL DEFAULT 'pending',
            md_path TEXT,
            md_attachment_key TEXT,
            error TEXT,
            created_at TEXT NOT NULL,
            updated_at TEXT NOT NULL
        );
        """
    )
    conn.commit()


def upsert_job(
    conn: sqlite3.Connection,
    target: Target,
    *,
    status: str | None = None,
    md_path: Path | None = None,
    md_attachment_key: str | None = None,
    error: str | None = None,
) -> None:
    ts = now_utc()
    conn.execute(
        f"""
        INSERT INTO {TABLE_NAME} (
            pdf_key, parent_key, title, authors, year, source_path, source_md5,
            size_mb, page_count, classification, basis, priority, status,
            md_path, md_attachment_key, error, created_at, updated_at
        ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
        ON CONFLICT(pdf_key) DO UPDATE SET
            parent_key=excluded.parent_key,
            title=excluded.title,
            authors=excluded.authors,
            year=excluded.year,
            source_path=excluded.source_path,
            source_md5=excluded.source_md5,
            size_mb=excluded.size_mb,
            page_count=excluded.page_count,
            classification=excluded.classification,
            basis=excluded.basis,
            priority=excluded.priority,
            status=COALESCE(excluded.status, status),
            md_path=COALESCE(excluded.md_path, md_path),
            md_attachment_key=COALESCE(excluded.md_attachment_key, md_attachment_key),
            error=excluded.error,
            updated_at=excluded.updated_at
        """,
        (
            target.pdf_key,
            target.parent_key,
            target.title,
            target.authors,
            target.year,
            str(target.source_path),
            target.source_md5,
            target.size_mb,
            target.page_count,
            target.classification,
            target.basis,
            target.priority,
            status,
            str(md_path) if md_path else None,
            md_attachment_key,
            error,
            ts,
            ts,
        ),
    )
    conn.commit()


def update_job(
    conn: sqlite3.Connection,
    pdf_key: str,
    *,
    status: str,
    md_path: Path | None = None,
    md_attachment_key: str | None = None,
    error: str | None = None,
) -> None:
    conn.execute(
        f"""
        UPDATE {TABLE_NAME}
        SET status=?,
            md_path=COALESCE(?, md_path),
            md_attachment_key=COALESCE(?, md_attachment_key),
            error=?,
            updated_at=?
        WHERE pdf_key=?
        """,
        (
            status,
            str(md_path) if md_path else None,
            md_attachment_key,
            error,
            now_utc(),
            pdf_key,
        ),
    )
    conn.commit()


def existing_markdown_maps(local: ZoteroLocalClient) -> tuple[set[str], set[str]]:
    parent_has_markdown: set[str] = set()
    source_pdf_keys: set[str] = set()
    start = 0
    while True:
        batch = local.get("items", itemType="attachment", limit=100, start=start, format="json")
        if not batch:
            break
        for item in batch:
            data = item.get("data", {})
            content_type = data.get("contentType")
            filename = str(data.get("filename") or "")
            if content_type == "text/markdown" or filename.lower().endswith(".md"):
                parent = data.get("parentItem")
                if parent:
                    parent_has_markdown.add(parent)
                source_key = source_key_from_provenance_note(data.get("note"))
                if source_key:
                    source_pdf_keys.add(source_key)
                for tag in data.get("tags") or []:
                    value = tag.get("tag") if isinstance(tag, dict) else tag
                    if isinstance(value, str) and value.startswith("ocr-source:"):
                        source_pdf_keys.add(value.split(":", 1)[1])
        start += len(batch)
        if len(batch) < 100:
            break
    return parent_has_markdown, source_pdf_keys


def collect_targets(local: ZoteroLocalClient, conn: sqlite3.Connection) -> list[Target]:
    parent_has_markdown, source_pdf_keys = existing_markdown_maps(local)
    parent_cache: dict[str | None, dict[str, Any]] = {}
    targets: list[Target] = []
    for attachment in local.iter_pdf_attachments():
        if attachment.key in source_pdf_keys:
            continue
        if attachment.parent_key and attachment.parent_key in parent_has_markdown:
            continue
        try:
            size_mb = attachment.path.stat().st_size / (1024 * 1024)
        except OSError:
            continue
        if size_mb > MAX_SIZE_MB:
            continue
        try:
            pages = pdf_page_count(attachment.path)
        except Exception as exc:
            logging.warning("Skipping %s: page count failed: %s", attachment.key, exc)
            continue
        if pages > MAX_PAGES:
            continue
        parent_data = parent_cache.get(attachment.parent_key)
        if parent_data is None:
            parent_data = read_parent_data(local, attachment.parent_key)
            parent_cache[attachment.parent_key] = parent_data
        classification, basis, priority = classify_target(attachment, parent_data)
        if not classification:
            continue
        target = Target(
            pdf_key=attachment.key,
            parent_key=attachment.parent_key,
            title=attachment.parent_title or attachment.title or "未命名",
            authors=author_summary(attachment),
            year=item_year(attachment),
            source_path=attachment.path,
            source_md5=md5_file(attachment.path),
            size_mb=size_mb,
            page_count=pages,
            classification=classification,
            basis=basis,
            priority=priority,
        )
        upsert_job(conn, target, status="pending")
        targets.append(target)
    targets.sort(key=lambda item: (item.priority, item.page_count, item.size_mb, item.title.casefold()))
    conn.execute(
        f"""
        UPDATE {TABLE_NAME}
        SET status='skipped_page_limit',
            error='effective MinerU page limit is 200 pages',
            updated_at=?
        WHERE page_count > ?
          AND status IN ('pending', 'error')
        """,
        (now_utc(), MAX_PAGES),
    )
    conn.commit()
    return targets


def is_parent_markdown_present(local: ZoteroLocalClient, parent_key: str | None) -> bool:
    if not parent_key:
        return False
    try:
        children = local.get(f"items/{parent_key}/children", format="json", limit=100)
    except Exception:
        return False
    for child in children:
        data = child.get("data", {})
        filename = str(data.get("filename") or "")
        if data.get("contentType") == "text/markdown" or filename.lower().endswith(".md"):
            return True
    return False


def require_single_mineru_markdown(output_dir: Path) -> Path:
    candidates = list(output_dir.rglob("*.md"))
    if len(candidates) != 1:
        raise WorkerError(
            f"MinerU must produce exactly one Markdown result under {output_dir}; "
            f"observed {len(candidates)}"
        )
    return candidates[0]


def run_mineru_extract(target: Target, attachment: Attachment, output_root: Path, timeout: int) -> Path:
    work_dir = output_root / ".state" / "mineru" / target.pdf_key / target.source_md5
    result_dir = work_dir / "result"
    if result_dir.exists():
        shutil.rmtree(result_dir)
    result_dir.mkdir(parents=True, exist_ok=True)
    command = [
        "mineru-open-api",
        "extract",
        str(target.source_path),
        "-o",
        str(result_dir),
        "--model",
        "vlm",
        "--ocr",
        "--timeout",
        str(timeout),
    ]
    logging.info("MINERU start %s pages=%s size=%.1fMB title=%s", target.pdf_key, target.page_count, target.size_mb, target.title)
    result = subprocess.run(
        command,
        check=False,
        capture_output=True,
        text=True,
        timeout=timeout + 30,
    )
    if result.returncode != 0:
        message = "\n".join(part.strip() for part in (result.stdout, result.stderr) if part.strip())
        raise WorkerError(f"MinerU extract failed with exit {result.returncode}: {message[:2000]}")
    mineru_md = require_single_mineru_markdown(result_dir)
    staging_dir = output_root / ".state" / "staging" / target.pdf_key
    staging_dir.mkdir(parents=True, exist_ok=True)
    final_md = staging_dir / markdown_filename(attachment)
    shutil.copyfile(mineru_md, final_md)
    sidecar = final_md.with_suffix(".mineru.json")
    sidecar.write_text(
        json.dumps(
            {
                "source_pdf_key": target.pdf_key,
                "parent_item_key": target.parent_key,
                "source_pdf_md5": target.source_md5,
                "source_pdf_pages": target.page_count,
                "source_pdf_path": str(target.source_path),
                "conversion_route": "mineru-open-api",
                "mineru_model": "vlm",
                "mineru_ocr": True,
                "generated_at": now_utc(),
            },
            ensure_ascii=False,
            indent=2,
        )
        + "\n",
        encoding="utf-8",
    )
    if final_md.stat().st_size == 0:
        raise WorkerError(f"MinerU Markdown is empty: {final_md}")
    return final_md


def should_stop_for_remote_error(exc: Exception) -> bool:
    message = str(exc).casefold()
    return any(token in message for token in ("quota", "limit", "insufficient", "401", "403", "token", "unauthorized"))


def upload_markdown(config: Any, attachment: Attachment, markdown_path: Path) -> str:
    return ZoteroWebClient(config).create_markdown_attachment(
        parent_key=attachment.parent_key,
        title=markdown_path.name,
        markdown_path=markdown_path,
        tags=config.zotero_tags,
        note=attachment_provenance_note(attachment, "mineru-open-api", config),
    )


def completed_mineru_row(conn: sqlite3.Connection, pdf_key: str, source_md5: str) -> sqlite3.Row | None:
    return conn.execute(
        """
        SELECT *
        FROM documents
        WHERE pdf_key=? AND source_md5=? AND route='mineru-open-api' AND status='completed'
        ORDER BY updated_at DESC
        LIMIT 1
        """,
        (pdf_key, source_md5),
    ).fetchone()


def write_report(conn: sqlite3.Connection) -> None:
    REPORT_PATH.parent.mkdir(parents=True, exist_ok=True)
    rows = conn.execute(
        f"""
        SELECT *
        FROM {TABLE_NAME}
        ORDER BY
            CASE status
                WHEN 'completed' THEN 4
                WHEN 'skipped_parent_has_markdown' THEN 5
                WHEN 'error' THEN 3
                WHEN 'pending' THEN 1
                ELSE 2
            END,
            priority,
            page_count,
            title
        """
    ).fetchall()
    status_counts: dict[str, int] = {}
    for row in rows:
        status_counts[row["status"]] = status_counts.get(row["status"], 0) + 1
    lines = [
        "# MinerU Law Theory / Politics Markdown",
        "",
        f"- Updated: `{dt.datetime.now().astimezone().isoformat(timespec='seconds')}`",
        "- Engine: `mineru-open-api extract --model vlm --ocr`",
        f"- Limits: `<= {MAX_SIZE_MB:g} MB`, `<= {MAX_PAGES} pages`",
        "- Skip rule: parent Zotero item already has a Markdown child attachment.",
        "- Attachment rule: upload Markdown under the same parent item, filename `作者_年份_题名.md`.",
        "- Priority: philosophy/legal philosophy/political philosophy matches first.",
        "",
        "## Summary",
        "",
    ]
    for status, count in sorted(status_counts.items()):
        lines.append(f"- `{status}`: `{count}`")
    lines.extend(
        [
            "",
            "## Jobs",
            "",
            "| # | PDF key | Parent key | Priority | Class | Status | Pages | Size | Authors | Year | Title | Markdown attachment | Error |",
            "|---:|---|---|---:|---|---|---:|---:|---|---|---|---|---|",
        ]
    )
    for index, row in enumerate(rows, 1):
        lines.append(
            "| "
            + " | ".join(
                [
                    str(index),
                    md_escape(row["pdf_key"]),
                    md_escape(row["parent_key"]),
                    md_escape(row["priority"]),
                    md_escape(row["classification"]),
                    md_escape(row["status"]),
                    md_escape(row["page_count"]),
                    f"{float(row['size_mb'] or 0):.1f} MB",
                    md_escape(row["authors"]),
                    md_escape(row["year"]),
                    md_escape(row["title"]),
                    md_escape(row["md_attachment_key"]),
                    md_escape(row["error"]),
                ]
            )
            + " |"
        )
    REPORT_PATH.write_text("\n".join(lines) + "\n", encoding="utf-8")


def process_targets(args: argparse.Namespace) -> int:
    mineru = shutil.which("mineru-open-api")
    if not mineru:
        raise WorkerError("mineru-open-api is not installed or not on PATH")
    config = get_config(zotero_tags=args.zotero_tag)
    config.output_root.mkdir(parents=True, exist_ok=True)
    (config.output_root / ".state").mkdir(parents=True, exist_ok=True)
    configure_logging(config.output_root)
    state = StateDB(config.output_root / ".state" / "zotero_llm.sqlite3")
    init_schema(state.conn)
    local = ZoteroLocalClient(config.zotero_local_api, config.zotero_storage, config.request_timeout)
    local.ping()
    targets = collect_targets(local, state.conn)
    write_report(state.conn)
    logging.info("MinerU target queue size=%s", len(targets))
    if args.dry_run or args.report_only:
        return 0

    deadline = time.time() + args.max_runtime_minutes * 60
    processed = 0
    for target in targets:
        if args.limit and processed >= args.limit:
            break
        if time.time() + 30 > deadline:
            logging.info("Deadline reached before next MinerU target")
            break
        if is_parent_markdown_present(local, target.parent_key):
            update_job(state.conn, target.pdf_key, status="skipped_parent_has_markdown", error=None)
            write_report(state.conn)
            continue
        try:
            attachment = local.get_pdf_attachment(target.pdf_key)
            existing = completed_mineru_row(state.conn, target.pdf_key, target.source_md5)
            if existing and existing["zotero_attachment_key"]:
                update_job(
                    state.conn,
                    target.pdf_key,
                    status="completed",
                    md_path=Path(existing["output_path"]) if existing["output_path"] else None,
                    md_attachment_key=existing["zotero_attachment_key"],
                    error=None,
                )
                write_report(state.conn)
                continue
            markdown_path = run_mineru_extract(target, attachment, config.output_root, args.mineru_timeout_seconds)
            zotero_key = upload_markdown(config, attachment, markdown_path)
            state.upsert_document(
                attachment=attachment,
                source_md5=target.source_md5,
                route="mineru-open-api",
                status="completed",
                page_count=target.page_count,
                output_path=markdown_path,
                sidecar_path=markdown_path.with_suffix(".mineru.json"),
                zotero_attachment_key=zotero_key,
            )
            update_job(
                state.conn,
                target.pdf_key,
                status="completed",
                md_path=markdown_path,
                md_attachment_key=zotero_key,
                error=None,
            )
            processed += 1
            logging.info("MINERU completed %s markdown_attachment=%s", target.pdf_key, zotero_key)
        except Exception as exc:
            update_job(state.conn, target.pdf_key, status="error", error=str(exc)[:2000])
            logging.exception("MinerU failed %s", target.pdf_key)
            write_report(state.conn)
            if should_stop_for_remote_error(exc):
                logging.error("Stopping MinerU batch after auth/quota/limit-looking error")
                break
            continue
        write_report(state.conn)
    write_report(state.conn)
    return 0


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(description="Convert law-theory/politics Zotero PDFs to Markdown with MinerU Open API.")
    parser.add_argument("--dry-run", action="store_true", help="Build queue/report without calling MinerU or Zotero upload.")
    parser.add_argument("--report-only", action="store_true", help="Alias for dry-run report generation.")
    parser.add_argument("--limit", type=int, help="Maximum number of PDFs to convert in this run.")
    parser.add_argument("--max-runtime-minutes", type=float, default=55.0)
    parser.add_argument("--mineru-timeout-seconds", type=int, default=1800)
    add_zotero_tag_argument(parser)
    return parser


def main() -> int:
    return process_targets(build_parser().parse_args())


if __name__ == "__main__":
    raise SystemExit(main())
