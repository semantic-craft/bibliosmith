---
name: local-book-reading-pipeline
description: Use when processing user-provided EPUB, PDF, HTML, Markdown, TXT, or academic paper files into clean local reading projects, translated manuscripts, HTML, EPUB, bilingual EPUB, digest-style reading editions, or QA-checked local outputs.
---

# Local Book Reading Pipeline

Use this skill for books and papers already present on the user's computer.

## Hard Boundary

- Do not remove DRM or bypass access controls.
- Do not search for book-length source text online.
- Do not publish or commit real source text, translations, QA, or generated EPUBs unless the user explicitly asks.

## Create Or Find The Project

If no book project exists, create one from the repository root:

```bash
python3 tools/create_local_book_project.py "书名_作者" --source-file "/path/to/book.epub"
```

Projects live under:

```text
books/local/{target}/{number}_{title_author}/
```

The project source of truth is `metadata/source_manifest.json`. It records local-file evidence only, not legal conclusions.

## Workflow

1. Inspect `metadata/source_manifest.json` and `source/original.*`.
2. Extract readable text into `source/source.md`.
3. Split stable units into `chapters/src/`.
4. Build `glossary/terms.csv` and update `metadata/style_profile.md`.
5. Translate into `chapters/translated/` when requested.
6. Review and promote clean text to `chapters/final/`.
7. Build local outputs under `output/reading/`.
8. Run the smallest useful check: EPUBCheck for EPUB, spot-read samples for Markdown/HTML, and full chapter checks for translated chapters.

## Extraction Hints

- EPUB: prefer `pandoc`, `ebook-convert`, or an existing local EPUB parser if available.
- PDF with text: prefer `pdftotext -layout` or `pdfplumber`.
- Scanned PDF: route to OCR first; keep OCR notes in `qa/`.
- Academic papers: preserve headings, abstract, footnotes, figures, tables, bibliography, and page anchors when possible.

If extraction is messy, keep `source/source.md` honest and add cleanup notes in `qa/status.md`.

## Translation Rules

- Do not summarize, compress, merge chapters, or drop notes unless the user asks.
- Keep traceability between source units and translated units.
- Use `skills/expert-translation-quality/SKILL.md` for fidelity, terminology, expert prose, or context-dependent word choice.
- Use `skills/translation-quality-defect-families/SKILL.md` only when a recurring translation-quality issue is found.
- Use `skills/print-compatible-book-layout/SKILL.md` for EPUB/HTML layout choices.

## Done

A local reading project is done only when the requested output exists in `output/reading/` and the relevant check has passed or the remaining risks are explicitly recorded in `qa/status.md`.
