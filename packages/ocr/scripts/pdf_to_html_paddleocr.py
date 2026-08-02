#!/usr/bin/env python3
"""
Local PDF folder converter: direct text extraction, or Baidu PaddleOCR-VL-1.6.

Every book is routed before anything is uploaded. A born-digital PDF carries the
text already, so it is extracted locally and costs nothing; only a scan or a
low-text document goes to the paid remote OCR. Before #137 this folder route
uploaded every PDF unconditionally, which on the live corpus meant paying for
roughly nine books in ten that a local reader could have handled — a 600-page
book is 50 remote round trips at 12 pages a chunk.

The routing question is the one `zotero_llm_worker.sample_text_layer()` already
answers on the Zotero side, and it is imported from there rather than restated:
PyMuPDF's non-space character count over five sampled pages. pdf-inspector's own
per-page OCR flags are deliberately not consulted — on this corpus they flag
hundreds of pages that PyMuPDF reads perfectly well, so they would put the paid
route back. See docs/planning/pdf-inspector-adoption.md.

Direct text route:
- packages/ocr/pdf_text.py extracts structured Markdown (pdf-inspector, with a
  PyMuPDF fallback), which is then rendered to standalone HTML
- no network call of any kind is made

Remote OCR route:
- Splits PDF into page chunks (max 12 pages / 49 MB per job)
- Submits to Baidu PaddleOCR async API
- Polls for completion, downloads JSONL results
- Extracts markdown text + images
- Downloads images locally, renders markdown to standalone HTML

Both routes write <book>/<book>.md next to <book>/<book>.html, which is what the
translation handoff picks up.

Usage:
    /opt/homebrew/bin/python3.11 scripts/pdf_to_html_paddleocr.py \
        --input-dir "编程书" \
        --output-dir output/html_books \
        [--workers 2] \
        [--limit-books 1] \
        [--force-text "Some Book.pdf"] \
        [--force-ocr "Scanned Book.pdf"] \
        [--route-plan-only]

BAIDU_PADDLEOCR_TOKEN is required in the process environment or monorepo-root
.env only when at least one book actually routes to remote OCR. A folder of
born-digital books converts with no credential configured at all.
"""

from __future__ import annotations

import argparse
import base64
import hashlib
import json
import logging
import os
import re
import shutil
import sys
import tempfile
import time
import unicodedata
import urllib.parse
from concurrent.futures import ThreadPoolExecutor, as_completed
from dataclasses import dataclass
from functools import lru_cache
from pathlib import Path
from typing import Any, Callable, Iterable

from progress import OperationProgress

import markdown
import requests

APP_ROOT = Path(__file__).resolve().parents[1]
if str(APP_ROOT) not in sys.path:
    sys.path.insert(0, str(APP_ROOT))
# The extractor and the classifier the Zotero route already uses, imported
# rather than reimplemented: one routing decision and one extraction engine
# across both entry points is the whole point of #137.
import pdf_text  # noqa: E402
import zotero_llm_worker  # noqa: E402

# ---------------------------------------------------------------------------
# PDF tooling
# ---------------------------------------------------------------------------
try:
    import fitz  # PyMuPDF
except Exception:
    fitz = None

try:
    from pypdf import PdfReader, PdfWriter
except Exception:
    PdfReader = None
    PdfWriter = None

# ---------------------------------------------------------------------------
# Constants (mirror zotero_llm_worker.py)
# ---------------------------------------------------------------------------
DEFAULT_BAIDU_JOB_URL = "https://paddleocr.aistudio-app.com/api/v2/ocr/jobs"
DEFAULT_BAIDU_MODEL = "PaddleOCR-VL-1.6"
LAYOUT_MODELS = {DEFAULT_BAIDU_MODEL, "PaddleOCR-VL-1.5", "PaddleOCR-VL", "PP-StructureV3"}
NETWORK_RETRY_DELAYS = (3, 6, 12, 24, 30)

# The two routes a PDF in a local folder can take. The names are the launcher's
# own route kinds (book_pipeline.rs), so a plan line can be read straight into a
# route item without a translation table in between.
ROUTE_DIRECT_TEXT = "direct_text"
ROUTE_REMOTE_PADDLEOCR = "remote_paddleocr"
#: Marker the launcher greps for when it previews a folder. Same shape as the
#: Zotero worker's BOOK_PIPELINE_ATTACHMENT_EVIDENCE: prefix, then one JSON
#: object per line, carrying its own schema version.
ROUTE_PLAN_MARKER = "BOOK_PIPELINE_LOCAL_PDF_ROUTE"
ROUTE_PLAN_SCHEMA = "local-pdf-route-plan-v1"
MISSING_TOKEN_MESSAGE = (
    "BAIDU_PADDLEOCR_TOKEN is not set. "
    "Export it or add it to .env in the monorepo root."
)

# ---------------------------------------------------------------------------
# Logging
# ---------------------------------------------------------------------------
logging.basicConfig(
    level=logging.INFO,
    format="%(asctime)s %(levelname)s %(message)s",
    datefmt="%H:%M:%S",
)

# ---------------------------------------------------------------------------
# Exceptions
# ---------------------------------------------------------------------------
class ConverterError(Exception):
    pass

class RetryableRemoteError(ConverterError):
    pass

class QuotaExhaustedError(ConverterError):
    pass

class DeadlineReached(ConverterError):
    pass

# ---------------------------------------------------------------------------
# Config
# ---------------------------------------------------------------------------
@dataclass(frozen=True, slots=True)
class Config:
    baidu_token: str
    baidu_job_url: str
    baidu_model: str
    max_ocr_pages_per_job: int
    baidu_max_upload_mb: int
    request_timeout: int
    poll_seconds: int
    workers: int

    @property
    def max_upload_bytes(self) -> int:
        return self.baidu_max_upload_mb * 1024 * 1024


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


def load_root_dotenv(start: Path = Path(__file__).resolve().parent) -> None:
    for candidate in (start.resolve(), *start.resolve().parents):
        if (candidate / "pyproject.toml").is_file() and (candidate / "packages").is_dir():
            load_dotenv(candidate / ".env")
            return


def load_config() -> Config:
    """Read the run's settings; a missing OCR credential is not an error here.

    It used to be: the token was demanded before a single PDF had been looked
    at, so a folder of born-digital books could not be converted at all without
    a paid account it was never going to use. The check moved to
    `require_ocr_credential()`, which runs once the routing plan says some book
    really is going to the remote engine.
    """
    load_root_dotenv()
    return Config(
        baidu_token=os.environ.get("BAIDU_PADDLEOCR_TOKEN", "").strip(),
        baidu_job_url=os.environ.get("BAIDU_PADDLEOCR_JOB_URL", DEFAULT_BAIDU_JOB_URL).rstrip("/"),
        baidu_model=os.environ.get("BAIDU_PADDLEOCR_MODEL", DEFAULT_BAIDU_MODEL).strip() or DEFAULT_BAIDU_MODEL,
        max_ocr_pages_per_job=int(os.environ.get("MAX_OCR_PAGES_PER_JOB", "12")),
        baidu_max_upload_mb=int(os.environ.get("BAIDU_MAX_UPLOAD_MB", "49")),
        request_timeout=int(os.environ.get("REQUEST_TIMEOUT_SECONDS", "120")),
        poll_seconds=int(os.environ.get("BAIDU_POLL_SECONDS", "5")),
        workers=int(os.environ.get("WORKERS", "2")),
    )


def is_layout_model(config: Config) -> bool:
    return config.baidu_model in LAYOUT_MODELS


def require_ocr_credential(config: Config) -> None:
    if not config.baidu_token:
        raise ConverterError(MISSING_TOKEN_MESSAGE)

# ---------------------------------------------------------------------------
# Routing
# ---------------------------------------------------------------------------
@lru_cache(maxsize=1)
def text_layer_thresholds() -> Any:
    """The worker's Config, read once, for its text-sampling thresholds.

    `get_config()` needs no credential of any kind — every field it reads has a
    default — so this is safe to call on a machine with nothing configured. It
    is cached because it re-parses the environment and the root `.env` on every
    call and the answer cannot change inside one run.
    """
    return zotero_llm_worker.get_config()


def classify_route(pdf_path: Path, page_count: int, thresholds: Any) -> tuple[str, str]:
    """Decide whether this book needs the paid engine, and say why.

    Deliberately the PyMuPDF character count and nothing else. A degraded text
    layer — the mojibake case, where the glyphs did not survive extraction —
    routes to OCR here rather than to the Zotero route's manual MinerU review:
    this entry point has no review step to hold a book in, and re-reading a
    broken text layer is the one case where paying for OCR is the right answer.
    """
    sample = zotero_llm_worker.sample_text_layer(pdf_path, page_count, thresholds)
    if sample.degraded:
        return ROUTE_REMOTE_PADDLEOCR, f"degraded text layer ({sample.reason})"
    if sample.extractable:
        return (
            ROUTE_DIRECT_TEXT,
            f"extractable text chars={sample.chars} sample_pages={sample.sample_pages}",
        )
    return (
        ROUTE_REMOTE_PADDLEOCR,
        f"low extractable text chars={sample.chars} sample_pages={sample.sample_pages}",
    )


def plan_routes(
    pdf_files: list[Path],
    *,
    force_text: Iterable[str] = (),
    force_ocr: Iterable[str] = (),
) -> dict[Path, tuple[str, str]]:
    """Route every book in the batch, honouring the launcher's overrides first.

    A forced book is never sampled: the user's re-route in the wizard is the
    decision, and re-deriving one that disagrees with the chip they were shown
    is exactly the drift this function exists to prevent. Names are matched
    exactly against the file name, which is what `book_pipeline.rs` sends.
    """
    forced_text = set(force_text)
    forced_ocr = set(force_ocr)
    for name in sorted(forced_text & forced_ocr):
        raise ConverterError(f"{name} was forced to both text extraction and OCR")
    named = {path.name for path in pdf_files}
    for name in sorted((forced_text | forced_ocr) - named):
        logging.warning("Route override for %r matches no PDF in this folder", name)

    plan: dict[Path, tuple[str, str]] = {}
    for path in pdf_files:
        if path.name in forced_text:
            plan[path] = (ROUTE_DIRECT_TEXT, "forced by route override")
        elif path.name in forced_ocr:
            plan[path] = (ROUTE_REMOTE_PADDLEOCR, "forced by route override")
        else:
            try:
                page_count = pdf_page_count(path)
            except Exception as exc:  # noqa: BLE001 - one unreadable file must not end the run
                plan[path] = (ROUTE_REMOTE_PADDLEOCR, f"page count failed: {exc}")
                continue
            plan[path] = classify_route(path, page_count, text_layer_thresholds())
    return plan


def emit_route_plan(plan: dict[Path, tuple[str, str]]) -> None:
    """Publish the plan on the log stream for the launcher's route preview."""
    for path, (route, reason) in plan.items():
        payload = {
            "schemaVersion": ROUTE_PLAN_SCHEMA,
            "path": str(path),
            "name": path.name,
            "route": route,
            "reason": reason,
        }
        logging.info(
            "%s %s",
            ROUTE_PLAN_MARKER,
            json.dumps(payload, ensure_ascii=False, separators=(",", ":")),
        )

# ---------------------------------------------------------------------------
# PDF helpers
# ---------------------------------------------------------------------------
def pdf_page_count(path: Path) -> int:
    if fitz is not None:
        with fitz.open(path) as doc:
            return int(doc.page_count)
    result = __import__("subprocess").run(
        ["pdfinfo", str(path)], capture_output=True, text=True, check=False
    )
    for line in result.stdout.splitlines():
        if line.startswith("Pages:"):
            return int(line.split(":", 1)[1].strip())
    raise ConverterError(f"Cannot determine page count for {path}")


def page_chunks(pages: list[int], chunk_size: int) -> list[list[int]]:
    chunks: list[list[int]] = []
    current: list[int] = []
    previous = None
    for p in pages:
        if previous is not None and p != previous + 1:
            if current:
                chunks.append(current)
            current = []
        if len(current) >= chunk_size:
            chunks.append(current)
            current = []
        current.append(p)
        previous = p
    if current:
        chunks.append(current)
    return chunks


def write_pdf_chunk(source: Path, pages: list[int], chunk_path: Path, *, prefer_fitz: bool = False) -> None:
    chunk_path.parent.mkdir(parents=True, exist_ok=True)
    tmp = chunk_path.with_suffix(".tmp.pdf")
    try:
        if fitz is not None and prefer_fitz:
            with fitz.open(source) as src:
                out = fitz.open()
                for page_no in pages:
                    out.insert_pdf(src, from_page=page_no - 1, to_page=page_no - 1)
                out.save(tmp)
                out.close()
        elif PdfReader is not None and PdfWriter is not None:
            reader = PdfReader(str(source))
            if reader.is_encrypted:
                try:
                    reader.decrypt("")
                except Exception as exc:
                    raise ConverterError(f"Encrypted PDF: {source}") from exc
            writer = PdfWriter()
            for page_no in pages:
                writer.add_page(reader.pages[page_no - 1])
            with tmp.open("wb") as fh:
                writer.write(fh)
        elif fitz is not None:
            with fitz.open(source) as src:
                out = fitz.open()
                for page_no in pages:
                    out.insert_pdf(src, from_page=page_no - 1, to_page=page_no - 1)
                out.save(tmp)
                out.close()
        else:
            raise ConverterError("Neither pypdf nor PyMuPDF available")
    except Exception:
        tmp.unlink(missing_ok=True)
        raise
    tmp.replace(chunk_path)


def make_chunk_specs(source: Path, pages: list[int], chunk_dir: Path, max_bytes: int) -> list[tuple[list[int], Path]]:
    raw_chunks = page_chunks(pages, 12)
    specs: list[tuple[list[int], Path]] = []
    for chunk_pages in raw_chunks:
        start, end = chunk_pages[0], chunk_pages[-1]
        chunk_path = chunk_dir / f"pages-{start:04d}-{end:04d}.pdf"
        if not chunk_path.exists():
            write_pdf_chunk(source, chunk_pages, chunk_path)
        if chunk_path.stat().st_size <= max_bytes:
            specs.append((chunk_pages, chunk_path))
            continue
        if len(chunk_pages) == 1:
            raise ConverterError(
                f"Single page {chunk_pages[0]} exceeds {max_bytes} bytes: {chunk_path.stat().st_size}"
            )
        mid = len(chunk_pages) // 2
        left = make_chunk_specs(source, chunk_pages[:mid], chunk_dir, max_bytes)
        right = make_chunk_specs(source, chunk_pages[mid:], chunk_dir, max_bytes)
        specs.extend(left)
        specs.extend(right)
    return specs

# ---------------------------------------------------------------------------
# Baidu OCR client
# ---------------------------------------------------------------------------
class BaiduOCRClient:
    def __init__(self, config: Config):
        # The last gate before anything is uploaded: no code path may reach the
        # paid API without a credential, whatever the routing plan said.
        require_ocr_credential(config)
        self.config = config
        self.session = requests.Session()
        self.session.headers.update({"Authorization": f"bearer {config.baidu_token}"})

    def submit_job(self, pdf_path: Path, batch_id: str) -> str:
        if is_layout_model(self.config):
            optional_payload = {"useDocOrientationClassify": False, "useDocUnwarping": False, "useChartRecognition": False}
        else:
            optional_payload = {"useDocOrientationClassify": False, "useDocUnwarping": False, "useTextlineOrientation": False}
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
                self._sleep_or_raise(exc, "submit", attempt)
        if response is None:
            raise RetryableRemoteError("Baidu submit failed before response")
        payload = self._checked_json(response, "submit")
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
                self._sleep_or_raise(exc, f"poll {job_id}", 0, deadline)
                continue
            try:
                payload = self._checked_json(response, "poll")
            except RetryableRemoteError as exc:
                self._sleep_or_raise_retryable(exc, f"poll {job_id}", 0, deadline)
                continue
            data = payload.get("data", {})
            state = data.get("state")
            if state == "done":
                return data["resultUrl"]["jsonUrl"]
            if state == "failed":
                raise ConverterError(f"Baidu job failed: {data.get('errorMsg')}")
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
                progress_key = (state, progress.get("extractedPages"), progress.get("totalPages"))
                if progress_key != last_progress:
                    last_progress = progress_key
                    last_progress_at = time.time()
                total_pages = progress.get("totalPages")
                try:
                    total_pages_int = int(total_pages)
                except Exception:
                    total_pages_int = 0
                effective = stalled_seconds
                if stalled_seconds and total_pages_int <= 1:
                    effective = max(stalled_seconds, 180)
                elif stalled_seconds and total_pages_int == 0:
                    effective = max(stalled_seconds * 2, 180)
                if stalled_seconds and time.time() - last_progress_at > effective:
                    raise ConverterError(f"Job stalled for {effective}s: {progress}")
                logging.info("Job %s %s (%s/%s)", job_id, state, progress.get("extractedPages", "?"), progress.get("totalPages", "?"))
                time.sleep(self.config.poll_seconds)
                continue
            raise ConverterError(f"Unexpected state for {job_id}: {state}")
        raise DeadlineReached(f"Deadline reached polling {job_id}")

    def download_jsonl(self, json_url: str) -> str:
        for attempt in range(len(NETWORK_RETRY_DELAYS) + 1):
            try:
                response = requests.get(json_url, timeout=self.config.request_timeout)
                response.raise_for_status()
                return response.text
            except requests.exceptions.RequestException as exc:
                self._sleep_or_raise(exc, "download", attempt)
        raise RetryableRemoteError("Download failed")

    def _sleep_or_raise(self, exc: requests.exceptions.RequestException, phase: str, attempt: int, deadline: float | None = None):
        if attempt >= len(NETWORK_RETRY_DELAYS):
            raise RetryableRemoteError(f"Baidu network error during {phase}: {type(exc).__name__}") from exc
        delay = NETWORK_RETRY_DELAYS[attempt]
        if deadline is not None and time.time() + delay > deadline:
            raise DeadlineReached(f"Deadline reached during {phase}") from exc
        logging.warning("Retry %s/%s in %ss for %s: %s", attempt + 1, len(NETWORK_RETRY_DELAYS), delay, phase, type(exc).__name__)
        time.sleep(delay)

    def _sleep_or_raise_retryable(self, exc: RetryableRemoteError, phase: str, attempt: int, deadline: float | None = None):
        if attempt >= len(NETWORK_RETRY_DELAYS):
            raise exc
        delay = NETWORK_RETRY_DELAYS[attempt]
        if deadline is not None and time.time() + delay > deadline:
            raise DeadlineReached(f"Deadline reached during {phase}") from exc
        logging.warning("Retryable %s/%s in %ss for %s: %s", attempt + 1, len(NETWORK_RETRY_DELAYS), delay, phase, exc)
        time.sleep(delay)

    def _checked_json(self, response: requests.Response, phase: str) -> dict[str, Any]:
        if response.status_code in {429, 503, 504}:
            raise RetryableRemoteError(f"Throttled: {response.status_code}")
        if response.status_code in {401, 403}:
            raise QuotaExhaustedError(f"Forbidden/quota: {response.status_code} {response.text}")
        if response.status_code >= 400:
            raise ConverterError(f"Baidu {phase} failed: {response.status_code} {response.text}")
        payload = response.json()
        code = payload.get("code", 0)
        if code in {12001}:
            raise QuotaExhaustedError(f"Daily quota exhausted: {payload}")
        if code in {10010, 12002}:
            raise RetryableRemoteError(f"Temporary throttling: {payload}")
        if code != 0:
            raise ConverterError(f"Baidu API error: {payload}")
        return payload

# ---------------------------------------------------------------------------
# Result parsing
# ---------------------------------------------------------------------------
def extract_layout_results(raw: dict[str, Any]) -> list[dict[str, Any]]:
    result = raw.get("result", raw)
    candidates = result.get("layoutParsingResults") or raw.get("layoutParsingResults") or []
    return [item for item in candidates if isinstance(item, dict)]


def parse_jsonl_results(jsonl_text: str) -> list[dict[str, Any]]:
    results: list[dict[str, Any]] = []
    for line in jsonl_text.splitlines():
        line = line.strip()
        if not line:
            continue
        try:
            parsed = json.loads(line)
        except json.JSONDecodeError:
            continue
        results.extend(extract_layout_results(parsed))
    return results

# ---------------------------------------------------------------------------
# Image download
# ---------------------------------------------------------------------------
def download_image(url: str, dest: Path, timeout: int = 60) -> bool:
    if dest.exists():
        return True
    dest.parent.mkdir(parents=True, exist_ok=True)
    for attempt in range(3):
        try:
            response = requests.get(url, timeout=timeout)
            response.raise_for_status()
            dest.write_bytes(response.content)
            return True
        except Exception as exc:
            logging.warning("Image download attempt %s/3 failed for %s: %s", attempt + 1, url, exc)
            time.sleep(2 ** attempt)
    return False


def guess_extension(content_type: str) -> str:
    mapping = {
        "image/jpeg": ".jpg",
        "image/jpg": ".jpg",
        "image/png": ".png",
        "image/webp": ".webp",
        "image/gif": ".gif",
    }
    return mapping.get(content_type.lower(), ".png")


def safe_filename(text: str, max_len: int = 80) -> str:
    text = re.sub(r"[\\/:*?\"<>|]", "_", text)
    text = re.sub(r"\s+", "_", text)
    text = text.strip("._")
    if len(text) > max_len:
        text = text[:max_len]
    return text or "img"


def directory_key(name: str) -> str:
    """The identity a name has in the filesystem's directory namespace.

    APFS and NTFS fold case and unicode normalization, so "Deep_Learning" and
    "deep_learning" are one directory on the platforms this ships on. Comparing
    the raw strings would call them distinct and hand both books the same path.
    """
    return unicodedata.normalize("NFC", name).casefold()


def assign_output_names(pdf_files: list[Path]) -> dict[Path, str]:
    """Give every PDF its own output directory name.

    safe_filename() is many-to-one — "Deep Learning.pdf" and "Deep_Learning.pdf"
    both normalize to "Deep_Learning" — and every per-book path (output dir,
    assets, .html, _state.json, chunk dir) derives from that one name. Two
    colliding books would therefore share a resume state and a chunk directory,
    and the second one silently assembles the first one's OCR results instead of
    its own. The first source in sorted order keeps the plain name; later
    collisions get a suffix derived from their own stem, so a book's directory
    stays the same across runs.
    """
    assigned: dict[Path, str] = {}
    taken: set[str] = set()
    for path in pdf_files:
        base = safe_filename(path.stem, max_len=120)
        name = base
        if directory_key(name) in taken:
            digest = hashlib.sha256(path.stem.encode("utf-8")).hexdigest()
            width = 6
            name = f"{base}_{digest[:width]}"
            while directory_key(name) in taken and width < len(digest):
                width += 2
                name = f"{base}_{digest[:width]}"
        taken.add(directory_key(name))
        assigned[path] = name
    return assigned

# ---------------------------------------------------------------------------
# Markdown -> HTML
# ---------------------------------------------------------------------------
def build_html(title: str, body_html: str, css: str | None = None) -> str:
    if css is None:
        css = """
        body { font-family: -apple-system, BlinkMacSystemFont, "Segoe UI", Roboto, "Noto Sans SC", "PingFang SC", "Microsoft YaHei", sans-serif; line-height: 1.7; max-width: 900px; margin: 40px auto; padding: 0 20px; color: #222; background: #fff; }
        h1 { font-size: 1.9em; border-bottom: 2px solid #ddd; padding-bottom: 0.3em; margin-top: 1.5em; }
        h2 { font-size: 1.5em; border-bottom: 1px solid #eee; padding-bottom: 0.2em; margin-top: 1.3em; }
        h3 { font-size: 1.25em; margin-top: 1.2em; }
        h4 { font-size: 1.1em; }
        img { max-width: 100%; height: auto; display: block; margin: 1em 0; border: 1px solid #eee; border-radius: 4px; }
        pre { background: #f6f8fa; padding: 1em; overflow-x: auto; border-radius: 6px; font-size: 0.9em; }
        code { background: #f0f0f0; padding: 0.15em 0.35em; border-radius: 3px; font-family: "SFMono-Regular", Consolas, "Liberation Mono", Menlo, monospace; font-size: 0.9em; }
        pre code { background: none; padding: 0; }
        table { border-collapse: collapse; width: 100%; margin: 1em 0; }
        th, td { border: 1px solid #ddd; padding: 0.5em; text-align: left; }
        th { background: #f6f8fa; }
        blockquote { border-left: 4px solid #ddd; margin: 1em 0; padding-left: 1em; color: #555; }
        ul, ol { padding-left: 1.5em; }
        .page-break { border-top: 2px dashed #ccc; margin: 2em 0; text-align: center; color: #999; font-size: 0.85em; }
        """
    return f"""<!DOCTYPE html>
<html lang="zh-CN">
<head>
<meta charset="UTF-8">
<meta name="viewport" content="width=device-width, initial-scale=1.0">
<title>{title}</title>
<style>{css}</style>
</head>
<body>
{body_html}
</body>
</html>"""


def page_anchor(page_no: int) -> str:
    """Mark where a source page began.

    A comment rather than a visible separator: the assembled Markdown is the
    translation handoff source, so anything printable here becomes body text the
    splitter and the translator have to carry. The page number is kept so a
    reviewer can still map a translated passage back to a page of the original.
    """
    return f"<!-- page: {page_no} -->"


def page_separator(page_no: int) -> str:
    """The visible separator the standalone HTML shows for the same page."""
    return f'<div class="page-break">— Page {page_no} —</div>'


def md_to_html(md_text: str) -> str:
    md = markdown.Markdown(extensions=["tables", "fenced_code", "toc", "nl2br"])
    return md.convert(md_text)


# ---------------------------------------------------------------------------
# Core conversion for one book
# ---------------------------------------------------------------------------
def extract_book_text(
    pdf_path: Path,
    output_dir: Path,
    operation_progress: OperationProgress,
    output_name: str | None = None,
) -> Path:
    """Convert one born-digital PDF locally. Returns path to the HTML file.

    Same output shape as the OCR route — `<book>/<book>.md` beside
    `<book>/<book>.html` and a `_state.json` — because the launcher scans the
    job output tree for artifacts and the translation handoff reads the
    Markdown; a book converted this way has to be indistinguishable downstream
    from one that was uploaded.

    No `_assets` directory: a text layer has no images to download, and an empty
    sidecar would travel into every reading project for nothing.

    The whole document is rewritten on every run rather than resumed. Local
    extraction is free and takes seconds, so there is nothing to resume from and
    a rerun after a fix must not leave the previous text in place.
    """
    book_name = pdf_path.stem
    safe_name = output_name or safe_filename(book_name, max_len=120)
    book_output_dir = output_dir / safe_name
    book_output_dir.mkdir(parents=True, exist_ok=True)
    html_path = book_output_dir / f"{safe_name}.html"
    md_path = book_output_dir / f"{safe_name}.md"
    state_path = book_output_dir / "_state.json"
    assets_dir = book_output_dir / f"{safe_name}_assets"

    operation_progress.update_item(book_name, 0, "extracting")
    logging.info("[%s] Extracting the embedded text layer…", book_name)
    extracted = pdf_text.extract_markdown(pdf_path)
    logging.info(
        "[%s] Extracted with %s%s",
        book_name,
        extracted.engine,
        f" ({extracted.fallback_reason})" if extracted.fallback_reason else "",
    )

    # A book can be re-routed after an earlier OCR run. The launcher carries a
    # sibling `_assets` directory into the translation project, so leaving that
    # sidecar behind would attach stale OCR figures to the new text-layer output.
    # Wait until extraction succeeds before retiring the earlier route's files.
    if assets_dir.is_symlink() or assets_dir.is_file():
        assets_dir.unlink()
    elif assets_dir.is_dir():
        shutil.rmtree(assets_dir)

    full_md = f"# {book_name}\n\n{extracted.markdown}".rstrip() + "\n"
    md_path.write_text(full_md, encoding="utf-8")
    html_path.write_text(
        build_html(title=book_name, body_html=md_to_html(full_md)), encoding="utf-8"
    )
    state_path.write_text(
        json.dumps(
            {
                "source_name": pdf_path.name,
                "route": ROUTE_DIRECT_TEXT,
                "engine": extracted.engine,
                "fallback_reason": extracted.fallback_reason,
                "running_heads_removed": list(extracted.running_heads),
                "chunks_done": [],
                "pages_total": extracted.page_count,
                "pages_done": extracted.page_count,
                "markdown_path": str(md_path),
                "html_path": str(html_path),
                "image_count": 0,
            },
            ensure_ascii=False,
            indent=2,
        ),
        encoding="utf-8",
    )
    operation_progress.update_item(book_name, extracted.page_count, "assembling")
    logging.info("[%s] Markdown saved: %s", book_name, md_path)
    logging.info("[%s] HTML saved: %s", book_name, html_path)
    return html_path


def process_book(
    pdf_path: Path,
    output_dir: Path,
    config: Config,
    temp_root: Path,
    operation_progress: OperationProgress,
    output_name: str | None = None,
    route: str | None = None,
) -> Path:
    """Convert a single PDF to markdown plus standalone HTML. Returns path to HTML file.

    ``output_name`` is the directory name to write under. main() passes the
    collision-free name from assign_output_names(); it falls back to the plain
    derivation when a single book is converted on its own.

    ``route`` is the decision `plan_routes()` already made, which main() passes
    down so the launcher's overrides are honoured and the book is not sampled
    twice. ``None`` means classify here, which is what a caller converting one
    book on its own gets.
    """
    book_name = pdf_path.stem
    safe_name = output_name or safe_filename(book_name, max_len=120)

    if route is None:
        route, reason = classify_route(
            pdf_path, pdf_page_count(pdf_path), text_layer_thresholds()
        )
        logging.info("[%s] Routed to %s: %s", book_name, route, reason)
    if route == ROUTE_DIRECT_TEXT:
        return extract_book_text(pdf_path, output_dir, operation_progress, output_name)

    book_output_dir = output_dir / safe_name
    assets_dir = book_output_dir / f"{safe_name}_assets"
    assets_dir.mkdir(parents=True, exist_ok=True)

    html_path = book_output_dir / f"{safe_name}.html"
    md_path = book_output_dir / f"{safe_name}.md"
    state_path = book_output_dir / "_state.json"

    # Load existing state, but only if it was written for this same PDF. Chunks
    # are named by page range alone, so resuming from another book's state would
    # assemble that book's OCR results under this book's title without erroring.
    # State written before this field existed carries no owner and is trusted, so
    # a book converted by an older build still resumes instead of paying to redo.
    state: dict[str, Any] = {"chunks_done": [], "pages_total": 0, "pages_done": 0}
    if state_path.exists():
        try:
            existing = json.loads(state_path.read_text(encoding="utf-8"))
        except Exception:
            existing = None
        if isinstance(existing, dict):
            owner = existing.get("source_name")
            if owner is None or owner == pdf_path.name:
                state = existing
            else:
                logging.warning(
                    "[%s] %s holds state for %r; starting this book from scratch",
                    book_name,
                    state_path,
                    owner,
                )
    state["source_name"] = pdf_path.name

    page_count = pdf_page_count(pdf_path)
    state["pages_total"] = page_count
    state_path.write_text(json.dumps(state, ensure_ascii=False, indent=2), encoding="utf-8")
    operation_progress.update_item(
        book_name, min(int(state.get("pages_done", 0)), page_count), "starting"
    )

    all_pages = list(range(1, page_count + 1))
    chunk_dir = temp_root / safe_name / "chunks"
    chunk_dir.mkdir(parents=True, exist_ok=True)

    logging.info("[%s] %s pages, chunking…", book_name, page_count)
    chunk_specs = make_chunk_specs(pdf_path, all_pages, chunk_dir, config.max_upload_bytes)
    logging.info("[%s] -> %s chunks", book_name, len(chunk_specs))

    # Determine which chunks still need processing
    done_chunks: set[str] = set(state.get("chunks_done", []))
    pending_specs = [(pages, path) for pages, path in chunk_specs if path.name not in done_chunks]

    client = BaiduOCRClient(config)

    # Process pending chunks
    for idx, (pages, chunk_path) in enumerate(pending_specs, 1):
        batch_id = f"{safe_name}-{pages[0]:04d}-{pages[-1]:04d}"
        logging.info("[%s] Chunk %s/%s  pages %s-%s  batch=%s", book_name, idx, len(pending_specs), pages[0], pages[-1], batch_id)

        # Submit
        deadline = time.time() + 1800  # 30 min per chunk
        try:
            operation_progress.touch("uploading")
            job_id = client.submit_job(chunk_path, batch_id)
        except ConverterError as submit_exc:
            err_msg = str(submit_exc)
            if "10005" in err_msg or "无法解析" in err_msg:
                logging.warning("[%s] pypdf chunk rejected (10005), retrying with PyMuPDF…", book_name)
                write_pdf_chunk(pdf_path, pages, chunk_path, prefer_fitz=True)
                job_id = client.submit_job(chunk_path, batch_id)
            else:
                raise
        logging.info("[%s] Job submitted: %s", book_name, job_id)

        # Poll
        completed_before_chunk = int(state.get("pages_done", 0))
        json_url = client.poll_json_url(
            job_id,
            deadline,
            on_progress=lambda extracted, _reported_total: operation_progress.update_item(
                book_name,
                completed_before_chunk
                + min(max(extracted or 0, 0), len(pages)),
                "extracting",
            ),
        )
        operation_progress.touch("downloading")
        logging.info("[%s] Job done, downloading result…", book_name)

        # Download JSONL
        jsonl_text = client.download_jsonl(json_url)
        jsonl_path = chunk_dir / f"{chunk_path.stem}.jsonl"
        jsonl_path.write_text(jsonl_text, encoding="utf-8")

        done_chunks.add(chunk_path.name)
        state["chunks_done"] = sorted(done_chunks)
        chunk_pages_map = {cp.name: len(p) for p, cp in chunk_specs}
        state["pages_done"] = sum(chunk_pages_map.get(c, 0) for c in done_chunks)
        state_path.write_text(json.dumps(state, ensure_ascii=False, indent=2), encoding="utf-8")
        operation_progress.update_item(book_name, state["pages_done"], "extracting")

        # Optional: clean chunk PDF to save disk
        chunk_path.unlink(missing_ok=True)

    # ------------------------------------------------------------------
    # Assemble HTML
    # ------------------------------------------------------------------
    logging.info("[%s] Assembling HTML…", book_name)
    operation_progress.update_item(book_name, page_count, "assembling")

    all_results: list[tuple[int, dict[str, Any]]] = []
    for pages, chunk_path in chunk_specs:
        jsonl_path = chunk_dir / f"{chunk_path.stem}.jsonl"
        if not jsonl_path.exists():
            raise ConverterError(f"Missing JSONL for chunk {chunk_path.stem}")
        results = parse_jsonl_results(jsonl_path.read_text(encoding="utf-8"))
        for i, res in enumerate(results):
            page_no = pages[0] + i if i < len(pages) else pages[0]
            all_results.append((page_no, res))

    # Sort by page number
    all_results.sort(key=lambda x: x[0])

    # Build markdown sections
    md_sections: list[str] = []
    html_sections: list[str] = []
    image_map: dict[str, str] = {}  # original_url -> local_relative_path

    for page_no, res in all_results:
        md_data = res.get("markdown") or {}
        text = str(md_data.get("text") or "").strip()
        images = md_data.get("images") or {}

        if isinstance(images, dict):
            for img_path, img_url in images.items():
                if not img_path or not img_url:
                    continue
                # Derive a safe local filename
                parsed = urllib.parse.urlparse(img_url)
                base = Path(parsed.path).name or f"img_{page_no}"
                if not base or "." not in base:
                    base = f"img_{page_no}_{hashlib.md5(img_url.encode()).hexdigest()[:8]}.png"
                local_name = safe_filename(base, max_len=100)
                local_path = assets_dir / local_name
                rel_path = f"{safe_name}_assets/{local_name}"

                if img_url not in image_map:
                    if download_image(img_url, local_path):
                        image_map[img_url] = rel_path
                    else:
                        # fallback: keep original URL
                        image_map[img_url] = img_url

                # Replace markdown image reference with local relative path
                # PaddleOCR may embed images as ![](path) or just path
                text = text.replace(str(img_path), str(image_map[img_url]))
                # Also try replacing bare URLs that might appear
                if img_url in text:
                    text = text.replace(img_url, str(image_map[img_url]))

        if text:
            # Two assemblies rather than one plus a substitution pass: rewriting
            # anchors afterwards would also rewrite an identical line that came
            # out of the book itself, which a technical book can easily contain.
            md_sections.append(f"\n{page_anchor(page_no)}\n\n{text}")
            html_sections.append(f"\n{page_separator(page_no)}\n\n{text}")

    full_md = f"# {book_name}\n\n" + "\n\n".join(md_sections)
    # The translation handoff reads this file, so it has to land on disk
    md_path.write_text(full_md, encoding="utf-8")
    body_html = md_to_html(f"# {book_name}\n\n" + "\n\n".join(html_sections))
    html = build_html(title=book_name, body_html=body_html)
    html_path.write_text(html, encoding="utf-8")

    logging.info("[%s] Markdown saved: %s", book_name, md_path)
    logging.info("[%s] HTML saved: %s", book_name, html_path)
    state["markdown_path"] = str(md_path)
    state["html_path"] = str(html_path)
    state["assets_dir"] = str(assets_dir)
    state["image_count"] = len(image_map)
    state_path.write_text(json.dumps(state, ensure_ascii=False, indent=2), encoding="utf-8")

    return html_path


# ---------------------------------------------------------------------------
# CLI
# ---------------------------------------------------------------------------
def main() -> int:
    parser = argparse.ArgumentParser(
        description="Local PDF folder to Markdown + HTML, direct text or PaddleOCR-VL-1.6"
    )
    parser.add_argument("--input-dir", required=True, help="Directory containing PDF files")
    parser.add_argument("--output-dir", default="output/html_books", help="Output directory")
    parser.add_argument("--workers", type=int, default=2, help="Concurrent books")
    parser.add_argument("--limit-books", type=int, default=0, help="Process only N books (0 = all)")
    parser.add_argument("--book", default="", help="Process a single book by filename substring")
    parser.add_argument(
        "--force-text",
        action="append",
        default=[],
        metavar="FILENAME",
        help="Extract this book's text layer locally, whatever the sample says (repeatable)",
    )
    parser.add_argument(
        "--force-ocr",
        action="append",
        default=[],
        metavar="FILENAME",
        help="Send this book to remote OCR, whatever the sample says (repeatable)",
    )
    parser.add_argument(
        "--route-plan-only",
        action="store_true",
        help="Print the routing decision for every book and exit without converting",
    )
    args = parser.parse_args()

    config = load_config()

    input_dir = Path(args.input_dir).resolve()
    output_dir = Path(args.output_dir).resolve()
    # Not in plan-only mode: the launcher previews a folder with the default
    # output directory, and creating it would leave an empty tree in the OCR
    # package root every time the wizard looked at a folder.
    if not args.route_plan_only:
        output_dir.mkdir(parents=True, exist_ok=True)

    pdf_files = sorted([p for p in input_dir.glob("*.pdf") if p.is_file()])
    if not pdf_files:
        logging.error("No PDF files found in %s", input_dir)
        return 1

    # Assigned over the unfiltered folder so --book/--limit-books cannot move a
    # book to a different output directory than a full run would give it.
    output_names = assign_output_names(pdf_files)

    if args.book:
        pdf_files = [p for p in pdf_files if args.book in p.name]
        if not pdf_files:
            logging.error("No PDF matching '%s'", args.book)
            return 1

    if args.limit_books > 0:
        pdf_files = pdf_files[:args.limit_books]

    try:
        plan = plan_routes(
            pdf_files, force_text=args.force_text, force_ocr=args.force_ocr
        )
    except ConverterError as exc:
        logging.error("%s", exc)
        return 1
    emit_route_plan(plan)
    if args.route_plan_only:
        return 0

    needs_ocr = [path for path, (route, _) in plan.items() if route != ROUTE_DIRECT_TEXT]
    if needs_ocr and not config.baidu_token:
        logging.error(
            "%s Needed by: %s",
            MISSING_TOKEN_MESSAGE,
            ", ".join(path.name for path in needs_ocr),
        )
        return 1

    logging.info(
        "Books to process: %s (%s local text, %s remote OCR)",
        len(pdf_files),
        len(pdf_files) - len(needs_ocr),
        len(needs_ocr),
    )
    for p in pdf_files:
        route, reason = plan[p]
        logging.info("  - %s (%s pages) -> %s: %s", p.name, pdf_page_count(p), route, reason)
        if output_names[p] != safe_filename(p.stem, max_len=120):
            logging.warning(
                "[%s] name collides with another PDF in this folder; writing to %s",
                p.name,
                output_names[p],
            )

    temp_root = output_dir / ".temp"
    temp_root.mkdir(parents=True, exist_ok=True)

    total_pages = sum(pdf_page_count(p) for p in pdf_files)
    ocr_pages = sum(pdf_page_count(p) for p in needs_ocr)
    logging.info(
        "Total pages: %s  Est. chunks: %s (from %s pages of remote OCR)  Workers: %s",
        total_pages,
        (ocr_pages + 11) // 12,
        ocr_pages,
        config.workers,
    )
    operation_progress = OperationProgress.from_environment(
        "extract", "pages", total=total_pages
    )
    operation_progress.start("starting")

    # Process books
    failed_books: list[str] = []
    if config.workers > 1 and len(pdf_files) > 1:
        with ThreadPoolExecutor(max_workers=config.workers) as executor:
            futures = {
                executor.submit(
                    process_book,
                    p,
                    output_dir,
                    config,
                    temp_root,
                    operation_progress,
                    output_names[p],
                    plan[p][0],
                ): p
                for p in pdf_files
            }
            for future in as_completed(futures):
                pdf = futures[future]
                try:
                    html_path = future.result()
                    logging.info("DONE: %s -> %s", pdf.name, html_path)
                except Exception as exc:
                    logging.error("FAILED: %s -> %s", pdf.name, exc, exc_info=True)
                    failed_books.append(pdf.name)
    else:
        for p in pdf_files:
            try:
                html_path = process_book(
                    p,
                    output_dir,
                    config,
                    temp_root,
                    operation_progress,
                    output_names[p],
                    plan[p][0],
                )
                logging.info("DONE: %s -> %s", p.name, html_path)
            except Exception as exc:
                logging.error("FAILED: %s -> %s", p.name, exc, exc_info=True)
                failed_books.append(p.name)

    if failed_books:
        operation_progress.touch("failed")
        logging.error("Completed with %s failed book(s): %s", len(failed_books), ", ".join(failed_books))
        return 1
    operation_progress.update(
        completed=total_pages,
        total=total_pages,
        phase="completed",
    )
    logging.info("All done. Output: %s", output_dir)
    return 0


if __name__ == "__main__":
    sys.exit(main())
