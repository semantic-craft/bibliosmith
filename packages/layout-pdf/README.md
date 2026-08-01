# layout-pdf

The layout-preserving half of the two book tracks (see
`docs/planning/island-ui-minimal-redesign.md`). The reflow track turns a PDF
into Markdown, translates it chapter by chapter behind two approval gates and
rebuilds an EPUB. This track hands the PDF to
[BabelDOC](https://github.com/funstory-ai/babeldoc) (AGPL-3.0, same licence as
this repository), which translates in place and writes a side-by-side bilingual
PDF that still looks like the original. One pass, no gates.

Text PDFs only. Scanned books have no text layer for BabelDOC to work from and
stay on the OCR route; `book_pipeline.rs` only offers this track for a
`direct_text` route.

## Why `babeldoc` is behind an extra

BabelDOC pulls onnxruntime, opencv, scipy, scikit-image and around eighty other
wheels. Declared under `[project.optional-dependencies]` it is resolved into
`uv.lock` but never installed by `uv sync --all-packages`, so the shared
workspace venv that every other suite runs in stays light. The launcher asks for
it explicitly at run time:

```bash
uv run --package layout-pdf --extra babeldoc layout-pdf --input book.pdf --output-dir out
```

The first such run installs the extra (from uv's cache after the first time) and
downloads BabelDOC's layout model and CJK fonts through `assets.warmup()`. A
later `uv sync` prunes the extra back out again; the next run reinstalls it from
cache. The tests are written against a stub translate step and need none of it.

## Contract with the Launcher

| | |
| --- | --- |
| Endpoint | `LAYOUT_PDF_BASE_URL`, `LAYOUT_PDF_API_KEY`, `LAYOUT_PDF_MODEL`, injected from the active model slot |
| Progress | `BIBLIOSMITH_PROGRESS_PATH` / `BIBLIOSMITH_PROGRESS_SCOPE`, `book-pipeline-progress-v1`, stage `extract`, unit `pages` |
| Warnings | `BOOK_PIPELINE_MARKER warning=<kind> count=<n>` on stdout, kinds mirrored by `LAYOUT_PDF_WARNING_KINDS` in `book_pipeline.rs` |
| Output | exactly one file, `<stem>.<lang-out>.bilingual.pdf`, in `--output-dir` |

That last row is load-bearing: the Launcher registers every PDF it finds under
the job output root as a deliverable, so BabelDOC's own output directory and
scratch files are kept in a temporary directory and only the finished PDF is
moved across.

## Known limitations (BabelDOC's, not ours)

- Pages larger than 1200×2000pt may come back untranslated. BabelDOC warns, and
  the warning is counted and forwarded as `warning=large_page`.
- Author and reference sections parse poorly; paragraphs there can end up
  merged. BabelDOC emits no runtime warning for this, so the Launcher states it
  as a fixed note on every layout-track run instead.
- BabelDOC is built for papers rather than books. Runtime scales with page
  count; the pipeline allows a long timeout for this stage.
