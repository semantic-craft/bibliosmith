# Local Reading Translations

本仓库是一个本地优先的书籍/论文阅读与翻译工作台。

目标很简单：把你电脑上已有的 EPUB、PDF、论文和书稿，整理成干净的 Markdown、中文译稿、HTML、EPUB 或双语 EPUB，方便自己阅读和研究。

## What Changed

- 默认流程不再做公版书搜索、版权状态判断、public-domain release 或 private-use declaration。
- 新书默认放在 `books/local/{target}/{number}_{title_author}/`。
- 本地书源复制到 `source/original.*`，只记录文件名、格式、SHA-256、语言和抽取状态。
- 最终阅读产物放在 `output/reading/`，不再使用 `output/release/` 或 `output/private_artifacts/` 作为完成标准。
- 上游 `template/epub_pipeline/` 和旧 `skills/public-domain-epub-pipeline/` 保留作参考，不是本仓库的默认启动路径。

本仓库不提供 DRM 移除、绕过访问控制、盗版全文查找或公开发布授权判断。

中文说明见 [README.zh-CN.md](README.zh-CN.md)。

## Install and run the app

The desktop app is **BiblioSmith Launcher**, a Tauri application. Releases are
built for **macOS on Apple Silicon** only; the DMG is the sole published
artifact. The repository has Windows-conditional code, but no Windows build is
produced or tested.

### 1. Download

Releases are published to a separate download repository:

<https://github.com/semantic-craft/bibliosmith-releases/releases>

Take the newest `BiblioSmith.Launcher_<version>_aarch64.dmg`, open it, and drag
`BiblioSmith Launcher.app` into `/Applications`.

### 2. Get past Gatekeeper on first open

The DMG is **ad-hoc signed and not notarized** (`"signingIdentity": "-"` in
`src-tauri/tauri.conf.json`). macOS therefore refuses the first launch with a
message about an unidentified developer or damaged app. This is expected.

1. Double-click the app once and let macOS refuse it.
2. Open **System Settings → Privacy & Security**, scroll to the Security
   section, and click **Open Anyway** next to the BiblioSmith Launcher entry.
3. Confirm at the next prompt. macOS remembers the decision.

If that entry does not appear, clear the quarantine attribute instead:

```sh
xattr -dr com.apple.quarantine "/Applications/BiblioSmith Launcher.app"
```

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
npx tauri build --bundles dmg   # produces the same DMG the release job ships
```

`npx tauri dev` compiles the Rust backend on first run, which takes a while.
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
7. Build `output/reading/book.html` and `output/reading/book.epub`.
8. Run EPUBCheck or a practical reader check.

For long academic books, keep one book per project and one major translation run per thread.

## Skills

Default skill:

- `skills/local-book-reading-pipeline/SKILL.md`

Useful supporting skills:

- `skills/expert-translation-quality/SKILL.md`
- `skills/translation-quality-defect-families/SKILL.md`
- `skills/print-compatible-book-layout/SKILL.md`

Do not use `skills/public-domain-epub-pipeline/SKILL.md` unless you intentionally want to work on the upstream public-domain workflow.

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

Expected counts, measured 2026-07-26: translation engine 81, OCR 18, Zotero CLI
62, repository suites 89, launcher backend 209, launcher frontend 121.

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

Earlier license texts are preserved under `license/`. Do not package sample book content, release EPUBs, covers, or translations as your own skill assets.
