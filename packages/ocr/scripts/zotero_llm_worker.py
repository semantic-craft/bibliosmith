#!/usr/bin/env python3
"""Daily Zotero PDF-to-Markdown worker.

Routes selectable/born-digital PDFs through local text extraction and routes
scanned or otherwise non-extractable PDFs through Baidu AI Studio PaddleOCR-VL.
"""

from __future__ import annotations

import argparse
import datetime as dt
import hashlib
import json
import logging
import os
import re
import shutil
import sqlite3
import subprocess
import sys
import tempfile
import time
from dataclasses import dataclass
from pathlib import Path
from typing import Any, Callable, Iterable
from urllib.parse import unquote, urlparse

import requests

from progress import OperationProgress

try:
    import fitz  # PyMuPDF
except Exception:  # pragma: no cover - checked at runtime
    fitz = None

try:
    from pypdf import PdfReader, PdfWriter
except Exception:  # pragma: no cover - checked at runtime
    PdfReader = None
    PdfWriter = None


APP_ROOT = Path(__file__).resolve().parents[1]
if str(APP_ROOT) not in sys.path:
    sys.path.insert(0, str(APP_ROOT))
# Imported as a module, not by name: `pdf_text` imports this one back for the
# mojibake thresholds, so resolving the entry point at call time is what keeps
# the pair importable in either order.
import pdf_text  # noqa: E402

DEFAULT_OUTPUT_ROOT = Path.home() / "Zotero" / "OCR_OUTPUT"
DEFAULT_ZOTERO_STORAGE = Path.home() / "Zotero" / "storage"
DEFAULT_LOCAL_API = "http://127.0.0.1:23119/api/users/0"
DEFAULT_BAIDU_JOB_URL = "https://paddleocr.aistudio-app.com/api/v2/ocr/jobs"
DEFAULT_BAIDU_MODEL = "PaddleOCR-VL-1.6"
LAYOUT_MODELS = {DEFAULT_BAIDU_MODEL, "PaddleOCR-VL-1.5", "PaddleOCR-VL", "PP-StructureV3"}
NETWORK_RETRY_DELAYS = (3, 6, 12, 24, 30)
ROUTE_NEEDS_MINERU = "needs-mineru"
ROUTE_MINERU = "mineru"
WORKER_ATTACHMENT_EVIDENCE_SCHEMA = "zotero-worker-attachment-evidence-v1"
WORKER_EXTRACTION_CONTRACT_VERSION = "zotero-worker-extraction-v1"


class WorkerError(Exception):
    pass


class RetryableRemoteError(WorkerError):
    pass


class QuotaExhaustedError(WorkerError):
    pass


class DeadlineReached(WorkerError):
    pass


@dataclass
class Config:
    output_root: Path
    zotero_storage: Path
    zotero_local_api: str
    zotero_library_id: str
    zotero_library_type: str
    zotero_api_key: str
    baidu_token: str
    mineru_token_available: bool
    mineru_language: str
    baidu_job_url: str
    baidu_model: str
    max_ocr_pages_per_run: int
    max_ocr_pages_per_job: int
    baidu_max_upload_mb: int
    text_sample_min_chars: int
    text_page_min_chars: int
    dirty_text_guard: bool
    dirty_text_min_chars: int
    dirty_text_private_use_ratio: float
    force_ocr_item_types: set[str]
    request_timeout: int
    poll_seconds: int
    zotero_tags: tuple[str, ...] = ()


@dataclass
class Attachment:
    key: str
    title: str
    path: Path
    parent_key: str | None
    parent_item_type: str | None
    parent_title: str | None
    parent_creators: list[dict[str, Any]]
    parent_date: str | None
    content_type: str


@dataclass(frozen=True)
class TextLayerSample:
    extractable: bool
    chars: int
    sample_pages: list[int]
    degraded: bool
    reason: str


def load_dotenv(path: Path) -> None:
    if not path.exists():
        return
    for raw in path.read_text(encoding="utf-8").splitlines():
        line = raw.strip()
        if not line or line.startswith("#") or "=" not in line:
            continue
        key, value = line.split("=", 1)
        key = key.strip()
        value = value.strip().strip('"').strip("'")
        if key and value and key not in os.environ:
            os.environ[key] = value


def load_root_dotenv(start: Path = APP_ROOT) -> None:
    for candidate in (start.resolve(), *start.resolve().parents):
        if (candidate / "pyproject.toml").is_file() and (candidate / "packages").is_dir():
            load_dotenv(candidate / ".env")
            return


def normalize_zotero_tags(tags: Iterable[str]) -> tuple[str, ...]:
    """Keep only explicit, non-empty tag names while preserving CLI order."""
    normalized: list[str] = []
    seen: set[str] = set()
    for raw in tags:
        tag = raw.strip()
        if tag and tag not in seen:
            normalized.append(tag)
            seen.add(tag)
    return tuple(normalized)


def get_config(*, zotero_tags: Iterable[str] = ()) -> Config:
    load_root_dotenv()

    def env_int(name: str, default: int) -> int:
        value = os.environ.get(name, "").strip()
        return int(value) if value else default

    def env_float(name: str, default: float) -> float:
        value = os.environ.get(name, "").strip()
        return float(value) if value else default

    def env_bool(name: str, default: bool) -> bool:
        value = os.environ.get(name, "").strip().casefold()
        if not value:
            return default
        return value in {"1", "true", "yes", "on"}

    def env_path(name: str, default: Path) -> Path:
        value = os.environ.get(name, "").strip()
        if not value:
            return default
        return Path(os.path.expandvars(os.path.expanduser(value)))

    force_types = {
        item.strip()
        for item in os.environ.get("FORCE_OCR_ITEM_TYPES", "").split(",")
        if item.strip()
    }
    library_type = os.environ.get("ZOTERO_LIBRARY_TYPE", "users").strip() or "users"
    if library_type == "user":
        library_type = "users"
    elif library_type == "group":
        library_type = "groups"
    return Config(
        output_root=env_path("OCR_OUTPUT_ROOT", DEFAULT_OUTPUT_ROOT),
        zotero_storage=env_path("ZOTERO_STORAGE", DEFAULT_ZOTERO_STORAGE),
        zotero_local_api=os.environ.get("ZOTERO_LOCAL_API", DEFAULT_LOCAL_API).rstrip("/"),
        zotero_library_id=os.environ.get("ZOTERO_LIBRARY_ID", "YOUR_LIBRARY_ID"),
        zotero_library_type=library_type,
        zotero_api_key=os.environ.get("ZOTERO_API_KEY", "").strip(),
        baidu_token=os.environ.get("BAIDU_PADDLEOCR_TOKEN", "").strip(),
        mineru_token_available=bool(
            os.environ.get("MINERU_API_TOKEN", "").strip()
            or os.environ.get("MINERU_TOKEN", "").strip()
        ),
        mineru_language=os.environ.get("MINERU_LANGUAGE", "ch").strip() or "ch",
        baidu_job_url=os.environ.get("BAIDU_PADDLEOCR_JOB_URL", DEFAULT_BAIDU_JOB_URL).rstrip("/"),
        baidu_model=os.environ.get("BAIDU_PADDLEOCR_MODEL", DEFAULT_BAIDU_MODEL).strip() or DEFAULT_BAIDU_MODEL,
        max_ocr_pages_per_run=env_int("MAX_OCR_PAGES_PER_RUN", 10000),
        max_ocr_pages_per_job=env_int("MAX_OCR_PAGES_PER_JOB", 12),
        baidu_max_upload_mb=env_int("BAIDU_MAX_UPLOAD_MB", 49),
        text_sample_min_chars=env_int("TEXT_EXTRACT_SAMPLE_MIN_CHARS", 600),
        text_page_min_chars=env_int("TEXT_EXTRACT_PAGE_MIN_CHARS", 80),
        dirty_text_guard=env_bool("DIRTY_TEXT_LAYER_GUARD", True),
        dirty_text_min_chars=env_int("DIRTY_TEXT_MIN_SAMPLE_CHARS", 1000),
        dirty_text_private_use_ratio=env_float("DIRTY_TEXT_PRIVATE_USE_RATIO", 0.005),
        force_ocr_item_types=force_types,
        request_timeout=env_int("REQUEST_TIMEOUT_SECONDS", 60),
        poll_seconds=env_int("BAIDU_POLL_SECONDS", 5),
        zotero_tags=normalize_zotero_tags(zotero_tags),
    )


def configure_logging(output_root: Path) -> None:
    log_dir = output_root / ".state" / "logs"
    log_dir.mkdir(parents=True, exist_ok=True)
    log_path = log_dir / f"zotero-llm-{dt.datetime.now():%Y%m%d}.log"
    logging.basicConfig(
        level=logging.INFO,
        format="%(asctime)s %(levelname)s %(message)s",
        handlers=[
            logging.StreamHandler(sys.stdout),
            logging.FileHandler(log_path, encoding="utf-8"),
        ],
    )


def now_utc() -> str:
    return dt.datetime.now(dt.UTC).replace(microsecond=0).isoformat()


def safe_slug(value: str, max_len: int = 80) -> str:
    cleaned = re.sub(r"[\\/:*?\"<>|\x00-\x1f]+", " ", value)
    cleaned = re.sub(r"\s+", " ", cleaned).strip()
    if not cleaned:
        return "untitled"
    return cleaned[:max_len].rstrip()


def md5_file(path: Path) -> str:
    digest = hashlib.md5()
    with path.open("rb") as fh:
        for chunk in iter(lambda: fh.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def sha256_file(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as fh:
        for chunk in iter(lambda: fh.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def count_nonspace(text: str) -> int:
    return sum(1 for ch in text if not ch.isspace())


def is_private_use(ch: str) -> bool:
    code = ord(ch)
    return 0xE000 <= code <= 0xF8FF


def normalize_text(text: str) -> str:
    lines = [line.rstrip() for line in text.replace("\x00", "").splitlines()]
    out: list[str] = []
    blank = False
    for line in lines:
        if not line.strip():
            if not blank:
                out.append("")
            blank = True
        else:
            out.append(line)
            blank = False
    return "\n".join(out).strip()


def parse_pages(page_spec: str | None, total_pages: int) -> list[int]:
    if not page_spec:
        return list(range(1, total_pages + 1))
    pages: set[int] = set()
    for part in page_spec.split(","):
        part = part.strip()
        if not part:
            continue
        if "-" in part:
            start_s, end_s = part.split("-", 1)
            start = int(start_s)
            end = int(end_s)
            if start > end:
                raise ValueError(f"Invalid page range: {part}")
            pages.update(range(start, end + 1))
        else:
            pages.add(int(part))
    valid = sorted(page for page in pages if 1 <= page <= total_pages)
    if not valid:
        raise ValueError(f"No valid pages in range {page_spec!r} for {total_pages}-page PDF")
    return valid


def pdf_page_count(path: Path) -> int:
    if fitz is not None:
        with fitz.open(path) as doc:
            return int(doc.page_count)
    result = subprocess.run(
        ["pdfinfo", str(path)],
        check=True,
        capture_output=True,
        text=True,
    )
    for line in result.stdout.splitlines():
        if line.startswith("Pages:"):
            return int(line.split(":", 1)[1].strip())
    raise WorkerError(f"Could not determine page count for {path}")


def extract_text_pages(path: Path, pages: Iterable[int]) -> list[tuple[int, str]]:
    if fitz is None:
        raise WorkerError("PyMuPDF is required for direct PDF text extraction")
    extracted: list[tuple[int, str]] = []
    with fitz.open(path) as doc:
        for page_no in pages:
            page = doc.load_page(page_no - 1)
            extracted.append((page_no, normalize_text(page.get_text("text", sort=True))))
    return extracted


def text_layer_quality(text: str, chars: int, config: Config) -> tuple[bool, str]:
    """Flag a text layer whose glyphs did not survive extraction.

    Private-use characters are the signal: a broken ToUnicode CMap drops digits
    into U+F73x, and OCR text layers dump punctuation into U+E5xx.

    There used to be a second check on the ratio of fullwidth ASCII. It was
    measured against the live Zotero corpus in #138 and removed in #140: of the
    1123 real PDFs, it fired alone on 31 books and every one of them was
    legitimate GB/T typesetting, not mojibake \u2014 Chinese journals set embedded
    Latin and whole reference pages fullwidth. It found nothing this check
    misses, so do not add it back without corpus evidence.
    """
    if not config.dirty_text_guard or chars < config.dirty_text_min_chars:
        return False, ""
    nonspace = [ch for ch in text if not ch.isspace()]
    if not nonspace:
        return False, ""
    private_ratio = sum(1 for ch in nonspace if is_private_use(ch)) / len(nonspace)
    if private_ratio >= config.dirty_text_private_use_ratio:
        return True, f"private_use_ratio={private_ratio:.3f}>={config.dirty_text_private_use_ratio:.3f}"
    return False, ""


def sample_text_layer(path: Path, total_pages: int, config: Config) -> TextLayerSample:
    candidates = [1, 2, 3, max(1, total_pages // 2), total_pages]
    sample_pages = sorted({page for page in candidates if 1 <= page <= total_pages})
    try:
        sampled = extract_text_pages(path, sample_pages)
    except Exception as exc:
        logging.warning("Text sampling failed for %s: %s", path, exc)
        return TextLayerSample(False, 0, sample_pages, False, "")
    sample_text = "\n".join(text for _, text in sampled)
    chars = count_nonspace(sample_text)
    threshold = min(config.text_sample_min_chars, len(sample_pages) * config.text_page_min_chars)
    degraded, reason = text_layer_quality(sample_text, chars, config)
    return TextLayerSample(chars >= threshold, chars, sample_pages, degraded, reason)


def markdown_frontmatter(metadata: dict[str, Any]) -> str:
    lines = ["---"]
    for key, value in metadata.items():
        if value is None:
            lines.append(f"{key}: null")
        elif isinstance(value, (int, float)):
            lines.append(f"{key}: {value}")
        else:
            escaped = str(value).replace('"', '\\"')
            lines.append(f'{key}: "{escaped}"')
    lines.append("---")
    return "\n".join(lines)


def render_markdown(
    *,
    title: str,
    metadata: dict[str, Any],
    pages: list[tuple[int, str]],
) -> str:
    body = [markdown_frontmatter(metadata), "", f"# {title}", ""]
    for page_no, text in pages:
        body.append(f"## Page {page_no}")
        body.append("")
        body.append(text if text else "[no extractable text]")
        body.append("")
    return "\n".join(body).rstrip() + "\n"


def render_extracted_markdown(*, title: str, metadata: dict[str, Any], body: str) -> str:
    """Front matter, the book's title, then the document as extracted.

    No per-page heading, which is the whole difference from `render_markdown()`.
    `## Page N` is not a heading any book has, and the split stage cuts a new
    chapter at every heading of the shallowest level it finds, so a 629-page
    PDF came out as 629 chapters named "Page 1" through "Page 629" — the entire
    table of contents of the finished EPUB, with no real chapter name in it.
    """
    return "\n".join([markdown_frontmatter(metadata), "", f"# {title}", "", body]).rstrip() + "\n"


class StateDB:
    def __init__(self, path: Path):
        path.parent.mkdir(parents=True, exist_ok=True)
        self.conn = sqlite3.connect(path)
        self.conn.row_factory = sqlite3.Row
        self.init_schema()

    def init_schema(self) -> None:
        self.conn.executescript(
            """
            CREATE TABLE IF NOT EXISTS documents (
                pdf_key TEXT NOT NULL,
                source_md5 TEXT NOT NULL,
                parent_key TEXT,
                parent_item_type TEXT,
                title TEXT,
                route TEXT,
                status TEXT NOT NULL,
                page_count INTEGER,
                output_path TEXT,
                sidecar_path TEXT,
                zotero_attachment_key TEXT,
                extraction_contract_version TEXT,
                error TEXT,
                created_at TEXT NOT NULL,
                updated_at TEXT NOT NULL,
                PRIMARY KEY (pdf_key, source_md5)
            );
            CREATE TABLE IF NOT EXISTS chunks (
                pdf_key TEXT NOT NULL,
                source_md5 TEXT NOT NULL,
                start_page INTEGER NOT NULL,
                end_page INTEGER NOT NULL,
                chunk_path TEXT,
                job_id TEXT,
                status TEXT NOT NULL,
                jsonl_path TEXT,
                error TEXT,
                updated_at TEXT NOT NULL,
                PRIMARY KEY (pdf_key, source_md5, start_page, end_page)
            );
            """
        )
        document_columns = {
            row[1] for row in self.conn.execute("PRAGMA table_info(documents)").fetchall()
        }
        if "extraction_contract_version" not in document_columns:
            self.conn.execute(
                "ALTER TABLE documents ADD COLUMN extraction_contract_version TEXT"
            )
        self.conn.commit()

    def completed(self, pdf_key: str, source_md5: str) -> sqlite3.Row | None:
        row = self.conn.execute(
            "SELECT * FROM documents WHERE pdf_key=? AND source_md5=? AND status='completed'",
            (pdf_key, source_md5),
        ).fetchone()
        return row

    def document(self, pdf_key: str, source_md5: str) -> sqlite3.Row | None:
        return self.conn.execute(
            "SELECT * FROM documents WHERE pdf_key=? AND source_md5=?",
            (pdf_key, source_md5),
        ).fetchone()

    def same_parent_source_row(self, attachment: Attachment, source_md5: str) -> sqlite3.Row | None:
        if not attachment.parent_key:
            return None
        return self.conn.execute(
            """
            SELECT *
            FROM documents
            WHERE parent_key=?
              AND source_md5=?
              AND pdf_key<>?
              AND status IN (
                  'completed',
                  'local_complete',
                  'blocked_dirty_text_layer',
                  'skipped_completed'
              )
            ORDER BY
              CASE status
                WHEN 'completed' THEN 1
                WHEN 'local_complete' THEN 2
                WHEN 'blocked_dirty_text_layer' THEN 3
                ELSE 4
              END,
              updated_at DESC
            LIMIT 1
            """,
            (attachment.parent_key, source_md5, attachment.key),
        ).fetchone()

    def upsert_document(
        self,
        *,
        attachment: Attachment,
        source_md5: str,
        route: str,
        status: str,
        page_count: int,
        output_path: Path | None = None,
        sidecar_path: Path | None = None,
        zotero_attachment_key: str | None = None,
        error: str | None = None,
    ) -> None:
        ts = now_utc()
        self.conn.execute(
            """
            INSERT INTO documents (
                pdf_key, source_md5, parent_key, parent_item_type, title, route,
                status, page_count, output_path, sidecar_path, zotero_attachment_key,
                extraction_contract_version, error, created_at, updated_at
            ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
            ON CONFLICT(pdf_key, source_md5) DO UPDATE SET
                parent_key=excluded.parent_key,
                parent_item_type=excluded.parent_item_type,
                title=excluded.title,
                route=excluded.route,
                status=excluded.status,
                page_count=excluded.page_count,
                output_path=excluded.output_path,
                sidecar_path=excluded.sidecar_path,
                zotero_attachment_key=excluded.zotero_attachment_key,
                extraction_contract_version=excluded.extraction_contract_version,
                error=excluded.error,
                updated_at=excluded.updated_at
            """,
            (
                attachment.key,
                source_md5,
                attachment.parent_key,
                attachment.parent_item_type,
                attachment.title,
                route,
                status,
                page_count,
                str(output_path) if output_path else None,
                str(sidecar_path) if sidecar_path else None,
                zotero_attachment_key,
                WORKER_EXTRACTION_CONTRACT_VERSION,
                error,
                ts,
                ts,
            ),
        )
        self.conn.commit()

    def chunk(self, pdf_key: str, source_md5: str, start: int, end: int) -> sqlite3.Row | None:
        return self.conn.execute(
            """
            SELECT * FROM chunks
            WHERE pdf_key=? AND source_md5=? AND start_page=? AND end_page=?
            """,
            (pdf_key, source_md5, start, end),
        ).fetchone()

    def upsert_chunk(
        self,
        *,
        pdf_key: str,
        source_md5: str,
        start_page: int,
        end_page: int,
        status: str,
        chunk_path: Path | None = None,
        job_id: str | None = None,
        jsonl_path: Path | None = None,
        error: str | None = None,
    ) -> None:
        ts = now_utc()
        self.conn.execute(
            """
            INSERT INTO chunks (
                pdf_key, source_md5, start_page, end_page, chunk_path, job_id,
                status, jsonl_path, error, updated_at
            ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
            ON CONFLICT(pdf_key, source_md5, start_page, end_page) DO UPDATE SET
                chunk_path=excluded.chunk_path,
                job_id=excluded.job_id,
                status=excluded.status,
                jsonl_path=excluded.jsonl_path,
                error=excluded.error,
                updated_at=excluded.updated_at
            """,
            (
                pdf_key,
                source_md5,
                start_page,
                end_page,
                str(chunk_path) if chunk_path else None,
                job_id,
                status,
                str(jsonl_path) if jsonl_path else None,
                error,
                ts,
            ),
        )
        self.conn.commit()


class ZoteroLocalClient:
    def __init__(self, base_url: str, storage_root: Path, timeout: int):
        self.base_url = base_url.rstrip("/")
        self.storage_root = storage_root
        self.timeout = timeout
        self.session = requests.Session()

    def get(self, path: str, **params: Any) -> Any:
        response = self.session.get(
            f"{self.base_url}/{path.lstrip('/')}",
            params=params,
            timeout=self.timeout,
        )
        response.raise_for_status()
        return response.json()

    def ping(self) -> None:
        response = self.session.get(self.base_url.rsplit("/api/", 1)[0] + "/connector/ping", timeout=3)
        response.raise_for_status()

    def iter_pdf_attachments(self, limit: int | None = None) -> Iterable[Attachment]:
        parent_cache: dict[str, tuple[str | None, str | None, list[dict[str, Any]], str | None]] = {}
        start = 0
        page_size = 100
        yielded = 0
        while True:
            batch = self.get(
                "items",
                itemType="attachment",
                limit=page_size,
                start=start,
                format="json",
            )
            if not batch:
                break
            for item in batch:
                data = item.get("data", {})
                if data.get("contentType") != "application/pdf":
                    continue
                path = self.attachment_path(item)
                if not path or not path.exists():
                    logging.warning("Skipping missing PDF %s at %s", data.get("key"), path)
                    continue
                parent_key = data.get("parentItem")
                parent_type = None
                parent_title = None
                parent_creators: list[dict[str, Any]] = []
                parent_date = None
                if parent_key:
                    if parent_key not in parent_cache:
                        try:
                            parent = self.get(f"items/{parent_key}", format="json")
                            parent_data = parent.get("data", {})
                            parent_cache[parent_key] = (
                                parent_data.get("itemType"),
                                parent_data.get("title"),
                                parent_data.get("creators") or [],
                                parent_data.get("date"),
                            )
                        except Exception as exc:
                            logging.warning("Could not read parent %s: %s", parent_key, exc)
                            parent_cache[parent_key] = (None, None, [], None)
                    parent_type, parent_title, parent_creators, parent_date = parent_cache[parent_key]
                yielded += 1
                yield Attachment(
                    key=data["key"],
                    title=data.get("title") or path.stem,
                    path=path,
                    parent_key=parent_key,
                    parent_item_type=parent_type,
                    parent_title=parent_title,
                    parent_creators=parent_creators,
                    parent_date=parent_date,
                    content_type=data.get("contentType", ""),
                )
                if limit and yielded >= limit:
                    return
            start += len(batch)
            if len(batch) < page_size:
                break

    def search_pdf_attachments(self, query: str, limit: int | None = None) -> Iterable[Attachment]:
        """Find PDF attachments by a title/creator/year text search on the parent.

        Uses Zotero's own quick-search (qmode=titleCreatorYear) against the live
        local library rather than a pre-built index, so a result is never stale
        and there is no separate sync step to run first.
        """
        hits = self.get(
            "items",
            q=query,
            qmode="titleCreatorYear",
            itemType="-attachment",
            limit=max(1, min(limit or 20, 50)),
            format="json",
        )
        yielded = 0
        for item in hits:
            parent_data = item.get("data", {})
            parent_key = parent_data.get("key")
            if not parent_key:
                continue
            children = self.get(f"items/{parent_key}/children", format="json")
            for child in children:
                child_data = child.get("data", {})
                if child_data.get("contentType") != "application/pdf":
                    continue
                path = self.attachment_path(child)
                if not path or not path.exists():
                    continue
                yield Attachment(
                    key=child_data["key"],
                    title=child_data.get("title") or path.stem,
                    path=path,
                    parent_key=parent_key,
                    parent_item_type=parent_data.get("itemType"),
                    parent_title=parent_data.get("title"),
                    parent_creators=parent_data.get("creators") or [],
                    parent_date=parent_data.get("date"),
                    content_type=child_data.get("contentType", ""),
                )
                yielded += 1
                if limit and yielded >= limit:
                    return

    def get_pdf_attachment(self, key: str) -> Attachment:
        item = self.get(f"items/{key}", format="json")
        data = item.get("data", {})
        if data.get("contentType") != "application/pdf":
            raise WorkerError(f"{key} is not a PDF attachment")
        path = self.attachment_path(item)
        if not path or not path.exists():
            raise WorkerError(f"Attachment file not found for {key}: {path}")
        parent_key = data.get("parentItem")
        parent_type = None
        parent_title = None
        parent_creators: list[dict[str, Any]] = []
        parent_date = None
        if parent_key:
            parent = self.get(f"items/{parent_key}", format="json")
            parent_data = parent.get("data", {})
            parent_type = parent_data.get("itemType")
            parent_title = parent_data.get("title")
            parent_creators = parent_data.get("creators") or []
            parent_date = parent_data.get("date")
        return Attachment(
            key=data["key"],
            title=data.get("title") or path.stem,
            path=path,
            parent_key=parent_key,
            parent_item_type=parent_type,
            parent_title=parent_title,
            parent_creators=parent_creators,
            parent_date=parent_date,
            content_type=data.get("contentType", ""),
        )

    def attachment_path(self, item: dict[str, Any]) -> Path | None:
        enclosure = item.get("links", {}).get("enclosure", {})
        href = enclosure.get("href")
        if href and href.startswith("file://"):
            parsed = urlparse(href)
            return Path(unquote(parsed.path))
        data = item.get("data", {})
        raw_path = data.get("path")
        if raw_path and raw_path.startswith("storage:"):
            return self.storage_root / data["key"] / raw_path.split(":", 1)[1]
        if raw_path and raw_path.startswith("file://"):
            parsed = urlparse(raw_path)
            return Path(unquote(parsed.path))
        if raw_path:
            linked_path = Path(os.path.expanduser(unquote(raw_path)))
            if linked_path.is_absolute():
                return linked_path
        filename = data.get("filename")
        if filename:
            return self.storage_root / data["key"] / filename
        return None


class ZoteroWebClient:
    def __init__(self, config: Config):
        if not config.zotero_api_key:
            raise WorkerError("ZOTERO_API_KEY is not configured")
        if config.zotero_library_type not in {"users", "groups"}:
            raise WorkerError("ZOTERO_LIBRARY_TYPE must be 'users' or 'groups'")
        self.base_url = f"https://api.zotero.org/{config.zotero_library_type}/{config.zotero_library_id}"
        self.session = requests.Session()
        self.session.headers.update(
            {
                "Zotero-API-Key": config.zotero_api_key,
                "Zotero-API-Version": "3",
            }
        )
        self.timeout = config.request_timeout

    def create_markdown_attachment(
        self,
        *,
        parent_key: str | None,
        title: str,
        markdown_path: Path,
        tags: Iterable[str] = (),
        note: str = "",
    ) -> str:
        item: dict[str, Any] = {
            "itemType": "attachment",
            "linkMode": "imported_file",
            "title": title,
            "contentType": "text/markdown",
            "charset": "utf-8",
            "filename": markdown_path.name,
            "tags": [{"tag": tag} for tag in tags],
            "note": note,
            "relations": {},
        }
        if parent_key:
            item["parentItem"] = parent_key
        response = self.session.post(
            f"{self.base_url}/items",
            headers={"Content-Type": "application/json"},
            data=json.dumps([item], ensure_ascii=False).encode("utf-8"),
            timeout=self.timeout,
        )
        if response.status_code not in {200, 201}:
            raise WorkerError(f"Zotero item create failed: {response.status_code} {response.text}")
        payload = response.json()
        successful = payload.get("successful", {})
        if "0" not in successful:
            raise WorkerError(f"Zotero item create returned no item key: {payload}")
        attachment_key = successful["0"]["key"]
        self.upload_file(attachment_key, markdown_path)
        return attachment_key

    def upload_file(self, attachment_key: str, path: Path) -> None:
        file_md5 = md5_file(path)
        mtime_ms = int(path.stat().st_mtime * 1000)
        data = {
            "md5": file_md5,
            "filename": path.name,
            "filesize": str(path.stat().st_size),
            "mtime": str(mtime_ms),
        }
        headers = {"If-None-Match": "*"}
        response = self.session.post(
            f"{self.base_url}/items/{attachment_key}/file",
            headers=headers,
            data=data,
            timeout=self.timeout,
        )
        if response.status_code == 429:
            retry_after = response.headers.get("Retry-After", "30")
            raise RetryableRemoteError(f"Zotero upload throttled; retry after {retry_after}s")
        if response.status_code != 200:
            raise WorkerError(f"Zotero upload auth failed: {response.status_code} {response.text}")
        auth = response.json()
        if auth.get("exists"):
            return
        body = auth["prefix"].encode("utf-8") + path.read_bytes() + auth["suffix"].encode("utf-8")
        upload = requests.post(
            auth["url"],
            headers={"Content-Type": auth["contentType"]},
            data=body,
            timeout=self.timeout,
        )
        if upload.status_code != 201:
            raise WorkerError(f"Zotero file upload failed: {upload.status_code} {upload.text}")
        register = self.session.post(
            f"{self.base_url}/items/{attachment_key}/file",
            headers={"If-None-Match": "*"},
            data={"upload": auth["uploadKey"]},
            timeout=self.timeout,
        )
        if register.status_code != 204:
            raise WorkerError(f"Zotero upload register failed: {register.status_code} {register.text}")

    def patch_item(self, item_key: str, fields: dict[str, Any]) -> None:
        response = self.session.get(f"{self.base_url}/items/{item_key}", timeout=self.timeout)
        if response.status_code != 200:
            raise WorkerError(f"Zotero item lookup failed before patch: {response.status_code} {response.text}")
        version = response.json()["data"]["version"]
        headers = {
            "Content-Type": "application/json",
            "If-Unmodified-Since-Version": str(version),
        }
        patch_response = self.session.patch(
            f"{self.base_url}/items/{item_key}",
            headers=headers,
            data=json.dumps(fields, ensure_ascii=False).encode("utf-8"),
            timeout=self.timeout,
        )
        if patch_response.status_code not in {200, 204}:
            raise WorkerError(f"Zotero item patch failed: {patch_response.status_code} {patch_response.text}")

    def delete_item(self, item_key: str) -> None:
        response = self.session.get(f"{self.base_url}/items/{item_key}", timeout=self.timeout)
        if response.status_code == 404:
            return
        if response.status_code != 200:
            raise WorkerError(f"Zotero item lookup failed before delete: {response.status_code} {response.text}")
        version = response.json()["data"]["version"]
        delete_response = self.session.delete(
            f"{self.base_url}/items/{item_key}",
            headers={"If-Unmodified-Since-Version": str(version)},
            timeout=self.timeout,
        )
        if delete_response.status_code not in {204, 404}:
            raise WorkerError(f"Zotero item delete failed: {delete_response.status_code} {delete_response.text}")


class BaiduOCRClient:
    def __init__(self, config: Config):
        if not config.baidu_token:
            raise WorkerError("BAIDU_PADDLEOCR_TOKEN is not configured")
        self.config = config
        self.session = requests.Session()
        self.session.headers.update({"Authorization": f"bearer {config.baidu_token}"})

    def submit_job(self, pdf_path: Path, batch_id: str) -> str:
        if is_layout_model(self.config):
            optional_payload = {
                "useDocOrientationClassify": False,
                "useDocUnwarping": False,
                "useChartRecognition": False,
            }
        else:
            optional_payload = {
                "useDocOrientationClassify": False,
                "useDocUnwarping": False,
                "useTextlineOrientation": False,
            }
        data = {
            "model": self.config.baidu_model,
            "optionalPayload": json.dumps(optional_payload),
            "batchId": batch_id,
        }
        response = None
        for attempt in range(len(NETWORK_RETRY_DELAYS) + 1):
            try:
                with pdf_path.open("rb") as fh:
                    files = {"file": (pdf_path.name, fh, "application/pdf")}
                    response = self.session.post(
                        self.config.baidu_job_url,
                        data=data,
                        files=files,
                        timeout=self.config.request_timeout,
                    )
                break
            except requests.exceptions.RequestException as exc:
                self._sleep_or_raise_network_error(exc, phase="submit", attempt=attempt)
        if response is None:
            raise RetryableRemoteError("Baidu submit failed before receiving a response")
        try:
            payload = self._checked_json(response, "submit")
        except RetryableRemoteError as exc:
            raise RetryableRemoteError(f"Retryable Baidu submit response after upload: {exc}") from exc
        return payload["data"]["jobId"]

    def poll_json_url(
        self,
        job_id: str,
        deadline: float,
        on_progress: Callable[[int | None, int | None], None] | None = None,
    ) -> str:
        stalled_seconds = int(os.environ.get("BAIDU_STALLED_JOB_SECONDS", "300") or "0")
        last_progress: tuple[Any, ...] | None = None
        last_progress_at = time.time()
        while time.time() < deadline:
            try:
                response = self.session.get(
                    f"{self.config.baidu_job_url}/{job_id}",
                    headers={"Content-Type": "application/json"},
                    timeout=self.config.request_timeout,
                )
            except requests.exceptions.RequestException as exc:
                self._sleep_or_raise_network_error(exc, phase=f"poll {job_id}", attempt=0, deadline=deadline)
                continue
            try:
                payload = self._checked_json(response, "poll")
            except RetryableRemoteError as exc:
                self._sleep_or_raise_retryable(exc, phase=f"poll {job_id}", attempt=0, deadline=deadline)
                continue
            data = payload.get("data", {})
            state = data.get("state")
            if state == "done":
                return data["resultUrl"]["jsonUrl"]
            if state == "failed":
                raise WorkerError(f"Baidu OCR job failed: {data.get('errorMsg')}")
            if state in {"pending", "running"}:
                progress = data.get("extractProgress", {})
                if on_progress is not None:
                    try:
                        extracted_pages = int(progress.get("extractedPages"))
                    except (TypeError, ValueError):
                        extracted_pages = None
                    try:
                        reported_total = int(progress.get("totalPages"))
                    except (TypeError, ValueError):
                        reported_total = None
                    on_progress(extracted_pages, reported_total)
                progress_key = (
                    state,
                    progress.get("extractedPages"),
                    progress.get("totalPages"),
                )
                if progress_key != last_progress:
                    last_progress = progress_key
                    last_progress_at = time.time()
                total_pages = progress.get("totalPages")
                try:
                    total_pages_int = int(total_pages)
                except Exception:
                    total_pages_int = 0
                effective_stalled_seconds = stalled_seconds
                if stalled_seconds and total_pages_int == 1:
                    effective_stalled_seconds = max(stalled_seconds, 180)
                elif stalled_seconds and total_pages_int == 2:
                    effective_stalled_seconds = max(stalled_seconds, 120)
                elif stalled_seconds and total_pages_int == 0:
                    effective_stalled_seconds = max(stalled_seconds * 2, 180)
                if stalled_seconds and time.time() - last_progress_at > effective_stalled_seconds:
                    raise WorkerError(
                        f"Baidu OCR job stalled for {effective_stalled_seconds}s at progress "
                        f"{progress.get('extractedPages', '?')}/{progress.get('totalPages', '?')}"
                    )
                logging.info(
                    "Baidu job %s is %s (%s/%s pages)",
                    job_id,
                    state,
                    progress.get("extractedPages", "?"),
                    progress.get("totalPages", "?"),
                )
                time.sleep(self.config.poll_seconds)
                continue
            raise WorkerError(f"Unexpected Baidu job state for {job_id}: {state}")
        raise DeadlineReached(f"Deadline reached while polling Baidu job {job_id}")

    def download_jsonl(self, json_url: str) -> str:
        for attempt in range(len(NETWORK_RETRY_DELAYS) + 1):
            try:
                response = requests.get(json_url, timeout=self.config.request_timeout)
                response.raise_for_status()
                return response.text
            except requests.exceptions.RequestException as exc:
                self._sleep_or_raise_network_error(exc, phase="download result", attempt=attempt)
        raise RetryableRemoteError("Baidu result download failed before receiving a response")

    def _sleep_or_raise_network_error(
        self,
        exc: requests.exceptions.RequestException,
        *,
        phase: str,
        attempt: int,
        deadline: float | None = None,
    ) -> None:
        if attempt >= len(NETWORK_RETRY_DELAYS):
            raise RetryableRemoteError(f"Baidu network error during {phase}: {type(exc).__name__}") from exc
        delay = NETWORK_RETRY_DELAYS[attempt]
        if deadline is not None and time.time() + delay > deadline:
            raise DeadlineReached(f"Deadline reached during Baidu network retry for {phase}") from exc
        logging.warning(
            "Baidu network error during %s; retrying in %ss (%s/%s): %s",
            phase,
            delay,
            attempt + 1,
            len(NETWORK_RETRY_DELAYS),
            type(exc).__name__,
        )
        time.sleep(delay)

    def _sleep_or_raise_retryable(
        self,
        exc: RetryableRemoteError,
        *,
        phase: str,
        attempt: int,
        deadline: float | None = None,
    ) -> None:
        if attempt >= len(NETWORK_RETRY_DELAYS):
            raise exc
        delay = NETWORK_RETRY_DELAYS[attempt]
        if deadline is not None and time.time() + delay > deadline:
            raise DeadlineReached(f"Deadline reached during Baidu retry for {phase}") from exc
        logging.warning(
            "Baidu retryable response during %s; retrying in %ss (%s/%s): %s",
            phase,
            delay,
            attempt + 1,
            len(NETWORK_RETRY_DELAYS),
            exc,
        )
        time.sleep(delay)

    def _checked_json(self, response: requests.Response, phase: str) -> dict[str, Any]:
        if response.status_code in {429, 503, 504}:
            raise RetryableRemoteError(f"Baidu {phase} throttled: {response.status_code}")
        if response.status_code in {401, 403}:
            raise QuotaExhaustedError(f"Baidu {phase} forbidden/quota/token: {response.status_code} {response.text}")
        if response.status_code >= 400:
            raise WorkerError(f"Baidu {phase} failed: {response.status_code} {response.text}")
        payload = response.json()
        code = payload.get("code", 0)
        if code in {12001}:
            raise QuotaExhaustedError(f"Baidu daily page quota exhausted: {payload}")
        if code in {10010, 12002}:
            raise RetryableRemoteError(f"Baidu temporary throttling: {payload}")
        if code not in {0, None}:
            raise WorkerError(f"Baidu API error during {phase}: {payload}")
        return payload


def output_paths(config: Config, attachment: Attachment, route: str) -> tuple[Path, Path]:
    out_dir = config.output_root / ".state" / "staging" / attachment.key
    out_dir.mkdir(parents=True, exist_ok=True)
    base = markdown_basename(attachment)
    return out_dir / f"{base}.md", out_dir / f"{base}.jsonl"


def creator_name(creator: dict[str, Any]) -> str:
    name = str(creator.get("name") or "").strip()
    if name:
        return name
    first = str(creator.get("firstName") or "").strip()
    last = str(creator.get("lastName") or "").strip()
    if first and last:
        return f"{first} {last}"
    return last or first


def creator_summary(attachment: Attachment) -> str:
    creators = [creator_name(item) for item in attachment.parent_creators]
    creators = [item for item in creators if item]
    if not creators:
        return "未知作者"
    return creators[0]


def item_year(attachment: Attachment) -> str:
    date = attachment.parent_date or ""
    match = re.search(r"(1[5-9]\d{2}|20\d{2}|21\d{2})", date)
    return match.group(1) if match else "未知年份"


def markdown_basename(attachment: Attachment) -> str:
    title = attachment.parent_title or attachment.title or "未命名"
    return safe_slug(f"{creator_summary(attachment)}_{item_year(attachment)}_{title}", max_len=180)


def markdown_filename(attachment: Attachment) -> str:
    return f"{markdown_basename(attachment)}.md"


def source_pdf_filename(attachment: Attachment) -> str:
    return f"{markdown_basename(attachment)}.pdf"


def normalize_source_pdf_attachment_name(
    *,
    attachment: Attachment,
    config: Config,
    zotero: ZoteroWebClient,
) -> Attachment:
    if not attachment.parent_key or not attachment.parent_title:
        logging.info("SKIP source PDF rename for %s without a parent item", attachment.key)
        return attachment
    target_name = source_pdf_filename(attachment)
    if attachment.title == target_name and attachment.path.name == target_name:
        return attachment

    source_path = attachment.path
    target_path = source_path.with_name(target_name)
    renamed_file = False
    storage_item_dir = config.zotero_storage / attachment.key
    is_imported_storage_file = source_path.parent == storage_item_dir
    if is_imported_storage_file and source_path.name != target_name:
        if target_path.exists():
            raise WorkerError(f"Target source PDF filename already exists for {attachment.key}: {target_path}")
        source_path.rename(target_path)
        renamed_file = True
    elif not is_imported_storage_file:
        target_path = source_path

    try:
        zotero.patch_item(attachment.key, {"title": target_name, "filename": target_name})
    except Exception:
        if renamed_file and target_path.exists() and not source_path.exists():
            target_path.rename(source_path)
        raise

    logging.info(
        "Normalized source PDF attachment %s title=%r filename=%s",
        attachment.key,
        target_name,
        target_path.name,
    )
    attachment.title = target_name
    attachment.path = target_path
    return attachment


def markdown_page_numbers(path: Path) -> list[int]:
    """Page numbers a generated Markdown file records inline.

    Two shapes, because two are in circulation: the `<!-- page: N -->` anchor
    the PaddleOCR assembler and the PyMuPDF fallback write, and the `## Page N`
    heading carried by every file converted before the pdf-text route stopped
    emitting one. Structured pdf-text Markdown has neither — page breaks are
    not part of what pdf-inspector reconstructs — so its page list lives in the
    sidecar instead; see `sidecar_page_numbers()`.
    """
    pages: list[int] = []
    pattern = re.compile(r"^(?:## Page (\d+)|<!-- page: (\d+) -->)$")
    for line in path.read_text(encoding="utf-8").splitlines():
        match = pattern.match(line.strip())
        if match:
            pages.append(int(match.group(1) or match.group(2)))
    return pages


def sidecar_page_numbers(path: Path) -> list[int]:
    """Page numbers from a pdf-text sidecar's `pages` array."""
    if not path.exists():
        return []
    try:
        parsed = json.loads(path.read_text(encoding="utf-8"))
    except Exception:
        return []
    if not isinstance(parsed, dict) or not isinstance(parsed.get("pages"), list):
        return []
    return [
        entry["page"]
        for entry in parsed["pages"]
        if isinstance(entry, dict) and isinstance(entry.get("page"), int)
    ]


def selection_status(pages: list[int], page_count: int, *, uploaded: bool) -> str:
    full = pages == list(range(1, page_count + 1))
    if uploaded:
        return "completed" if full else "uploaded_partial"
    return "local_complete" if full else "local_partial"


def route_attachment(
    attachment: Attachment,
    path: Path,
    page_count: int,
    config: Config,
    *,
    pipeline_route: bool = False,
) -> tuple[str, str, int]:
    if attachment.parent_item_type in config.force_ocr_item_types:
        if pipeline_route and not config.baidu_token:
            return "missing-paddleocr-token", "PaddleOCR credential is not configured", 0
        return "paddle-ocr", f"parent itemType={attachment.parent_item_type}", 0
    sample = sample_text_layer(path, page_count, config)
    if sample.extractable:
        if sample.degraded:
            return (
                ROUTE_NEEDS_MINERU,
                f"extractable but degraded text layer ({sample.reason}) sample_pages={sample.sample_pages}",
                sample.chars,
            )
        return "pdf-text", f"extractable text chars={sample.chars} sample_pages={sample.sample_pages}", sample.chars
    if (
        pipeline_route
        and attachment.parent_item_type == "journalArticle"
        and config.mineru_token_available
    ):
        return (
            ROUTE_MINERU,
            f"pipeline MinerU route for low-text journal article sample_pages={sample.sample_pages}",
            sample.chars,
        )
    if pipeline_route and not config.baidu_token:
        return (
            "missing-paddleocr-token",
            f"PaddleOCR credential is not configured for low-text PDF sample_pages={sample.sample_pages}",
            sample.chars,
        )
    if attachment.parent_item_type == "book":
        return (
            "paddle-ocr",
            f"book with low extractable text chars={sample.chars} sample_pages={sample.sample_pages}",
            sample.chars,
        )
    return "paddle-ocr", f"low extractable text chars={sample.chars} sample_pages={sample.sample_pages}", sample.chars


def page_chunks(pages: list[int], chunk_size: int) -> list[list[int]]:
    chunks: list[list[int]] = []
    current: list[int] = []
    previous = None
    for page in pages:
        if previous is not None and page != previous + 1:
            if current:
                chunks.append(current)
            current = [page]
        else:
            current.append(page)
        previous = page
        if len(current) >= chunk_size:
            chunks.append(current)
            current = []
            previous = None
    if current:
        chunks.append(current)
    return chunks


def write_pdf_chunk(source: Path, pages: list[int], chunk_path: Path) -> None:
    if PdfReader is None or PdfWriter is None:
        raise WorkerError("pypdf is required for Paddle OCR PDF chunking")
    chunk_path.parent.mkdir(parents=True, exist_ok=True)
    tmp = chunk_path.with_suffix(".tmp.pdf")
    try:
        reader = PdfReader(str(source))
        if reader.is_encrypted:
            try:
                reader.decrypt("")
            except Exception as exc:
                raise WorkerError(f"Encrypted PDF cannot be chunked: {source}") from exc
        writer = PdfWriter()
        for page_no in pages:
            writer.add_page(reader.pages[page_no - 1])
        with tmp.open("wb") as fh:
            writer.write(fh)
    except Exception as exc:
        tmp.unlink(missing_ok=True)
        if fitz is None:
            raise WorkerError(f"Could not chunk PDF with pypdf and PyMuPDF is unavailable: {source}") from exc
        logging.warning(
            "pypdf could not chunk %s pages %s-%s; retrying with PyMuPDF: %s",
            source,
            pages[0],
            pages[-1],
            exc,
        )
        try:
            with fitz.open(source) as src:
                out = fitz.open()
                for page_no in pages:
                    out.insert_pdf(src, from_page=page_no - 1, to_page=page_no - 1)
                out.save(tmp)
                out.close()
        except Exception as fallback_exc:
            tmp.unlink(missing_ok=True)
            raise WorkerError(f"Could not chunk PDF with pypdf or PyMuPDF: {source}") from fallback_exc
    tmp.replace(chunk_path)


def ensure_chunk_under_limit(source: Path, pages: list[int], chunk_dir: Path, max_bytes: int) -> list[tuple[list[int], Path]]:
    if not pages:
        return []
    start, end = pages[0], pages[-1]
    chunk_path = chunk_dir / f"pages-{start:04d}-{end:04d}.pdf"
    if not chunk_path.exists():
        write_pdf_chunk(source, pages, chunk_path)
    if chunk_path.stat().st_size <= max_bytes:
        return [(pages, chunk_path)]
    if len(pages) == 1:
        raise WorkerError(f"Single-page chunk exceeds upload limit: {chunk_path}")
    chunk_path.unlink(missing_ok=True)
    midpoint = len(pages) // 2
    return ensure_chunk_under_limit(source, pages[:midpoint], chunk_dir, max_bytes) + ensure_chunk_under_limit(
        source, pages[midpoint:], chunk_dir, max_bytes
    )


def baidu_batch_id(pdf_key: str, source_md5: str, start_page: int, end_page: int) -> str:
    """Return a stable per-chunk batchId for Baidu OCR.

    Baidu rejects the 101st job created under the same batchId. Very large
    PDFs can exceed that limit, so batch ids must vary by chunk, not by whole
    document.
    """

    return f"{pdf_key}-{source_md5[:8]}-{start_page:04d}-{end_page:04d}"


def split_pages(pages: list[int]) -> tuple[list[int], list[int]]:
    midpoint = len(pages) // 2
    return pages[:midpoint], pages[midpoint:]


def should_split_failed_ocr(exc: Exception) -> bool:
    message = str(exc)
    retry_markers = [
        "状态码 500",
        "System Error",
        "OCR服务请求失败",
        "job parsing failed",
        "Baidu OCR job failed",
        "Baidu OCR job stalled",
    ]
    return any(marker in message for marker in retry_markers)


def is_layout_model(config: Config) -> bool:
    return config.baidu_model in LAYOUT_MODELS


def extract_ocr_result_texts(res: dict[str, Any]) -> list[str]:
    pruned = res.get("prunedResult") or {}
    texts: list[str] = []
    rec_texts = pruned.get("rec_texts")
    if isinstance(rec_texts, list):
        texts.extend(str(item) for item in rec_texts if str(item).strip())
        return texts
    for key in ("rec_text", "text", "content"):
        value = pruned.get(key)
        if isinstance(value, str) and value.strip():
            texts.append(value.strip())
    return texts


def extract_ocr_result_objects(raw: dict[str, Any]) -> list[dict[str, Any]]:
    result = raw.get("result", raw)
    ocr_results = result.get("ocrResults") or []
    return [item for item in ocr_results if isinstance(item, dict)]


def read_ocr_pages(jsonl_paths: list[tuple[int, Path]]) -> tuple[list[tuple[int, str]], list[str]]:
    pages: list[tuple[int, str]] = []
    combined_lines: list[str] = []
    for absolute_start, path in jsonl_paths:
        relative_page = 0
        for raw_line in path.read_text(encoding="utf-8").splitlines():
            line = raw_line.strip()
            if not line:
                continue
            parsed = json.loads(line)
            for res in extract_ocr_result_objects(parsed):
                texts = extract_ocr_result_texts(res)
                page_no = absolute_start + relative_page
                pages.append((page_no, "\n".join(texts).strip()))
                combined_lines.append(json.dumps({"page": page_no, "raw": res}, ensure_ascii=False))
                relative_page += 1
    pages.sort(key=lambda item: item[0])
    return pages, combined_lines


def extract_layout_results(raw: dict[str, Any]) -> list[dict[str, Any]]:
    result = raw.get("result", raw)
    candidates = result.get("layoutParsingResults") or raw.get("layoutParsingResults") or []
    return [item for item in candidates if isinstance(item, dict)]


def result_file_matches_model(path: Path, config: Config) -> bool:
    try:
        for raw_line in path.read_text(encoding="utf-8").splitlines():
            line = raw_line.strip()
            if not line:
                continue
            parsed = json.loads(line)
            if is_layout_model(config):
                return bool(extract_layout_results(parsed))
            return bool(extract_ocr_result_objects(parsed))
    except Exception:
        return False
    return False


def read_layout_markdown(jsonl_paths: list[tuple[int, Path]]) -> tuple[str, list[str], int]:
    sections: list[str] = []
    combined_lines: list[str] = []
    page_count = 0
    for absolute_start, path in jsonl_paths:
        relative_page = 0
        for raw_line in path.read_text(encoding="utf-8").splitlines():
            line = raw_line.strip()
            if not line:
                continue
            parsed = json.loads(line)
            for res in extract_layout_results(parsed):
                markdown = res.get("markdown") or {}
                text = str(markdown.get("text") or "").strip()
                images = markdown.get("images") or {}
                if isinstance(images, dict):
                    for image_path, image_url in images.items():
                        if image_path and image_url:
                            text = text.replace(str(image_path), str(image_url))
                page_no = absolute_start + relative_page
                if text:
                    sections.append(text)
                combined_lines.append(json.dumps({"page": page_no, "raw": res}, ensure_ascii=False))
                relative_page += 1
                page_count += 1
    return "\n\n".join(sections).strip() + "\n", combined_lines, page_count


def build_metadata(
    attachment: Attachment,
    *,
    source_md5: str,
    page_count: int,
    route: str,
    route_reason: str,
    source_path: Path,
) -> dict[str, Any]:
    return {
        "source_pdf_key": attachment.key,
        "parent_item_key": attachment.parent_key,
        "parent_item_type": attachment.parent_item_type,
        "source_pdf_md5": source_md5,
        "source_pdf_pages": page_count,
        "source_pdf_path": str(source_path),
        "conversion_route": route,
        "route_reason": route_reason,
        "generated_at": now_utc(),
    }


def attachment_title(attachment: Attachment, route: str) -> str:
    return markdown_filename(attachment)


def attachment_tags(config: Config) -> list[str]:
    """Return only tag names explicitly supplied by the user on the command line."""
    return list(config.zotero_tags)


def attachment_provenance_note(attachment: Attachment, route: str, config: Config) -> str:
    lines = [
        f"OCR Source Key: {attachment.key}",
        f"Conversion Route: {route}",
    ]
    if route == "paddle-ocr":
        lines.append(f"OCR Model: {config.baidu_model}")
    lines.append("Generated By: book-ocr-conversion")
    return "\n".join(lines)


def source_key_from_provenance_note(note: object) -> str | None:
    if not isinstance(note, str):
        return None
    match = re.search(r"OCR Source Key:\s*([A-Z0-9]{8})\b", note, flags=re.IGNORECASE)
    return match.group(1).upper() if match else None


def process_text_route(
    *,
    attachment: Attachment,
    config: Config,
    state: StateDB,
    source_md5: str,
    page_count: int,
    pages: list[int],
    route_reason: str,
    no_upload: bool,
    replace_attachment_key: str | None = None,
) -> str:
    markdown_path, sidecar_path = output_paths(config, attachment, "pdf-text")
    extracted = pdf_text.extract_markdown(attachment.path, pages=pages, dirty_text=config)
    logging.info(
        "Extracted %s with %s%s",
        attachment.key,
        extracted.engine,
        f" ({extracted.fallback_reason})" if extracted.fallback_reason else "",
    )
    metadata = build_metadata(
        attachment,
        source_md5=source_md5,
        page_count=page_count,
        route="pdf-text",
        route_reason=route_reason,
        source_path=attachment.path,
    )
    markdown_path.write_text(
        render_extracted_markdown(
            title=attachment.title, metadata=metadata, body=extracted.markdown
        ),
        encoding="utf-8",
    )
    sidecar_path.write_text(
        json.dumps(
            {
                "source_pdf_key": attachment.key,
                "route": "pdf-text",
                "engine": extracted.engine,
                "fallback_reason": extracted.fallback_reason,
                "running_heads_removed": list(extracted.running_heads),
                "pages": [{"page": page_no, "chars": chars} for page_no, chars in extracted.page_chars],
            },
            ensure_ascii=False,
            indent=2,
        ),
        encoding="utf-8",
    )
    if no_upload:
        state.upsert_document(
            attachment=attachment,
            source_md5=source_md5,
            route="pdf-text",
            status=selection_status(pages, page_count, uploaded=False),
            page_count=page_count,
            output_path=markdown_path,
            sidecar_path=sidecar_path,
        )
        return selection_status(pages, page_count, uploaded=False)
    zotero = ZoteroWebClient(config)
    zotero_key = zotero.create_markdown_attachment(
        parent_key=attachment.parent_key,
        title=attachment_title(attachment, "pdf-text"),
        markdown_path=markdown_path,
        tags=attachment_tags(config),
        note=attachment_provenance_note(attachment, "pdf-text", config),
    )
    if replace_attachment_key and replace_attachment_key != zotero_key:
        zotero.delete_item(replace_attachment_key)
    status = selection_status(pages, page_count, uploaded=True)
    state.upsert_document(
        attachment=attachment,
        source_md5=source_md5,
        route="pdf-text",
        status=status,
        page_count=page_count,
        output_path=markdown_path,
        sidecar_path=sidecar_path,
        zotero_attachment_key=zotero_key,
    )
    return status


def process_ocr_route(
    *,
    attachment: Attachment,
    config: Config,
    state: StateDB,
    source_md5: str,
    page_count: int,
    pages: list[int],
    route_reason: str,
    no_upload: bool,
    deadline: float,
    ocr_pages_remaining: int,
    replace_attachment_key: str | None = None,
) -> tuple[str, int]:
    if not config.baidu_token:
        state.upsert_document(
            attachment=attachment,
            source_md5=source_md5,
            route="paddle-ocr",
            status="blocked_missing_baidu_token",
            page_count=page_count,
            error="BAIDU_PADDLEOCR_TOKEN is not configured",
        )
        return "blocked_missing_baidu_token", 0

    baidu = BaiduOCRClient(config)
    operation_progress = OperationProgress.from_environment(
        "extract", "pages", total=len(pages)
    )
    operation_progress.start("starting")
    markdown_path, sidecar_path = output_paths(config, attachment, "paddle-ocr")
    chunk_dir = config.output_root / ".state" / "chunks" / attachment.key / source_md5
    max_bytes = config.baidu_max_upload_mb * 1024 * 1024
    page_groups = page_chunks(pages, config.max_ocr_pages_per_job)
    chunk_specs: list[tuple[list[int], Path]] = []
    for group in page_groups:
        chunk_specs.extend(ensure_chunk_under_limit(attachment.path, group, chunk_dir, max_bytes))

    completed_jsonl: list[tuple[int, Path]] = []
    pages_used = 0
    progress_done = 0
    queue = list(chunk_specs)
    while queue:
        chunk_pages, chunk_path = queue.pop(0)
        start_page, end_page = chunk_pages[0], chunk_pages[-1]
        if len(chunk_pages) > ocr_pages_remaining:
            logging.info("OCR page budget exhausted before %s pages %s-%s", attachment.key, start_page, end_page)
            state.upsert_document(
                attachment=attachment,
                source_md5=source_md5,
                route="paddle-ocr",
                status="partial_budget_exhausted",
                page_count=page_count,
                output_path=markdown_path if markdown_path.exists() else None,
                sidecar_path=sidecar_path if sidecar_path.exists() else None,
            )
            return "partial_budget_exhausted", pages_used
        if time.time() + 30 > deadline:
            raise DeadlineReached("Run deadline is too close for another OCR chunk")
        chunk_row = state.chunk(attachment.key, source_md5, start_page, end_page)
        if chunk_row and chunk_row["status"] == "done" and chunk_row["jsonl_path"] and Path(chunk_row["jsonl_path"]).exists():
            cached_path = Path(chunk_row["jsonl_path"])
            if result_file_matches_model(cached_path, config):
                completed_jsonl.append((start_page, cached_path))
                progress_done += len(chunk_pages)
                operation_progress.update(
                    completed=progress_done,
                    total=len(pages),
                    phase="extracting",
                )
                continue
            logging.info("Ignoring cached chunk %s pages %s-%s from another Baidu model", attachment.key, start_page, end_page)
            chunk_row = None
        jsonl_path = chunk_path.with_suffix(".jsonl")
        if chunk_row and chunk_row["status"] == "failed_split":
            left, right = split_pages(chunk_pages)
            retry_specs = []
            for part in (left, right):
                retry_specs.extend(ensure_chunk_under_limit(attachment.path, part, chunk_dir, max_bytes))
            queue = retry_specs + queue
            continue
        job_id = chunk_row["job_id"] if chunk_row and chunk_row["job_id"] else None
        try:
            if not job_id:
                operation_progress.touch("uploading")
                job_id = baidu.submit_job(
                    chunk_path,
                    batch_id=baidu_batch_id(attachment.key, source_md5, start_page, end_page),
                )
                state.upsert_chunk(
                    pdf_key=attachment.key,
                    source_md5=source_md5,
                    start_page=start_page,
                    end_page=end_page,
                    status="submitted",
                    chunk_path=chunk_path,
                    job_id=job_id,
                )
            completed_before_chunk = progress_done
            json_url = baidu.poll_json_url(
                job_id,
                deadline,
                on_progress=lambda extracted, _reported_total: operation_progress.update(
                    completed=completed_before_chunk
                    + min(max(extracted or 0, 0), len(chunk_pages)),
                    total=len(pages),
                    phase="extracting",
                ),
            )
            operation_progress.touch("downloading")
            jsonl_text = baidu.download_jsonl(json_url)
            jsonl_path.write_text(jsonl_text, encoding="utf-8")
            state.upsert_chunk(
                pdf_key=attachment.key,
                source_md5=source_md5,
                start_page=start_page,
                end_page=end_page,
                status="done",
                chunk_path=chunk_path,
                job_id=job_id,
                jsonl_path=jsonl_path,
            )
            completed_jsonl.append((start_page, jsonl_path))
            pages_used += len(chunk_pages)
            progress_done += len(chunk_pages)
            operation_progress.update(
                completed=progress_done,
                total=len(pages),
                phase="extracting",
            )
            ocr_pages_remaining -= len(chunk_pages)
        except WorkerError as exc:
            if should_split_failed_ocr(exc) and len(chunk_pages) > 1:
                logging.warning(
                    "Baidu chunk %s pages %s-%s failed; splitting and retrying: %s",
                    attachment.key,
                    start_page,
                    end_page,
                    exc,
                )
                state.upsert_chunk(
                    pdf_key=attachment.key,
                    source_md5=source_md5,
                    start_page=start_page,
                    end_page=end_page,
                    status="failed_split",
                    chunk_path=chunk_path,
                    job_id=job_id,
                    error=str(exc),
                )
                left, right = split_pages(chunk_pages)
                retry_specs = []
                for part in (left, right):
                    retry_specs.extend(ensure_chunk_under_limit(attachment.path, part, chunk_dir, max_bytes))
                queue = retry_specs + queue
                continue
            raise

    operation_progress.update(
        completed=len(pages), total=len(pages), phase="assembling"
    )
    if is_layout_model(config):
        markdown_text, combined_lines, parsed_pages = read_layout_markdown(completed_jsonl)
        if parsed_pages and parsed_pages != len(pages):
            logging.warning("Parsed %s markdown pages for %s expected %s", parsed_pages, attachment.key, len(pages))
        markdown_path.write_text(markdown_text, encoding="utf-8")
        sidecar_path.write_text("\n".join(combined_lines) + "\n", encoding="utf-8")
    else:
        ocr_pages, combined_lines = read_ocr_pages(completed_jsonl)
        metadata = build_metadata(
            attachment,
            source_md5=source_md5,
            page_count=page_count,
            route="paddle-ocr",
            route_reason=route_reason,
            source_path=attachment.path,
        )
        markdown_path.write_text(
            render_markdown(title=attachment.title, metadata=metadata, pages=ocr_pages),
            encoding="utf-8",
        )
        sidecar_path.write_text("\n".join(combined_lines) + "\n", encoding="utf-8")
    if no_upload:
        state.upsert_document(
            attachment=attachment,
            source_md5=source_md5,
            route="paddle-ocr",
            status=selection_status(pages, page_count, uploaded=False),
            page_count=page_count,
            output_path=markdown_path,
            sidecar_path=sidecar_path,
        )
        return selection_status(pages, page_count, uploaded=False), pages_used
    zotero = ZoteroWebClient(config)
    zotero_key = zotero.create_markdown_attachment(
        parent_key=attachment.parent_key,
        title=attachment_title(attachment, "paddle-ocr"),
        markdown_path=markdown_path,
        tags=attachment_tags(config),
        note=attachment_provenance_note(attachment, "paddle-ocr", config),
    )
    if replace_attachment_key and replace_attachment_key != zotero_key:
        zotero.delete_item(replace_attachment_key)
    status = selection_status(pages, page_count, uploaded=True)
    state.upsert_document(
        attachment=attachment,
        source_md5=source_md5,
        route="paddle-ocr",
        status=status,
        page_count=page_count,
        output_path=markdown_path,
        sidecar_path=sidecar_path,
        zotero_attachment_key=zotero_key,
    )
    return status, pages_used


def format_page_ranges(pages: list[int]) -> str:
    ranges: list[str] = []
    start = previous = pages[0]
    for page in pages[1:]:
        if page == previous + 1:
            previous = page
            continue
        ranges.append(str(start) if start == previous else f"{start}-{previous}")
        start = previous = page
    ranges.append(str(start) if start == previous else f"{start}-{previous}")
    return ",".join(ranges)


def _is_relative_mineru_reference(value: str) -> bool:
    reference = value.strip().strip("<>")
    parsed = urlparse(reference)
    return bool(reference) and not reference.startswith(("#", "/")) and not parsed.scheme


def rewrite_mineru_references(markdown: str, prefix: str) -> str:
    """Keep MinerU's relative assets reachable beside the staged Markdown."""

    def markdown_replacement(match: re.Match[str]) -> str:
        opening, raw_reference, closing = match.groups()
        if not _is_relative_mineru_reference(raw_reference):
            return match.group(0)
        wrapped = raw_reference.startswith("<") and raw_reference.endswith(">")
        reference = raw_reference.strip("<>")
        rewritten = f"{prefix}/{reference}"
        if wrapped:
            rewritten = f"<{rewritten}>"
        return f"{opening}{rewritten}{closing}"

    rewritten = re.sub(
        r"(!?\[[^\]]*\]\()([^\s)]+)([^)]*\))",
        markdown_replacement,
        markdown,
    )

    def html_replacement(match: re.Match[str]) -> str:
        attribute, quote, reference = match.groups()
        if not _is_relative_mineru_reference(reference):
            return match.group(0)
        return f"{attribute}={quote}{prefix}/{reference}{quote}"

    return re.sub(r"\b(src|href)=(['\"])([^'\"]+)\2", html_replacement, rewritten)


def process_mineru_route(
    *,
    attachment: Attachment,
    config: Config,
    state: StateDB,
    source_md5: str,
    page_count: int,
    pages: list[int],
    route_reason: str,
    no_upload: bool,
    deadline: float,
    replace_attachment_key: str | None = None,
) -> str:
    mineru_script = APP_ROOT / "mineru.py"
    if not mineru_script.is_file():
        raise WorkerError("MinerU adapter is not installed in the OCR package")
    run_root = config.output_root / ".state" / "mineru" / attachment.key / source_md5
    run_root.mkdir(parents=True, exist_ok=True)
    run_dir = Path(tempfile.mkdtemp(prefix="run-", dir=run_root))
    markdown_path, sidecar_path = output_paths(config, attachment, ROUTE_MINERU)
    artifact_dir = markdown_path.with_suffix(".mineru")
    timeout_seconds = max(1, int(deadline - time.time()))
    try:
        command = [
            sys.executable,
            str(mineru_script),
            str(attachment.path),
            "--output-dir",
            str(run_dir),
            "--mode",
            "batch",
            "--language",
            config.mineru_language,
            "--max-runtime-seconds",
            str(timeout_seconds),
        ]
        if pages != list(range(1, page_count + 1)):
            command.extend(["--page-ranges", format_page_ranges(pages)])
        try:
            completed = subprocess.run(
                command,
                cwd=APP_ROOT,
                capture_output=True,
                text=True,
                timeout=timeout_seconds,
                check=False,
            )
        except subprocess.TimeoutExpired as exc:
            raise DeadlineReached("MinerU single-attachment adapter exceeded the run deadline") from exc
        if completed.returncode != 0:
            raise WorkerError(
                f"MinerU single-attachment adapter failed with status {completed.returncode}"
            )
        markdown_candidates = list(run_dir.rglob("full.md"))
        if len(markdown_candidates) != 1:
            raise WorkerError(
                "MinerU single-attachment adapter did not produce exactly one Markdown result"
            )
        content = markdown_candidates[0].read_text(encoding="utf-8").strip()
        if not content:
            raise WorkerError("MinerU single-attachment adapter produced empty Markdown")
        staged_artifact_dir = Path(
            tempfile.mkdtemp(
                prefix=f".{artifact_dir.name}-",
                dir=artifact_dir.parent,
            )
        )
        try:
            shutil.copytree(
                markdown_candidates[0].parent,
                staged_artifact_dir,
                dirs_exist_ok=True,
            )
            if artifact_dir.exists():
                if not artifact_dir.is_dir():
                    raise WorkerError(
                        f"MinerU artifact destination is not a directory: {artifact_dir}"
                    )
                shutil.rmtree(artifact_dir)
            staged_artifact_dir.replace(artifact_dir)
        except BaseException:
            shutil.rmtree(staged_artifact_dir, ignore_errors=True)
            raise
        content = rewrite_mineru_references(content, artifact_dir.name)
    finally:
        shutil.rmtree(run_dir, ignore_errors=True)
    metadata = build_metadata(
        attachment,
        source_md5=source_md5,
        page_count=page_count,
        route=ROUTE_MINERU,
        route_reason=route_reason,
        source_path=attachment.path,
    )
    markdown_path.write_text(f"{markdown_frontmatter(metadata)}\n\n{content}\n", encoding="utf-8")
    sidecar_path.write_text(
        json.dumps(
            {
                "source_pdf_key": attachment.key,
                "route": ROUTE_MINERU,
                "pages": pages,
                "mineru_language": config.mineru_language,
                "mineru_artifact_dir": str(artifact_dir),
                "mineru_manifest_path": str(artifact_dir / "mineru_manifest.json"),
            },
            ensure_ascii=False,
            indent=2,
        )
        + "\n",
        encoding="utf-8",
    )
    status = selection_status(pages, page_count, uploaded=not no_upload)
    if no_upload:
        state.upsert_document(
            attachment=attachment,
            source_md5=source_md5,
            route=ROUTE_MINERU,
            status=status,
            page_count=page_count,
            output_path=markdown_path,
            sidecar_path=sidecar_path,
        )
        return status
    zotero = ZoteroWebClient(config)
    zotero_key = zotero.create_markdown_attachment(
        parent_key=attachment.parent_key,
        title=attachment_title(attachment, ROUTE_MINERU),
        markdown_path=markdown_path,
        tags=attachment_tags(config),
        note=attachment_provenance_note(attachment, ROUTE_MINERU, config),
    )
    if replace_attachment_key and replace_attachment_key != zotero_key:
        zotero.delete_item(replace_attachment_key)
    state.upsert_document(
        attachment=attachment,
        source_md5=source_md5,
        route=ROUTE_MINERU,
        status=status,
        page_count=page_count,
        output_path=markdown_path,
        sidecar_path=sidecar_path,
        zotero_attachment_key=zotero_key,
    )
    return status


def process_needs_mineru_route(
    *,
    attachment: Attachment,
    state: StateDB,
    source_md5: str,
    page_count: int,
    route_reason: str,
) -> str:
    state.upsert_document(
        attachment=attachment,
        source_md5=source_md5,
        route=ROUTE_NEEDS_MINERU,
        status="blocked_dirty_text_layer",
        page_count=page_count,
        error=route_reason,
    )
    logging.warning(
        "Blocked %s because the embedded text layer looks degraded; keep or regenerate with MinerU: %s",
        attachment.key,
        route_reason,
    )
    return "blocked_dirty_text_layer"


def process_attachment(
    *,
    attachment: Attachment,
    config: Config,
    state: StateDB,
    page_spec: str | None,
    no_upload: bool,
    dry_run: bool,
    force_route: str | None,
    deadline: float,
    ocr_pages_remaining: int,
    normalize_source: bool = True,
    pipeline_route: bool = False,
) -> tuple[str, int]:
    if normalize_source and not dry_run and not no_upload:
        attachment = normalize_source_pdf_attachment_name(
            attachment=attachment,
            config=config,
            zotero=ZoteroWebClient(config),
        )
    source_md5 = md5_file(attachment.path)
    completed_row = state.completed(attachment.key, source_md5)
    replace_attachment_key = None
    completed_route_matches = not force_route or completed_row is None or completed_row["route"] == force_route
    if completed_row and completed_route_matches and completed_row_is_current(completed_row, config):
        if no_upload or completed_row["zotero_attachment_key"]:
            logging.info("SKIP completed %s %s", attachment.key, attachment.title)
            return "skipped_completed", 0
        logging.info("REBUILD completed %s because uploaded Zotero attachment is missing", attachment.key)
    if completed_row:
        logging.info("REBUILD completed %s with current Baidu model %s", attachment.key, config.baidu_model)
        replace_attachment_key = completed_row["zotero_attachment_key"]
    page_count = pdf_page_count(attachment.path)
    if not force_route:
        sibling_row = state.same_parent_source_row(attachment, source_md5)
        if sibling_row is not None:
            state.upsert_document(
                attachment=attachment,
                source_md5=source_md5,
                route=sibling_row["route"],
                status="skipped_duplicate_source",
                page_count=page_count,
                error=(
                    f"same parent/source already handled by {sibling_row['pdf_key']} "
                    f"status={sibling_row['status']}"
                ),
            )
            logging.info(
                "SKIP duplicate source %s; same parent/source already handled by %s status=%s",
                attachment.key,
                sibling_row["pdf_key"],
                sibling_row["status"],
            )
            return "skipped_duplicate_source", 0
    pages = parse_pages(page_spec, page_count)
    if force_route:
        route = force_route
        route_reason = f"forced by CLI: {force_route}"
        sampled_chars = 0
    else:
        route, route_reason, sampled_chars = route_attachment(
            attachment,
            attachment.path,
            page_count,
            config,
            pipeline_route=pipeline_route,
        )
    logging.info(
        "PLAN %s route=%s pages=%s selected=%s parent_type=%s sampled_chars=%s title=%s",
        attachment.key,
        route,
        page_count,
        len(pages),
        attachment.parent_item_type,
        sampled_chars,
        attachment.title,
    )
    if dry_run:
        return "dry_run", 0
    if route == "pdf-text":
        return (
            process_text_route(
                attachment=attachment,
                config=config,
                state=state,
                source_md5=source_md5,
                page_count=page_count,
                pages=pages,
                route_reason=route_reason,
                no_upload=no_upload,
                replace_attachment_key=replace_attachment_key,
            ),
            0,
        )
    if route == "paddle-ocr":
        return process_ocr_route(
            attachment=attachment,
            config=config,
            state=state,
            source_md5=source_md5,
            page_count=page_count,
            pages=pages,
            route_reason=route_reason,
            no_upload=no_upload,
            deadline=deadline,
            ocr_pages_remaining=ocr_pages_remaining,
            replace_attachment_key=replace_attachment_key,
        )
    if route == ROUTE_MINERU:
        return (
            process_mineru_route(
                attachment=attachment,
                config=config,
                state=state,
                source_md5=source_md5,
                page_count=page_count,
                pages=pages,
                route_reason=route_reason,
                no_upload=no_upload,
                deadline=deadline,
                replace_attachment_key=replace_attachment_key,
            ),
            0,
        )
    if route == ROUTE_NEEDS_MINERU:
        return (
            process_needs_mineru_route(
                attachment=attachment,
                state=state,
                source_md5=source_md5,
                page_count=page_count,
                route_reason=route_reason,
            ),
            0,
        )
    raise WorkerError(f"Unknown route: {route}")


def attachment_matches_filters(attachment: Attachment, args: argparse.Namespace) -> bool:
    if attachment.key in set(args.exclude_attachment_key or []):
        logging.info("FILTER skip %s excluded attachment key", attachment.key)
        return False
    if args.parent_item_type and attachment.parent_item_type != args.parent_item_type:
        return False
    if args.min_size_mb is not None:
        size_mb = attachment.path.stat().st_size / (1024 * 1024)
        if size_mb <= args.min_size_mb:
            return False
    return True


def completed_row_is_current(row: sqlite3.Row, config: Config) -> bool:
    if row["extraction_contract_version"] != WORKER_EXTRACTION_CONTRACT_VERSION:
        return False
    output_path = row["output_path"]
    if not output_path or not Path(output_path).exists():
        return False
    if row["route"] != "paddle-ocr" or not is_layout_model(config):
        return True
    sample = Path(output_path).read_text(encoding="utf-8", errors="ignore")[:5000]
    return not sample.lstrip().startswith("---") and "\n## Page " not in sample


def emit_attachment_evidence(
    *,
    attachment: Attachment,
    state: StateDB,
    observed_status: str,
) -> None:
    source_md5 = md5_file(attachment.path)
    row = state.document(attachment.key, source_md5)
    if (
        row is None
        or row["status"] != "completed"
        or row["extraction_contract_version"] != WORKER_EXTRACTION_CONTRACT_VERSION
    ):
        return
    output_path = Path(row["output_path"] or "")
    markdown_attachment_key = str(row["zotero_attachment_key"] or "").strip()
    parent_item_key = str(row["parent_key"] or "").strip()
    if (
        not output_path.is_file()
        or output_path.stat().st_size == 0
        or not markdown_attachment_key
        or not parent_item_key
    ):
        return
    payload = {
        "schemaVersion": WORKER_ATTACHMENT_EVIDENCE_SCHEMA,
        "extractionContractVersion": row["extraction_contract_version"],
        "status": "already_completed" if observed_status == "skipped_completed" else "completed",
        "route": str(row["route"] or ""),
        "pdfAttachmentKey": attachment.key,
        "parentItemKey": parent_item_key,
        "sourceSha256": sha256_file(attachment.path),
        "markdownPath": str(output_path.resolve()),
        "markdownSha256": sha256_file(output_path),
        "markdownAttachmentKey": markdown_attachment_key,
    }
    logging.info(
        "BOOK_PIPELINE_ATTACHMENT_EVIDENCE %s",
        json.dumps(payload, ensure_ascii=False, separators=(",", ":")),
    )


def infer_generated_markdown_route(markdown_path: Path, sidecar_path: Path) -> str:
    if sidecar_path.exists():
        try:
            for raw_line in sidecar_path.read_text(encoding="utf-8").splitlines():
                line = raw_line.strip()
                if not line:
                    continue
                parsed = json.loads(line)
                if parsed.get("route") == "pdf-text":
                    return "pdf-text"
                raw = parsed.get("raw") if isinstance(parsed, dict) else None
                if isinstance(raw, dict) and (extract_layout_results(raw) or raw.get("markdown")):
                    return "paddle-ocr"
        except Exception:
            pass
    sample = markdown_path.read_text(encoding="utf-8", errors="ignore")[:5000]
    if sample.lstrip().startswith("---") or "\n## Page " in sample:
        return "pdf-text"
    return "paddle-ocr"


def upload_test(config: Config, state: StateDB, local: ZoteroLocalClient, key: str) -> None:
    attachment = local.get_pdf_attachment(key)
    staging_dir = config.output_root / ".state" / "staging" / key
    legacy_dir = config.output_root / key
    candidates = list(staging_dir.glob("*.md")) + list(legacy_dir.glob("*.md"))
    candidates = sorted(candidates, key=lambda p: p.stat().st_mtime, reverse=True)
    if not candidates:
        raise WorkerError(f"No generated Markdown exists for {key}. Run --smoke --no-upload first.")
    markdown_path = candidates[0]
    sidecar_path = markdown_path.with_suffix(".jsonl")
    route = infer_generated_markdown_route(markdown_path, sidecar_path)
    zotero_key = ZoteroWebClient(config).create_markdown_attachment(
        parent_key=attachment.parent_key,
        title=attachment_title(attachment, route),
        markdown_path=markdown_path,
        tags=attachment_tags(config),
        note=attachment_provenance_note(attachment, route, config),
    )
    source_md5 = md5_file(attachment.path)
    page_count = pdf_page_count(attachment.path)
    pages = sidecar_page_numbers(sidecar_path) or markdown_page_numbers(markdown_path)
    status = selection_status(pages, page_count, uploaded=True)
    state.upsert_document(
        attachment=attachment,
        source_md5=source_md5,
        route=route,
        status=status,
        page_count=page_count,
        output_path=markdown_path,
        sidecar_path=sidecar_path if sidecar_path.exists() else None,
        zotero_attachment_key=zotero_key,
    )
    logging.info("Uploaded %s to Zotero attachment %s status=%s", markdown_path, zotero_key, status)


def install_dependencies_check() -> None:
    missing: list[str] = []
    if fitz is None:
        missing.append("PyMuPDF")
    if PdfReader is None or PdfWriter is None:
        missing.append("pypdf")
    if missing:
        raise WorkerError(f"Missing Python dependencies: {', '.join(missing)}")


def add_zotero_tag_argument(parser: argparse.ArgumentParser) -> None:
    parser.add_argument(
        "--zotero-tag",
        action="append",
        default=[],
        help="Add exactly this Zotero tag to uploaded Markdown. Repeat for multiple tags; omitted means no tags.",
    )


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(description="Convert Zotero PDFs to Markdown for LLM use.")
    parser.add_argument("--dry-run", action="store_true", help="Enumerate and route PDFs without writing output.")
    parser.add_argument("--limit", type=int, help="Maximum number of PDF attachments to inspect/process.")
    parser.add_argument("--attachment-key", help="Process one Zotero PDF attachment key.")
    parser.add_argument(
        "--query",
        help="Find PDF attachments by title/creator/year text search instead of "
        "enumerating the whole library.",
    )
    parser.add_argument("--parent-item-type", help="Only process PDFs whose parent item has this Zotero itemType.")
    parser.add_argument("--min-size-mb", type=float, help="Only process PDFs larger than this many MiB.")
    parser.add_argument(
        "--exclude-attachment-key",
        action="append",
        default=[],
        help="Skip a Zotero PDF attachment key. Can be repeated.",
    )
    parser.add_argument("--pages", help="Limit processing to pages like '1-3' or '1,3,5-7'.")
    parser.add_argument("--no-upload", action="store_true", help="Generate local Markdown without Zotero Web API upload.")
    parser.add_argument("--smoke", action="store_true", help="Alias for one-attachment test mode; requires --attachment-key.")
    parser.add_argument("--upload-test", action="store_true", help="Upload the latest generated Markdown for --attachment-key.")
    parser.add_argument("--max-runtime-minutes", type=float, default=55.0)
    parser.add_argument("--force-ocr", action="store_true", help="Force Paddle OCR route.")
    parser.add_argument("--force-text", action="store_true", help="Force direct PDF text extraction route.")
    parser.add_argument("--force-mineru", action="store_true", help="Force MinerU route through the single-attachment worker adapter.")
    parser.add_argument(
        "--preserve-source",
        action="store_true",
        help="Do not rename the frozen source PDF or patch its Zotero attachment metadata.",
    )
    parser.add_argument(
        "--pipeline-route",
        action="store_true",
        help="Emit credential-aware route evidence for one durable pipeline attachment.",
    )
    add_zotero_tag_argument(parser)
    return parser


def main(argv: list[str] | None = None) -> int:
    args = build_parser().parse_args(argv)
    config = get_config(zotero_tags=args.zotero_tag)
    config.output_root.mkdir(parents=True, exist_ok=True)
    (config.output_root / ".state").mkdir(parents=True, exist_ok=True)
    configure_logging(config.output_root)
    install_dependencies_check()

    if args.smoke and not args.attachment_key:
        raise WorkerError("--smoke requires --attachment-key")
    if args.upload_test and not args.attachment_key:
        raise WorkerError("--upload-test requires --attachment-key")
    forced_routes = sum(bool(value) for value in (args.force_ocr, args.force_text, args.force_mineru))
    if forced_routes > 1:
        raise WorkerError("Use only one of --force-ocr, --force-text, or --force-mineru")

    state = StateDB(config.output_root / ".state" / "zotero_llm.sqlite3")
    local = ZoteroLocalClient(config.zotero_local_api, config.zotero_storage, config.request_timeout)
    local.ping()
    if args.upload_test:
        upload_test(config, state, local, args.attachment_key)
        return 0

    deadline = time.time() + args.max_runtime_minutes * 60
    force_route = (
        "paddle-ocr"
        if args.force_ocr
        else "pdf-text"
        if args.force_text
        else ROUTE_MINERU
        if args.force_mineru
        else None
    )
    ocr_pages_remaining = config.max_ocr_pages_per_run
    processed = 0
    statuses: dict[str, int] = {}
    attachments: Iterable[Attachment]
    if args.attachment_key:
        attachments = [local.get_pdf_attachment(args.attachment_key)]
    elif args.query:
        attachments = local.search_pdf_attachments(args.query, limit=args.limit)
    else:
        attachments = local.iter_pdf_attachments(limit=args.limit)

    for attachment in attachments:
        if not attachment_matches_filters(attachment, args):
            continue
        if time.time() + 15 > deadline:
            logging.info("Deadline reached before next attachment")
            break
        try:
            status, ocr_pages_used = process_attachment(
                attachment=attachment,
                config=config,
                state=state,
                page_spec=args.pages,
                no_upload=args.no_upload,
                dry_run=args.dry_run,
                force_route=force_route,
                deadline=deadline,
                ocr_pages_remaining=ocr_pages_remaining,
                normalize_source=not args.preserve_source,
                pipeline_route=args.pipeline_route,
            )
            emit_attachment_evidence(
                attachment=attachment,
                state=state,
                observed_status=status,
            )
            ocr_pages_remaining -= ocr_pages_used
            statuses[status] = statuses.get(status, 0) + 1
            processed += 1
            if status == "partial_budget_exhausted":
                logging.info("Stopping batch after OCR page budget was exhausted")
                break
        except QuotaExhaustedError as exc:
            logging.error("Quota/token stop: %s", exc)
            break
        except RetryableRemoteError as exc:
            logging.warning("Retryable remote error for %s: %s", attachment.key, exc)
            statuses["retryable_error"] = statuses.get("retryable_error", 0) + 1
            time.sleep(10)
        except DeadlineReached as exc:
            logging.info("Deadline reached: %s", exc)
            break
        except Exception as exc:
            logging.exception("Failed %s: %s", attachment.key, exc)
            try:
                source_md5 = md5_file(attachment.path)
                page_count = pdf_page_count(attachment.path)
                state.upsert_document(
                    attachment=attachment,
                    source_md5=source_md5,
                    route="unknown",
                    status="error",
                    page_count=page_count,
                    error=str(exc),
                )
            except Exception:
                pass
            statuses["error"] = statuses.get("error", 0) + 1
    logging.info("SUMMARY processed=%s statuses=%s ocr_pages_remaining=%s", processed, statuses, ocr_pages_remaining)
    return 0


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except KeyboardInterrupt:
        raise SystemExit(130)
    except Exception as exc:
        logging.exception("Fatal error: %s", exc)
        raise SystemExit(1)
