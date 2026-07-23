# Local Reading Agent Instructions

本仓库的默认任务是本地书籍/论文阅读与翻译，不是公版书发布。

## Default Workflow

- Use `skills/local-book-reading-pipeline/SKILL.md` for local EPUB/PDF/book/paper work.
- Treat `.agents/skills/` as the active project skill whitelist; `skills/` contains the source skill folders.
- Create new local projects with `tools/create_local_book_project.py`.
- Put real source files and generated work under `books/local/{target}/{number}_{title_author}/`.
- Keep final reading outputs under `output/reading/`.
- Keep book-specific source text, translations, QA, and EPUBs out of Git.

## Do Not Start The Upstream Rights Workflow

Do not run public-domain search, rights checks, `metadata/rights_checklist.md`, private-use declarations, `output/release/`, or `output/private_artifacts/` unless the user explicitly asks to work on the upstream public-domain BiblioSmith workflow.

The upstream files under `template/epub_pipeline/` remain for reference and possible reuse. They are not the source of truth for this local-reading fork.

## Local Project Contract

Each local book project should contain:

```text
source/original.*
source/source.md
chapters/src/
chapters/translated/
chapters/final/
glossary/terms.csv
metadata/source_manifest.json
metadata/style_profile.md
qa/
output/reading/
```

`metadata/source_manifest.json` records local-file evidence only: file name, SHA-256, format, source language, target language, extraction status, and notes. It must not contain public-domain conclusions or copyright legal analysis.

## Quality Bar

- Preserve source structure and footnotes where possible.
- Do not summarize or compress unless the user asks for digest mode.
- For translation, keep source-to-target traceability at chapter or paragraph-block level.
- Use `skills/expert-translation-quality/SKILL.md` when fidelity, terminology, prose quality, or context-dependent word choice matters.
- Use `skills/translation-quality-defect-families/SKILL.md` only for recurring translation-quality lessons.
- Use `skills/print-compatible-book-layout/SKILL.md` when building EPUB/HTML layout.

## Upstream Boundary

Keep upstream license files. Do not copy upstream sample books, release EPUBs, covers, or translations into local skill packages.

## Agent skills

### Issue tracker

Issues are tracked in GitHub Issues; external PRs are not a triage request surface. See `docs/agents/issue-tracker.md`.

### Triage labels

Use the canonical labels: `needs-triage`, `needs-info`, `ready-for-agent`, `ready-for-human`, `wontfix`. See `docs/agents/triage-labels.md`.

### Domain docs

This repo uses a single-context layout: root `CONTEXT.md` plus `docs/adr/`. See `docs/agents/domain.md`.
