# ADR 0004: Immutable prompt packs and ephemeral compilation

- Status: Accepted
- Date: 2026-08-05
- Decision map: `semantic-craft/bibliosmith` issue 164

BiblioSmith manages translation behaviour as immutable Prompt Pack revisions,
not as per-book free-text instructions. Built-in revisions live with read-only
application resources; local custom revisions live in Application Support; a
Book Pipeline job stores only its effective pack ID, revision ID, and content
hash. Local edits may change only stage templates and the declared open
style/quality parameters; executor constraints and expert evidence policies are
part of the copied immutable revision and cannot be removed by changing the
pack ID. Programmatic and expert-agent packs share this management contract but
remain incompatible executors. Template content may persist, while an actual
prompt containing source text, neighbouring context, glossary entries, or
review material is compiled on demand by the same module used for execution and
is never written to job state, logs, diagnostics, checkpoints, or prompt
history. Expert receipts cite project-relative `qa/` evidence files and their
SHA-256 values; the launcher verifies the referenced bytes and requires every
stage-evidence document to bind its evidence type, Prompt Pack reference,
translation-handoff hash, and passed status before accepting a gate. Independent
review counts and defect-family closure data are read from those bound evidence
documents rather than trusted as detached receipt fields, including distinct
candidate-scan, repair, and recheck evidence for a closed defect family. This
preserves reproducibility without duplicating private book text, and deliberately
replaces the old `customInstructions` path rather than supporting two prompt
systems.
