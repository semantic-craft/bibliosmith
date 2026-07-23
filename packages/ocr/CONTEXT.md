# Book OCR Conversion

This repository converts Zotero PDF attachments and standalone book PDFs into Markdown, HTML, EPUB-facing bundles, and other LLM-readable deliverables. It also manages OCR routing, Zotero attachment upload, PDF filename normalization, and conservative cleanup queues.

## Language

**Source PDF**:
The original Zotero PDF child attachment being inspected, OCR'd, renamed, or deleted after successful Markdown creation.
_Avoid_: input file, original file, raw file

**Parent item**:
The Zotero book or article item that owns the Source PDF and will also own the Markdown child attachment.
_Avoid_: record, entry, Zotero row

**Markdown child attachment**:
The generated `.md` file uploaded back to Zotero under the same Parent item as the Source PDF.
_Avoid_: output file, result file

**PDF text route**:
The path for selectable or born-digital PDFs where embedded text is extracted directly without OCR.
_Avoid_: normal route, fast path

**PaddleOCR route**:
The remote Baidu AI Studio async jobs API path for scanned or non-extractable PDFs using `PaddleOCR-VL-1.6`.
_Avoid_: OCR fallback, Baidu route, local PaddlePaddle, local PaddleOCR

**MinerU API route**:
The MinerU Precision Extract API path for scanned/image-heavy books and academic papers, usually through `mineru.py` or a repository queue script.
_Avoid_: local MinerU assumption, translation route

**Dirty text layer**:
An embedded PDF text layer that is technically extractable but degraded, often visible as fullwidth ASCII or private-use glyphs in Chinese-heavy samples.
_Avoid_: bad OCR, garbled PDF

**MinerU review**:
The conservative handling for Dirty text layers where existing MinerU output may be kept or regenerated instead of forcing PaddleOCR.
_Avoid_: exception route, manual fallback

**State DB**:
The SQLite database at `$HOME/Zotero/OCR_OUTPUT/.state/zotero_llm.sqlite3` that tracks document and chunk processing status.
_Avoid_: cache, log database

**Staging artifact**:
Local Markdown, JSONL, chunk PDF, or OCR result files under `$HOME/Zotero/OCR_OUTPUT/.state/`.
_Avoid_: final output, deliverable

**Standalone book PDF**:
A local PDF under a project book directory, such as `编程书/`, processed for Markdown, HTML, EPUB-facing output, or OCR evidence without necessarily touching Zotero.
_Avoid_: translation source, manuscript

**Deliverable bundle**:
A user-facing export under `output/`, such as standalone HTML, EPUB, cleaned Markdown, export indexes, assets, or previews.
_Avoid_: runtime state, temporary chunk output

**Windows runner**:
`scripts/run_windows.ps1`, the Legion task dispatcher that creates `.venv`, installs `requirements-win.txt`, and calls the same repository scripts.
_Avoid_: separate product, global tool

**Cleanup job**:
A size- or collection-scoped queue that OCRs PDFs and may delete source PDFs only after Markdown upload is verified.
_Avoid_: batch script, deletion script
