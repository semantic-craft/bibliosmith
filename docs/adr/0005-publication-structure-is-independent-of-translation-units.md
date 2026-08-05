# ADR 0005: Publication structure is independent of translation units

- Status: Accepted
- Date: 2026-08-05
- Decision tickets: #180–#184 (`semantic-craft/bibliosmith`)

## Context

The pipeline historically used one chapter-shaped record for both a reader's
book structure and the bounded batches sent to a translation model. Large
chapters were split at deeper headings or paragraphs to stay below the model
limit, and the EPUB builder then exposed those file boundaries in its spine and
navigation. EPUBCheck could still pass because the package was syntactically
valid.

The alternatives were to keep reconstructing chapters inside each output
builder, to preserve the shared record and add display-only exceptions, or to
make publication structure a first-class book-owned artifact.

## Decision

The pipeline records two independent artifacts:

1. `metadata/publication_map.json` is the versioned source of truth for
   reader-visible structure, roles, titles, hierarchy, source ranges, and
   semantic references.
2. `metadata/source_map.json` records bounded translation units and their
   mapping to publication sections and source ranges.

Translation limits may change the number and boundaries of translation units.
They must not create, remove, rename, reorder, or promote publication sections.
Reading-output builders consume the publication map and reassemble translation
units behind that interface. They do not infer structure from filenames.

Package validity, structural readability, and reader acceptance are separate
conclusions. Package and structure are automated gates; reader acceptance is
optional artifact-bound evidence and is reported as not recorded when absent.

Extractor evidence is part of the source contract, not transient worker state.
Every referenced extractor document is retained with a digest inside the local
book project, and every recovered section has an exact source range. Invalid
roles, duplicate identities, broken parent links, cycles, and ranges that do not
match their source heading fail the preproduction gate instead of being repaired
implicitly. Translation handoff copies the producer sidecar byte-for-byte and
binds its relative path and SHA-256 in `source_manifest.json`; split rejects a
missing or rewritten sidecar before parsing it. Section and Note source-file
bindings remain explicit fields in the compiled Publication Map rather than
surviving only as transient validation inputs.

When automatic recovery fails, the App exposes a source-bound correction draft.
It may change reader-facing titles, hierarchy, roles, and kinds, but cannot move
source order, ranges, pages, retained files, or anchors. The saved correction is
bound to both the source Markdown and the exact recovered structure; a later
extractor result cannot silently inherit a stale human correction.

Extractor evidence also carries a common Note contract for EPUB, direct PDF,
PaddleOCR, and MinerU sources: stable note and reference IDs, kind, owning
publication section, definition/reference ranges, source pages, source files,
and anomalies. Canonical Markdown note syntax remains the transport format, but
the split stage compares it with the extractor contract and fails if either side
loses a definition, reference, source binding, or identity. EPUB navigation
ranges may span several spine documents; a parent section includes the ranges
and retained XHTML evidence of its descendants rather than being truncated at
its own file boundary.

Structural rendering is an offline validation operation over untrusted book
content. It disables JavaScript, blocks network requests, rejects remote package
resources, checks the full package rather than spine documents alone, and
records the exact managed renderer used. Duplicate manifest identities, global
XHTML IDs, missing local resources, or unresolved fragments fail the structure
gate. A missing output or an unrun check is never equivalent to a passing check.

## Consequences

Every extractor must produce or support recovery of a publication map before
translation. Translation, QA, promotion, standard EPUB, bilingual EPUB, and
validation must preserve stable section and semantic-reference identities.
Changing the model batch limit no longer changes the book readers see. Invalid
or low-confidence structure becomes an explicit preproduction outcome instead
of a successful generic EPUB. Source evidence remains locally auditable after
worker scratch files are removed, and validation cannot execute or disclose
book content while checking it.
