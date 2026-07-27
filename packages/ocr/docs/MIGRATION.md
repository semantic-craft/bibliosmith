# Migration Notes

Created on 2026-06-30 from `D:\Projects\book-translations-win`.

## Included

- `编程书/`: 12 local book PDFs.
- `output/`: existing OCR/conversion deliverables and intermediate evidence, excluding the prior migration inventory report.
- `scripts/`: OCR, Zotero, inventory, cleanup, filename normalization, PaddleOCR, and MinerU scripts.
- `mineru.py`, `paddle.py`, `requirements-win.txt`, the then-package-local `.env.example`, `.firecrawl/`, `CONTEXT.md`, `.gitignore`.
- `docs/windows.md` and `docs/adr/`.
- `docs/archive/search-log.md`: research/search notes preserved as migrated project material.
- `tmp/`: small smoke/debug samples.

## Excluded

- `.env`, `.venv`, `.git`, `__pycache__`, `.DS_Store`.
- Project-local translation skills and Claude skill links.
- `book-translation-grid/`.
- `book-translation-grid-de/`.
- `docs/translation/`.
- `prompts/`.
- `scripts/translate_markdown.py`.
- `scripts/zotero-import-cn-md.py`.

## Secrets

The original `.env` was not available by the time secrets were approved for migration: the old project contents had already been cleared, and no matching OCR `.env` backup or process/user/machine environment variables were found. Issue #70 later retired the package-local template; create `D:\Projects\bibliosmith\.env` from the monorepo-root `.env.example` before running OCR jobs that need PaddleOCR, MinerU, or Zotero credentials.

## Data Totals At Migration

- `编程书/`: 12 files, about 988.75 MB.
- `output/`: 4691 copied files, about 1.16 GB.
- `scripts/`: 24 copied files.

The original project contents were cleared after migration. Its empty root directory may remain temporarily if a running process still holds a handle to `D:\Projects\book-translations-win`.
