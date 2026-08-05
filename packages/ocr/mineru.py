#!/usr/bin/env python3
"""Batch CLI for MinerU Precision Extract API.

Designed for scanned/image-heavy books and academic papers. It uses only the v4
Precision Extract API, automatically selects VLM or MinerU-HTML, physically
splits oversized PDFs, supports signed local batch uploads and URL tasks, and
reassembles downloaded part archives into one page-ordered full.md per source.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import re
import sys
import tempfile
import time
import zipfile
from dataclasses import dataclass
from pathlib import Path
from urllib.parse import urlparse

import requests

_PROGRESS_SCRIPTS = Path(__file__).resolve().parent / "scripts"
_PACKAGE_ROOT = Path(__file__).resolve().parent
if str(_PACKAGE_ROOT) not in sys.path:
    sys.path.insert(0, str(_PACKAGE_ROOT))
if str(_PROGRESS_SCRIPTS) not in sys.path:
    sys.path.insert(0, str(_PROGRESS_SCRIPTS))
from progress import OperationProgress
from publication_evidence import (
    normalize_extracted_markdown_notes,
    persist_source_document,
    write_markdown_evidence,
)


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
}
MAX_FILE_BYTES = 200 * 1024 * 1024
MAX_PAGES = 200
TERMINAL_STATES = {"done", "failed"}
OPERATION_PROGRESS = OperationProgress.from_environment("extract", "pages")
HTML_SUFFIXES = {".html"}


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
    source_data_id: str | None = None
    source_name: str | None = None
    source_pages: int | None = None
    part_index: int = 1
    part_count: int = 1
    selected_pages: tuple[int, ...] | None = None


@dataclass(frozen=True)
class DownloadedPart:
    item: WorkItem
    markdown_path: Path


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
    if os.environ.get("BIBLIOSMITH_DISABLE_DOTENV") == "1":
        return
    for candidate in (start.resolve(), *start.resolve().parents):
        if (candidate / "pyproject.toml").is_file() and (candidate / "packages").is_dir():
            load_dotenv(candidate / ".env")
            return


def is_url(value: str) -> bool:
    parsed = urlparse(value)
    return parsed.scheme in {"http", "https"}


def item_suffix(item: WorkItem) -> str:
    return Path(urlparse(item.url or item.name).path).suffix.lower()


def model_version_for(item: WorkItem, args: argparse.Namespace) -> str:
    is_html = item_suffix(item) in HTML_SUFFIXES
    requested = args.model_version
    if requested == "auto":
        return "MinerU-HTML" if is_html else "vlm"
    if is_html and requested != "MinerU-HTML":
        raise MinerUError("HTML sources require model_version=MinerU-HTML")
    if not is_html and requested == "MinerU-HTML":
        raise MinerUError("MinerU-HTML can only parse HTML sources")
    return requested


def data_id_for(value: str) -> str:
    digest = hashlib.sha256(value.encode("utf-8")).hexdigest()[:24]
    return f"doc-{digest}"


def iter_local_files(path: Path) -> list[Path]:
    if path.is_file():
        return [path] if path.suffix.lower() in SUPPORTED_SUFFIXES else []
    if path.is_dir():
        ignored = {".git", ".venv", ".state", "tmp", "__pycache__"}
        files: list[Path] = []
        for candidate in path.rglob("*"):
            if not candidate.is_file():
                continue
            # Matched against the parts below the directory being scanned, not
            # the absolute path: the caller named this root explicitly, so a
            # component of it (a book folder under ~/tmp, or /tmp itself) must
            # not filter out everything inside it.
            if ignored.intersection(candidate.relative_to(path).parts):
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


def validate_local_file(path: Path) -> int | None:
    suffix = path.suffix.lower()
    if suffix not in SUPPORTED_SUFFIXES:
        raise MinerUError(f"Unsupported file type: {path}")
    size = path.stat().st_size
    pages = pdf_page_count(path)
    if suffix != ".pdf" and size > MAX_FILE_BYTES:
        raise MinerUError(
            f"File exceeds MinerU Precision API 200MB limit: {path} ({size / 1024 / 1024:.1f}MB)"
        )
    if suffix == ".pdf" and pages is None:
        raise MinerUError(f"Cannot read PDF page count required for MinerU preflight: {path}")
    if pages is not None and pages <= 0:
        raise MinerUError(f"PDF contains no pages: {path}")
    return pages


def page_number(value: int, total_pages: int) -> int:
    if value == 0:
        raise MinerUError("Page ranges cannot contain page 0")
    resolved = value if value > 0 else total_pages + value + 1
    if resolved < 1 or resolved > total_pages:
        raise MinerUError(f"Page {value} is outside this {total_pages}-page PDF")
    return resolved


def parse_page_ranges(spec: str, total_pages: int) -> list[int]:
    selected: list[int] = []
    seen: set[int] = set()
    for raw_part in spec.split(","):
        part = raw_part.strip()
        match = re.fullmatch(r"(-?\d+)(?:-(-?\d+))?", part)
        if not match:
            raise MinerUError(f"Invalid page range: {part!r}")
        start = page_number(int(match.group(1)), total_pages)
        end = page_number(int(match.group(2)), total_pages) if match.group(2) else start
        if end < start:
            raise MinerUError(f"Descending page range is not supported: {part!r}")
        for page in range(start, end + 1):
            if page not in seen:
                seen.add(page)
                selected.append(page)
    if not selected:
        raise MinerUError("Page range selected no pages")
    return selected


def write_pdf_selection(source: Path, destination: Path, pages: list[int]) -> None:
    try:
        from pypdf import PdfReader, PdfWriter  # type: ignore

        reader = PdfReader(str(source))
        writer = PdfWriter()
        for page in pages:
            writer.add_page(reader.pages[page - 1])
        with destination.open("wb") as handle:
            writer.write(handle)
    except Exception as exc:
        raise MinerUError(
            f"Failed to split PDF {source} for original pages {pages[0]}-{pages[-1]}: {exc}"
        ) from exc


def split_pdf_groups(
    source: Path,
    selected_pages: list[int],
    temporary_dir: Path,
) -> list[tuple[Path, tuple[int, ...]]]:
    pending = [
        selected_pages[index : index + MAX_PAGES]
        for index in range(0, len(selected_pages), MAX_PAGES)
    ]
    completed: list[tuple[Path, tuple[int, ...]]] = []
    candidate_index = 0
    while pending:
        pages = pending.pop(0)
        candidate_index += 1
        candidate = temporary_dir / f"candidate-{candidate_index:04d}.pdf"
        write_pdf_selection(source, candidate, pages)
        if candidate.stat().st_size <= MAX_FILE_BYTES:
            completed.append((candidate, tuple(pages)))
            continue
        candidate.unlink(missing_ok=True)
        if len(pages) == 1:
            raise MinerUError(
                f"Original PDF page {pages[0]} exceeds MinerU Precision API 200MB limit after splitting: {source}"
            )
        midpoint = len(pages) // 2
        pending[0:0] = [pages[:midpoint], pages[midpoint:]]
    return completed


def prepare_local_items(
    args: argparse.Namespace,
    local_items: list[WorkItem],
    temporary_dir: Path,
) -> list[WorkItem]:
    prepared: list[WorkItem] = []
    for item in local_items:
        assert item.local_path is not None
        needs_split = (
            item.local_path.suffix.lower() == ".pdf"
            and item.source_pages is not None
            and (item.source_pages > MAX_PAGES or item.local_path.stat().st_size > MAX_FILE_BYTES)
        )
        if not needs_split:
            prepared.append(item)
            continue
        selected_pages = (
            parse_page_ranges(item.page_ranges, item.source_pages)
            if item.page_ranges
            else list(range(1, item.source_pages + 1))
        )
        source_dir = temporary_dir / (item.source_data_id or item.data_id)
        source_dir.mkdir(parents=True, exist_ok=True)
        parts = split_pdf_groups(item.local_path, selected_pages, source_dir)
        part_count = len(parts)
        for part_index, (candidate, pages) in enumerate(parts, start=1):
            part_name = f"{item.local_path.stem}.part-{part_index:04d}-of-{part_count:04d}.pdf"
            part_path = candidate.with_name(part_name)
            candidate.replace(part_path)
            prepared.append(
                WorkItem(
                    source=item.source,
                    name=part_name,
                    data_id=f"{item.source_data_id or item.data_id}-part-{part_index:04d}",
                    local_path=part_path,
                    page_ranges=None,
                    source_data_id=item.source_data_id or item.data_id,
                    source_name=item.source_name or item.name,
                    source_pages=item.source_pages,
                    part_index=part_index,
                    part_count=part_count,
                    selected_pages=pages,
                )
            )
    return prepared


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
                WorkItem(
                    source=raw,
                    name=name,
                    data_id=data_id_for(raw),
                    url=raw,
                    page_ranges=args.page_ranges,
                )
            )
            continue
        for local_path in iter_local_files(Path(raw).expanduser().resolve()):
            pages = validate_local_file(local_path)
            data_id = data_id_for(str(local_path))
            local_items.append(
                WorkItem(
                    source=str(local_path),
                    name=local_path.name,
                    data_id=data_id,
                    local_path=local_path,
                    page_ranges=args.page_ranges,
                    source_data_id=data_id,
                    source_name=local_path.name,
                    source_pages=pages,
                )
            )
    return local_items, url_items


def chunks(items: list[WorkItem], size: int) -> list[list[WorkItem]]:
    if size <= 0:
        raise ValueError("batch size must be positive")
    return [items[index : index + size] for index in range(0, len(items), size)]


def model_batches(
    items: list[WorkItem], args: argparse.Namespace, max_batch_size: int
) -> list[list[WorkItem]]:
    grouped: dict[str, list[WorkItem]] = {}
    for item in items:
        grouped.setdefault(model_version_for(item, args), []).append(item)
    return [batch for group in grouped.values() for batch in chunks(group, max_batch_size)]


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


def common_payload(
    args: argparse.Namespace,
    model_version: str,
    *,
    include_cache: bool,
) -> dict:
    payload: dict = {"model_version": model_version}
    if model_version != "MinerU-HTML":
        payload.update(
            {
                "language": args.language,
                "enable_formula": args.enable_formula,
                "enable_table": args.enable_table,
            }
        )
    if args.extra_format and model_version != "MinerU-HTML":
        payload["extra_formats"] = args.extra_format
    if include_cache and args.no_cache:
        payload["no_cache"] = True
    if include_cache and args.cache_tolerance is not None:
        payload["cache_tolerance"] = args.cache_tolerance
    return payload


def file_entry(
    item: WorkItem,
    args: argparse.Namespace,
    *,
    include_url: bool,
    model_version: str,
) -> dict:
    entry = {"data_id": item.data_id}
    if model_version != "MinerU-HTML":
        entry["is_ocr"] = args.is_ocr
    if include_url:
        entry["url"] = item.url
    else:
        entry["name"] = item.name
    if item.page_ranges and model_version != "MinerU-HTML":
        entry["page_ranges"] = item.page_ranges
    return entry


def submit_local_batch(
    session: requests.Session,
    args: argparse.Namespace,
    token: str,
    batch: list[WorkItem],
) -> str:
    model_version = model_version_for(batch[0], args)
    payload = common_payload(args, model_version, include_cache=False)
    payload["files"] = [
        file_entry(item, args, include_url=False, model_version=model_version) for item in batch
    ]
    response = session.post(
        FILE_URLS_BATCH_URL,
        headers=auth_headers(token),
        json=payload,
        timeout=args.timeout_seconds,
    )
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


def submit_url_batch(
    session: requests.Session,
    args: argparse.Namespace,
    token: str,
    batch: list[WorkItem],
) -> str:
    model_version = model_version_for(batch[0], args)
    payload = common_payload(args, model_version, include_cache=True)
    payload["files"] = [
        file_entry(item, args, include_url=True, model_version=model_version) for item in batch
    ]
    response = session.post(
        URL_TASK_BATCH_URL,
        headers=auth_headers(token),
        json=payload,
        timeout=args.timeout_seconds,
    )
    data = checked_json(response, "URL batch submit")["data"]
    return data["batch_id"]


def single_url_payload(item: WorkItem, args: argparse.Namespace) -> dict:
    model_version = model_version_for(item, args)
    payload = common_payload(args, model_version, include_cache=True)
    payload["url"] = item.url
    if model_version != "MinerU-HTML":
        payload["is_ocr"] = args.is_ocr
    payload["data_id"] = item.data_id
    if item.page_ranges and model_version != "MinerU-HTML":
        payload["page_ranges"] = item.page_ranges
    return payload


def submit_single_url_task(
    session: requests.Session,
    args: argparse.Namespace,
    token: str,
    item: WorkItem,
) -> str:
    response = session.post(
        SINGLE_TASK_URL,
        headers=auth_headers(token),
        json=single_url_payload(item, args),
        timeout=args.timeout_seconds,
    )
    data = checked_json(response, "single URL submit")["data"]
    return data["task_id"]


def poll_single_task(
    session: requests.Session,
    args: argparse.Namespace,
    token: str,
    task_id: str,
) -> dict:
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


def matched_batch_results(results: list[dict], batch: list[WorkItem]) -> list[dict] | None:
    by_id = {str(result.get("data_id")): result for result in results if result.get("data_id")}
    by_name: dict[str, list[dict]] = {}
    for result in results:
        if result.get("file_name"):
            by_name.setdefault(str(result["file_name"]), []).append(result)
    matched: list[dict] = []
    for item in batch:
        result = by_id.get(item.data_id)
        if result is None and len(by_name.get(item.name, [])) == 1:
            result = by_name[item.name][0]
        if result is None:
            return None
        matched.append(result)
    return matched


def poll_batch(
    session: requests.Session,
    args: argparse.Namespace,
    token: str,
    batch_id: str,
    batch: list[WorkItem],
) -> list[dict]:
    deadline = time.time() + args.max_runtime_seconds
    last_summary = ""
    while time.time() < deadline:
        url = RESULTS_BATCH_URL.format(batch_id=batch_id)
        response = session.get(url, headers=auth_headers(token), timeout=args.timeout_seconds)
        payload = checked_json(response, f"poll {batch_id}")
        results = payload.get("data", {}).get("extract_result", [])
        OPERATION_PROGRESS.touch("extracting")
        items_by_id = {item.data_id: item for item in batch}
        items_by_name: dict[str, list[WorkItem]] = {}
        for item in batch:
            items_by_name.setdefault(item.name, []).append(item)
        for result in results:
            item = items_by_id.get(str(result.get("data_id")))
            if item is None and len(items_by_name.get(str(result.get("file_name")), [])) == 1:
                item = items_by_name[str(result["file_name"])][0]
            if item is None:
                continue
            progress = result.get("extract_progress") or {}
            try:
                extracted_pages = max(0, int(progress.get("extracted_pages")))
            except (TypeError, ValueError):
                extracted_pages = 0
            try:
                total_pages = max(0, int(progress.get("total_pages"))) or None
            except (TypeError, ValueError):
                total_pages = None
            known_pages = page_count_for_item(item)
            if result.get("state") == "done" and known_pages is not None:
                extracted_pages = max(extracted_pages, known_pages)
                total_pages = max(total_pages or 0, known_pages)
            OPERATION_PROGRESS.update_item(
                item.data_id,
                extracted_pages,
                "extracting",
                total=total_pages,
            )
        counts: dict[str, int] = {}
        for result in results:
            state = result.get("state", "unknown")
            counts[state] = counts.get(state, 0) + 1
        summary = (
            ", ".join(f"{key}={value}" for key, value in sorted(counts.items()))
            or "no-results"
        )
        if summary != last_summary:
            print(f"batch={batch_id} {summary}")
            last_summary = summary
        matched = matched_batch_results(results, batch)
        if matched is not None and all(
            result.get("state") in TERMINAL_STATES for result in matched
        ):
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


def download_results(
    args: argparse.Namespace,
    batch_id: str,
    results: list[dict],
    batch: list[WorkItem],
) -> list[DownloadedPart]:
    OPERATION_PROGRESS.touch("downloading")
    output_root = Path(args.output_dir).resolve()
    batch_dir = output_root / ".mineru_batches" / batch_id
    batch_dir.mkdir(parents=True, exist_ok=True)
    (batch_dir / "batch_results.json").write_text(
        json.dumps(results, ensure_ascii=False, indent=2) + "\n",
        encoding="utf-8",
    )
    matched = matched_batch_results(results, batch)
    if matched is None:
        raise MinerUError(f"MinerU batch {batch_id} returned an incomplete result set")
    failed = [
        f"{result.get('file_name') or result.get('data_id')}: {result.get('err_msg', '')}"
        for result in matched
        if result.get("state") == "failed"
    ]
    if failed:
        raise MinerUError("MinerU batch failed: " + "; ".join(failed))
    if args.no_download:
        return []
    results_by_id = {
        str(result.get("data_id")): result for result in results if result.get("data_id")
    }
    results_by_name: dict[str, list[dict]] = {}
    for result in results:
        if result.get("file_name"):
            results_by_name.setdefault(str(result["file_name"]), []).append(result)
    downloaded: list[DownloadedPart] = []
    errors: list[str] = []
    for item in batch:
        result = results_by_id.get(item.data_id)
        if result is None and len(results_by_name.get(item.name, [])) == 1:
            result = results_by_name[item.name][0]
        if result is None:
            errors.append(f"missing result for {item.name} ({item.data_id})")
            continue
        state = result.get("state")
        source_data_id = item.source_data_id or item.data_id
        part_dir = output_root / source_data_id / "parts" / f"{item.part_index:04d}"
        part_dir.mkdir(parents=True, exist_ok=True)
        (part_dir / "result.json").write_text(
            json.dumps(result, ensure_ascii=False, indent=2) + "\n",
            encoding="utf-8",
        )
        if state != "done":
            errors.append(f"{item.name}: state={state} err={result.get('err_msg', '')}")
            continue
        zip_url = result.get("full_zip_url")
        if not zip_url:
            errors.append(f"{item.name}: missing full_zip_url")
            continue
        zip_path = part_dir / "result.zip"
        download_file(zip_url, zip_path, args.timeout_seconds)
        extract_dir = part_dir / "extracted"
        unpack_result_zip(zip_path, extract_dir)
        full_markdown = list(extract_dir.rglob("full.md"))
        if len(full_markdown) != 1:
            errors.append(f"{item.name}: result archive contains {len(full_markdown)} full.md files")
            continue
        part_markdown = full_markdown[0].with_name("part.md")
        full_markdown[0].replace(part_markdown)
        downloaded.append(DownloadedPart(item=item, markdown_path=part_markdown))
        print(f"downloaded={item.name} zip={zip_path}")
        print(f"markdown-part={part_markdown}")
    if errors:
        raise MinerUError("MinerU batch did not complete cleanly: " + "; ".join(errors))
    return downloaded


def is_relative_reference(value: str) -> bool:
    reference = value.strip().strip("<>")
    parsed = urlparse(reference)
    return bool(reference) and not reference.startswith(("#", "/")) and not parsed.scheme


def rewrite_relative_references(markdown: str, prefix: str) -> str:
    def markdown_replacement(match: re.Match[str]) -> str:
        opening, raw_reference, closing = match.groups()
        if not is_relative_reference(raw_reference):
            return match.group(0)
        wrapped = raw_reference.startswith("<") and raw_reference.endswith(">")
        reference = raw_reference.strip("<>")
        rewritten = f"{prefix}/{reference}"
        if wrapped:
            rewritten = f"<{rewritten}>"
        return f"{opening}{rewritten}{closing}"

    rewritten = re.sub(r"(!?\[[^\]]*\]\()([^\s)]+)([^)]*\))", markdown_replacement, markdown)

    def html_replacement(match: re.Match[str]) -> str:
        attribute, quote, reference = match.groups()
        if not is_relative_reference(reference):
            return match.group(0)
        return f"{attribute}={quote}{prefix}/{reference}{quote}"

    return re.sub(r"\b(src|href)=(['\"])([^'\"]+)\2", html_replacement, rewritten)


def page_count_for_item(item: WorkItem) -> int | None:
    if item.selected_pages is not None:
        return len(item.selected_pages)
    if item.page_ranges and item.source_pages is not None:
        return len(parse_page_ranges(item.page_ranges, item.source_pages))
    return item.source_pages


def aggregate_known_page_total(items: list[WorkItem]) -> int | None:
    counts = [page_count_for_item(item) for item in items]
    if not counts or any(count is None for count in counts):
        return None
    return sum(count for count in counts if count is not None)


def merge_downloaded_parts(
    args: argparse.Namespace,
    items: list[WorkItem],
    downloaded: list[DownloadedPart],
) -> None:
    if args.no_download or args.no_wait:
        return
    items_by_source: dict[str, list[WorkItem]] = {}
    downloaded_by_source: dict[str, list[DownloadedPart]] = {}
    for item in items:
        items_by_source.setdefault(item.source_data_id or item.data_id, []).append(item)
    for part in downloaded:
        downloaded_by_source.setdefault(
            part.item.source_data_id or part.item.data_id, []
        ).append(part)
    output_root = Path(args.output_dir).resolve()
    for source_data_id, source_items in items_by_source.items():
        parts = sorted(
            downloaded_by_source.get(source_data_id, []),
            key=lambda part: part.item.part_index,
        )
        if len(parts) != len(source_items):
            raise MinerUError(
                f"Cannot assemble {source_items[0].source_name or source_items[0].name}: "
                f"downloaded {len(parts)} of {len(source_items)} parts"
            )
        source_dir = output_root / source_data_id
        merged_sections: list[str] = []
        manifest_parts: list[dict] = []
        source_document_specs: list[tuple[Path, str, int, int, tuple[int, ...]]] = []
        next_line = 1
        for part in parts:
            relative_parent = part.markdown_path.parent.relative_to(source_dir).as_posix()
            content = part.markdown_path.read_text(encoding="utf-8").strip()
            rewritten_content = normalize_extracted_markdown_notes(
                rewrite_relative_references(content, relative_parent)
            )
            merged_sections.append(rewritten_content)
            line_count = max(1, len(rewritten_content.splitlines()))
            original_pages = tuple(part.item.selected_pages or ())
            source_document_specs.append(
                (
                    part.markdown_path,
                    part.markdown_path.relative_to(source_dir).as_posix(),
                    next_line,
                    next_line + line_count - 1,
                    original_pages,
                )
            )
            next_line += line_count + 1
            manifest_parts.append(
                {
                    "part_index": part.item.part_index,
                    "part_count": part.item.part_count,
                    "file_name": part.item.name,
                    "data_id": part.item.data_id,
                    "page_count": page_count_for_item(part.item),
                    "original_pages": list(part.item.selected_pages or ()),
                    "markdown": part.markdown_path.relative_to(source_dir).as_posix(),
                }
            )
        final_markdown = source_dir / "full.md"
        final_markdown.write_text(
            "\n\n".join(merged_sections).rstrip() + "\n", encoding="utf-8"
        )
        source_documents = [
            persist_source_document(
                final_markdown,
                source_path,
                relative_path,
                start_line=start_line,
                end_line=end_line,
                pages=pages,
                kind="mineru_part_markdown",
            )
            for source_path, relative_path, start_line, end_line, pages in source_document_specs
        ]
        write_markdown_evidence(
            final_markdown,
            source_format="mineru",
            extraction_engine=model_version_for(source_items[0], args),
            source_documents=source_documents,
            title=Path(source_items[0].source_name or source_items[0].name).stem,
            extraction_facts={
                "partCount": len(parts),
                "sourcePageCount": source_items[0].source_pages,
            },
        )
        manifest = {
            "source": source_items[0].source,
            "source_name": source_items[0].source_name or source_items[0].name,
            "source_pages": source_items[0].source_pages,
            "model_version": model_version_for(source_items[0], args),
            "parts": manifest_parts,
            "full_markdown": final_markdown.name,
        }
        (source_dir / "mineru_manifest.json").write_text(
            json.dumps(manifest, ensure_ascii=False, indent=2) + "\n",
            encoding="utf-8",
        )
        print(f"markdown={final_markdown}")


def download_single_result(args: argparse.Namespace, task_id: str, result: dict) -> None:
    OPERATION_PROGRESS.touch("downloading")
    task_dir = Path(args.output_dir).resolve() / task_id
    task_dir.mkdir(parents=True, exist_ok=True)
    (task_dir / "task_result.json").write_text(
        json.dumps(result, ensure_ascii=False, indent=2) + "\n",
        encoding="utf-8",
    )
    state = result.get("state")
    if state != "done":
        raise MinerUError(
            f"MinerU task {task_id} failed: state={state} err={result.get('err_msg', '')}"
        )
    if args.no_download:
        return
    zip_url = result.get("full_zip_url")
    if not zip_url:
        raise MinerUError(f"MinerU task {task_id} completed without full_zip_url")
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


def process_batches(
    args: argparse.Namespace,
    token: str,
    local_items: list[WorkItem],
    url_items: list[WorkItem],
) -> None:
    session = requests.Session()
    batch_index = 0
    max_batch_size = min(args.batch_size, 50)
    with tempfile.TemporaryDirectory(prefix="bibliosmith-mineru-") as temporary_directory:
        prepared_local_items = prepare_local_items(args, local_items, Path(temporary_directory))
        downloaded_local_parts: list[DownloadedPart] = []
        for batch in model_batches(prepared_local_items, args, max_batch_size):
            batch_index += 1
            print(f"submit-local-batch={batch_index} files={len(batch)}")
            batch_id = submit_local_batch(session, args, token, batch)
            print(f"batch_id={batch_id}")
            if args.no_wait:
                continue
            results = poll_batch(session, args, token, batch_id, batch)
            downloaded_local_parts.extend(download_results(args, batch_id, results, batch))
        merge_downloaded_parts(args, prepared_local_items, downloaded_local_parts)
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
    for batch in model_batches(url_items, args, max_batch_size):
        batch_index += 1
        print(f"submit-url-batch={batch_index} files={len(batch)}")
        batch_id = submit_url_batch(session, args, token, batch)
        print(f"batch_id={batch_id}")
        if args.no_wait:
            continue
        results = poll_batch(session, args, token, batch_id, batch)
        downloaded_url_parts = download_results(args, batch_id, results, batch)
        merge_downloaded_parts(args, batch, downloaded_url_parts)


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(
        description="Batch convert files with MinerU Precision Extract API."
    )
    parser.add_argument("inputs", nargs="*", help="Files, directories, or HTTP(S) URLs.")
    parser.add_argument("--url-list", help="Text file with one URL per line.")
    parser.add_argument(
        "--mode",
        choices=["auto", "single", "batch"],
        default="auto",
        help=(
            "Auto chooses local files=batch, one URL=single, multiple URLs=batch. "
            "Use single/batch to force URL handling."
        ),
    )
    parser.add_argument(
        "--url-mode",
        choices=["auto", "batch", "single"],
        help="Deprecated alias for --mode, kept for old commands.",
    )
    parser.add_argument("-o", "--output-dir", default="output/mineru")
    parser.add_argument(
        "--model-version",
        choices=["auto", "pipeline", "vlm", "MinerU-HTML"],
        default=os.environ.get("MINERU_MODEL_VERSION", "auto"),
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
    parser.add_argument("--poll-seconds", type=int, default=10)
    parser.add_argument("--timeout-seconds", type=int, default=120)
    parser.add_argument("--upload-timeout-seconds", type=int, default=600)
    parser.add_argument("--max-runtime-seconds", type=int, default=7200)
    parser.add_argument(
        "--no-wait",
        action="store_true",
        help="Submit/upload only; print batch IDs without polling.",
    )
    parser.add_argument(
        "--no-download",
        action="store_true",
        help="Poll results but do not download result zip files.",
    )
    parser.add_argument("--self-test", action="store_true", help="Check config/imports without submitting work.")
    return parser


def main(argv: list[str] | None = None) -> int:
    load_root_dotenv()
    args = build_parser().parse_args(argv)
    token = os.environ.get("MINERU_API_TOKEN", "").strip() or os.environ.get(
        "MINERU_TOKEN", ""
    ).strip()
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
    known_total = aggregate_known_page_total([*local_items, *url_items])
    OPERATION_PROGRESS.start("uploading", total=known_total)
    print(f"local_files={len(local_items)} urls={len(url_items)}")
    process_batches(args, token, local_items, url_items)
    if args.no_wait:
        OPERATION_PROGRESS.touch("submitted")
    elif OPERATION_PROGRESS.total is not None:
        OPERATION_PROGRESS.update(
            completed=OPERATION_PROGRESS.total,
            total=OPERATION_PROGRESS.total,
            phase="completed",
        )
    else:
        OPERATION_PROGRESS.touch("completed")
    return 0


def cli(argv: list[str] | None = None) -> int:
    try:
        return main(argv)
    except MinerUError as exc:
        OPERATION_PROGRESS.touch("failed")
        print(f"error: {exc}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(cli())
