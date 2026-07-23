# Book OCR Conversion Agent Instructions

This folder is a book OCR and document-conversion pipeline. Load and follow `SKILL.md` before changing code, running OCR, uploading Markdown, generating HTML/EPUB deliverables, renaming Zotero attachments, or cleaning PDF queues.

Start by reading:

1. `SKILL.md`
2. `README.md`
3. `CONTEXT.md`
4. `docs/windows.md` when working on Legion/Windows

Key constraints:

- This is folder-scoped. Do not treat it as a global skill unless the user asks.
- Use `/opt/homebrew/bin/python3.11` for repo scripts unless a script explicitly provides another isolated runtime.
- On Windows/Legion, use `scripts/run_windows.ps1`; it creates `.venv`, preferring uv-managed Python 3.11 and falling back to `python -m venv` when uv cannot use its managed interpreter.
- In this repository, `PaddleOCR` means the remote Baidu AI Studio async jobs API, currently `PaddleOCR-VL-1.6`.
- Do not install or use local PaddlePaddle, local PaddleOCR inference, or `ocrmypdf-paddleocr` for this pipeline.
- Do not echo `.env`, API keys, tokens, or Zotero credentials.
- Do not delete source PDF attachments unless the relevant cleanup script has verified the Markdown output and Zotero attachment key.
- Runtime state lives under `$HOME/Zotero/OCR_OUTPUT/.state/`; generated files there are not source files.
- User-facing deliverables such as EPUB, standalone HTML, export indexes, and cleaned Markdown should be written under this repository's `output/` directory.
- This project intentionally excludes the previous full-book translation workflow.
