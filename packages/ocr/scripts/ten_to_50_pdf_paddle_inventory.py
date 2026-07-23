#!/usr/bin/env python3.11
"""Inventory 10-50MB Zotero PDFs for PaddleOCR-VL Markdown cleanup.

Read-only: this script does not OCR, upload, delete, or mutate Zotero/state DB.
It classifies current local PDF attachments into:
- clear-law: convert to PaddleOCR Markdown but keep source PDF
- adjacent-delete / nonlaw-delete: convert to PaddleOCR Markdown, then delete source PDF after approval
"""

from __future__ import annotations

import datetime as dt
import html
import re
import sqlite3
import sys
from dataclasses import dataclass
from pathlib import Path
from typing import Any

SCRIPT_DIR = Path(__file__).resolve().parent
APP_ROOT = SCRIPT_DIR.parent
sys.path.insert(0, str(SCRIPT_DIR))

from zotero_llm_worker import (  # noqa: E402
    Attachment,
    ZoteroLocalClient,
    creator_name,
    get_config,
    item_year,
    pdf_page_count,
)


MIN_SIZE_MB = 10.0
MAX_SIZE_MB = 50.0
REPORT_PATH = APP_ROOT / "reports" / "ten_to_50_pdf_paddle_inventory.md"

LAW_KEYWORDS = (
    "法律",
    "法学",
    "法理",
    "法哲",
    "法治",
    "法制",
    "法科",
    "司法",
    "审判",
    "法院",
    "检察",
    "裁判",
    "判例",
    "案例",
    "诉讼",
    "仲裁",
    "证据",
    "宪法",
    "憲法",
    "刑法",
    "刑事",
    "民法",
    "民事",
    "行政法",
    "商法",
    "公司法",
    "合同法",
    "物权",
    "侵权",
    "犯罪",
    "法解",
    "法解释",
    "法解釋",
    "法释义",
    "指导案例",
    "案例指导",
    "反不正当竞争",
    "知识产权",
    "智慧財產",
    "著作权",
    "版權",
    "版权",
    "专利",
    "專利",
    "商标",
    "商標",
    "个人信息",
    "個人資料",
    "数据保护",
    "資料保護",
    "隐私",
    "隱私",
    "律师",
    "律師",
    "law",
    "legal",
    "jurisprudence",
    "juridical",
    "judicial",
    "court",
    "courts",
    "judge",
    "criminal",
    "civil procedure",
    "constitutional",
    "administrative law",
    "contract law",
    "tort",
    "property law",
    "intellectual property",
    "copyright",
    "patent",
    "trademark",
    "privacy",
    "data protection",
    "legal education",
    "precedent",
    "precedents",
    "guiding cases",
    "master of laws",
    "trade dress",
    "lesi",
    "rechtsfindung",
    "privatrecht",
    "privatrechts",
    "richterlich",
    "richterlichen",
    "entscheidungspraxis",
    "recht",
    "dworkin",
    "德沃金",
    "waldron",
    "沃尔德伦",
    "maccormick",
    "summers",
    "tamanaha",
    "esser",
    "sunstein",
    "lessig",
    "lemley",
)

ADJACENT_KEYWORDS = (
    "公共理性",
    "政治自由主义",
    "政治哲学",
    "政治理论",
    "实用主义政治",
    "自由主义",
    "民主",
    "正义",
    "权利",
    "權利",
    "契约",
    "契約",
    "治理",
    "规制",
    "規制",
    "监管",
    "監管",
    "公共政策",
    "国家",
    "國家",
    "宪政",
    "憲政",
    "public reason",
    "political liberalism",
    "political philosophy",
    "democracy",
    "rights",
    "justice",
    "liberalism",
    "governance",
    "regulation",
)


@dataclass(frozen=True)
class ExistingMarkdown:
    attachment_key: str | None
    output_path: str | None
    status: str | None


@dataclass(frozen=True)
class Candidate:
    attachment: Attachment
    parent_data: dict[str, Any]
    size_mb: float
    page_count: int | None
    page_error: str | None
    classification: str
    delete_policy: str
    existing_md: ExistingMarkdown | None
    suggested_action: str
    basis: str


def md_escape(value: object) -> str:
    text = "" if value is None else str(value)
    return text.replace("|", "\\|").replace("\n", " ")


def author_summary(attachment: Attachment) -> str:
    names = [creator_name(creator) for creator in attachment.parent_creators]
    names = [name for name in names if name]
    return "; ".join(names) if names else "未知作者"


def item_title(attachment: Attachment) -> str:
    return attachment.parent_title or attachment.title or "未命名"


def parent_field_text(parent_data: dict[str, Any]) -> str:
    fields: list[str] = []
    for key in (
        "title",
        "shortTitle",
        "publicationTitle",
        "journalAbbreviation",
        "seriesTitle",
        "bookTitle",
        "conferenceName",
        "publisher",
        "place",
        "abstractNote",
        "extra",
    ):
        value = parent_data.get(key)
        if value:
            fields.append(str(value))
    for creator in parent_data.get("creators") or []:
        name = creator_name(creator)
        if name:
            fields.append(name)
    for tag in parent_data.get("tags") or []:
        value = tag.get("tag") if isinstance(tag, dict) else tag
        if value:
            fields.append(str(value))
    return " ".join(fields)


def ascii_keyword_matches(lowered: str, keyword: str) -> bool:
    escaped = re.escape(keyword.casefold())
    if " " in keyword or "-" in keyword:
        pattern = rf"(?<![a-z0-9]){escaped}(?![a-z0-9])"
    else:
        pattern = rf"(?<![a-z0-9]){escaped}s?(?![a-z0-9])"
    return re.search(pattern, lowered) is not None


def should_match_by_substring(keyword: str) -> bool:
    if any(ord(ch) > 127 for ch in keyword):
        return True
    return keyword in {
        "intellectual propert",
        "data protection",
        "legal education",
        "guiding cases",
        "master of laws",
        "trade dress",
        "civil procedure",
        "administrative law",
        "contract law",
        "property law",
        "rechtsfindung",
        "privatrecht",
        "privatrechts",
        "richterlich",
        "richterlichen",
        "entscheidungspraxis",
    }


def matched_keywords(text: str, keywords: tuple[str, ...]) -> list[str]:
    lowered = text.casefold()
    matches: list[str] = []
    for keyword in keywords:
        if should_match_by_substring(keyword):
            matched = keyword.casefold() in lowered
        else:
            matched = ascii_keyword_matches(lowered, keyword)
        if matched:
            matches.append(keyword)
    return matches


def classify(attachment: Attachment, parent_data: dict[str, Any]) -> tuple[str, str]:
    text = " ".join(
        [
            attachment.title or "",
            attachment.path.name,
            attachment.parent_item_type or "",
            parent_field_text(parent_data),
        ]
    )
    law_matches = matched_keywords(text, LAW_KEYWORDS)
    if law_matches:
        return "clear-law", "law keyword: " + ", ".join(law_matches[:5])
    adjacent_matches = matched_keywords(text, ADJACENT_KEYWORDS)
    if adjacent_matches:
        return "adjacent-delete", "adjacent keyword: " + ", ".join(adjacent_matches[:5])
    return "nonlaw-delete", "no law keyword matched"


def delete_policy_for(classification: str) -> str:
    return "keep" if classification == "clear-law" else "delete"


def suggested_action(classification: str, existing_md: ExistingMarkdown | None) -> str:
    delete_policy = delete_policy_for(classification)
    if existing_md and existing_md.attachment_key:
        if delete_policy == "delete":
            return "delete-source-existing-paddle-md"
        return "skip-existing-paddle-md-keep-source"
    if delete_policy == "delete":
        return "ocr-delete-source"
    return "ocr-keep-source"


def completed_paddle_markdown(conn: sqlite3.Connection, pdf_key: str) -> ExistingMarkdown | None:
    rows = conn.execute(
        """
        SELECT status, output_path, zotero_attachment_key
        FROM documents
        WHERE pdf_key=? AND route='paddle-ocr' AND status='completed'
        ORDER BY updated_at DESC
        """,
        (pdf_key,),
    ).fetchall()
    for row in rows:
        output_path = row["output_path"]
        if not row["zotero_attachment_key"]:
            continue
        if output_path and Path(output_path).exists() and Path(output_path).stat().st_size > 0:
            return ExistingMarkdown(
                attachment_key=row["zotero_attachment_key"],
                output_path=output_path,
                status=row["status"],
            )
    return None


def read_parent_data(local: ZoteroLocalClient, parent_key: str | None) -> dict[str, Any]:
    if not parent_key:
        return {}
    try:
        parent = local.get(f"items/{parent_key}", format="json")
    except Exception:
        return {}
    return parent.get("data", {}) or {}


def page_count_for(path: Path) -> tuple[int | None, str | None]:
    try:
        return pdf_page_count(path), None
    except Exception as exc:
        return None, f"{type(exc).__name__}: {exc}"


def collect_candidates() -> list[Candidate]:
    config = get_config()
    local = ZoteroLocalClient(config.zotero_local_api, config.zotero_storage, config.request_timeout)
    local.ping()
    state = sqlite3.connect(config.output_root / ".state" / "zotero_llm.sqlite3")
    state.row_factory = sqlite3.Row

    candidates: list[Candidate] = []
    for attachment in local.iter_pdf_attachments():
        try:
            size_mb = attachment.path.stat().st_size / (1024 * 1024)
        except OSError:
            continue
        if not (MIN_SIZE_MB < size_mb <= MAX_SIZE_MB):
            continue
        parent_data = read_parent_data(local, attachment.parent_key)
        classification, basis = classify(attachment, parent_data)
        existing_md = completed_paddle_markdown(state, attachment.key)
        pages, page_error = page_count_for(attachment.path)
        candidates.append(
            Candidate(
                attachment=attachment,
                parent_data=parent_data,
                size_mb=size_mb,
                page_count=pages,
                page_error=page_error,
                classification=classification,
                delete_policy=delete_policy_for(classification),
                existing_md=existing_md,
                suggested_action=suggested_action(classification, existing_md),
                basis=basis,
            )
        )
    return sorted(
        candidates,
        key=lambda item: (
            item.delete_policy != "delete",
            item.existing_md is not None,
            -item.size_mb,
            item.attachment.key,
        ),
    )


def render_report(candidates: list[Candidate]) -> str:
    generated = dt.datetime.now().astimezone().isoformat(timespec="seconds")
    clear_law = sum(1 for item in candidates if item.classification == "clear-law")
    adjacent = sum(1 for item in candidates if item.classification == "adjacent-delete")
    nonlaw = sum(1 for item in candidates if item.classification == "nonlaw-delete")
    delete_count = sum(1 for item in candidates if item.delete_policy == "delete")
    keep_count = sum(1 for item in candidates if item.delete_policy == "keep")
    existing_md = sum(1 for item in candidates if item.existing_md and item.existing_md.attachment_key)
    ocr_needed = sum(1 for item in candidates if item.suggested_action.startswith("ocr-"))
    delete_after_existing = sum(1 for item in candidates if item.suggested_action == "delete-source-existing-paddle-md")

    lines = [
        "# 10-50MB PDF PaddleOCR Inventory",
        "",
        f"- Generated: `{generated}`",
        "- Source: Zotero local API `http://127.0.0.1:23119/api/users/0`.",
        f"- Scope: all current Zotero local PDF attachments with size `>{MIN_SIZE_MB:g}MB` and `<= {MAX_SIZE_MB:g}MB`.",
        "- This report is read-only: no OCR, no upload, no deletion.",
        "- Proposed rule: `clear-law` -> PaddleOCR Markdown and keep source PDF; `adjacent-delete` / `nonlaw-delete` -> PaddleOCR Markdown then delete source PDF after approval.",
        "- Classification is conservative keyword matching over title, filename, parent metadata, creators, and tags; review borderline rows before destructive deletion.",
        "",
        "## Summary",
        "",
        f"- Total PDFs: `{len(candidates)}`",
        f"- clear-law / keep source: `{clear_law}`",
        f"- adjacent-delete / delete source: `{adjacent}`",
        f"- nonlaw-delete / delete source: `{nonlaw}`",
        f"- Proposed delete-source total: `{delete_count}`",
        f"- Proposed keep-source total: `{keep_count}`",
        f"- Already has completed PaddleOCR Markdown: `{existing_md}`",
        f"- Needs PaddleOCR Markdown run: `{ocr_needed}`",
        f"- Existing PaddleOCR Markdown, source can be deleted after confirmation: `{delete_after_existing}`",
        "",
        "## Proposed Delete Queue",
        "",
    ]

    delete_items = [item for item in candidates if item.delete_policy == "delete"]
    if delete_items:
        lines.extend(
            [
                "| # | PDF key | Parent key | Item type | Size | Pages | Title | Author | Year | Classification | Suggested action | Existing Markdown | Basis | Path |",
                "|---:|---|---|---|---:|---:|---|---|---|---|---|---|---|---|",
            ]
        )
        for index, item in enumerate(delete_items, 1):
            attachment = item.attachment
            existing = item.existing_md.attachment_key if item.existing_md else ""
            pages = item.page_count if item.page_count is not None else "ERROR"
            lines.append(
                f"| {index} | {attachment.key} | {md_escape(attachment.parent_key)} | "
                f"{md_escape(attachment.parent_item_type)} | {item.size_mb:.1f} MB | {pages} | "
                f"{md_escape(item_title(attachment))} | {md_escape(author_summary(attachment))} | "
                f"{md_escape(item_year(attachment))} | {item.classification} | {item.suggested_action} | "
                f"{md_escape(existing)} | {md_escape(item.basis)} | {md_escape(attachment.path)} |"
            )
    else:
        lines.append("No delete-source candidates found.")

    keep_items = [item for item in candidates if item.delete_policy == "keep"]
    lines.extend(["", "## Proposed Keep Queue", ""])
    if keep_items:
        lines.extend(
            [
                "| # | PDF key | Parent key | Item type | Size | Pages | Title | Author | Year | Suggested action | Existing Markdown | Basis | Path |",
                "|---:|---|---|---|---:|---:|---|---|---|---|---|---|---|",
            ]
        )
        for index, item in enumerate(keep_items, 1):
            attachment = item.attachment
            existing = item.existing_md.attachment_key if item.existing_md else ""
            pages = item.page_count if item.page_count is not None else "ERROR"
            lines.append(
                f"| {index} | {attachment.key} | {md_escape(attachment.parent_key)} | "
                f"{md_escape(attachment.parent_item_type)} | {item.size_mb:.1f} MB | {pages} | "
                f"{md_escape(item_title(attachment))} | {md_escape(author_summary(attachment))} | "
                f"{md_escape(item_year(attachment))} | {item.suggested_action} | "
                f"{md_escape(existing)} | {md_escape(item.basis)} | {md_escape(attachment.path)} |"
            )
    else:
        lines.append("No keep-source candidates found.")

    lines.extend(["", "## Page Count Errors", ""])
    errors = [item for item in candidates if item.page_error]
    if errors:
        for item in errors:
            lines.append(f"- `{item.attachment.key}` {html.escape(item_title(item.attachment))}: `{md_escape(item.page_error)}`")
    else:
        lines.append("No page count errors.")

    return "\n".join(lines).rstrip() + "\n"


def main() -> int:
    candidates = collect_candidates()
    REPORT_PATH.parent.mkdir(parents=True, exist_ok=True)
    REPORT_PATH.write_text(render_report(candidates), encoding="utf-8")
    print(f"Wrote {REPORT_PATH}")
    print(f"total={len(candidates)}")
    print(f"delete={sum(1 for item in candidates if item.delete_policy == 'delete')}")
    print(f"keep={sum(1 for item in candidates if item.delete_policy == 'keep')}")
    print(f"existing_paddle_md={sum(1 for item in candidates if item.existing_md and item.existing_md.attachment_key)}")
    print(f"ocr_needed={sum(1 for item in candidates if item.suggested_action.startswith('ocr-'))}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
