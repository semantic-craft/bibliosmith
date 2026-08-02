# PDF→Markdown conversion benchmark

Measures the PDF→Markdown conversion this repository ships against real books,
so a change to the extraction side can be shown to be a win or a regression
instead of argued about.

This is T1 of `docs/planning/pdf-inspector-adoption.md` and issue #133. It is a
measurement tool: it changes nothing, makes no network call, and reaches no
paid API. Both engines are local parsers.

## Running it

`pdf-inspector` is deliberately **not** a dependency of `packages/ocr` yet — it
is declared there by #134 — so it is supplied per run:

```bash
uv run --with pdf-inspector python tools/pdf_benchmark/pdf_markdown_benchmark.py --classify-only
```

That is the fast pass: classification only, hundreds of files in seconds. Drop
`--classify-only` to add extraction, which is the expensive pass:

```bash
uv run --with pdf-inspector python tools/pdf_benchmark/pdf_markdown_benchmark.py --extract-stride 4
```

Useful flags:

| Flag | Effect |
|---|---|
| `--corpus-root`, `--glob` | Where to sample from. Default `~/Zotero/storage` and `*/*.pdf`. |
| `--stride`, `--offset`, `--limit` | Deterministic sampling of the sorted listing. Default stride 7. |
| `--file-list` | Explicit paths, one per line, instead of sampling. |
| `--classify-only` | Skip extraction. |
| `--extract-stride`, `--extract-limit` | Bound the extraction pass without changing the corpus. |
| `--max-pages` | Skip extraction for books longer than N pages. |
| `--engines` | `pymupdf`, `pdf-inspector`, or both. |
| `--keep-markdown` | Also write each engine's Markdown under the run directory. |

Sampling is a fixed stride rather than a random draw so that two runs compare
against each other. `--stride 7` over the sorted `~/Zotero/storage/*/*.pdf`
listing is the corpus the adoption document was measured on.

## Where results go

`output/pdf-benchmark/<run>/`, which is gitignored:

- `report.json` — every per-file record, including absolute paths
- `files.csv` — the same, one row per file per engine, for sorting by hand
- `summary.md` — aggregates only

**Results are never committed.** They are measurements of a personal Zotero
library: real book titles and real home paths. `summary.md` is written without
either, so it is the one artifact of a run that can be quoted in an issue or a
pull request.

## What is measured, exactly

Per file, per engine, for classification and extraction separately: wall clock,
status (`ok` / `empty` / `error` / `skipped`), and the engine's own verdict —
our `direct_text` / `needs_ocr` / `dirty_text_layer`, its
`text_based` / `mixed` / `scanned` / `image_based`.

On the produced Markdown:

| Metric | Definition |
|---|---|
| Real headings | ATX heading lines that are not page scaffolding |
| `## Page N` scaffolding headings | Heading lines matching `#{1,6} Page <digits>`, any level, case-insensitive |
| Table rows | Lines delimited by `\|` that are not the `\|---\|` delimiter row |
| Links | Markdown inline links plus autolinks; images do not count |
| Code blocks | Fenced blocks |
| Text volume | Non-space characters in the whole document |

Headings, tables and links are counted outside fenced code blocks, so an
engine that recovers a code listing is not charged for its contents.

Two definitions carry the weight of the whole exercise:

**Real headings and scaffolding headings are separate columns.** The shipped
route emits `## Page 1`, `## Page 2`, … once per page. Merged into a single
heading count, a converter that finds nothing scores identically to one that
finds every chapter — and the 629-page book that becomes 629 TOC entries named
"Page N" looks like a success.

**Text volume is reported next to the structure counts** so that "structure
improved" cannot hide "text was lost". An engine that returns beautiful
Markdown for half the book has to show it here.

## What the PyMuPDF column actually runs

The production functions, imported from
`packages/ocr/scripts/zotero_llm_worker.py` rather than reimplemented:
`pdf_page_count`, `sample_text_layer` (with `get_config()`, so the live
thresholds) for classification, and `extract_text_pages` for extraction.

The only thing the harness renders itself is the page scaffolding, because
`render_markdown` also emits front matter and a `# {title}` line that come from
the Zotero item and not from the PDF — crediting an engine for metadata it was
handed would make the comparison meaningless. `test_page_body_matches_the_worker_renderer`
asserts the benchmark's page body stays byte-identical to the worker's.

## Why real books and not fixtures

Everything interesting on this corpus is invisible to a generated fixture:
CNKI downloads whose file trailer no strict parser accepts, CID fonts that
decode to `!"#$%&'()*+,-./0`, image scans carrying a good text layer, and
Zotero attachments that are JSON stored under a `.pdf` name. A harness built on
hand-written PDFs would be green through all of it. See
`docs/planning/pdf-inspector-adoption.md` for what those cases did to the
candidate engine.

## Tests

```bash
uv run --package ocr pytest tools/pdf_benchmark
```

Stdlib-only and touching no PDF: they pin the metric definitions and the
agreement with the production renderer. CI runs them in the repository-suites
step.
