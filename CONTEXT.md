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

## Book structure

- **Publication section**: a reader-visible node in a book's nested structure,
  such as front matter, a part, chapter, section, notes, bibliography, or
  appendix. Its identity and title do not change when processing limits change.
- **Translation unit**: a bounded, source-traceable batch sent through the
  translation workflow. It belongs to a publication section but is never itself
  a reader-visible chapter or navigation title.
- **Publication map**: the book-owned, versioned tree of publication sections,
  roles, source ranges, and semantic references used to build reading outputs.
- **Source mapping**: the traceability relationship from each translation unit
  and semantic reference back to its publication section and source range.
- **Semantic Note**: an extractor-independent note entity whose stable ID,
  references, definition range, owning publication section, source files/pages,
  target-content state, and backlinks survive split, translation, and EPUB
  construction. Extractor evidence and canonical Markdown must agree.
- **Package validity**: machine evidence that a reading artifact is a valid,
  internally consistent package.
- **Structural readability**: machine evidence that a reading artifact preserves
  the book's publication structure, metadata, navigation, notes, and reflow rules.
- **Reader acceptance**: optional, artifact-bound evidence recorded from an
  actual reading system. Absence means not recorded, never accepted.

## Translation prompt language

- **Translation Prompt Pack**: a named translation behaviour contract with a
  compatible executor, language direction, stage templates, and provenance.
  Avoid: prompt, preset, custom instructions.
- **Prompt Pack revision**: one immutable version of a Translation Prompt Pack,
  identified by a revision ID and content hash. It includes stage templates,
  explicitly open style/quality parameters, provenance, and any expert evidence
  policy, so a local copy cannot shed its gates. Avoid: saved edit, current text.
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
  checkout or a user workspace. A self-update replaces this layer whole, so
  nothing that must survive an update may be written inside the bundle.
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
- [ADR 0004](docs/adr/0004-immutable-prompt-packs-and-ephemeral-compilation.md):
  immutable Prompt Pack revisions, incompatible executors, verified expert
  evidence references, and ephemeral actual-prompt compilation.
- [ADR 0005](docs/adr/0005-publication-structure-is-independent-of-translation-units.md):
  publication sections and bounded translation units are separate contracts;
  package, structure, and reader acceptance are separate conclusions.
- [ADR 0006](docs/adr/0006-user-confirmed-signed-self-update.md): the launcher
  checks for updates once per launch but never installs one on its own; the
  updater bundle carries its own signature, checked against the public key in
  the running build, and an install is refused while jobs are running.
