# Planning documents migrated from the archived repository

The PRD and the Wayfinder map for the monorepo and dual-mode translation
pipeline used to live as issues `#52` and `#40` in what is now
`semantic-craft/bibliosmith-private-archive`. That repository is archived and
therefore read-only: nobody can comment on them, correct them, or close them
ever again.

They are kept here as documents rather than as new issues, because:

- Both are finished planning artifacts, not open work. The map reached its own
  stated endpoint on 2026-07-19, and the PRD is a destination spec. Neither
  describes something to be done, so neither belongs in a work tracker.
- As documents they are versioned, reviewable in a pull request, and diffable
  against the code they describe. The `docs/book-pipeline-capability-matrix.md`
  drift that #36 had to repair is exactly what a frozen issue body cannot avoid.
- Every issue number in the originals belongs to a numbering the open-source
  rebuild replaced. In a document each reference can be annotated in place; a
  migrated issue would have had to mint fresh numbers and re-link everything.

The originals stay readable at any time:

```bash
gh issue view 52 --repo semantic-craft/bibliosmith-private-archive
```

## Issue numbers in these documents

Archive numbers are rewritten to this repository's numbers wherever a live
successor exists. The mapping, established 2026-07-26:

| Archive | Here | Note |
| --- | --- | --- |
| `#85` Sample & Compare | #31 | Main body landed; #31 covers the residuals only. |
| `#88` full-chain auto-advance | — | Resolved as option B; see the story 16 entry in the PRD. |
| `#89` bounded auto-retry | #27 | |
| `#90` digest UI gate | — | Resolved: the wizard has the checkbox (`NewJobWizard.tsx`). |
| `#91` sample missing textCleanup/customInstructions | — | Resolved: the parameters are passed through. |
| `#92` cleanup approval record | #21 | |
| `#93` runtime disrepair (zsearch timer) | #33 | |
| `#94` chapter-level parallelism | #30 | |
| `#95` webhook repeat delivery | #28 | |
| `#96` reader-verification evidence slot | #29 | |
| `#97` Windows worker runbook paths | #34 | |
| `#98` glossary output-side check | #32 | |
| `#99` two bilingual builders | #35 | Resolved: the old manual publishing builder was removed; the Launcher builder is documented in `docs/bilingual-epub-builder.md`. |
| `#41`–`#51`, `#80`, `#87` | — | Closed map tickets; archive numbering, cited for their decisions. |

Numbers written as `archive #NN` below always mean the archived repository.
Bare `#NN` always means `semantic-craft/bibliosmith`.
