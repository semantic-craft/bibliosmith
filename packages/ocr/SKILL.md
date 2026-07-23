---
name: book-ocr-conversion
description: Folder-scoped instructions for this book OCR and conversion repository. Use when working inside this folder, processing local book PDFs, OCRing Zotero PDF attachments, generating Markdown/HTML/EPUB deliverables, using MinerU or Baidu PaddleOCR-VL, uploading Markdown child attachments, normalizing Zotero filenames, or cleaning PDF queues.
---

# Book OCR Conversion

This skill is scoped to this repository only. It teaches agents that this folder is a book OCR and document-conversion pipeline, not a translation project or general LLM chat app.

## Quick Start

1. Read `README.md` for project scope, commands, and routing policy.
2. Read `CONTEXT.md` for project vocabulary.
3. Read `docs/windows.md` before touching the Windows worker setup.
4. Check `git status --short --branch` if this directory is a Git repository.
5. Do not print `.env`, API keys, tokens, or Zotero credentials.
6. Use Homebrew Python 3.11 for macOS repo scripts:

```bash
/opt/homebrew/bin/python3.11 scripts/zotero_llm_worker.py --dry-run --limit 5
```

## Core Workflow

Use `scripts/zotero_llm_worker.py` as the shared Zotero engine.

- Selectable PDF: direct PDF text extraction to Markdown.
- Scanned or low-text PDF: remote Baidu AI Studio async OCR jobs API with `PaddleOCR-VL-1.6`.
- `PaddleOCR` in this repository never means local PaddlePaddle inference. Do not install `paddlepaddle`, `paddleocr`, or `ocrmypdf-paddleocr` to handle this pipeline.
- Degraded Chinese embedded text layer: block as `blocked_dirty_text_layer`; keep or regenerate with MinerU instead of forcing PaddleOCR.
- Finished Markdown: upload as a Zotero child Markdown attachment under the same parent item.
- Zotero tags are opt-in only. Never add workflow, provenance, subject, or status tags unless the user explicitly supplies each name with `--zotero-tag`.
- Store OCR provenance in the Markdown attachment note and the State DB/sidecar, not in Zotero tags.
- Source PDF names and Markdown attachment names should follow `作者_年份_题名.pdf` and `作者_年份_题名.md`.
- Standalone local PDFs can be converted to HTML with `scripts/pdf_to_html_paddleocr.py`.
- `paddle.py` and `scripts/paddleocr_vl_cli.py` are single-file Baidu PaddleOCR-VL clients.
- `mineru.py` is a single-file MinerU Precision Extract API client for scanned books and academic papers.

## Safety Rules

- Never delete a source PDF just because OCR started.
- Delete a source PDF only when the cleanup script has a completed PaddleOCR Markdown row, a local Markdown file, and a Zotero Markdown attachment key.
- Treat files under `$HOME/Zotero/OCR_OUTPUT/.state/` as runtime state, not user-facing deliverables.
- Put user-facing export bundles under this repository's `output/` directory.
- Keep `reports/`, `tmp/`, generated OCR chunks, and local LaunchAgent plists out of Git unless the user explicitly asks.
- If a script path accepts `$HOME` or `~`, verify it is expanded before using it in Python.

## Common Commands

macOS:

```bash
# Route inspection
/opt/homebrew/bin/python3.11 scripts/zotero_llm_worker.py --dry-run --limit 5

# One attachment, local Markdown only
/opt/homebrew/bin/python3.11 scripts/zotero_llm_worker.py --attachment-key RJTL3RRZ --pages 1-1 --no-upload --force-text

# Upload previously generated Markdown
/opt/homebrew/bin/python3.11 scripts/zotero_llm_worker.py --upload-test --attachment-key RJTL3RRZ

# Add a tag only when the user explicitly requests it
/opt/homebrew/bin/python3.11 scripts/zotero_llm_worker.py --attachment-key RJTL3RRZ --zotero-tag user-chosen-tag

# Audit Zotero PDF attachment names
./scripts/normalize_pdf_attachment_names.py --dry-run
```

Windows worker:

```powershell
.\scripts\run_windows.ps1 worker --dry-run --limit 5
.\scripts\run_windows.ps1 html --input-dir .\编程书 --output-dir .\output\html_books --limit-books 1
.\scripts\run_windows.ps1 paddleocr .\编程书\some-book.pdf -o .\output\paddle\some-book
.\scripts\run_windows.ps1 mineru --dry-run --limit 5
.\scripts\smoke_windows.ps1
```

## When Improving This Repo

Borrow the small-skill style: concise instructions, domain glossary in `CONTEXT.md`, and short ADRs in `docs/adr/` only for decisions future agents would otherwise misunderstand.
