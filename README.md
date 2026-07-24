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
- `.agents/skills/local-book-reading-pipeline` is the project-level discovery entry.

Useful supporting skills:

- `skills/expert-translation-quality/SKILL.md`
- `skills/translation-quality-defect-families/SKILL.md`
- `skills/print-compatible-book-layout/SKILL.md`

Do not use `skills/public-domain-epub-pipeline/SKILL.md` unless you intentionally want to work on the upstream public-domain workflow.

Skill source files live under `skills/`. The `.agents/skills` directory is the project whitelist for Codex, Claude, Copilot, and OpenCode; `.claude/skills` points to the same whitelist.

## Tests

`.github/workflows/ci.yml` runs these suites on every pull request, on every
push to `main`, and on every `v*` tag before the release build starts. Run them
locally the same way:

```sh
# Python suites, from the repository root
uv run --package translation-engine pytest packages/translation-engine/tests
uv run --package ocr pytest packages/ocr/tests
uv run --package zotero-cli-agent --extra dev --extra mcp pytest packages/zotero-cli/tests

# Launcher backend (177 tests)
cd tools/bibliosmith-launcher/source/src-tauri && cargo test

# Launcher frontend: typecheck and startup contract
cd tools/bibliosmith-launcher/source && npm ci && npx tsc --noEmit && npm run test:startup-contract
```

`--package translation-engine` is not optional: it installs the workspace member
so the CLI tests can reach its console scripts. A plain `uv sync` at the
repository root uninstalls that member, after which a bare `pytest` fails to
import `translation_engine`. See `packages/translation-engine/README.md`, and
`CONTRIBUTING.md` for the two naming traps in the Zotero CLI command.

## Book Storage

`books/local/` is ignored by Git. It is meant for real local books, source text, drafts, QA files, and generated EPUBs.

The repository itself can be synced to another machine. Real books should be copied by explicit sync, not committed.

## Project Home

Open-source home: <https://github.com/semantic-craft/bibliosmith>

Earlier license texts are preserved under `license/`. Do not package sample book content, release EPUBs, covers, or translations as your own skill assets.
