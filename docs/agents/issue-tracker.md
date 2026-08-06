# Issue tracker: GitHub

Issues and PRDs for this repo live as GitHub issues. Use the `gh` CLI for all
operations.

PRs as a request surface: no.

## Which repository

All new issues and live work go to **`semantic-craft/bibliosmith`**. Always pass
`--repo semantic-craft/bibliosmith`; do not infer the target from
`git remote -v`, because this checkout has multiple GitHub remotes.

```bash
gh issue list --repo semantic-craft/bibliosmith
gh issue create --repo semantic-craft/bibliosmith
```

**`semantic-craft/bibliosmith-private-archive` is read-only history.** Read it
when older documents refer to archived issue numbers, but never attempt to
create, comment on, edit, label, or close issues there:

```bash
gh issue view <number> --repo semantic-craft/bibliosmith-private-archive
```

Issue numbers restarted in the public repository. A bare `#NN` written before
2026-07-24 usually refers to the archive. Check `docs/planning/README.md` for
migrated issue mappings. The archived PRD and Wayfinder map were migrated into
`docs/planning/`.

## Conventions

- **Create:** `gh issue create --repo semantic-craft/bibliosmith --title "..." --body "..."`
- **Read:** `gh issue view <number> --repo semantic-craft/bibliosmith --comments`
- **List:** `gh issue list --repo semantic-craft/bibliosmith --state open --json number,title,body,labels,comments`
- **Comment:** `gh issue comment <number> --repo semantic-craft/bibliosmith --body "..."`
- **Apply/remove labels:** `gh issue edit <number> --repo semantic-craft/bibliosmith --add-label "..."` or `--remove-label "..."`
- **Close:** `gh issue close <number> --repo semantic-craft/bibliosmith --comment "..."`

GitHub shares one number space across issues and pull requests. Resolve an
ambiguous number explicitly with `gh pr view` or `gh issue view`, always passing
`--repo`.

## Skill operations

When a skill says “publish to the issue tracker,” create an issue in
`semantic-craft/bibliosmith`.

When a skill says “fetch the relevant ticket,” run:

```bash
gh issue view <number> --repo semantic-craft/bibliosmith --comments
```

See `docs/agents/triage-labels.md` before applying workflow labels.
