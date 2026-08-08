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
- 自更新：启动时后台检查一次新版本，是否安装由用户在设置里决定；见下面的「自动更新」。

## 平台

**只构建并测试 macOS（Apple Silicon）**。`.github/workflows/release-launcher.yml`
产出的是 DMG 加一份同版本的更新包（`.app.tar.gz` + `.sig` + `latest.json`），
`ci.yml` 的后端作业跑在 `macos-latest`。更新清单里只有 `darwin-aarch64` 一个平台条目，
其它平台上的 Launcher 会得到「没有更新」而不是一个跑不起来的包。树里还有 Windows
条件编译的代码，但没有任何 Windows 产物被构建或验证 —— 不要把它当作受支持的平台。

## 开发

前置：Node.js 20；Rust 由本目录的 `rust-toolchain.toml` 钉在 1.88.0，rustup 会自动
选择，不必手工切换。

```sh
npm ci
npx tauri dev
```

本地打包（不会注入发布 Secrets）：

```sh
npx tauri build --bundles dmg --no-sign
```

`--no-sign` 现在同时跳过两件事：Apple 代码签名，以及更新包的 minisign 签名。
`tauri.conf.json` 里配了 `plugins.updater.pubkey` 之后，只要构建 `app` 产物且没给
`--no-sign`，Tauri 就要求环境里有 `TAURI_SIGNING_PRIVATE_KEY`，否则直接构建失败 ——
这是故意的，宁可打不出来也不要发一个装不上的未签名更新包。持有私钥时这样打带更新
包的本地构建：

```sh
TAURI_SIGNING_PRIVATE_KEY="$(cat ~/.tauri/bibliosmith-launcher.key)" \
TAURI_SIGNING_PRIVATE_KEY_PASSWORD="" \
npx tauri build --bundles app dmg
```

产物在 `src-tauri/target/release/bundle/dmg/`。配置默认指向
`Developer ID Application`；没有证书的普通开发机使用上面的 `--no-sign`。持有证书的
发布维护者可以去掉该参数，或用 `APPLE_SIGNING_IDENTITY` 指定精确身份。正式 Release
会先从 Secrets 导入证书，再由 Tauri 完成 Apple 公证和 staple，并在发布前验证
Gatekeeper 将应用识别为 `Notarized Developer ID`。

更新 Apple App 专用密码时运行：

```sh
./scripts/set-apple-password-secret-macos.sh
```

脚本通过 macOS 隐藏输入框接收密码，再直接写入 `semantic-craft/bibliosmith` 的
`APPLE_PASSWORD` GitHub Secret，不会把密码放进 shell 历史或命令参数；留空或取消
不会改动现有 Secret。

## 自动更新

装好的 Launcher 每次启动会在后台向 GitHub Release 问一次有没有新版本。发现新版本
时只做两件事：弹一条提示，并在设置齿轮上留一个小圆点；**下载和安装都要用户在设置
里点。** 不静默安装，也不在退出时偷偷替换——安装会整包替换 App，而流水线任务正在
用这个包里的 Python、Node 和 Chromium，所以只要还有任务在跑，安装按钮就是禁用的，
并写明原因。

信任链是两条独立的签名，谁也替不了谁：

- **Apple Developer ID 签名 + 公证**：用户双击打开时 Gatekeeper 查的是这条。
- **minisign 签名**：装好的 Launcher 替换自己之前查的是这条，用编译进当前版本的公钥
  校验更新包。所以即使有人换掉了 Release 里的文件、或者更新端点被劫持，装不上去。

对应的密钥与 Secret：

| 东西 | 位置 |
| --- | --- |
| 公钥 | `src-tauri/tauri.conf.json` 的 `plugins.updater.pubkey`，随代码提交 |
| 私钥 | 维护者本地的 `~/.tauri/bibliosmith-launcher.key`，**不进仓库** |
| CI 私钥 | GitHub Secret `TAURI_SIGNING_PRIVATE_KEY` |
| CI 私钥密码 | GitHub Secret `TAURI_SIGNING_PRIVATE_KEY_PASSWORD`（没设密码就是空串，但 Secret 必须存在） |

首次配置或轮换密钥时运行：

```sh
./scripts/set-updater-signing-secret-macos.sh
```

**私钥丢了就没有回头路**：装在用户机器上的 Launcher 只认它当初编译进去的那个公钥，
换新密钥意味着所有老用户的自动更新永久失效，只能让他们手动重新下载一次 DMG。备份
好 `~/.tauri/bibliosmith-launcher.key`。

发布流水线相应多了四步，都在 `release-launcher.yml` 里：构建时一并产出 `app` 与更新
包、解开更新包确认里面的 App 确实已公证且 stapled、由构建产物生成 `latest.json`、
发布后再真的去请求一次更新端点确认它返回的是这一版。最后一步是有意的：端点 404 在
所有已安装的 Launcher 看来和「没有新版本」完全一样，不主动验就永远不会有人发现。

## 测试

```sh
npx tsc --noEmit
npm test
npm run test:startup-contract
cd src-tauri && cargo test
```

`cargo test` 不需要先构建前端，测试不读 `dist/`。全仓的套件清单与实测数量见
`CONTRIBUTING.md`。

### cargo test 反复弹钥匙串密码

`tauri.conf.json` 的 `signingIdentity` 只作用于 `tauri build` 打出的 bundle，管不到
`cargo test` 与 `tauri dev` 直接跑的 rustc 二进制——那些是 ad-hoc 签名，designated
requirement 是内容 cdhash，每编一次就变，所以「Always Allow」只对当次构建有效。

持有 Apple Development 证书的话，用固定 bundle identifier 重签一次即可：

```sh
./scripts/codesign-dev-macos.sh
```

之后在钥匙串里对 `com.bibliosmith.launcher.models` 各条目 Always Allow 一次，重编不再
失效。可用 `BIBLIOSMITH_CODESIGN_IDENTITY` 指定证书。没有证书就跳过这节——弹窗只影响
本地开发体验，不影响测试结果。

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

以下都不存在，文档里也不要再声称有：应用内自更新、Windows 安装包、自动下载或更新
本地阅读工作台仓库。
