# Local Reading Book Pipeline Context

This repository runs a private, local-first reading and translation pipeline. The
desktop launcher owns orchestration, but it does not own every component's data.

## Core terms

- **Book Pipeline job state**: the launcher-owned parent/child/stage state used to
  route, resume, approve, retry, and present work.
- **Component store**: data owned and written by one subsystem, such as OCR
  SQLite, a local book project, the translation engine checkpoint files, or the
  sqlite-vec index.
- **Evidence reference**: an identifier, contract version, SHA-256, and privacy
  category that lets job state prove what a component produced without embedding
  private source text.
- **Reconciliation**: validation of persisted evidence references before a stage
  is reused or resumed. Missing or mismatched evidence produces an explicit
  blocked/failed state; it never fabricates completion.
- **Terminal event**: a safe notification emitted for a terminal job outcome. It
  contains job metadata and aggregate progress only, never source titles, paths,
  logs, private text, or credentials.

## Boundaries

- `tools/bibliosmith-launcher` owns Book Pipeline orchestration and its public state.
- `packages/ocr` owns OCR worker data and SQLite.
- `books/local/...` owns each private local-reading project and its manifests,
  chapters, QA records, checkpoints, and reading output.
- `packages/zotero-cli` owns the item/chunk semantic index.
- GitHub Issues are the tracker. Local ADRs record decisions; they do not silently
  close or mutate tickets.

## Decisions

- [ADR 0001](docs/adr/0001-federated-book-pipeline-state.md): federated component
  stores with explicit evidence references and reconciliation.
- [ADR 0002](docs/adr/0002-progress-and-terminal-notifications.md): Tauri stage/unit
  progress plus terminal webhook notifications for v1; SRT, DOCX, and TTS are
  deferred follow-up capabilities.
