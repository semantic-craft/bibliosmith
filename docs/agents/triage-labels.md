# Triage Labels

These are the labels that exist on `semantic-craft/bibliosmith`. The list is
written from the repository rather than from a skill's vocabulary, because
`gh issue edit --add-label <name>` fails outright on a label that does not
exist. Verify before adding anything not listed here:

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

An issue with no workflow label is untriaged. There is no `needs-triage` label,
and adding one would only restate the absence.

## Kind labels

`bug`, `documentation`, `enhancement`, `question`, `duplicate`, `invalid`,
`help wanted`, `good first issue` — GitHub's defaults, used as they read.

`dependencies`, `github_actions`, `rust`, `javascript` — applied by Dependabot;
also used by hand to mark which subsystem an issue lands in.

## Mapping from the mattpocock/skills vocabulary

| Label in mattpocock/skills | Use here |
| --- | --- |
| `needs-triage` | no label |
| `needs-info` | `question` |
| `ready-for-agent` | `ready-for-agent` |
| `ready-for-human` | `needs-decision` |
| `wontfix` | `wontfix` |

Four of those five used to be listed as canonical here, and only two of them
had ever been created. If a workflow genuinely needs `needs-triage`,
`needs-info`, or `ready-for-human` as distinct labels, create them first and
then extend this table; do not assume they are there.

Labels on `semantic-craft/bibliosmith-private-archive` are irrelevant: that
repository is archived, so its labels cannot be applied to anything.
