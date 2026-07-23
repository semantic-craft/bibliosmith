# Windows Setup

This package runs from the private `semantic-craft/books-translation` monorepo:

```powershell
D:\Projects\books-translation\packages\ocr
```

It is focused on book OCR, Markdown extraction, standalone HTML generation, Zotero Markdown child attachments, PDF attachment naming, and conservative cleanup queues.

## Windows worker checkout

Authenticate to the private repository with an SSH deploy key or a GitHub PAT stored in Windows Credential Manager/Git Credential Manager. Never put repository credentials in `.env` or tracked files. Then use a sparse checkout so Windows worker does not download `books/` data or local OCR inputs:

```powershell
git clone --filter=blob:none --no-checkout git@github.com:semantic-craft/books-translation.git D:\Projects\books-translation
git -C D:\Projects\books-translation sparse-checkout init --no-cone
git -C D:\Projects\books-translation sparse-checkout set /packages/ocr/ /pyproject.toml /uv.lock /.env.example /.gitignore
git -C D:\Projects\books-translation checkout main
```

Non-cone mode is intentional: it materializes only the OCR package and the exact
root files required by the workspace, credential template, and ignore rules. The
root `pyproject.toml` and `uv.lock` provide the shared workspace environment;
`.env.example` lets the runner create an ignored root `.env`; `.gitignore` keeps
that file out of Git. The existing Windows runner remains available with its
package-local `.venv` for compatibility.

When running the same command from Git Bash instead of PowerShell, disable MSYS
path conversion for the sparse patterns and use the Windows form for `-C`:

```bash
MSYS_NO_PATHCONV=1 git -C D:/Projects/books-translation sparse-checkout set /packages/ocr/ /pyproject.toml /uv.lock /.env.example /.gitignore
```

## Runtime

Use the Windows runner:

```powershell
cd D:\Projects\books-translation\packages\ocr
.\scripts\run_windows.ps1 -Install worker --dry-run --limit 5
```

Or verify the workspace member from the monorepo root without contacting Zotero or an OCR service:

```powershell
cd D:\Projects\books-translation
uv run --package ocr python packages/ocr/scripts/zotero_llm_worker.py --help
```

The runner creates `.venv`, preferring uv-managed Python 3.11 and falling back to `python -m venv` when uv cannot use its managed interpreter. It installs `requirements-win.txt`, creates the monorepo-root `.env` from the root `.env.example` when needed, sets `HOME` to `USERPROFILE`, and dispatches to the project scripts.

## Verified Windows worker cutover

Verified on Windows worker on 2026-07-17:

- Existing SSH credentials authenticated to the private repository with
  `git ls-remote origin HEAD`; no deploy key or PAT was copied into the checkout.
- The sparse checkout materialized 45 tracked files, occupied approximately
  1.7 MB before the runtime venv, and did not create `books/`.
- `scripts/smoke_windows.ps1` passed: worker/paddle help, Python dependency
  imports, root environment-file discovery, and Zotero storage discovery all
  succeeded. uv could not inspect its managed Python 3.11 because Windows marked
  its mount point untrusted, so the runner used its documented system-Python
  fallback.
- Root `.env` and `packages/ocr/.venv/` were confirmed ignored and untracked.

## Cutover and rollback

Before changing any Windows Scheduled Task, shortcut, or wrapper, record its
current working directory and command and keep the previous checkout intact. Point
the runtime to `D:\Projects\books-translation\packages\ocr` only after the smoke
check passes.

To roll back:

1. Stop the active OCR process or disable the affected Scheduled Task.
2. Restore the recorded previous command and working directory. If this was a
   fresh Windows worker install with no previous checkout, leave the task disabled.
3. Keep `%USERPROFILE%\Zotero\OCR_OUTPUT\.state\zotero_llm.sqlite3` unchanged;
   both paths use the same external state database, so no state migration or
   database copy is part of rollback.
4. Keep the new sparse checkout for diagnosis until the old path has completed a
   dry run. Removing it is a separate, explicit cleanup action.

Common tasks:

```powershell
# Zotero route inspection; no OCR upload work.
.\scripts\run_windows.ps1 worker --dry-run --limit 5

# One Zotero attachment, local Markdown only.
.\scripts\run_windows.ps1 worker --attachment-key RJTL3RRZ --pages 1-1 --no-upload --force-text

# Standalone PDF folder to HTML via remote PaddleOCR-VL.
.\scripts\run_windows.ps1 html --input-dir .\编程书 --output-dir .\output\html_books --limit-books 1

# One local PDF or URL through the PaddleOCR-VL 1.6 API client.
.\scripts\run_windows.ps1 paddleocr .\编程书\some-book.pdf -o .\output\paddle\some-book

# MinerU queue/report.
.\scripts\run_windows.ps1 mineru --dry-run --limit 5

# Zotero PDF attachment-name audit.
.\scripts\run_windows.ps1 normalize --dry-run --limit 20

# 10-50MB PDF inventory report.
.\scripts\run_windows.ps1 inventory
```

For a local smoke check that does not require Zotero to be running:

```powershell
.\scripts\smoke_windows.ps1
```

Add `-CheckZotero` when Zotero is open and its local API is enabled:

```powershell
.\scripts\smoke_windows.ps1 -CheckZotero
```

## PaddleOCR Boundary

In this repository, `PaddleOCR` means Baidu AI Studio remote async OCR jobs, currently `PaddleOCR-VL-1.6`.

Do not install local `paddlepaddle`, local `paddleocr`, or `ocrmypdf-paddleocr` for this pipeline. The Windows dependency set only needs HTTP/PDF/Markdown helpers:

```powershell
.\.venv\Scripts\python.exe -m pip install -r requirements-win.txt
```

Required monorepo-root `.env` keys for scanned/non-extractable PDFs:

```text
BAIDU_PADDLEOCR_TOKEN=
ZOTERO_API_KEY=
ZOTERO_LIBRARY_ID=
```

Do not print these values in logs or terminal output.

OCR entrypoints read only `D:\Projects\books-translation\.env`; already exported environment variables take precedence. Package-local `.env` files are not read.

The project-local single-file CLI is:

```powershell
D:\Projects\books-translation\packages\ocr\paddle.py
```

Examples:

```powershell
python .\paddle.py --self-test
python .\paddle.py .\编程书\some-book.pdf -o .\output\paddle\sample
```

WSL can call the same client through a wrapper if one is installed:

```bash
paddle --self-test
paddle /mnt/d/Projects/books-translation/packages/ocr/编程书/some-book.pdf -o /mnt/d/Projects/books-translation/packages/ocr/output/paddle/some-book
```

## MinerU

Current MinerU docs resolve to `/opendatalab/mineru`. The documented CLI supports:

```powershell
mineru -p <input_path> -o <output_path> -m ocr -l ch
```

Optional install inside the venv:

```powershell
uv pip install --python .\.venv\Scripts\python.exe -U "mineru[all]"
```

If you need the native MinerU CLI, force its explicit executable path through the monorepo-root `.env`:

```text
MINERU_COMMAND=D:\Projects\books-translation\packages\ocr\.venv\Scripts\mineru.exe
MINERU_METHOD=ocr
MINERU_LANG=ch
MINERU_BACKEND=
```

If native Windows MinerU dependencies become fragile, use WSL2/Docker for MinerU and keep the Baidu PaddleOCR route remote-only.

## MinerU API CLI

For scanned books and academic papers, prefer the MinerU Precision Extract API with:

- `model_version=vlm`
- `is_ocr=true`
- table and formula recognition enabled
- local upload batches capped at 50 files per request
- each source file within the documented Precision API limits: `<=200MB` and `<=200 pages`
- URL inputs selected automatically: one URL uses `/api/v4/extract/task`, multiple URLs use `/api/v4/extract/task/batch`

Project-local single-file CLI:

```powershell
D:\Projects\books-translation\packages\ocr\mineru.py
```

Examples:

```powershell
python .\mineru.py --self-test
python .\mineru.py .\编程书 -o .\output\mineru\books
python .\mineru.py .\paper.pdf --page-ranges 1-200 -o .\output\mineru\paper
python .\mineru.py https://cdn-mineru.openxlab.org.cn/demo/example.pdf -o .\output\mineru\single-url
```

Completed tasks download `full_zip_url` archives and extract them under the output directory. The main Markdown file is `full.md`; MinerU JSON artifacts are kept next to it.
