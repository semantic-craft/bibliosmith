#!/usr/bin/env python3
"""Batch CLI for MinerU Precision Extract API.

Designed for scanned/image-heavy books and academic papers. It uses the v4
Precision Extract API with the VLM model by default, supports local files and
URLs, polls batch results, downloads each full_zip_url, and extracts the result
archive containing full.md and JSON artifacts.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import re
import sys
import time
import zipfile
from dataclasses import dataclass
from pathlib import Path
from urllib.parse import urlparse

import requests

_PROGRESS_SCRIPTS = Path(__file__).resolve().parent / "scripts"
if str(_PROGRESS_SCRIPTS) not in sys.path:
    sys.path.insert(0, str(_PROGRESS_SCRIPTS))
from progress import OperationProgress


APP_ROOT = Path(__file__).resolve().parent
FILE_URLS_BATCH_URL = "https://mineru.net/api/v4/file-urls/batch"
SINGLE_TASK_URL = "https://mineru.net/api/v4/extract/task"
URL_TASK_BATCH_URL = "https://mineru.net/api/v4/extract/task/batch"
SINGLE_TASK_RESULT_URL = "https://mineru.net/api/v4/extract/task/{task_id}"
RESULTS_BATCH_URL = "https://mineru.net/api/v4/extract-results/batch/{batch_id}"
SUPPORTED_SUFFIXES = {
    ".pdf",
    ".png",
    ".jpg",
    ".jpeg",
    ".jp2",
    ".webp",
    ".gif",
    ".bmp",
    ".doc",
    ".docx",
    ".ppt",
    ".pptx",
    ".xls",
    ".xlsx",
    ".html",
    ".htm",
}
MAX_FILE_BYTES = 200 * 1024 * 1024
MAX_PAGES = 200
TERMINAL_STATES = {"done", "failed"}
OPERATION_PROGRESS = OperationProgress.from_environment("extract", "pages")


class MinerUError(Exception):
    pass


@dataclass(frozen=True)
class WorkItem:
    source: str
    name: str
    data_id: str
    local_path: Path | None = None
    url: str | None = None
    page_ranges: str | None = None


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


def is_url(value: str) -> bool:
    parsed = urlparse(value)
    return parsed.scheme in {"http", "https"}


def safe_slug(value: str, max_len: int = 100) -> str:
    cleaned = re.sub(r"[\\/:*?\"<>|\x00-\x1f]+", "_", value)
    cleaned = re.sub(r"\s+", " ", cleaned).strip(" ._")
    return (cleaned or "document")[:max_len].rstrip(" ._")


def data_id_for(value: str) -> str:
    stem = safe_slug(Path(urlparse(value).path).name or Path(value).name or "document", max_len=80)
    digest = hashlib.sha1(value.encode("utf-8")).hexdigest()[:10]
    return f"{stem}-{digest}"


def iter_local_files(path: Path) -> list[Path]:
    if path.is_file():
        return [path] if path.suffix.lower() in SUPPORTED_SUFFIXES else []
    if path.is_dir():
        ignored = {".git", ".venv", ".state", "tmp", "__pycache__"}
        files: list[Path] = []
        for candidate in path.rglob("*"):
            if not candidate.is_file():
                continue
            if ignored.intersection(candidate.parts):
                continue
            if candidate.suffix.lower() in SUPPORTED_SUFFIXES:
                files.append(candidate)
        return sorted(files)
    raise FileNotFoundError(path)


def pdf_page_count(path: Path) -> int | None:
    if path.suffix.lower() != ".pdf":
        return None
    try:
        import fitz  # type: ignore

        with fitz.open(path) as doc:
            return int(doc.page_count)
    except Exception:
        pass
    try:
        from pypdf import PdfReader  # type: ignore

        return len(PdfReader(str(path)).pages)
    except Exception:
        return None


def validate_local_file(path: Path, *, allow_over_limit: bool) -> None:
    suffix = path.suffix.lower()
    if suffix not in SUPPORTED_SUFFIXES:
        raise MinerUError(f"Unsupported file type: {path}")
    size = path.stat().st_size
    if size > MAX_FILE_BYTES and not allow_over_limit:
        raise MinerUError(
            f"File exceeds MinerU Precision API 200MB limit: {path} ({size / 1024 / 1024:.1f}MB)"
        )
    pages = pdf_page_count(path)
    if pages is not None and pages > MAX_PAGES and not allow_over_limit:
        raise MinerUError(f"PDF exceeds MinerU Precision API 200-page limit: {path} ({pages} pages)")


def read_url_list(path: Path) -> list[str]:
    urls: list[str] = []
    for raw in path.read_text(encoding="utf-8").splitlines():
        line = raw.strip()
        if line and not line.startswith("#"):
            urls.append(line)
    return urls


def collect_items(args: argparse.Namespace) -> tuple[list[WorkItem], list[WorkItem]]:
    local_items: list[WorkItem] = []
    url_items: list[WorkItem] = []
    raw_inputs = list(args.inputs or [])
    if args.url_list:
        raw_inputs.extend(read_url_list(Path(args.url_list).expanduser()))
    for raw in raw_inputs:
        if is_url(raw):
            name = Path(urlparse(raw).path).name or f"url-{len(url_items) + 1}.pdf"
            url_items.append(
                WorkItem(source=raw, name=name, data_id=data_id_for(raw), url=raw, page_ranges=args.page_ranges)
            )
            continue
        for local_path in iter_local_files(Path(raw).expanduser().resolve()):
            validate_local_file(local_path, allow_over_limit=args.allow_over_limit)
            local_items.append(
                WorkItem(
                    source=str(local_path),
                    name=local_path.name,
                    data_id=data_id_for(str(local_path)),
                    local_path=local_path,
                    page_ranges=args.page_ranges,
                )
            )
    return local_items, url_items


def chunks(items: list[WorkItem], size: int) -> list[list[WorkItem]]:
    if size <= 0:
        raise ValueError("batch size must be positive")
    return [items[index : index + size] for index in range(0, len(items), size)]


def auth_headers(token: str) -> dict[str, str]:
    return {
        "Authorization": f"Bearer {token}",
        "Content-Type": "application/json",
        "Accept": "*/*",
    }


def checked_json(response: requests.Response, phase: str) -> dict:
    if response.status_code != 200:
        raise MinerUError(f"{phase} failed: HTTP {response.status_code}: {response.text[:1000]}")
    payload = response.json()
    if payload.get("code") != 0:
        raise MinerUError(f"{phase} failed: {payload.get('code')} {payload.get('msg')}")
    return payload


def common_payload(args: argparse.Namespace) -> dict:
    payload: dict = {
        "model_version": args.model_version,
        "language": args.language,
        "enable_formula": args.enable_formula,
        "enable_table": args.enable_table,
    }
    if args.extra_format:
        payload["extra_formats"] = args.extra_format
    if args.no_cache:
        payload["no_cache"] = True
    if args.cache_tolerance is not None:
        payload["cache_tolerance"] = args.cache_tolerance
    return payload


def file_entry(item: WorkItem, args: argparse.Namespace, *, include_url: bool) -> dict:
    entry = {
        "data_id": item.data_id,
        "is_ocr": args.is_ocr,
    }
    if include_url:
        entry["url"] = item.url
    else:
        entry["name"] = item.name
    if item.page_ranges:
        entry["page_ranges"] = item.page_ranges
    return entry


def submit_local_batch(session: requests.Session, args: argparse.Namespace, token: str, batch: list[WorkItem]) -> str:
    payload = common_payload(args)
    payload["files"] = [file_entry(item, args, include_url=False) for item in batch]
    response = session.post(FILE_URLS_BATCH_URL, headers=auth_headers(token), json=payload, timeout=args.timeout_seconds)
    data = checked_json(response, "local batch submit")["data"]
    batch_id = data["batch_id"]
    upload_urls = data["file_urls"]
    if len(upload_urls) != len(batch):
        raise MinerUError(f"upload URL count mismatch: {len(upload_urls)} != {len(batch)}")
    for item, upload_url in zip(batch, upload_urls):
        assert item.local_path is not None
        with item.local_path.open("rb") as fh:
            upload_response = requests.put(upload_url, data=fh, timeout=args.upload_timeout_seconds)
        if upload_response.status_code != 200:
            raise MinerUError(f"upload failed for {item.name}: HTTP {upload_response.status_code}")
        print(f"uploaded={item.name}")
    return batch_id


def submit_url_batch(session: requests.Session, args: argparse.Namespace, token: str, batch: list[WorkItem]) -> str:
    payload = common_payload(args)
    payload["files"] = [file_entry(item, args, include_url=True) for item in batch]
    response = session.post(URL_TASK_BATCH_URL, headers=auth_headers(token), json=payload, timeout=args.timeout_seconds)
    data = checked_json(response, "URL batch submit")["data"]
    return data["batch_id"]


def single_url_payload(item: WorkItem, args: argparse.Namespace) -> dict:
    payload = common_payload(args)
    payload["url"] = item.url
    payload["is_ocr"] = args.is_ocr
    payload["data_id"] = item.data_id
    if item.page_ranges:
        payload["page_ranges"] = item.page_ranges
    return payload


def submit_single_url_task(session: requests.Session, args: argparse.Namespace, token: str, item: WorkItem) -> str:
    response = session.post(
        SINGLE_TASK_URL,
        headers=auth_headers(token),
        json=single_url_payload(item, args),
        timeout=args.timeout_seconds,
    )
    data = checked_json(response, "single URL submit")["data"]
    return data["task_id"]


def poll_single_task(session: requests.Session, args: argparse.Namespace, token: str, task_id: str) -> dict:
    deadline = time.time() + args.max_runtime_seconds
    last_state = ""
    while time.time() < deadline:
        url = SINGLE_TASK_RESULT_URL.format(task_id=task_id)
        response = session.get(url, headers=auth_headers(token), timeout=args.timeout_seconds)
        payload = checked_json(response, f"poll {task_id}")
        result = payload.get("data", {})
        state = result.get("state", "unknown")
        progress = result.get("extract_progress") or {}
        try:
            extracted_pages = int(progress.get("extracted_pages"))
        except (TypeError, ValueError):
            extracted_pages = 0
        try:
            total_pages = int(progress.get("total_pages"))
        except (TypeError, ValueError):
            total_pages = None
        OPERATION_PROGRESS.update(
            completed=extracted_pages,
            total=total_pages,
            phase="extracting",
        )
        state_line = "task={} state={} extracted={}/{}".format(
            task_id,
            state,
            progress.get("extracted_pages", "?"),
            progress.get("total_pages", "?"),
        )
        if state_line != last_state:
            print(state_line)
            last_state = state_line
        if state in TERMINAL_STATES:
            return result
        time.sleep(args.poll_seconds)
    raise MinerUError(f"Timed out after {args.max_runtime_seconds}s waiting for task {task_id}")


def poll_batch(session: requests.Session, args: argparse.Namespace, token: str, batch_id: str) -> list[dict]:
    deadline = time.time() + args.max_runtime_seconds
    last_summary = ""
    while time.time() < deadline:
        url = RESULTS_BATCH_URL.format(batch_id=batch_id)
        response = session.get(url, headers=auth_headers(token), timeout=args.timeout_seconds)
        payload = checked_json(response, f"poll {batch_id}")
        results = payload.get("data", {}).get("extract_result", [])
        progress_rows = [result.get("extract_progress") or {} for result in results]
        try:
            extracted_pages = sum(int(row.get("extracted_pages")) for row in progress_rows)
            total_pages = sum(int(row.get("total_pages")) for row in progress_rows)
        except (TypeError, ValueError):
            extracted_pages = 0
            total_pages = None
        OPERATION_PROGRESS.update(
            completed=extracted_pages,
            total=total_pages,
            phase="extracting",
        )
        counts: dict[str, int] = {}
        for result in results:
            state = result.get("state", "unknown")
            counts[state] = counts.get(state, 0) + 1
        summary = ", ".join(f"{key}={value}" for key, value in sorted(counts.items())) or "no-results"
        if summary != last_summary:
            print(f"batch={batch_id} {summary}")
            last_summary = summary
        if results and all(result.get("state") in TERMINAL_STATES for result in results):
            return results
        time.sleep(args.poll_seconds)
    raise MinerUError(f"Timed out after {args.max_runtime_seconds}s waiting for batch {batch_id}")


def download_file(url: str, path: Path, timeout_seconds: int) -> None:
    response = requests.get(url, timeout=timeout_seconds)
    response.raise_for_status()
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_bytes(response.content)


def unpack_result_zip(zip_path: Path, extract_dir: Path) -> None:
    extract_dir.mkdir(parents=True, exist_ok=True)
    with zipfile.ZipFile(zip_path) as archive:
        archive.extractall(extract_dir)


def download_results(args: argparse.Namespace, batch_id: str, results: list[dict]) -> None:
    OPERATION_PROGRESS.touch("downloading")
    batch_dir = Path(args.output_dir).resolve() / batch_id
    batch_dir.mkdir(parents=True, exist_ok=True)
    (batch_dir / "batch_results.json").write_text(
        json.dumps(results, ensure_ascii=False, indent=2) + "\n",
        encoding="utf-8",
    )
    if args.no_download:
        return
    for result in results:
        state = result.get("state")
        name = result.get("data_id") or result.get("file_name") or "document"
        doc_dir = batch_dir / safe_slug(str(name))
        doc_dir.mkdir(parents=True, exist_ok=True)
        (doc_dir / "result.json").write_text(
            json.dumps(result, ensure_ascii=False, indent=2) + "\n",
            encoding="utf-8",
        )
        if state != "done":
            print(f"skip-download={name} state={state} err={result.get('err_msg', '')}")
            continue
        zip_url = result.get("full_zip_url")
        if not zip_url:
            print(f"skip-download={name} missing-full_zip_url")
            continue
        zip_path = doc_dir / "result.zip"
        download_file(zip_url, zip_path, args.timeout_seconds)
        unpack_result_zip(zip_path, doc_dir / "extracted")
        full_md = next((doc_dir / "extracted").rglob("full.md"), None)
        print(f"downloaded={name} zip={zip_path}")
        if full_md:
            print(f"markdown={full_md}")


def download_single_result(args: argparse.Namespace, task_id: str, result: dict) -> None:
    OPERATION_PROGRESS.touch("downloading")
    task_dir = Path(args.output_dir).resolve() / task_id
    task_dir.mkdir(parents=True, exist_ok=True)
    (task_dir / "task_result.json").write_text(
        json.dumps(result, ensure_ascii=False, indent=2) + "\n",
        encoding="utf-8",
    )
    if args.no_download:
        return
    state = result.get("state")
    if state != "done":
        print(f"skip-download={task_id} state={state} err={result.get('err_msg', '')}")
        return
    zip_url = result.get("full_zip_url")
    if not zip_url:
        print(f"skip-download={task_id} missing-full_zip_url")
        return
    zip_path = task_dir / "result.zip"
    download_file(zip_url, zip_path, args.timeout_seconds)
    unpack_result_zip(zip_path, task_dir / "extracted")
    full_md = next((task_dir / "extracted").rglob("full.md"), None)
    print(f"downloaded={task_id} zip={zip_path}")
    if full_md:
        print(f"markdown={full_md}")


def effective_url_mode(args: argparse.Namespace, url_items: list[WorkItem]) -> str:
    requested = args.url_mode or args.mode
    if requested == "auto":
        return "single" if len(url_items) == 1 else "batch"
    return requested


def process_batches(args: argparse.Namespace, token: str, local_items: list[WorkItem], url_items: list[WorkItem]) -> None:
    session = requests.Session()
    batch_index = 0
    max_batch_size = min(args.batch_size, 50)
    for batch in chunks(local_items, max_batch_size):
        batch_index += 1
        print(f"submit-local-batch={batch_index} files={len(batch)}")
        batch_id = submit_local_batch(session, args, token, batch)
        print(f"batch_id={batch_id}")
        if args.no_wait:
            continue
        results = poll_batch(session, args, token, batch_id)
        download_results(args, batch_id, results)
    url_mode = effective_url_mode(args, url_items)
    if url_items:
        print(f"url_mode={url_mode}")
    if url_mode == "single":
        for item in url_items:
            print(f"submit-single-url={item.name}")
            task_id = submit_single_url_task(session, args, token, item)
            print(f"task_id={task_id}")
            if args.no_wait:
                continue
            result = poll_single_task(session, args, token, task_id)
            download_single_result(args, task_id, result)
        return
    for batch in chunks(url_items, max_batch_size):
        batch_index += 1
        print(f"submit-url-batch={batch_index} files={len(batch)}")
        batch_id = submit_url_batch(session, args, token, batch)
        print(f"batch_id={batch_id}")
        if args.no_wait:
            continue
        results = poll_batch(session, args, token, batch_id)
        download_results(args, batch_id, results)


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(description="Batch convert files with MinerU Precision Extract API.")
    parser.add_argument("inputs", nargs="*", help="Files, directories, or HTTP(S) URLs.")
    parser.add_argument("--url-list", help="Text file with one URL per line.")
    parser.add_argument(
        "--mode",
        choices=["auto", "single", "batch"],
        default="auto",
        help="Auto chooses local files=batch, one URL=single, multiple URLs=batch. Use single/batch to force URL handling.",
    )
    parser.add_argument(
        "--url-mode",
        choices=["auto", "batch", "single"],
        help="Deprecated alias for --mode, kept for old commands.",
    )
    parser.add_argument("-o", "--output-dir", default="output/mineru")
    parser.add_argument(
        "--model-version",
        choices=["pipeline", "vlm", "MinerU-HTML"],
        default=os.environ.get("MINERU_MODEL_VERSION", "vlm"),
    )
    parser.add_argument("--language", default=os.environ.get("MINERU_LANGUAGE", "ch"))
    parser.add_argument("--ocr", dest="is_ocr", action="store_true", default=True)
    parser.add_argument("--no-ocr", dest="is_ocr", action="store_false")
    parser.add_argument("--enable-table", action=argparse.BooleanOptionalAction, default=True)
    parser.add_argument("--enable-formula", action=argparse.BooleanOptionalAction, default=True)
    parser.add_argument("--extra-format", action="append", choices=["docx", "html", "latex"])
    parser.add_argument("--page-ranges", help='Page range string such as "1-200" or "2,4-6".')
    parser.add_argument("--no-cache", action="store_true")
    parser.add_argument("--cache-tolerance", type=int)
    parser.add_argument("--batch-size", type=int, default=50)
    parser.add_argument("--allow-over-limit", action="store_true", help="Submit even if local preflight sees >200MB or >200 PDF pages.")
    parser.add_argument("--poll-seconds", type=int, default=10)
    parser.add_argument("--timeout-seconds", type=int, default=120)
    parser.add_argument("--upload-timeout-seconds", type=int, default=600)
    parser.add_argument("--max-runtime-seconds", type=int, default=7200)
    parser.add_argument("--no-wait", action="store_true", help="Submit/upload only; print batch IDs without polling.")
    parser.add_argument("--no-download", action="store_true", help="Poll results but do not download result zip files.")
    parser.add_argument("--self-test", action="store_true", help="Check config/imports without submitting work.")
    return parser


def main(argv: list[str] | None = None) -> int:
    load_root_dotenv()
    args = build_parser().parse_args(argv)
    token = os.environ.get("MINERU_API_TOKEN", "").strip() or os.environ.get("MINERU_TOKEN", "").strip()
    if args.self_test:
        print(f"token={'present' if token else 'missing'}")
        print(f"mode={args.url_mode or args.mode}")
        print(f"model_version={args.model_version}")
        print(f"language={args.language}")
        print(f"file_urls_batch_url={FILE_URLS_BATCH_URL}")
        print(f"single_task_url={SINGLE_TASK_URL}")
        print(f"url_task_batch_url={URL_TASK_BATCH_URL}")
        print("requests=present")
        return 0 if token else 1
    if not token:
        raise MinerUError("MINERU_API_TOKEN is not configured")
    local_items, url_items = collect_items(args)
    if not local_items and not url_items:
        raise MinerUError("No supported local files or URLs found")
    OPERATION_PROGRESS.start("uploading")
    print(f"local_files={len(local_items)} urls={len(url_items)}")
    process_batches(args, token, local_items, url_items)
    return 0


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except MinerUError as exc:
        print(f"error: {exc}", file=sys.stderr)
        raise SystemExit(1)
