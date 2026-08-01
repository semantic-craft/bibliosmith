# OCR package

`packages/ocr` in the `semantic-craft/bibliosmith` monorepo. This package provides book OCR, PDF-to-Markdown extraction, standalone HTML/EPUB deliverables, Zotero Markdown child attachments, PDF filename normalization, and conservative cleanup reports.

Agents should start with [SKILL.md](SKILL.md), [AGENTS.md](AGENTS.md), and [CONTEXT.md](CONTEXT.md). On the Windows worker, also read [docs/windows.md](docs/windows.md). This repository is intentionally folder-scoped and intentionally excludes the previous full-book translation workflow.

Known monorepo/package roots:

- macOS: `$HOME/Projects/bibliosmith` / `$HOME/Projects/bibliosmith/packages/ocr`
- Windows worker: `D:\Projects\bibliosmith` / `D:\Projects\bibliosmith\packages\ocr`

## Routing

- Selectable/born-digital PDFs: extract embedded PDF text directly and save Markdown.
- Selectable/born-digital books: extract embedded PDF text directly too.
- PDFs with an embedded but degraded Chinese text layer are blocked as `blocked_dirty_text_layer` instead of being forced through PaddleOCR. Typical signals include many fullwidth ASCII letters/digits or private-use glyphs in a Chinese-heavy sample. Keep the existing MinerU output or rerun a MinerU/chunked parser for these cases.
- Scanned/non-extractable PDFs, including large scanned books: use Baidu AI Studio `PaddleOCR-VL-1.6` and preserve its structured Markdown output.
- Scanned PDFs that need Markdown output: use the Baidu AI Studio OCR jobs API. The pipeline does not use local PaddlePaddle inference, local PaddleOCR inference, or `ocrmypdf-paddleocr`.
- Finished Markdown is uploaded to Zotero as a child Markdown attachment under the same parent book/article item as the source PDF.
- Uploaded attachments receive no Zotero tags by default. OCR provenance is stored in the attachment note plus the State DB/sidecar. A tag is written only when its exact name is passed with repeatable `--zotero-tag` options.
- Markdown attachment filenames follow `作者_年份_题名.md`, for example `申卫星_2025_个人信息的司法保护：理论学说与案例解析.md`.
- Source PDF child attachments are normalized too: the PDF title, Zotero filename, and imported storage filename should follow `作者_年份_题名.pdf`, matching the Markdown basename. Do not leave processed PDFs displayed as `PDF`, numeric download names, SS-number names, or download-site tail names.
- Local Markdown/JSONL files are internal staging artifacts under `$HOME/Zotero/OCR_OUTPUT/.state/staging/`; they are not the user-facing destination.
- User-facing export bundles, including EPUB, standalone HTML, export indexes, and cleaned Markdown, should be written under this repository's `output/` directory.
- Standalone local book PDFs can be converted to HTML with `scripts/pdf_to_html_paddleocr.py`.
- MinerU Precision Extract API work uses `mineru.py` or the existing MinerU queue scripts; it is API-based unless `docs/windows.md` explicitly says otherwise for a local native CLI experiment.

## Files

- `SKILL.md`: folder-scoped agent skill for this repository.
- `AGENTS.md`: short entrypoint telling agents to load `SKILL.md`.
- `CONTEXT.md`: project vocabulary for Zotero/OCR concepts.
- `docs/windows.md`: Windows worker, MinerU, and PaddleOCR notes.
- `docs/MIGRATION.md`: Windows worker split from the previous translation workspace.
- `paddle.py`: single-file Baidu AI Studio `PaddleOCR-VL-1.6` client.
- `mineru.py`: single-file MinerU Precision Extract API client.
- `requirements-win.txt`: Windows runner dependency set.
- `scripts/zotero_llm_worker.py`: main worker.
- `scripts/run_windows.ps1`: Windows task dispatcher.
- `scripts/smoke_windows.ps1`: Windows smoke check.
- `scripts/pdf_to_html_paddleocr.py`: standalone PDF folder to HTML converter.
- `scripts/paddleocr_vl_cli.py`: script-local PaddleOCR-VL client used by the Windows runner.
- `scripts/normalize_pdf_attachment_names.py`: audits/fixes PDF attachment display names and storage filenames.
- `scripts/run_daily.sh`: launchd entrypoint, capped to 55 minutes.
- `launchd/com.example.zotero-llm.plist`: daily 09:00 LaunchAgent, usually kept local/ignored.
- `scripts/install_launch_agent.sh`: installs/enables the daily LaunchAgent.
- `scripts/test_launch_agent.sh`: temporary RunAtLoad dry-run launchd test.
- Repository-root `.env`: local secrets/config, chmod 600 and never committed.
- Repository-root [`.env.example`](../../.env.example): committed registry for all active keys.

## Setup

macOS:

```bash
cd $HOME/Projects/bibliosmith
uv sync --package ocr
cp .env.example .env
chmod 600 .env
```

Windows worker:

```powershell
cd D:\Projects\bibliosmith\packages\ocr
.\scripts\run_windows.ps1 -Install worker --dry-run --limit 5
```

OCR entrypoints load only the monorepo-root `.env`. Already exported environment variables take precedence, and package-local `.env` files are not read.

Fill:

```bash
BAIDU_PADDLEOCR_TOKEN=
MINERU_API_TOKEN=
ZOTERO_API_KEY=
ZOTERO_LIBRARY_ID=
```

If `BAIDU_PADDLEOCR_TOKEN` is blank, the worker still processes selectable/born-digital PDFs and records scanned PDFs as blocked until the token is added. If `MINERU_API_TOKEN` is blank, MinerU API commands fail their self-test but the Zotero/Paddle route still works.

## Manual Checks

The direct-script commands below assume the current directory is `packages/ocr`. From the monorepo root, use the shared workspace instead:

```bash
uv run --package ocr python packages/ocr/scripts/zotero_llm_worker.py --help
```

Dry-run route check:

```bash
/opt/homebrew/bin/python3.11 scripts/zotero_llm_worker.py --dry-run --limit 5
```

After merge, a user with the real repository-root `.env` should perform this HITL credential check. It may contact Zotero, so agents must not run it without explicit credential access:

```bash
cd $HOME/Projects/bibliosmith
uv run --package ocr python packages/ocr/scripts/zotero_llm_worker.py --dry-run --limit 5
```

Generate local Markdown for one PDF without uploading:

```bash
/opt/homebrew/bin/python3.11 scripts/zotero_llm_worker.py --attachment-key RJTL3RRZ --pages 1-1 --no-upload --force-text
```

Upload the latest generated Markdown for one PDF:

```bash
/opt/homebrew/bin/python3.11 scripts/zotero_llm_worker.py --upload-test --attachment-key RJTL3RRZ
```

Explicitly add a user-chosen tag when needed:

```bash
/opt/homebrew/bin/python3.11 scripts/zotero_llm_worker.py --attachment-key RJTL3RRZ --zotero-tag user-chosen-tag
```

Audit and fix obvious dirty PDF attachment names:

```bash
./scripts/normalize_pdf_attachment_names.py --dry-run
./scripts/normalize_pdf_attachment_names.py
```

Standalone book folder to HTML:

```bash
/opt/homebrew/bin/python3.11 scripts/pdf_to_html_paddleocr.py --input-dir 编程书 --output-dir output/html_books --limit-books 1
```

Single PDF through PaddleOCR:

```bash
/opt/homebrew/bin/python3.11 scripts/paddleocr_vl_cli.py 编程书/some-book.pdf -o output/paddle/some-book
```

MinerU Precision Extract API self-test:

```bash
/opt/homebrew/bin/python3.11 mineru.py --self-test
```

Windows smoke:

```powershell
.\scripts\smoke_windows.ps1
```

## launchd Cutover

Keep the LaunchAgent plist machine-local. Set its working directory to the monorepo root and replace its program arguments with the absolute `uv` path followed by:

```text
run
--package
ocr
python
packages/ocr/scripts/zotero_llm_worker.py
```

Equivalent shell form from the monorepo root:

```bash
uv run --package ocr python packages/ocr/scripts/zotero_llm_worker.py
```

`uv run --package` selects the workspace member but does not change the working directory, so the shorter `python scripts/zotero_llm_worker.py` path is valid only when the working directory is `packages/ocr`.

Keep `$HOME/Zotero/OCR_OUTPUT/.state/zotero_llm.sqlite3` unchanged. The worker discovers and loads the monorepo-root `.env` itself, so launchd needs only the repository root as its working directory; already exported launchd variables still take precedence.

Install daily 09:00 schedule:

```bash
./scripts/install_launch_agent.sh
```

Test launchd without processing data:

```bash
./scripts/test_launch_agent.sh
```

## State and Logs

- State DB: `$HOME/Zotero/OCR_OUTPUT/.state/zotero_llm.sqlite3`
- Staged Markdown/JSONL: `$HOME/Zotero/OCR_OUTPUT/.state/staging/`
- Chunk PDFs/JSONL: `$HOME/Zotero/OCR_OUTPUT/.state/chunks/`
- Logs: `$HOME/Zotero/OCR_OUTPUT/.state/logs/`
- User-facing exports: `./output/`

The state key is Zotero PDF attachment key plus source PDF MD5. If a PDF changes, it is processed again; otherwise fully completed items are skipped. Partial page smoke tests are recorded as partial and do not block a later full-book run.

## API Notes

- Baidu PaddleOCR async API uses `POST /api/v2/ocr/jobs` with `Authorization: bearer <token>`.
- `PaddleOCR` means this remote API in this repository. If a request mentions installing dependencies, only `requests` is needed for API calls; do not install local PaddlePaddle.
- The local run budget is set to 10000 OCR pages. OCR chunks default to 12 pages because Baidu frequently returned server-side 500 errors on 100-page batches and still occasionally failed on 25-page batches.
- The OCR model is `PaddleOCR-VL-1.6`, not `PP-OCRv5`; the worker writes `layoutParsingResults[*].markdown.text` directly so the Markdown keeps heading/paragraph structure.
- The dirty-text-layer guard is controlled by `DIRTY_TEXT_LAYER_GUARD=1` and threshold env vars `DIRTY_TEXT_FULLWIDTH_ALNUM_RATIO`, `DIRTY_TEXT_PRIVATE_USE_RATIO`, and `DIRTY_TEXT_MIN_SAMPLE_CHARS`.
- MinerU-to-Paddle replacement respects the dirty-text-layer guard and will keep the MinerU attachment instead of deleting it when the source PDF is classified as needing MinerU/quality review.
- MinerU work uses only the authenticated v4 Precision Extract endpoints: one URL uses `/api/v4/extract/task`; local files use `/api/v4/file-urls/batch`; multiple URLs use `/api/v4/extract/task/batch`; all batch results use `/api/v4/extract-results/batch/{batch_id}`. The unauthenticated Agent API is not used.
- `mineru.py` defaults to automatic Precision model routing: non-HTML sources use `vlm`, while `.html` sources use `MinerU-HTML`. An explicit `pipeline`, `vlm`, or `MinerU-HTML` selection is validated against the source type.
- Local uploads are grouped by required model and capped at 50 files per signed-upload request. Upload PUT requests intentionally omit `Content-Type`, as required by MinerU.
- In the desktop launcher, a local PDF folder whose runnable items are all forced to MinerU runs through this same signed batch client with `vlm`; a mixed MinerU/Paddle folder is rejected explicitly instead of silently ignoring per-file route choices.
- PDFs over 200 pages, or over 200 MB when their page count can be read, are physically split into parts of at most 200 pages and 200 MB before upload. A single page larger than 200 MB fails preflight instead of bypassing the API limit.
- Downloaded part archives and JSON stay under `output/mineru/<source-data-id>/parts/`; the page-ordered reading result is `full.md`, with `mineru_manifest.json` recording the original page order. A failed or missing item makes the CLI run fail rather than reporting partial success.
- MinerU credentials come from `MINERU_API_TOKEN` or `MINERU_TOKEN`; OCR, table recognition, and formula recognition are enabled for `pipeline`/`vlm` by default.
- Zotero file upload follows the Web API flow: create imported-file attachment item, request upload authorization, upload bytes, register upload.
