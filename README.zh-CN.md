# 本地阅读翻译工作台

BiblioSmith 本地阅读/翻译工作台。

它只处理你电脑上已有的 EPUB/PDF：抽取、拆章、翻译、审校、生成 Markdown/HTML/EPUB。

English: [README.md](README.md)。

## 安装与运行

桌面端叫 **BiblioSmith Launcher**，是一个 Tauri 应用。发行版**只构建 macOS Apple
Silicon**：手动安装用 DMG，装好之后的升级走应用内自动更新。仓库里有 Windows 条件
编译的代码，但目前不构建也不测试 Windows 版本。

### 1. 下载

发行版就发布在本仓：

<https://github.com/semantic-craft/bibliosmith/releases>

1.12.0 及更早的版本发在单独的下载仓——那是源码还私有时的产物——继续留在原处：
<https://github.com/semantic-craft/bibliosmith-releases/releases>

取最新的 `BiblioSmith.Launcher_<版本>_aarch64.dmg`，打开后把
`BiblioSmith Launcher.app` 拖进 `/Applications`。

### 2. 打开应用

发布流程使用 **Developer ID Application** 证书签名，向 Apple 公证，并在发布前附加
公证票据。Gatekeeper 会将应用识别为 `Notarized Developer ID`；拖入
`/Applications` 后直接双击即可，不需要去「隐私与安全性」放行，也不需要清除隔离属性。

### 升级

装好之后应用会自己更新：每次启动在后台问一次 GitHub Release 有没有新版本，有的话弹
一条提示并在设置齿轮上留一个圆点。**下载和安装都要你自己在「设置 → 应用更新」里点**，
它不会静默替换。更新包用本项目的密钥签名，装之前先验签，验不过就不装。

有流水线任务在跑时安装按钮是禁用的：安装会整包替换 App，而任务正在用这个包里的
Python、Node 和 Chromium。等任务结束再装。

DMG 仍然照常发布，手动下载覆盖安装也一样可用。

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
npx tauri build --bundles dmg --no-sign  # 不要求证书的本地构建
```

`npx tauri dev` 第一次要编译整个 Rust 后端，会等一会儿。
仓库的打包配置默认指向 `Developer ID Application`。持有该证书的发布维护者可以去掉
`--no-sign`，或用 `APPLE_SIGNING_IDENTITY` 指定精确身份；只有 Release workflow 会注入
签名与公证 Secrets。

macOS 发布维护者可用下面的脚本更新 Apple App 专用密码，避免密码进入 shell 历史或
命令参数：

```sh
./tools/bibliosmith-launcher/source/scripts/set-apple-password-secret-macos.sh
```

脚本会弹出隐藏输入框，并把内容直接写入 `semantic-craft/bibliosmith` 的
`APPLE_PASSWORD` GitHub Secret；留空或取消不会改动现有 Secret。

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

## 可选 Digest

如果还需要一份压缩后的速读版，可以在 Launcher 中明确勾选 **BiblioSmith Digest**。
手动运行时，在书籍工程根目录写入 `digest.config.json`，然后执行：

```sh
python -m digest.bibliosmith_digest --book-root books/local/{target}/{number}_{title_author}
```

输出仍然是标准 EPUB。配置与审阅步骤见
[Digest 中文说明](readme/digest/README.zh-CN.md)。

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

Zotero CLI 那条命令里三个命名陷阱（包名与目录名不同、`--extra dev`、`--extra mcp`）
见 `CONTRIBUTING.md`。

## 其他

仓库只接受本地书源，不包含选书或书籍公开发布工具；也不提供 DRM 移除、访问控制绕过
或未授权全文查找。

提交信息必须带 `ZH:` / `EN:` / `JA:` 三段，标签独占一行；CI 在 PR 上检查整条分支历史。
