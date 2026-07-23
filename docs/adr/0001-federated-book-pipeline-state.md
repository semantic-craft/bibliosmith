# ADR 0001: Federated Book Pipeline state

- Status: Accepted
- Date: 2026-07-17
- Decision ticket: #50

## Context

The monorepo contains four established persistence domains: OCR SQLite
(`documents`/`chunks`), launcher Book Pipeline JSON, per-book local project files,
and the sqlite-vec semantic index. The translation engine also needs resumable
checkpoints. Combining these into one physical store would give the launcher
write authority over component internals, increase migration coupling, and make
private content easier to copy into orchestration state.

## Decision

Keep persistence **federated**. Book Pipeline state is the single source of truth
for orchestration status only. Each subsystem remains the single writer for its
own component store.

| Store | Owner and writer | Book Pipeline records |
| --- | --- | --- |
| Launcher job JSON | Tauri Book Pipeline | Parent/child/stage status, attempts, approvals, safe errors, evidence references, aggregate progress, notification delivery state |
| OCR SQLite | `packages/ocr` worker | Document identity, contract version, content/source hashes, output artifact IDs; no copied OCR text |
| Per-book project | Local reading pipeline and translation engine | Project-relative artifact paths, SHA-256, manifest/checkpoint contract versions, unit summaries; no chapter text |
| sqlite-vec index | `packages/zotero-cli` | Parent item key, source hash, chunk count, embedding profile and contract versions; no vectors or chunk text |

Translation checkpoints live under the private book project's `qa/` area and are
owned by the translation engine. Book Pipeline stores only the checkpoint
identity, input hash, contract version, completion summary, and artifact ID.

## Cross-store consistency

There is no distributed transaction. A component first commits its own output,
then returns an evidence record. Book Pipeline validates that evidence and
atomically commits the stage transition in its own state.

On start, retry, or resume, the stage adapter reconciles its stored references:

1. Resolve the referenced component record or artifact under its allowed root.
2. Verify identity, contract version, SHA-256, and required completion evidence.
3. Reuse the stage only when every required field matches.
4. Mark the stage explicitly blocked or failed when evidence is missing,
   mismatched, or no longer readable; never infer completion from a file name or
   a component row alone.

## Privacy rules

- Orchestration state may contain opaque IDs, hashes, counts, contract versions,
  safe relative references, and privacy categories.
- It must not embed source/translated text, vectors, credentials, webhook URLs,
  unrestricted logs, or private absolute paths in outbound events.
- References to private text use the existing `private_text` or
  `private_metadata` category. Redacted diagnostics use
  `redacted_diagnostic`.

## Consequences

Component schemas can evolve independently and private data stays near its owner.
Adapters must implement explicit reconciliation, and a component commit can
temporarily precede its launcher transition. That gap is an expected recoverable
state, not a reason to duplicate the component store inside launcher JSON.
