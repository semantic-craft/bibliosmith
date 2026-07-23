# Output directory for deliverables

Runtime OCR state stays under `$HOME/Zotero/OCR_OUTPUT/.state/`, because the worker uses that location for the State DB, chunks, JSONL, logs, and staging Markdown.

User-facing deliverables should not be placed there. Export bundles such as standalone HTML, EPUB, cleaned Markdown, index pages, and previews belong under this repository's `output/` directory.

Reason: `output/` is the visible project delivery surface and is already ignored by Git. Keeping deliverables there avoids mixing final artifacts with Zotero OCR runtime state.
