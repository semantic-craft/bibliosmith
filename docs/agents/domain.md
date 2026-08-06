# Domain Docs

This repo uses a single-context layout.

## Before exploring

- Read root `CONTEXT.md`.
- Read relevant ADRs under `docs/adr/`.
- When working in a package with supplemental local instructions, read those
  too. In particular, `packages/ocr/CONTEXT.md` and
  `packages/ocr/docs/adr/` document OCR-specific implementation constraints
  without defining a separate top-level context.

If these files do not exist, proceed silently. The domain-modeling flow creates
or extends them only when real terms or decisions need to be captured.

## File structure

```text
/
├── CONTEXT.md
├── docs/
│   └── adr/
├── packages/
└── tools/
```

Root `CONTEXT.md` and `docs/adr/` remain canonical for the product as a whole.
Package-local documents are supplemental and must not silently override root
decisions.

## Use the glossary’s vocabulary

When an issue title, proposal, hypothesis, or test names a domain concept, use
the term defined in `CONTEXT.md`. Do not drift to synonyms the glossary
explicitly avoids.

If a needed concept is absent, reconsider whether the term belongs to the
project or note the gap for the domain-modeling flow.

## Flag ADR conflicts

If proposed work contradicts an existing ADR, surface the conflict explicitly
instead of silently overriding the decision.
