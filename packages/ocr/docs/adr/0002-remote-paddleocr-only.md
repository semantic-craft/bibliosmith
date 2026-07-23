# Remote PaddleOCR only

This repository treats `PaddleOCR` as the remote Baidu AI Studio async jobs API, currently `PaddleOCR-VL-1.6` at `/api/v2/ocr/jobs`.

Local PaddlePaddle inference is out of scope. Do not install or use local `paddlepaddle`, local `paddleocr`, or `ocrmypdf-paddleocr` for this book OCR and Zotero PDF-to-Markdown pipeline. The only Python HTTP dependency needed for the remote API path is `requests`.

Reason: the project goal is Zotero PDF-to-Markdown conversion for LLM reading. Searchable-PDF generation and local OCR runtimes create a second product path, add native wheel fragility, and confuse future routing decisions. Scanned PDFs should be chunked and sent through `scripts/zotero_llm_worker.py`, which downloads the remote OCR JSONL and writes Markdown from `layoutParsingResults[*].markdown.text`.
