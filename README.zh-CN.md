# 本地阅读翻译工作台

BiblioSmith 本地阅读/翻译工作台。

默认用途不是发布公版书，而是处理你电脑上已有的 EPUB/PDF：抽取、拆章、翻译、审校、生成 Markdown/HTML/EPUB。

English: [README.md](README.md)。

## 安装与运行

桌面端叫 **BiblioSmith Launcher**，是一个 Tauri 应用。发行版**只构建 macOS Apple
Silicon**，产物只有 DMG 一种。仓库里有 Windows 条件编译的代码，但目前不构建也不测试
Windows 版本。

### 1. 下载

发行版就发布在本仓：

<https://github.com/semantic-craft/bibliosmith/releases>

1.12.0 及更早的版本发在单独的下载仓——那是源码还私有时的产物——继续留在原处：
<https://github.com/semantic-craft/bibliosmith-releases/releases>

取最新的 `BiblioSmith.Launcher_<版本>_aarch64.dmg`，打开后把
`BiblioSmith Launcher.app` 拖进 `/Applications`。

### 2. 首次打开过 Gatekeeper

DMG 是 **ad-hoc 签名、未公证**的（`src-tauri/tauri.conf.json` 里
`"signingIdentity": "-"`），所以 macOS 一定会拦下第一次启动，提示「无法验证开发者」
或「已损坏」。这是预期行为。

1. 先双击一次，让 macOS 拒绝。
2. 打开**系统设置 → 隐私与安全性**，翻到「安全性」一节，在 BiblioSmith Launcher
   那一条旁边点**仍要打开**。
3. 在弹出的确认框里再确认一次。之后 macOS 会记住这个决定。

如果那一条没出现，就直接清掉隔离属性：

```sh
xattr -dr com.apple.quarantine "/Applications/BiblioSmith Launcher.app"
```

应用没有自动更新。要升级就重新下载新的 DMG。

### 3. 首次启动要配什么

按顺序三样：

1. **仓库目录**。指向本仓库在本机的一个 clone。启动器不再自动下载项目内容，必须自己
   先 clone；目录既不是 BiblioSmith 检出、又不为空时它会直接报错。书籍项目会建在这个
   目录下的 `books/local/`。
2. **运行时**。启动器会检查 Python 和 Java。EPUBCheck 跑在 Java 上，没有 Java 校验
   这一步就过不去。两者都找不到时，启动器可以把私有运行时下载到自己的目录里，不动
   系统环境。
3. **凭证，在「设置」里配**。模型 API key 在模型设置面板里填，存进 macOS 钥匙串的
   `com.bibliosmith.launcher.models`，一个 provider 槽位一条 —— 不进仓库、不进作业
   记录、不进日志。OCR 与向量嵌入的凭证各有自己的面板。至少配好一个模型槽位，翻译
   才能开跑。

出问题时看日志：

```text
~/Library/Application Support/BiblioSmith/launcher/logs/bibliosmith-launcher.log
```

### 从源码跑

前置条件：**Node.js 20**、**Rust 1.88.0**（由
`tools/bibliosmith-launcher/source/rust-toolchain.toml` 钉住，rustup 会自动选择）、
装 Python 包用的 **[uv](https://docs.astral.sh/uv/)**、以及跑 EPUBCheck 用的 **JDK**。

```sh
cd tools/bibliosmith-launcher/source
npm ci
npx tauri dev                    # 开发窗口，带热重载
npx tauri build --bundles dmg    # 产出与发布流程相同的 DMG
```

`npx tauri dev` 第一次要编译整个 Rust 后端，会等一会儿。

## 起一本新书

```bash
cd bibliosmith
python3 tools/create_local_book_project.py "书名_作者" --source-file "/path/to/book.epub"
```

然后进入生成的目录，让 agent 使用：

```text
Use skills/local-book-reading-pipeline/SKILL.md to process this book.
```

技能源文件在 `skills/`，全新 clone 里也只有这一个技能目录。`.agents/skills` 与
`.claude/skills` 是每台机器自己维护的软链白名单，两者都在 gitignore 里，clone 出来
不存在，也不是使用技能的前提 —— 直接指 `skills/` 下的路径即可。

默认产物目录：

```text
books/local/zh-Hans/001_书名_作者/output/reading/
```

## 测试

`.github/workflows/ci.yml` 在每个 PR、每次推 `main`、以及每个 `v*` tag 开始构建前跑
下面这些套件。本地照同样的方式跑，顺序与 CI 一致：

```sh
# Python 套件，在仓库根目录跑
uv run --package translation-engine pytest packages/translation-engine/tests
uv run --package ocr pytest packages/ocr/tests
uv run --package zotero-cli-agent --extra dev --extra mcp pytest packages/zotero-cli/tests

# Repository suites：packages/*/tests 之外的全部，最容易漏；CI 是一个步骤，也当一条命令跑
uv run --package digest pytest \
  tests \
  tools/git \
  tools/bibliosmith-launcher/source/scripts/tests

# 启动器后端
cd tools/bibliosmith-launcher/source/src-tauri && cargo test

# 启动器前端：类型检查 + 单元测试 + 启动契约
cd tools/bibliosmith-launcher/source && npm ci && npx tsc --noEmit && npm test && npm run test:startup-contract
```

2026-07-26 实测数量：翻译引擎 81、OCR 18、Zotero CLI 62、repository suites 89、
启动器后端 209、启动器前端 122。

Zotero CLI 那条命令里三个命名陷阱（包名与目录名不同、`--extra dev`、`--extra mcp`）
见 `CONTRIBUTING.md`。

## 其他

本改造版不做公版搜索、版权状态判断、private-use 声明或 GitHub release。上游公版流水线仍保留在 `template/epub_pipeline/`，仅作参考。

提交信息必须带 `ZH:` / `EN:` / `JA:` 三段，标签独占一行；CI 在 PR 上检查整条分支历史。
