# Issue tracker: GitHub

Issues and PRDs for this repo live as GitHub issues. Use the `gh` CLI for all
operations.

PRs as a request surface: no.

## Which repository

**`semantic-craft/bibliosmith`.** All new issues and all live work go there.
Pass it explicitly rather than inferring it from `git remote -v`: this project
has two repositories, and only one of them accepts writes.

```bash
gh issue list --repo semantic-craft/bibliosmith
gh issue create --repo semantic-craft/bibliosmith
```

**`semantic-craft/bibliosmith-private-archive` is a read-only historical
reference.** It is the pre-open-source repository, and GitHub has it archived,
which makes the whole repository read-only: creating an issue, commenting,
closing, and editing labels are all rejected. Read it, never write to it:

```bash
gh issue view <N> --repo semantic-craft/bibliosmith-private-archive
```

Twelve issues were still open there when it was archived, and they will stay
open forever. Do not open a duplicate of one in `semantic-craft/bibliosmith`
without checking first — the live successors were renumbered, and the mapping is
in `docs/planning/README.md`.

Issue numbers restarted at 1 in the public repository. A bare `#NN` in a
document written before 2026-07-24 usually belongs to the archive's numbering,
not this one.

The live content of the two planning issues that were frozen in the archive —
PRD `#52` and Wayfinder map `#40` — was migrated into `docs/planning/`.

## Usage

When a skill says "publish to the issue tracker", create a GitHub issue in
`semantic-craft/bibliosmith`.

When a skill says "fetch the relevant ticket", run
`gh issue view <number> --repo semantic-craft/bibliosmith --comments`.

Use `gh issue create/view/list/comment/edit/close` for issue operations, always
with an explicit `--repo`.

See `docs/agents/triage-labels.md` for the labels that actually exist.
