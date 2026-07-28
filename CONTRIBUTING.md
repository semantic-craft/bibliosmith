# Contributing

## Local safety hooks

Run this once per clone, before your first commit:

```sh
git config core.hooksPath tools/git/hooks
```

Git deliberately never runs hooks straight out of a clone, so this one command
is what activates the two versioned hooks in
[`tools/git/hooks/`](tools/git/hooks):

- **pre-commit** refuses any `.env` file — even when staged with `git add -f` —
  and scans the staged tree with [gitleaks](https://github.com/gitleaks/gitleaks).
- **pre-push** refuses any ref that does not descend from the clean root commit,
  because the pre-open-source history is not publishable, and scans the tree
  being pushed.

[`.gitleaks.toml`](.gitleaks.toml) extends the default secret rules with generic
rules for developer home paths and private network addresses — neither of which
any secret scanner, GitHub push protection included, detects on its own. Those
rules stay generic on purpose: a guard against publishing personal data must not
publish that data itself, so identifier-specific checks live in
[`tests/test_public_privacy.py`](tests/test_public_privacy.py), which compares
hashes instead. Install the binary with `brew install gitleaks`; without it the
hooks still block `.env` files and unpublishable history, and the `secrets` job
in CI is the backstop that does not depend on local setup at all.

## Test suites

[`.github/workflows/ci.yml`](.github/workflows/ci.yml) runs every suite below on
every push to `main`, on every pull request, and — through `workflow_call` from
[`release-launcher.yml`](.github/workflows/release-launcher.yml) — on every `v*`
tag before the DMG is built.

| Suite | Command | Expected |
|---|---|---|
| Launcher Rust | `cd tools/bibliosmith-launcher/source/src-tauri && cargo test` | pass |
| Translation engine | `uv run --package translation-engine pytest packages/translation-engine/tests` | pass |
| OCR | `uv run --package ocr pytest packages/ocr/tests` | pass |
| Zotero CLI | `uv run --package zotero-cli-agent --extra dev --extra mcp pytest packages/zotero-cli/tests` | pass |
| Repository suites | `uv run --package digest pytest tests tools/git tools/bibliosmith-launcher/source/scripts/tests` | pass |
| Launcher frontend | `cd tools/bibliosmith-launcher/source && npm run version:check && npm ci && npx tsc --noEmit && npm run lint && npm test && npm run test:startup-contract` | pass |

The Repository suites row covers three directories, not one. `tests/` is the
obvious part; `tools/git` holds the commit-message validator that the
commit-messages job depends on, and
`tools/bibliosmith-launcher/source/scripts/tests` holds the runner's bilingual
EPUB builder tests. Both live outside `packages/` and outside `tests/`, so a
command naming only `tests` silently skips them — which is what this row used to
do.

Two more jobs run in CI and need nothing locally: the commit-message check (pull
requests only, validates the whole branch against the merge base) and a gitleaks
scan of both the published history and the tracked tree.

The frontend suite needs `npm ci` first. The Rust suite does **not** need a
frontend build — the tests never read `dist/`.

`npm test` runs vitest under jsdom over `src/**/*.test.{ts,tsx}`; `npm run
test:startup-contract` stays a pair of standalone node scripts with no runner,
because they compile `src/lib/markdown.ts` on its own to check what ships rather
than what a bundler resolves.

Three traps in the Zotero CLI row, all verified by wiping `.venv` and re-running:

- The distribution is named `zotero-cli-agent` while the directory is
  `packages/zotero-cli`, so `--package zotero-cli` fails with "the workspace
  does not have a member".
- `--extra dev` is what puts pytest in the venv. This package declares pytest
  under `[project.optional-dependencies]`, which uv does **not** install by
  default — unlike `translation-engine` and `ocr`, which use
  `[dependency-groups]` and therefore need no flag. Drop `--extra dev` and uv
  falls through to whatever `pytest` is on `PATH`; that interpreter cannot
  import `zotero_cli`, so you get 6 collection errors that look like a broken
  package rather than a missing flag.
- `--extra mcp` installs the optional dependency `test_mcp_server_builds` needs.
  Without it that test skips — 61 passed, 1 skipped — rather than failing.

The middle one is worth internalising: a suite that "passes" here may be reading
a `pytest` from Homebrew and a package from a stale `.venv`. When a result
surprises you, `rm -rf .venv` and run it again before believing it.

## Running the engine tests

Any of these work, from the repository root or from the package:

```sh
uv run --package translation-engine pytest packages/translation-engine/tests
uv run pytest packages/translation-engine/tests
cd packages/translation-engine && uv run pytest
```

They work because `packages/translation-engine/pyproject.toml` declares
`pythonpath = ["src"]`, which puts the sources on the import path directly.

This matters more than it looks. The workspace shares a single root `.venv`, and
different `uv` invocations re-sync it to different package sets — a plain
`uv sync` **uninstalls** `translation-engine` outright. Before `pythonpath` was
declared, the tests could only find `translation_engine` when it happened to be
installed, so the same command passed or failed with 16 collection errors
(`ModuleNotFoundError: No module named 'translation_engine'`) depending on
whatever ran last. If you need the console scripts rather than the tests, run
`uv sync --all-packages` to put every workspace package back.

## Where the tests live

Three places, and two of them are easy to miss. Most suites sit under
`packages/<name>/tests/`, while the repository-root `tests/` directory covers
Digest, EPUB builders, local-reading scope, privacy, and local-path gates. Two
further suites sit under `tools/`: `tools/git` and
`tools/bibliosmith-launcher/source/scripts/tests`. A search restricted to
`packages/*/tests` finds none of them.

That tree is run with `--package digest` because `tests/digest/` shells out to
`python -m digest.bibliosmith_digest`, which needs the member installed.

## Not covered by CI

- Live-provider behaviour is a manual check — see
  [`docs/runbooks/real-backend-smoke.md`](docs/runbooks/real-backend-smoke.md).
- Nothing enforces the gate at the branch level: `main` has no protection rule
  and no required status check, so a direct push lands whether or not `ci.yml`
  is green. The gate is real for tags — the release build waits on it — but on
  `main` it reports rather than blocks.

## Runners

This is a public repository, so GitHub-hosted runners are free. The Python,
commit-message, and launcher-frontend jobs run on `ubuntu-latest`; the
launcher-backend job runs on `macos-latest`, because the launcher is a Tauri
app built against macOS system frameworks. Each job gets its own hosted runner,
so they run in parallel.
