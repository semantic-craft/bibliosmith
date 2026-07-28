# Local Reading Translations

本仓库是一个本地优先的书籍/论文阅读与翻译工作台。

目标很简单：把你电脑上已有的 EPUB、PDF、论文和书稿，整理成干净的 Markdown、中文译稿、HTML、EPUB 或双语 EPUB，方便自己阅读和研究。

## Scope

- New projects live under `books/local/{target}/{number}_{title_author}/`.
- The local source is copied to `source/original.*`; its manifest records only file identity and processing state.
- Reading artifacts are written to `output/reading/`.
- The repository contains no book-discovery, source-rights review, or book-publication pipeline.

The project does not remove DRM, bypass access controls, locate unauthorized full text, or decide whether a book may be published.

中文说明见 [README.zh-CN.md](README.zh-CN.md)。

## Install and run the app

The desktop app is **BiblioSmith Launcher**, a Tauri application. Releases are
built for **macOS on Apple Silicon** only; the DMG is the sole published
artifact. The repository has Windows-conditional code, but no Windows build is
produced or tested.

### 1. Download

Releases are published here:

<https://github.com/semantic-craft/bibliosmith/releases>

Versions up to 1.12.0 were published to a separate download repository, back
when this source was private, and stay there:
<https://github.com/semantic-craft/bibliosmith-releases/releases>

Take the newest `BiblioSmith.Launcher_<version>_aarch64.dmg`, open it, and drag
`BiblioSmith Launcher.app` into `/Applications`.

### 2. Open the app

The release workflow signs the app with a **Developer ID Application**
certificate, notarizes it with Apple, and staples the notarization ticket before
publishing. Gatekeeper therefore accepts the app with source
`Notarized Developer ID`: after dragging it to `/Applications`, double-click it
normally. No Privacy & Security override or quarantine removal is required.

There is no auto-update. A new version means downloading the new DMG.

### 3. First launch

The launcher asks for three things, in this order:

1. **A repository folder.** Point it at a local clone of this repository. The
   launcher no longer downloads the project for you — pick an existing clone,
   and it refuses any directory that is neither a BiblioSmith checkout nor
   empty. This is where book projects are created, under `books/local/`.
2. **A runtime.** The launcher checks for Python and Java. Java is what
   EPUBCheck runs on, so validation stays blocked without it. If neither is
   found, the launcher can download a private runtime into its own directory
   rather than touching your system.
3. **Credentials, in Settings.** Model API keys go in the model settings panel
   and are stored in the macOS Keychain under
   `com.bibliosmith.launcher.models`, one entry per provider slot — never in
   the repository, in job state, or in logs. OCR and embedding credentials have
   their own panels. Translation cannot start until at least one model slot has
   a key.

Logs, if something goes wrong:

```text
~/Library/Application Support/BiblioSmith/launcher/logs/bibliosmith-launcher.log
```

### Running from source instead

Prerequisites: **Node.js 20**, **Rust 1.88.0** (pinned by
`tools/bibliosmith-launcher/source/rust-toolchain.toml`, so rustup selects it
for you), **[uv](https://docs.astral.sh/uv/)** for the Python packages, and a
**JDK** for EPUBCheck.

```sh
cd tools/bibliosmith-launcher/source
npm ci
npx tauri dev      # development window with hot reload
npx tauri build --bundles dmg --no-sign   # portable local build
```

`npx tauri dev` compiles the Rust backend on first run, which takes a while.
The committed bundle config targets `Developer ID Application`. Release
maintainers with that certificate can omit `--no-sign` or set an exact
`APPLE_SIGNING_IDENTITY`; only the Release workflow injects signing and
notarization secrets.

On macOS, a release maintainer can update the Apple app-specific password
without placing it in shell history or command arguments:

```sh
./tools/bibliosmith-launcher/source/scripts/set-apple-password-secret-macos.sh
```

The script opens a hidden-input dialog and sends the value directly to the
`APPLE_PASSWORD` GitHub Secret for `semantic-craft/bibliosmith`. An empty or
cancelled dialog leaves the existing Secret unchanged.
The Rust tests do not need the frontend bundle; see [Tests](#tests).

## Quick Start

Create a local book project from an existing EPUB or PDF:

```bash
cd bibliosmith
python3 tools/create_local_book_project.py "书名_作者" \
  --source-file "/path/to/book.epub" \
  --source-language en \
  --target-language zh-Hans
```

The command creates:

```text
books/local/zh-Hans/001_书名_作者/
  AGENTS.md
  README.md
  source/original.epub
  source/source.md
  chapters/src/
  chapters/translated/
  chapters/final/
  glossary/terms.csv
  metadata/source_manifest.json
  qa/
  output/reading/
```

Then ask an agent from the book directory:

```text
Use the local-book-reading-pipeline skill to process this book.
Extract source/original.epub to source/source.md, split chapters, translate to Chinese, and build output/reading/book.epub.
```

## Workflow

1. Create project from a local file.
2. Extract EPUB/PDF to `source/source.md`.
3. Split into `chapters/src/`.
4. Build a glossary and style profile.
5. Translate into `chapters/translated/`.
6. Review and promote clean text to `chapters/final/`.
7. Build semantic HTML under `output/reading/html/` and EPUB files under `output/reading/`.
8. Run EPUBCheck or a practical reader check.

For long academic books, keep one book per project and one major translation run per thread.

## Optional Digest

Enable **BiblioSmith Digest** when you want a compact reading edition in
addition to the regular outputs. The Launcher exposes this as an explicit
output choice. For a manual run, write `digest.config.json` in the book project
and run:

```sh
python -m digest.bibliosmith_digest --book-root books/local/{target}/{number}_{title_author}
```

The result remains a standard EPUB. See the
[Digest guide](readme/digest/README.en.md) for configuration and review steps.

## Skills

Default skill:

- `skills/local-book-reading-pipeline/SKILL.md`

Useful supporting skills:

- `skills/expert-translation-quality/SKILL.md`
- `skills/translation-quality-defect-families/SKILL.md`
- `skills/print-compatible-book-layout/SKILL.md`

Skill source files live under `skills/`, and that is the only skill directory a
fresh clone has. `.agents/skills` and `.claude/skills` are a per-developer
whitelist of symlinks: both are gitignored, so neither exists after cloning and
neither is required to use the skills — point your agent at the `skills/` path
directly.

## Tests

`.github/workflows/ci.yml` runs these suites on every pull request, on every
push to `main`, and on every `v*` tag before the release build starts. Run them
locally the same way — the list below is every step CI runs, in the same order:

```sh
# Python suites, from the repository root
uv run --package translation-engine pytest packages/translation-engine/tests
uv run --package ocr pytest packages/ocr/tests
uv run --package zotero-cli-agent --extra dev --extra mcp pytest packages/zotero-cli/tests

# Repository suites: everything outside packages/*/tests. Easy to forget, and
# it is a single CI step, so run it as one command.
uv run --package digest pytest \
  tests \
  tools/git \
  tools/bibliosmith-launcher/source/scripts/tests

# Launcher backend
cd tools/bibliosmith-launcher/source/src-tauri && cargo test

# Launcher frontend: typecheck, unit tests, startup contract
cd tools/bibliosmith-launcher/source && npm ci && npx tsc --noEmit && npm test && npm run test:startup-contract
```

`--package translation-engine` is not optional: it installs the workspace member
so the CLI tests can reach its console scripts. A plain `uv sync` at the
repository root uninstalls that member, after which a bare `pytest` fails to
import `translation_engine`. See `packages/translation-engine/README.md`, and
`CONTRIBUTING.md` for the three naming traps in the Zotero CLI command.

CI also runs a commit-message check on pull requests and a gitleaks scan; both
need no local setup, though `CONTRIBUTING.md` explains the local hooks that
catch the same problems earlier.

## Book Storage

`books/local/` is ignored by Git. It is meant for real local books, source text, drafts, QA files, and generated EPUBs.

The repository itself can be synced to another machine. Real books should be copied by explicit sync, not committed.

## Project Home

Open-source home: <https://github.com/semantic-craft/bibliosmith>

Inherited license texts are preserved under `license/`. Do not package sample book content, generated EPUBs, covers, or translations as your own skill assets.
