# ADR 0004: Immutable prompt packs and ephemeral compilation

- Status: Accepted
- Date: 2026-08-05
- Decision map: `semantic-craft/bibliosmith` issue 164

BiblioSmith manages translation behaviour as immutable Prompt Pack revisions,
not as per-book free-text instructions. Built-in revisions live with read-only
application resources; local custom revisions live in Application Support; a
Book Pipeline job stores only its effective pack ID, revision ID, and content
hash. Programmatic and expert-agent packs share this management contract but
remain incompatible executors. Template content may persist, while an actual
prompt containing source text, neighbouring context, glossary entries, or
review material is compiled on demand by the same module used for execution and
is never written to job state, logs, diagnostics, checkpoints, or prompt
history. This preserves reproducibility without duplicating private book text,
and deliberately replaces the old `customInstructions` path rather than
supporting two prompt systems.
