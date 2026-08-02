#!/usr/bin/env python3
"""Measure PDF→Markdown conversion against the real local corpus.

Hand-built fixtures cannot produce the failures this corpus actually hits —
CNKI downloads with a broken file trailer, CID fonts a parser cannot decode,
image scans that carry a perfectly good text layer, Zotero attachments that are
JSON with a ``.pdf`` name. Every one of those is invisible to a generated
fixture, so a conversion change can only be called a win or a regression
against real books.

This harness runs each file through the production PyMuPDF path — the same
``pdf_page_count`` / ``sample_text_layer`` / ``extract_text_pages`` functions
``zotero_llm_worker.py`` uses, imported rather than reimplemented — and through
a candidate engine, and reports the signals that decide the question:

* parse-failure rate, split by whether the failing document is Chinese, which
  is where the risk concentrates
* real headings and ``## Page N`` scaffolding headings counted **separately**:
  merged into one number, a converter that emits one heading per page scores
  the same as one that finds chapters
* table rows, links and code blocks — the structure the EPUB builder consumes
* non-space character volume, which catches "structure improved, text lost"
* per-file wall clock, per engine, for classification and extraction

Nothing here is written into the tree. Per-file records carry real book titles
and absolute paths, so every artifact lands under ``output/``, which is
gitignored. ``summary.md`` is written without titles or paths so it can be
quoted in a pull request; ``report.json`` and ``files.csv`` cannot.

The tool makes no network call and reaches no paid API: PyMuPDF and
pdf-inspector are both local parsers.

pdf-inspector is deliberately not a dependency of ``packages/ocr`` yet, so it
is supplied per-run::

    uv run --with pdf-inspector python \
        tools/pdf_benchmark/pdf_markdown_benchmark.py --classify-only

Dropping ``--classify-only`` adds the extraction pass, which is the expensive
one; ``--extract-stride`` and ``--extract-limit`` bound it.
"""

from __future__ import annotations

import argparse
import csv
import datetime as dt
import importlib.util
import json
import re
import sys
import time
from dataclasses import asdict, dataclass, field
from pathlib import Path
from types import ModuleType
from typing import Any, Sequence


REPO_ROOT = Path(__file__).resolve().parents[2]
OCR_SCRIPTS = REPO_ROOT / "packages" / "ocr" / "scripts"
DEFAULT_CORPUS_ROOT = Path.home() / "Zotero" / "storage"
DEFAULT_CORPUS_GLOB = "*/*.pdf"
DEFAULT_STRIDE = 7
DEFAULT_OUTPUT_ROOT = REPO_ROOT / "output" / "pdf-benchmark"

PYMUPDF_ENGINE = "pymupdf"
PDF_INSPECTOR_ENGINE = "pdf-inspector"
ALL_ENGINES = (PYMUPDF_ENGINE, PDF_INSPECTOR_ENGINE)

STATUS_OK = "ok"
STATUS_EMPTY = "empty"
STATUS_ERROR = "error"
STATUS_SKIPPED = "skipped"

# A filename carries the Better BibTeX title, so it answers the language
# question for free on the CNKI-style downloads that dominate the failures.
# The text ratio is the fallback for Latin-named files with Chinese content.
FILENAME_CJK_RATIO = 0.15
TEXT_CJK_RATIO = 0.20


# --------------------------------------------------------------------------
# Markdown metrics
# --------------------------------------------------------------------------

# ``## Page 12`` is scaffolding the current extractor emits once per page, not
# a heading recovered from the document. Counting it as a heading is exactly
# the mistake this harness exists to prevent, so it is matched first and kept
# in its own column.
PAGE_SCAFFOLD_RE = re.compile(r"^ {0,3}#{1,6}\s+page\s+\d+\s*$", re.IGNORECASE)
ATX_HEADING_RE = re.compile(r"^ {0,3}#{1,6}\s+\S")
TABLE_ROW_RE = re.compile(r"^\s*\|.*\|\s*$")
TABLE_DELIMITER_RE = re.compile(r"^\s*\|[\s:|-]*\|\s*$")
FENCE_RE = re.compile(r"^\s*(```|~~~)")
# Images (``![alt](src)``) are not links, so the lookbehind drops them.
INLINE_LINK_RE = re.compile(r"(?<!!)\[[^\[\]\n]*\]\([^()\n]*\)")
AUTOLINK_RE = re.compile(r"<[A-Za-z][A-Za-z0-9+.\-]*:[^ <>\n]+>")


@dataclass(frozen=True)
class MarkdownMetrics:
    """Structure and volume of one Markdown document."""

    real_headings: int = 0
    scaffolding_headings: int = 0
    table_rows: int = 0
    links: int = 0
    code_blocks: int = 0
    nonspace_chars: int = 0

    def __add__(self, other: "MarkdownMetrics") -> "MarkdownMetrics":
        return MarkdownMetrics(
            real_headings=self.real_headings + other.real_headings,
            scaffolding_headings=self.scaffolding_headings + other.scaffolding_headings,
            table_rows=self.table_rows + other.table_rows,
            links=self.links + other.links,
            code_blocks=self.code_blocks + other.code_blocks,
            nonspace_chars=self.nonspace_chars + other.nonspace_chars,
        )


def count_nonspace(text: str) -> int:
    return sum(1 for ch in text if not ch.isspace())


def cjk_ratio(text: str) -> float:
    nonspace = [ch for ch in text if not ch.isspace()]
    if not nonspace:
        return 0.0
    han = sum(1 for ch in nonspace if "一" <= ch <= "鿿")
    return han / len(nonspace)


def measure_markdown(markdown: str) -> MarkdownMetrics:
    """Count structural signals in a Markdown document.

    Headings, table rows and links are counted outside fenced code blocks
    only — a converter that recovers a code listing would otherwise be charged
    for whatever the listing happens to contain. ``nonspace_chars`` covers the
    whole document, including the scaffolding, because it measures the artifact
    that is handed downstream rather than the prose inside it.
    """

    real_headings = 0
    scaffolding_headings = 0
    table_rows = 0
    code_blocks = 0
    in_fence = False
    fence_marker = ""
    prose_lines: list[str] = []

    for line in markdown.splitlines():
        fence = FENCE_RE.match(line)
        if fence:
            marker = fence.group(1)
            if not in_fence:
                in_fence = True
                fence_marker = marker
                code_blocks += 1
            elif marker == fence_marker:
                in_fence = False
            continue
        if in_fence:
            continue
        prose_lines.append(line)
        if PAGE_SCAFFOLD_RE.match(line):
            scaffolding_headings += 1
        elif ATX_HEADING_RE.match(line):
            real_headings += 1
        elif TABLE_ROW_RE.match(line) and not TABLE_DELIMITER_RE.match(line):
            table_rows += 1

    prose = "\n".join(prose_lines)
    links = len(INLINE_LINK_RE.findall(prose)) + len(AUTOLINK_RE.findall(prose))
    return MarkdownMetrics(
        real_headings=real_headings,
        scaffolding_headings=scaffolding_headings,
        table_rows=table_rows,
        links=links,
        code_blocks=code_blocks,
        nonspace_chars=count_nonspace(markdown),
    )


# --------------------------------------------------------------------------
# Corpus selection
# --------------------------------------------------------------------------


def file_magic(path: Path) -> str:
    """Classify a file by its leading bytes.

    Zotero stores API error payloads under a ``.pdf`` name often enough that
    "is this even a PDF" has to be answered before either engine is blamed for
    rejecting it.
    """

    try:
        head = path.open("rb").read(1024)
    except OSError:
        return "unreadable"
    if head.startswith(b"%PDF"):
        return "pdf"
    stripped = head.lstrip()
    if stripped[:1] in (b"{", b"["):
        return "json"
    return "other"


def select_corpus(
    *,
    root: Path,
    glob: str,
    stride: int,
    offset: int,
    limit: int | None,
    file_list: Path | None,
) -> list[Path]:
    """Pick the files to measure, deterministically.

    Sampling is a fixed stride over the sorted listing rather than a random
    draw: two runs of the harness have to compare against each other, and a
    seed that only lives in one shell history is not a corpus definition.
    """

    if file_list is not None:
        paths = [
            Path(line.strip()).expanduser()
            for line in file_list.read_text(encoding="utf-8").splitlines()
            if line.strip() and not line.lstrip().startswith("#")
        ]
    else:
        paths = sorted(root.glob(glob))
        paths = paths[offset::stride]
    if limit is not None:
        paths = paths[:limit]
    return paths


def sub_sample(paths: Sequence[Path], *, stride: int, limit: int | None) -> list[Path]:
    selected = list(paths[::stride])
    if limit is not None:
        selected = selected[:limit]
    return selected


# --------------------------------------------------------------------------
# Engines
# --------------------------------------------------------------------------


@dataclass
class EngineOutcome:
    """One engine's result for one file, in one pass.

    ``markdown`` is kept off the report on purpose: it is book text, it is
    large, and it is only here so ``--keep-markdown`` can write it without
    running the conversion a second time.
    """

    status: str
    seconds: float
    error: str = ""
    detail: dict[str, Any] = field(default_factory=dict)
    metrics: MarkdownMetrics | None = None
    markdown: str | None = None

    def to_json(self) -> dict[str, Any]:
        payload: dict[str, Any] = {
            "status": self.status,
            "seconds": round(self.seconds, 4),
        }
        if self.error:
            payload["error"] = self.error
        if self.detail:
            payload["detail"] = self.detail
        if self.metrics is not None:
            payload["metrics"] = asdict(self.metrics)
        return payload


def describe_exception(exc: BaseException) -> str:
    text = " ".join(str(exc).split())
    return f"{type(exc).__name__}: {text}" if text else type(exc).__name__


def load_worker_module() -> ModuleType:
    """Import ``zotero_llm_worker`` so the benchmark measures production code.

    Loading by path rather than by package import mirrors what the OCR test
    suite already does, and keeps this file runnable from the repository root
    without installing anything.
    """

    if str(OCR_SCRIPTS) not in sys.path:
        sys.path.insert(0, str(OCR_SCRIPTS))
    path = OCR_SCRIPTS / "zotero_llm_worker.py"
    spec = importlib.util.spec_from_file_location("pdf_benchmark_zotero_llm_worker", path)
    if spec is None or spec.loader is None:
        raise RuntimeError(f"Cannot import the Zotero worker from {path}")
    module = importlib.util.module_from_spec(spec)
    # Registered before execution because `@dataclass` resolves annotations
    # through `sys.modules[cls.__module__]`, which is not yet populated while
    # the module body is still running.
    sys.modules[spec.name] = module
    spec.loader.exec_module(module)
    return module


def render_page_scaffold_markdown(pages: Sequence[tuple[int, str]]) -> str:
    """Render the Markdown body the ``pdf-text`` route produces today.

    This is ``zotero_llm_worker.render_markdown`` without its front matter and
    ``# {title}`` line, both of which come from the Zotero item rather than
    from the PDF: a conversion benchmark must not credit an engine for metadata
    it was handed. ``test_pdf_markdown_benchmark.py`` asserts the two stay in
    step.
    """

    body: list[str] = []
    for page_no, text in pages:
        body.append(f"## Page {page_no}")
        body.append("")
        body.append(text if text else "[no extractable text]")
        body.append("")
    return "\n".join(body).rstrip() + "\n"


class PyMuPdfEngine:
    """The route we ship today: PyMuPDF text per page under a page heading."""

    name = PYMUPDF_ENGINE

    def __init__(self, worker: ModuleType) -> None:
        self.worker = worker
        # The production thresholds, not a copy of them: this is what decides
        # `direct_text` versus OCR on the Zotero route.
        self._config = worker.get_config()

    def classify(self, path: Path) -> EngineOutcome:
        started = time.perf_counter()
        try:
            page_count = self.worker.pdf_page_count(path)
            sample = self.worker.sample_text_layer(path, page_count, self._config)
        except (KeyboardInterrupt, SystemExit):
            raise
        except BaseException as exc:  # noqa: BLE001 - one bad file must not end the run
            return EngineOutcome(STATUS_ERROR, time.perf_counter() - started, describe_exception(exc))
        seconds = time.perf_counter() - started
        label = "direct_text" if sample.extractable else "needs_ocr"
        if sample.degraded:
            label = "dirty_text_layer"
        return EngineOutcome(
            STATUS_OK,
            seconds,
            detail={
                "label": label,
                "page_count": page_count,
                "sample_pages": list(sample.sample_pages),
                "sample_nonspace_chars": sample.chars,
                "extractable": sample.extractable,
                "degraded": sample.degraded,
                "degraded_reason": sample.reason,
            },
        )

    def extract(self, path: Path) -> EngineOutcome:
        started = time.perf_counter()
        try:
            page_count = self.worker.pdf_page_count(path)
            pages = self.worker.extract_text_pages(path, range(1, page_count + 1))
            markdown = render_page_scaffold_markdown(pages)
        except (KeyboardInterrupt, SystemExit):
            raise
        except BaseException as exc:  # noqa: BLE001 - see classify()
            return EngineOutcome(STATUS_ERROR, time.perf_counter() - started, describe_exception(exc))
        seconds = time.perf_counter() - started
        # The page scaffolding is emitted whether or not a page had text, so
        # "produced Markdown" has to be judged on the extracted text, not on
        # the size of the rendered document.
        text_chars = sum(count_nonspace(text) for _, text in pages)
        return EngineOutcome(
            STATUS_OK if text_chars else STATUS_EMPTY,
            seconds,
            detail={"page_count": page_count, "title": None},
            metrics=measure_markdown(markdown),
            markdown=markdown,
        )


class PdfInspectorEngine:
    """The candidate: firecrawl/pdf-inspector, a local Rust structure engine."""

    name = PDF_INSPECTOR_ENGINE

    def __init__(self, module: ModuleType) -> None:
        self._pdf_inspector = module

    def classify(self, path: Path) -> EngineOutcome:
        started = time.perf_counter()
        try:
            result = self._pdf_inspector.classify_pdf(str(path))
        except (KeyboardInterrupt, SystemExit):
            raise
        except BaseException as exc:  # noqa: BLE001 - a PyO3 panic is not an Exception
            return EngineOutcome(STATUS_ERROR, time.perf_counter() - started, describe_exception(exc))
        seconds = time.perf_counter() - started
        return EngineOutcome(
            STATUS_OK,
            seconds,
            detail={
                "label": result.pdf_type,
                "page_count": result.page_count,
                "confidence": round(float(result.confidence), 4),
                "pages_needing_ocr": len(result.pages_needing_ocr),
            },
        )

    def extract(self, path: Path) -> EngineOutcome:
        started = time.perf_counter()
        try:
            result = self._pdf_inspector.process_pdf(str(path))
            markdown = result.markdown or ""
        except (KeyboardInterrupt, SystemExit):
            raise
        except BaseException as exc:  # noqa: BLE001 - see classify()
            return EngineOutcome(STATUS_ERROR, time.perf_counter() - started, describe_exception(exc))
        seconds = time.perf_counter() - started
        metrics = measure_markdown(markdown)
        return EngineOutcome(
            STATUS_EMPTY if metrics.nonspace_chars == 0 else STATUS_OK,
            seconds,
            detail={
                "page_count": result.page_count,
                "title": result.title,
                "pdf_type": result.pdf_type,
                "has_encoding_issues": bool(result.has_encoding_issues),
                "pages_needing_ocr": len(result.pages_needing_ocr),
            },
            metrics=metrics,
            markdown=markdown,
        )


def build_engines(names: Sequence[str]) -> dict[str, Any]:
    engines: dict[str, Any] = {}
    for name in names:
        if name == PYMUPDF_ENGINE:
            engines[name] = PyMuPdfEngine(load_worker_module())
        elif name == PDF_INSPECTOR_ENGINE:
            try:
                import pdf_inspector
            except ImportError as exc:
                raise SystemExit(
                    "pdf-inspector is not importable. It is not a project dependency yet, "
                    "so supply it for this run:\n"
                    "  uv run --with pdf-inspector python "
                    f"tools/pdf_benchmark/{Path(__file__).name} ...\n"
                    f"({exc})"
                ) from exc
            engines[name] = PdfInspectorEngine(pdf_inspector)
        else:
            raise SystemExit(f"Unknown engine: {name}")
    return engines


# --------------------------------------------------------------------------
# Passes
# --------------------------------------------------------------------------


@dataclass
class FileRecord:
    path: Path
    magic: str
    size_bytes: int
    chinese: bool = False
    language_source: str = ""
    page_count: int | None = None
    classify: dict[str, EngineOutcome] = field(default_factory=dict)
    extract: dict[str, EngineOutcome] = field(default_factory=dict)

    def to_json(self) -> dict[str, Any]:
        return {
            "path": str(self.path),
            "name": self.path.name,
            "magic": self.magic,
            "size_bytes": self.size_bytes,
            "chinese": self.chinese,
            "language_source": self.language_source,
            "page_count": self.page_count,
            "classify": {name: outcome.to_json() for name, outcome in self.classify.items()},
            "extract": {name: outcome.to_json() for name, outcome in self.extract.items()},
        }


def language_signal(record: FileRecord, worker: ModuleType | None) -> tuple[bool, str]:
    """Decide whether a document is Chinese, cheaply and outside the clock.

    The filename answers it for the CNKI/wanfang downloads without opening
    anything. Only when the filename is Latin does this sample text, and never
    inside a timed region, so the classification-speed comparison stays honest.
    """

    if cjk_ratio(record.path.stem) >= FILENAME_CJK_RATIO:
        return True, "filename"
    outcome = record.classify.get(PYMUPDF_ENGINE)
    if worker is None or outcome is None or outcome.status != STATUS_OK:
        return False, "filename"
    sample_pages = outcome.detail.get("sample_pages") or []
    if not sample_pages:
        return False, "filename"
    try:
        sampled = worker.extract_text_pages(record.path, sample_pages)
    except (KeyboardInterrupt, SystemExit):
        raise
    except BaseException:  # noqa: BLE001 - a language hint is never worth failing a run
        return False, "filename"
    text = "\n".join(text for _, text in sampled)
    return cjk_ratio(text) >= TEXT_CJK_RATIO, "text"


def run_classification(
    paths: Sequence[Path],
    engines: dict[str, Any],
    *,
    progress: bool,
) -> list[FileRecord]:
    records: list[FileRecord] = []
    worker = getattr(engines.get(PYMUPDF_ENGINE), "worker", None)
    for index, path in enumerate(paths, start=1):
        try:
            size = path.stat().st_size
        except OSError:
            size = 0
        record = FileRecord(path=path, magic=file_magic(path), size_bytes=size)
        for name, engine in engines.items():
            record.classify[name] = engine.classify(path)
        pymupdf_outcome = record.classify.get(PYMUPDF_ENGINE)
        if pymupdf_outcome is not None and pymupdf_outcome.status == STATUS_OK:
            record.page_count = pymupdf_outcome.detail.get("page_count")
        record.chinese, record.language_source = language_signal(record, worker)
        records.append(record)
        if progress:
            print(f"  classify {index}/{len(paths)} {record.magic}", file=sys.stderr)
    return records


def run_extraction(
    records: Sequence[FileRecord],
    engines: dict[str, Any],
    *,
    max_pages: int | None,
    keep_markdown_dir: Path | None,
    progress: bool,
) -> None:
    for index, record in enumerate(records, start=1):
        if max_pages is not None and record.page_count is not None and record.page_count > max_pages:
            for name in engines:
                record.extract[name] = EngineOutcome(
                    STATUS_SKIPPED, 0.0, f"page_count {record.page_count} > --max-pages {max_pages}"
                )
            continue
        for name, engine in engines.items():
            outcome = engine.extract(record.path)
            record.extract[name] = outcome
        if progress:
            summary = " ".join(
                f"{name}={record.extract[name].status}/{record.extract[name].seconds:.1f}s"
                for name in engines
            )
            print(f"  extract {index}/{len(records)} pages={record.page_count} {summary}", file=sys.stderr)
        if keep_markdown_dir is not None:
            dump_markdown(record, keep_markdown_dir)
        # Book text is the largest thing a run holds, and it has now been
        # measured and, if asked for, written. Keeping it would mean holding
        # the whole corpus in memory to no purpose.
        for outcome in record.extract.values():
            outcome.markdown = None


def dump_markdown(record: FileRecord, directory: Path) -> None:
    """Write each engine's Markdown for eyeballing.

    Off by default: the outputs are large and carry book text, so they are
    produced only when explicitly asked for.
    """

    directory.mkdir(parents=True, exist_ok=True)
    stem = re.sub(r"[^\w.\-]+", "_", record.path.stem)[:120]
    for name, outcome in record.extract.items():
        if outcome.markdown is None:
            continue
        (directory / f"{stem}.{name}.md").write_text(outcome.markdown, encoding="utf-8")


# --------------------------------------------------------------------------
# Aggregation
# --------------------------------------------------------------------------


def summarize(records: Sequence[FileRecord], engine_names: Sequence[str]) -> dict[str, Any]:
    real_pdfs = [record for record in records if record.magic == "pdf"]
    classification: dict[str, Any] = {
        "files": len(records),
        "real_pdfs": len(real_pdfs),
        "non_pdf_files": len(records) - len(real_pdfs),
        "chinese_files": sum(1 for record in records if record.chinese),
        "engines": {},
    }
    for name in engine_names:
        outcomes = [record.classify[name] for record in records if name in record.classify]
        failures = [
            record
            for record in real_pdfs
            if record.classify.get(name, EngineOutcome(STATUS_ERROR, 0.0)).status != STATUS_OK
        ]
        labels: dict[str, int] = {}
        for outcome in outcomes:
            if outcome.status == STATUS_OK:
                label = str(outcome.detail.get("label", "unknown"))
                labels[label] = labels.get(label, 0) + 1
        classification["engines"][name] = {
            "ok": sum(1 for outcome in outcomes if outcome.status == STATUS_OK),
            "seconds": round(sum(outcome.seconds for outcome in outcomes), 2),
            "real_pdf_failures": len(failures),
            "real_pdf_failure_rate": round(len(failures) / len(real_pdfs), 4) if real_pdfs else 0.0,
            "real_pdf_failures_chinese": sum(1 for record in failures if record.chinese),
            "non_pdf_rejected": sum(
                1
                for record in records
                if record.magic != "pdf"
                and record.classify.get(name, EngineOutcome(STATUS_OK, 0.0)).status != STATUS_OK
            ),
            "labels": dict(sorted(labels.items())),
        }

    attempted = [record for record in records if record.extract]
    summary: dict[str, Any] = {"classification": classification}
    if not attempted:
        return summary

    paired = [
        record
        for record in attempted
        if all(record.extract.get(name, EngineOutcome(STATUS_ERROR, 0.0)).status == STATUS_OK for name in engine_names)
    ]
    summary["extraction"] = {
        "attempted": len(attempted),
        "engines": {name: engine_extraction_totals(attempted, name) for name in engine_names},
        "paired": {
            "files": len(paired),
            "pages": sum(record.page_count or 0 for record in paired),
            "engines": {name: engine_extraction_totals(paired, name) for name in engine_names},
        },
    }
    return summary


def engine_extraction_totals(records: Sequence[FileRecord], name: str) -> dict[str, Any]:
    outcomes = [record.extract[name] for record in records if name in record.extract]
    metrics = MarkdownMetrics()
    for outcome in outcomes:
        if outcome.metrics is not None:
            metrics = metrics + outcome.metrics
    titles = sum(
        1
        for outcome in outcomes
        if outcome.status == STATUS_OK and str(outcome.detail.get("title") or "").strip()
    )
    return {
        "ok": sum(1 for outcome in outcomes if outcome.status == STATUS_OK),
        "empty": sum(1 for outcome in outcomes if outcome.status == STATUS_EMPTY),
        "error": sum(1 for outcome in outcomes if outcome.status == STATUS_ERROR),
        "skipped": sum(1 for outcome in outcomes if outcome.status == STATUS_SKIPPED),
        "seconds": round(sum(outcome.seconds for outcome in outcomes), 2),
        "titles_recovered": titles,
        **asdict(metrics),
    }


# --------------------------------------------------------------------------
# Reporting
# --------------------------------------------------------------------------


def home_relative(path: str) -> str:
    """Render a path without the home directory it sits under.

    The summary is meant to be quotable in a public issue, and `/Users/<name>`
    is exactly what the repository's personal-info scan exists to keep out.
    """

    home = str(Path.home())
    return "~" + path[len(home) :] if path.startswith(home) else path


def render_summary_markdown(report: dict[str, Any]) -> str:
    """Aggregates only.

    No book title and no home path reaches this file, so it is the one
    artifact of a run that is safe to paste into an issue or a pull request.
    """

    run = report["run"]
    summary = report["summary"]
    classification = summary["classification"]
    engine_names: list[str] = run["engines"]
    lines = [
        "# PDF→Markdown conversion benchmark",
        "",
        f"- Run: `{run['name']}`",
        f"- Corpus: `{home_relative(run['corpus'])}` files={classification['files']} "
        f"(stride={run['stride']}, offset={run['offset']}, limit={run['limit']})",
        f"- Engines: {', '.join(engine_names)}",
        f"- Real PDFs: {classification['real_pdfs']}; "
        f"non-PDF files: {classification['non_pdf_files']}; "
        f"Chinese: {classification['chinese_files']}",
        "",
        "## Classification",
        "",
        "| Signal | " + " | ".join(engine_names) + " |",
        "|---|" + "---|" * len(engine_names),
    ]

    def classification_row(label: str, key: str) -> str:
        cells = [str(classification["engines"][name][key]) for name in engine_names]
        return f"| {label} | " + " | ".join(cells) + " |"

    lines += [
        classification_row("Classified OK", "ok"),
        classification_row("Real-PDF parse failures", "real_pdf_failures"),
        classification_row("…of which Chinese", "real_pdf_failures_chinese"),
        classification_row("Non-PDF files rejected", "non_pdf_rejected"),
        classification_row("Wall clock (s)", "seconds"),
    ]
    failure_rates = [
        f"{classification['engines'][name]['real_pdf_failure_rate'] * 100:.1f}%" for name in engine_names
    ]
    lines.append("| Real-PDF failure rate | " + " | ".join(failure_rates) + " |")
    lines.append("")
    for name in engine_names:
        labels = classification["engines"][name]["labels"]
        rendered = ", ".join(f"{label}={count}" for label, count in labels.items()) or "none"
        lines.append(f"- `{name}` labels: {rendered}")

    extraction = summary.get("extraction")
    if extraction:
        paired = extraction["paired"]
        # With one engine there is nothing to pair against, so the sentence has
        # to stop claiming a comparison it is not making.
        produced = "Every engine produced" if len(engine_names) > 1 else "The engine produced"
        caveat = (
            "; the table below is that paired subset, so the columns describe the same documents."
            if len(engine_names) > 1
            else "; the table below covers those."
        )
        lines += [
            "",
            "## Extraction",
            "",
            f"Attempted {extraction['attempted']} files. "
            f"{produced} Markdown for {paired['files']} of them "
            f"({paired['pages']} pages){caveat}",
            "",
            "| Signal | " + " | ".join(engine_names) + " |",
            "|---|" + "---|" * len(engine_names),
        ]

        def paired_row(label: str, key: str) -> str:
            cells = [f"{paired['engines'][name][key]:,}" for name in engine_names]
            return f"| {label} | " + " | ".join(cells) + " |"

        lines += [
            paired_row("Real headings", "real_headings"),
            paired_row("`## Page N` scaffolding headings", "scaffolding_headings"),
            paired_row("Table rows", "table_rows"),
            paired_row("Links", "links"),
            paired_row("Code blocks", "code_blocks"),
            paired_row("Titles recovered", "titles_recovered"),
            paired_row("Text volume (non-space chars)", "nonspace_chars"),
        ]
        seconds = [f"{paired['engines'][name]['seconds']:.1f}" for name in engine_names]
        lines.append("| Wall clock (s) | " + " | ".join(seconds) + " |")
        lines += [
            "",
            "Across every attempted file, including the ones only one engine handled:",
            "",
            "| Outcome | " + " | ".join(engine_names) + " |",
            "|---|" + "---|" * len(engine_names),
        ]
        for label, key in (("ok", "ok"), ("empty Markdown", "empty"), ("error", "error"), ("skipped", "skipped")):
            cells = [str(extraction["engines"][name][key]) for name in engine_names]
            lines.append(f"| {label} | " + " | ".join(cells) + " |")
    lines.append("")
    return "\n".join(lines)


CSV_COLUMNS = (
    "path",
    "name",
    "magic",
    "chinese",
    "page_count",
    "classify_status",
    "classify_label",
    "classify_seconds",
    "classify_error",
    "extract_status",
    "extract_seconds",
    "extract_real_headings",
    "extract_scaffolding_headings",
    "extract_table_rows",
    "extract_links",
    "extract_code_blocks",
    "extract_nonspace_chars",
    "extract_title",
    "extract_error",
)


def write_files_csv(path: Path, records: Sequence[FileRecord], engine_names: Sequence[str]) -> None:
    with path.open("w", encoding="utf-8", newline="") as handle:
        writer = csv.writer(handle)
        writer.writerow(("engine",) + CSV_COLUMNS)
        for record in records:
            for name in engine_names:
                classify = record.classify.get(name)
                extract = record.extract.get(name)
                metrics = extract.metrics if extract is not None else None
                writer.writerow(
                    [
                        name,
                        str(record.path),
                        record.path.name,
                        record.magic,
                        int(record.chinese),
                        record.page_count if record.page_count is not None else "",
                        classify.status if classify else "",
                        (classify.detail.get("label", "") if classify else ""),
                        f"{classify.seconds:.4f}" if classify else "",
                        classify.error if classify else "",
                        extract.status if extract else "",
                        f"{extract.seconds:.4f}" if extract else "",
                        metrics.real_headings if metrics else "",
                        metrics.scaffolding_headings if metrics else "",
                        metrics.table_rows if metrics else "",
                        metrics.links if metrics else "",
                        metrics.code_blocks if metrics else "",
                        metrics.nonspace_chars if metrics else "",
                        (extract.detail.get("title") or "") if extract else "",
                        extract.error if extract else "",
                    ]
                )


# --------------------------------------------------------------------------
# CLI
# --------------------------------------------------------------------------


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(
        description=__doc__,
        formatter_class=argparse.RawDescriptionHelpFormatter,
    )
    parser.add_argument(
        "--corpus-root",
        type=Path,
        default=DEFAULT_CORPUS_ROOT,
        help="Directory to sample from (default: the local Zotero storage tree).",
    )
    parser.add_argument("--glob", default=DEFAULT_CORPUS_GLOB, help="Glob applied under --corpus-root.")
    parser.add_argument(
        "--stride",
        type=int,
        default=DEFAULT_STRIDE,
        help="Take every Nth file of the sorted listing (default: %(default)s).",
    )
    parser.add_argument("--offset", type=int, default=0, help="Offset applied before --stride.")
    parser.add_argument("--limit", type=int, default=None, help="Cap the corpus after striding.")
    parser.add_argument(
        "--file-list",
        type=Path,
        default=None,
        help="Read explicit paths from a file, one per line, instead of sampling.",
    )
    parser.add_argument(
        "--engines",
        default=",".join(ALL_ENGINES),
        help="Comma-separated engines to run (default: %(default)s).",
    )
    parser.add_argument(
        "--classify-only",
        action="store_true",
        help="Run the classification pass only. Hundreds of files in seconds.",
    )
    parser.add_argument(
        "--extract-stride",
        type=int,
        default=1,
        help="Take every Nth classified file into the extraction pass.",
    )
    parser.add_argument("--extract-limit", type=int, default=None, help="Cap the extraction pass.")
    parser.add_argument(
        "--max-pages",
        type=int,
        default=None,
        help="Skip extraction for files longer than this, so one outsized book cannot dominate a run.",
    )
    parser.add_argument("--output-dir", type=Path, default=DEFAULT_OUTPUT_ROOT, help="Where run directories go.")
    parser.add_argument("--run-name", default=None, help="Run directory name (default: a UTC timestamp).")
    parser.add_argument(
        "--keep-markdown",
        action="store_true",
        help="Also write each engine's Markdown under the run directory.",
    )
    parser.add_argument("--quiet", action="store_true", help="Do not print per-file progress.")
    return parser


def main(argv: Sequence[str] | None = None) -> int:
    args = build_parser().parse_args(argv)
    engine_names = [name.strip() for name in args.engines.split(",") if name.strip()]
    if not engine_names:
        raise SystemExit("--engines selected nothing")
    if args.stride < 1 or args.extract_stride < 1:
        raise SystemExit("--stride and --extract-stride must be >= 1")

    paths = select_corpus(
        root=args.corpus_root,
        glob=args.glob,
        stride=args.stride,
        offset=args.offset,
        limit=args.limit,
        file_list=args.file_list,
    )
    if not paths:
        raise SystemExit(f"No files selected under {args.corpus_root} matching {args.glob!r}")

    engines = build_engines(engine_names)
    run_name = args.run_name or dt.datetime.now(dt.timezone.utc).strftime("%Y%m%dT%H%M%SZ")
    run_dir = args.output_dir / run_name
    run_dir.mkdir(parents=True, exist_ok=True)
    progress = not args.quiet

    if progress:
        print(f"Classifying {len(paths)} files with {', '.join(engine_names)}", file=sys.stderr)
    started = time.perf_counter()
    records = run_classification(paths, engines, progress=progress)

    if not args.classify_only:
        # Non-PDFs are excluded here on purpose: classification already
        # recorded that both engines reject them, and extracting them only
        # re-measures the same rejection.
        candidates = [record for record in records if record.magic == "pdf"]
        selected_paths = set(
            sub_sample([record.path for record in candidates], stride=args.extract_stride, limit=args.extract_limit)
        )
        extraction_records = [record for record in candidates if record.path in selected_paths]
        if progress:
            print(f"Extracting {len(extraction_records)} files", file=sys.stderr)
        run_extraction(
            extraction_records,
            engines,
            max_pages=args.max_pages,
            keep_markdown_dir=(run_dir / "markdown") if args.keep_markdown else None,
            progress=progress,
        )

    report = {
        "run": {
            "name": run_name,
            "corpus": str(args.corpus_root) if args.file_list is None else str(args.file_list),
            "glob": args.glob,
            "stride": args.stride,
            "offset": args.offset,
            "limit": args.limit,
            "engines": engine_names,
            "classify_only": args.classify_only,
            "extract_stride": args.extract_stride,
            "extract_limit": args.extract_limit,
            "max_pages": args.max_pages,
            "wall_seconds": round(time.perf_counter() - started, 2),
        },
        "summary": summarize(records, engine_names),
        "files": [record.to_json() for record in records],
    }

    (run_dir / "report.json").write_text(json.dumps(report, ensure_ascii=False, indent=2), encoding="utf-8")
    write_files_csv(run_dir / "files.csv", records, engine_names)
    summary_markdown = render_summary_markdown(report)
    (run_dir / "summary.md").write_text(summary_markdown, encoding="utf-8")

    print(summary_markdown)
    print(f"Wrote {run_dir}", file=sys.stderr)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
