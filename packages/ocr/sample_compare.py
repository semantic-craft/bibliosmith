#!/usr/bin/env python3
"""Run PaddleOCR and MinerU over the same interior pages of one PDF.

The point is a decision, not a conversion: before spending a whole book on one
engine, extract a handful of representative pages, send those same pages to both
hosted APIs, and write a report the caller can put side by side.

Both engines stay remote, per docs/adr/0002-remote-paddleocr-only.md, so what
the sample shows is what the full run would produce. Cost is bounded by the
sampled page count rather than by book length: only the extracted pages are ever
uploaded.

Unlike the conversion workers, whose report goes to stdout, this one writes its
report to the path the manifest names and keeps it off stdout entirely. The
comparison has to survive the process that produced it -- the caller reads it
back to render the choice, possibly long after the run -- and it carries pages
of the user's book, which the caller's run log is not the place for.
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
from dataclasses import dataclass
from pathlib import Path
from typing import Any, Callable, Mapping

import requests

import mineru
import paddle


MANIFEST_SCHEMA = "ocr-sample-compare-v1"
REPORT_SCHEMA = "ocr-sample-compare-report-v1"
ENGINE_PADDLEOCR = "paddleocr"
ENGINE_MINERU = "mineru"
SUPPORTED_ENGINES = (ENGINE_PADDLEOCR, ENGINE_MINERU)
# Enough pages to see whether an engine handles this book's layout, few enough
# that a wrong guess costs a rounding error against converting the whole thing.
MAX_SAMPLE_PAGES = 10


class SampleCompareError(Exception):
    pass


@dataclass(frozen=True)
class EngineOutcome:
    """One engine's answer for the sampled pages."""

    markdown: str
    page_count: int | None = None


# (sample_pdf, work_dir) -> EngineOutcome. Injected by the tests so the offline
# suite exercises page selection, the report shape and the failure envelope
# without reaching either API.
EngineRunner = Callable[[Path, Path], EngineOutcome]


def select_internal_pages(total_pages: int, count: int) -> list[int]:
    """Uniformly spaced 1-based page numbers, excluding both endpoints.

    Mirrors translation_engine.sampling.select_internal_blocks, for the same
    reason it excludes the endpoints there: the first and last pages are cover,
    copyright and colophon, which say nothing about how an engine handles the
    body of the book. Duplicated rather than imported so the OCR package does
    not take a dependency on the translation engine.
    """
    if count < 1:
        raise SampleCompareError("invalid_sample_page_count")
    if total_pages < 1:
        raise SampleCompareError("empty_pdf")
    pages = list(range(1, total_pages + 1))
    internal_count = max(total_pages - 2, 0)
    if internal_count <= count:
        return pages[1:-1]

    selected_indices: list[int] = []
    for index in range(count):
        candidate = round((index + 1) * (total_pages - 1) / (count + 1))
        candidate = min(max(candidate, 1), total_pages - 2)
        if candidate not in selected_indices:
            selected_indices.append(candidate)
    return [pages[index] for index in selected_indices]


def sha256_file(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as handle:
        for block in iter(lambda: handle.read(1024 * 1024), b""):
            digest.update(block)
    return digest.hexdigest()


def paddle_markdown_from_jsonl(jsonl_text: str) -> tuple[str, int]:
    """Concatenate the per-page Markdown out of a PaddleOCR result stream.

    The same fields paddle.write_outputs writes to disk, minus the referenced
    images: a comparison is read as text, and fetching every figure would cost
    more time than the OCR call itself.
    """
    pages: list[str] = []
    for raw in jsonl_text.splitlines():
        line = raw.strip()
        if not line:
            continue
        try:
            payload = json.loads(line)
        except json.JSONDecodeError as error:
            raise SampleCompareError(f"paddleocr returned invalid JSONL: {error}") from error
        result = payload.get("result", payload)
        for entry in result.get("layoutParsingResults", []):
            pages.append((entry.get("markdown") or {}).get("text", ""))
    return "\n\n".join(pages), len(pages)


def run_paddleocr_sample(sample_pdf: Path, work_dir: Path) -> EngineOutcome:
    token = os.environ.get("BAIDU_PADDLEOCR_TOKEN", "").strip()
    if not token:
        raise SampleCompareError("BAIDU_PADDLEOCR_TOKEN is not configured")
    # Built through PaddleOCR's own parser so the sample inherits the CLI's
    # defaults -- model, endpoint and timeouts -- instead of a second copy that
    # can drift away from the run the sample is meant to preview.
    args = paddle.build_parser().parse_args([str(sample_pdf)])
    args.output_dir = str(work_dir)
    headers = {"Authorization": f"bearer {token}"}
    optional_payload = dict(paddle.DEFAULT_OPTIONAL_PAYLOAD)
    job_id = paddle.submit_job(args, headers, optional_payload)
    json_url = paddle.poll_json_url(args, headers, job_id)
    jsonl_text = paddle.download_jsonl(json_url, args.timeout_seconds)
    work_dir.mkdir(parents=True, exist_ok=True)
    # Scratch, like everything else under work_dir: the caller removes the tree
    # once the report is written.
    (work_dir / "paddleocr.jsonl").write_text(jsonl_text, encoding="utf-8")
    markdown, pages = paddle_markdown_from_jsonl(jsonl_text)
    return EngineOutcome(markdown=markdown, page_count=pages)


def run_mineru_sample(sample_pdf: Path, work_dir: Path) -> EngineOutcome:
    # Either spelling counts as configured, matching mineru.main and the
    # launcher's own credential status check.
    token = (
        os.environ.get("MINERU_API_TOKEN", "").strip()
        or os.environ.get("MINERU_TOKEN", "").strip()
    )
    if not token:
        raise SampleCompareError("MINERU_API_TOKEN is not configured")
    # Same reasoning as the PaddleOCR side: MinerU's own parser supplies
    # language, formula and table defaults. `vlm` is the model version the
    # launcher's local-PDF MinerU route already runs.
    args = mineru.build_parser().parse_args([])
    args.model_version = "vlm"
    args.output_dir = str(work_dir)
    item = mineru.WorkItem(
        source=str(sample_pdf),
        name=sample_pdf.name,
        data_id=mineru.data_id_for(str(sample_pdf)),
        local_path=sample_pdf,
        source_pages=mineru.pdf_page_count(sample_pdf),
    )
    session = requests.Session()
    batch_id = mineru.submit_local_batch(session, args, token, [item])
    results = mineru.poll_batch(session, args, token, batch_id, [item])
    parts = mineru.download_results(args, batch_id, results, [item])
    if not parts:
        raise SampleCompareError("mineru returned no Markdown for the sampled pages")
    markdown = parts[0].markdown_path.read_text(encoding="utf-8")
    return EngineOutcome(markdown=markdown, page_count=item.source_pages)


DEFAULT_ENGINE_RUNNERS: Mapping[str, EngineRunner] = {
    ENGINE_PADDLEOCR: run_paddleocr_sample,
    ENGINE_MINERU: run_mineru_sample,
}


def _required_string(manifest: Mapping[str, Any], key: str) -> str:
    value = manifest.get(key)
    if not isinstance(value, str) or not value.strip():
        raise SampleCompareError(f"invalid_{key}")
    return value


def _positive_integer(manifest: Mapping[str, Any], key: str, maximum: int) -> int:
    value = manifest.get(key)
    if not isinstance(value, int) or isinstance(value, bool) or value < 1 or value > maximum:
        raise SampleCompareError(f"invalid_{key}")
    return value


def _requested_engines(manifest: Mapping[str, Any]) -> list[str]:
    engines = manifest.get("engines")
    if not isinstance(engines, list) or not engines:
        raise SampleCompareError("invalid_engines")
    resolved: list[str] = []
    for engine in engines:
        if engine not in SUPPORTED_ENGINES:
            raise SampleCompareError(f"unsupported_engine:{engine}")
        if engine in resolved:
            raise SampleCompareError(f"duplicate_engine:{engine}")
        resolved.append(engine)
    return resolved


def _manifest_path(project_root: Path, value: str) -> Path:
    """Resolve a manifest-relative path and keep it inside the project root."""
    candidate = Path(value)
    resolved = (candidate if candidate.is_absolute() else project_root / candidate).resolve()
    if not resolved.is_relative_to(project_root):
        raise SampleCompareError("path_outside_project_root")
    return resolved


def run_sample_manifest(
    manifest_path: Path,
    *,
    engine_runners: Mapping[str, EngineRunner] | None = None,
) -> dict[str, Any]:
    runners = DEFAULT_ENGINE_RUNNERS if engine_runners is None else engine_runners
    try:
        manifest = json.loads(manifest_path.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError) as error:
        raise SampleCompareError(f"unreadable_manifest: {error}") from error
    if not isinstance(manifest, dict) or manifest.get("schema") != MANIFEST_SCHEMA:
        raise SampleCompareError("unsupported_manifest_schema")

    project_root = Path(_required_string(manifest, "projectRoot")).resolve()
    if not project_root.is_dir():
        raise SampleCompareError("missing_project_root")
    # The source PDF is deliberately allowed outside the project root: a Zotero
    # attachment or a watched folder lives wherever the user keeps it, and the
    # sample reads it without ever writing there.
    source_pdf = Path(_required_string(manifest, "sourcePdfPath")).expanduser().resolve()
    if not source_pdf.is_file():
        raise SampleCompareError("missing_source_pdf")
    report_path = _manifest_path(project_root, _required_string(manifest, "reportPath"))
    work_dir = _manifest_path(project_root, _required_string(manifest, "workDir"))
    sample_pages = _positive_integer(manifest, "samplePages", MAX_SAMPLE_PAGES)
    character_budget = _positive_integer(manifest, "characterBudget", 1_000_000)
    engines = _requested_engines(manifest)

    total_pages = mineru.pdf_page_count(source_pdf)
    if total_pages is None:
        raise SampleCompareError("unreadable_pdf_page_count")
    selected_pages = select_internal_pages(total_pages, sample_pages)
    if not selected_pages:
        # Two pages or fewer leaves nothing between the excluded endpoints. That
        # is a defined outcome, not a failure: the caller is told the book is too
        # short to preview rather than being shown its cover as evidence.
        raise SampleCompareError("pdf_too_short_to_sample")

    work_dir.mkdir(parents=True, exist_ok=True)
    results: list[dict[str, Any]] = []
    # Everything the engines write goes inside this directory, and none of it
    # outlives the run. Two reasons it cannot simply be left on disk: MinerU
    # drops the sampled pages as `part.md`, and the launcher's conversion stage
    # recursively scans the job output tree for artifacts -- a stray .md there
    # registers as the book's `markdown` output and can be handed off for
    # translation in place of the real conversion. It is also the raw, unbudgeted
    # text of the user's pages, which only the capped excerpt in the report is
    # meant to retain.
    with tempfile.TemporaryDirectory(prefix="ocr-sample-", dir=work_dir) as scratch:
        scratch_dir = Path(scratch)
        sample_pdf = scratch_dir / f"sample-{source_pdf.stem}.pdf"
        try:
            mineru.write_pdf_selection(source_pdf, sample_pdf, selected_pages)
        except mineru.MinerUError as error:
            raise SampleCompareError(f"sample_extraction_failed: {error}") from error
        for engine in engines:
            runner = runners.get(engine)
            if runner is None:
                raise SampleCompareError(f"unsupported_engine:{engine}")
            results.append(
                _run_one_engine(engine, runner, sample_pdf, scratch_dir, character_budget)
            )

    report = {
        "schema": REPORT_SCHEMA,
        "sourcePdfSha256": sha256_file(source_pdf),
        "totalPages": total_pages,
        "sampledPages": selected_pages,
        "characterBudget": character_budget,
        "engines": results,
    }
    report_path.parent.mkdir(parents=True, exist_ok=True)
    report_path.write_text(
        json.dumps(report, ensure_ascii=False, indent=2) + "\n", encoding="utf-8"
    )
    return report


_SIGNED_URL = re.compile(r"(https?://[^\s\"']*?)\?[^\s\"']*")
_BEARER = re.compile(r"(?i)\b(bearer)\s+\S+")
_TOKEN_ASSIGNMENT = re.compile(r"(?i)\b([A-Z0-9_]*(?:TOKEN|KEY|SECRET))\s*[=:]\s*\S+")


def redact_engine_error(message: str) -> str:
    """Strip credentials and signed URLs out of an engine's failure text.

    The message goes into the report, which lives on disk, so it gets the same
    treatment the launcher gives log lines: a result URL from PaddleOCR is
    signed in its query string, and an upstream error can echo an auth header
    back at us. A bare `MINERU_API_TOKEN is not configured` names no value and
    survives intact -- an over-redacted message that hides which key is missing
    would make the failure unactionable, which is the point of showing it.
    """
    # Truncate first: the URL pattern is lazy, so scanning a pathological
    # multi-megabyte exception body would cost far more than it is worth.
    redacted = _SIGNED_URL.sub(r"\1?<redacted>", message[:2000])
    redacted = _BEARER.sub(r"\1 <redacted>", redacted)
    return _TOKEN_ASSIGNMENT.sub(r"\1=<redacted>", redacted)


def _run_one_engine(
    engine: str,
    runner: EngineRunner,
    sample_pdf: Path,
    work_dir: Path,
    character_budget: int,
) -> dict[str, Any]:
    """Run one engine, recording a failure rather than raising it.

    A comparison with one side missing is still worth showing -- one absent
    token or one API outage should not throw away the result the other engine
    already paid for.
    """
    engine_dir = work_dir / engine
    started = time.monotonic()
    try:
        outcome = runner(sample_pdf, engine_dir)
    except Exception as error:  # noqa: BLE001 - the engine's failure is the report
        return {
            "engine": engine,
            "status": "failed",
            "markdownExcerpt": "",
            "characterCount": 0,
            "pageCount": None,
            "elapsedMs": round((time.monotonic() - started) * 1000),
            "error": redact_engine_error(f"{type(error).__name__}: {error}"),
        }
    markdown = outcome.markdown or ""
    return {
        "engine": engine,
        "status": "ok",
        "markdownExcerpt": markdown[:character_budget],
        "characterCount": len(markdown),
        "pageCount": outcome.page_count,
        "elapsedMs": round((time.monotonic() - started) * 1000),
        "error": None,
    }


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(
        description="Compare PaddleOCR and MinerU on sampled interior pages of one PDF."
    )
    parser.add_argument("--manifest", required=True, help=f"Path to an {MANIFEST_SCHEMA} manifest.")
    return parser


def main(argv: list[str] | None = None) -> int:
    paddle.load_root_dotenv()
    args = build_parser().parse_args(argv)
    report = run_sample_manifest(Path(args.manifest).expanduser().resolve())
    # Progress only. The excerpts are pages of the user's book -- the launcher
    # classifies the report as private text and reads it from disk -- and the
    # caller captures this stream into its run log, so the report itself must
    # not travel through it.
    for result in report["engines"]:
        print(
            "engine={} status={} characters={}".format(
                result["engine"], result["status"], result["characterCount"]
            )
        )
    print("pages={}".format(len(report["sampledPages"])))
    return 0


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except SampleCompareError as exc:
        print(f"error: {exc}", file=sys.stderr)
        raise SystemExit(1)
