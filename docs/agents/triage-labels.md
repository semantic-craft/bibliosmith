# Triage Labels

This file maps the engineering skills' canonical triage roles onto the labels
that actually exist in `semantic-craft/bibliosmith`. Verify before using a new
label:

```bash
gh label list --repo semantic-craft/bibliosmith
```

## Workflow labels

| Label | Meaning |
| --- | --- |
| `ready-for-agent` | Fully specified and ready for an AFK agent |
| `needs-decision` | Blocked on a human decision; do not implement unilaterally |
| `blocked` | Blocked by another ticket |
| `wontfix` | This will not be worked on |

An issue with no workflow label is untriaged.

## Skill vocabulary

| Canonical role | Use in this tracker |
| --- | --- |
| `needs-triage` | no workflow label |
| `needs-info` | `question` |
| `ready-for-agent` | `ready-for-agent` |
| `ready-for-human` | `needs-decision` |
| `wontfix` | `wontfix` |

Kind labels use GitHub's defaults (`bug`, `documentation`, `enhancement`,
`question`, `duplicate`, `invalid`, `help wanted`, and `good first issue`).
Dependency PRs also use `dependencies`, `github_actions`, `python:uv`, `rust`,
or `javascript` as appropriate.

Labels on `semantic-craft/bibliosmith-private-archive` are irrelevant because
that repository is archived and read-only.
