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

## Translation prompt language

- **Translation Prompt Pack**: a named translation behaviour contract with a
  compatible executor, language direction, stage templates, and provenance.
  Avoid: prompt, preset, custom instructions.
- **Prompt Pack revision**: one immutable version of a Translation Prompt Pack,
  identified by a revision ID and content hash. Avoid: saved edit, current text.
- **Prompt stage template**: the persistent, source-independent instruction for
  one translation or review stage. Avoid: actual prompt, system prompt snapshot.
- **Effective Prompt Pack**: the exact Prompt Pack revision resolved for one
  book after applying defaults and an optional book override. Avoid: selected
  mode, active prompt.
- **Actual prompt preview**: an ephemeral rendering of one prompt stage with the
  current book sample and executor-owned inputs injected. It is display data,
  never durable job evidence. Avoid: prompt history, saved preview.
- **Executor constraint**: a non-overridable rule owned by the programmatic
  engine or expert-agent runner, including structure, placeholders, glossary,
  privacy, retries, and evidence handling. Avoid: locked template text.

## Boundaries

- The installed App bundle owns read-only code, provider registries, packages,
  and runtime scripts. Production never executes those resources from a source
  checkout or a user workspace.
- The operating system's Application Support directory owns launcher config,
  logs, pipeline state, and managed runtime state. Credentials remain in the
  system Keychain.
- The operating system's Cache and temporary directories own disposable OCR
  staging, samples, download caches, and other reproducible intermediates.
- The configured user workspace (recommended: `~/Documents/BiblioSmith`) owns
  source books, translations, QA evidence, and final reading artifacts.
- `tools/bibliosmith-launcher` owns Book Pipeline orchestration and its public state.
- `packages/ocr` owns OCR worker data and SQLite.
- `books/local/...` owns each private local-reading project and its manifests,
  chapters, QA records, checkpoints, and reading output.
- `packages/zotero-cli` owns the item/chunk semantic index.
- GitHub Issues in `semantic-craft/bibliosmith` are the tracker.
  `semantic-craft/bibliosmith-private-archive` is archived and read-only; it is
  history to read, never a place to file work. See `docs/agents/issue-tracker.md`.
  Local ADRs record decisions; they do not silently close or mutate tickets.

## Decisions

- [ADR 0001](docs/adr/0001-federated-book-pipeline-state.md): federated component
  stores with explicit evidence references and reconciliation.
- [ADR 0002](docs/adr/0002-progress-and-terminal-notifications.md): Tauri stage/unit
  progress plus terminal webhook notifications for v1; SRT, DOCX, and TTS are
  deferred follow-up capabilities.
- [ADR 0003](docs/adr/0003-separate-app-resources-and-user-workspace.md): the App
  resource, Application Support, Cache/temp, and user workspace layers have
  distinct locations and ownership.
