# BiblioSmith Launcher

本仓库的桌面应用，Tauri 2 + React。**安装、首次配置、从源码运行的完整说明在仓库根
目录的 [README.zh-CN.md](../../../README.zh-CN.md)**；这里只写开发这个包本身需要知道
的事。

## 它做什么

- 驱动 Book Pipeline：路由本地 PDF 目录与 Zotero 附件、跑 OCR、把干净 Markdown 交接
  成本地阅读项目，再逐阶段推进拆章、翻译、专家 QA、晋升、出书与校验。
- 在人工闸门处停下等你拍板（`approve_translation` / `approve_promotion`），审批只记录
  一个哈希绑定的决定，不替你执行下一阶段。
- 管理凭证：模型 key 存 macOS 钥匙串，OCR 与向量嵌入各有自己的设置面板。

## 平台

**只构建并测试 macOS（Apple Silicon）**。`.github/workflows/release-launcher.yml`
产出的唯一产物是 DMG，`ci.yml` 的后端作业跑在 `macos-latest`。树里还有 Windows 条件
编译的代码，但没有任何 Windows 产物被构建或验证 —— 不要把它当作受支持的平台。

## 开发

前置：Node.js 20；Rust 由本目录的 `rust-toolchain.toml` 钉在 1.88.0，rustup 会自动
选择，不必手工切换。

```sh
npm ci
npx tauri dev
```

打包（与发布流程同一条命令）：

```sh
npx tauri build --bundles dmg
```

产物在 `src-tauri/target/release/bundle/dmg/`。DMG 是 ad-hoc 签名、未公证的，所以
装完第一次打开必然被 Gatekeeper 拦，处理办法见根 README。

## 测试

```sh
npx tsc --noEmit
npm run test:startup-contract
cd src-tauri && cargo test
```

`cargo test` 不需要先构建前端，测试不读 `dist/`。全仓的套件清单与实测数量见
`CONTRIBUTING.md`。

## 日志

```text
~/Library/Application Support/BiblioSmith/launcher/logs/bibliosmith-launcher.log
```

记录启动期 panic、前端未捕获错误、Python/Java 探测超时，以及私有运行时下载或解压
失败的原因。

## 安全规则

- 不保存明文 API key：模型凭证只进 macOS 钥匙串（服务名
  `com.bibliosmith.launcher.models`），运行引擎时才注入进程环境变量。
- 凭证不进作业状态、不进日志、不进仓库。
- Webhook 端点只从环境读取，载荷不含书名、路径、日志、私有正文或凭证。
- 清理源文件永远只记录审批，绝不由 runner 删除任何源文件。
- 提交信息必须带 `ZH:` / `EN:` / `JA:` 三段，标签独占一行；本地自查用
  `python3 tools/git/check_commit_messages.py --range main..HEAD`。

## 没有的功能

以下都不存在，文档里也不要再声称有：应用内自更新、Windows 安装包、把本仓库当作
「公版书翻译系统」自动下载更新。
