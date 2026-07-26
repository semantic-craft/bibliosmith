# Local Reading Agent Instructions

本仓库只处理用户已经拥有的本地书籍、论文和文稿。

## Default Workflow

- Use `skills/local-book-reading-pipeline/SKILL.md` for local EPUB/PDF/book/paper work.
- Treat `.agents/skills/` as the active project skill whitelist; `skills/` contains the source skill folders.
- Create new local projects with `tools/create_local_book_project.py`.
- Put real source files and generated work under `books/local/{target}/{number}_{title_author}/`.
- Keep final reading outputs under `output/reading/`.
- Keep book-specific source text, translations, QA, and EPUBs out of Git.

## Repository Scope

- Do not search for book-length source text or bypass DRM/access controls.
- Do not make publication or licensing decisions.
- Do not publish or commit real source text, translations, QA, or generated EPUBs unless the user explicitly requests a scoped export.
- Keep repository work on the local-reading pipeline and its supporting launcher, translation, OCR, Zotero, digest, and layout components.

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

`metadata/source_manifest.json` records local-file identity only: file name, SHA-256, format, source language, target language, extraction status, and notes.

## Quality Bar

- Preserve source structure and footnotes where possible.
- Do not summarize or compress unless the user asks for digest mode.
- For translation, keep source-to-target traceability at chapter or paragraph-block level.
- Use `skills/expert-translation-quality/SKILL.md` when fidelity, terminology, prose quality, or context-dependent word choice matters.
- Use `skills/translation-quality-defect-families/SKILL.md` only for recurring translation-quality lessons.
- Use `skills/print-compatible-book-layout/SKILL.md` when building EPUB/HTML layout.

## License Boundary

Keep inherited license files. Do not copy sample books, generated EPUBs, covers, or translations into local skill packages.

## Agent skills

### Issue tracker

Issues are tracked in GitHub Issues; external PRs are not a triage request surface.

New issues and all live work go in `semantic-craft/bibliosmith`. Pass `--repo` explicitly instead of inferring it from `git remote -v`. `semantic-craft/bibliosmith-private-archive` is a read-only historical reference — GitHub has it archived, so creating issues, commenting, and closing are all rejected there; read it with `gh issue view <N> --repo semantic-craft/bibliosmith-private-archive`. See `docs/agents/issue-tracker.md`.

### Triage labels

Use the labels that exist: `ready-for-agent`, `needs-decision`, `blocked`, `wontfix`, plus the kind labels. An untriaged issue carries no workflow label. See `docs/agents/triage-labels.md`.

### Domain docs

This repo uses a single-context layout: root `CONTEXT.md` plus `docs/adr/`. See `docs/agents/domain.md`. Migrated planning documents (the PRD and the Wayfinder map) live in `docs/planning/`.
