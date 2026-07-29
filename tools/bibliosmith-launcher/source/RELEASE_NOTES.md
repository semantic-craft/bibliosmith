# BiblioSmith Launcher 1.14.0

## ZH

上一版公开发布是 1.13.0（2026-07-26）。1.14.0 是第一版使用 Developer ID 正式签名、经 Apple 公证并装订的 BiblioSmith Launcher，同时收录了上一版之后已经合入的流水线交互与可靠性改进。

### 新增

- **被拦截的书可以原地改路由。** 遇到脏文本层或缺少凭据时，可直接选择 MinerU、PaddleOCR 或重新自动判断，无需删除后重新发车；选择会持久化，重启后仍然有效。
- **合集可以只从书架移除一本书。** 合集成员使用墓碑记录，既不破坏原始成员快照，也不会把同批其他书一起移除；移除最后一本时才删除整个批次。
- **阅读器实测记录现在有完整界面。** 工件页可以记录阅读器、版本、结论以及被检产物；记录绑定产物哈希，产物变化后会明确标记失效。
- **源文件清理审批现在可达、可审计。** 设置页会列出候选和逐项证据，只有阅读产物已验证且哈希匹配时才能批准；审批以结构化记录持久化，相关手动清理脚本会在删除源 PDF 前核验。

### 修复与改进

- **长时间流水线不再占住应用主线程。** 后端命令改为异步调度，翻译、OCR 与转换子进程都有明确但足够长的上限；卡死会返回可辨认的超时错误。
- **大量输出不会再被误判为超时。** stdout 与 stderr 在子进程运行期间持续抽取，长书完成后写出较大报告也不会因管道塞满而被杀掉。
- **Zotero 实时发现的交接预览补全。** 需要翻译交接时，向导现在会显示相应步骤，不再少报后续工作。
- **阅读产物统一写入 `output/reading/`。** HTML、单语 EPUB、双语 EPUB 与 Digest 不再分散在旧目录；没有旧式 `book.yaml` 的本地阅读项目会从正式的 `metadata/source_manifest.json` 读取来源、语言与标题信息。
- **跨书籍和跨项目的旧状态不再短暂串台。** 样本摘录、模型选择、教程文档和删除确认都跟随当前对象重建；React hooks 检查同时升级到完整预设。
- **更新中心不再声称执行并不存在的在线检查。** 原来的占位检查改为直接打开本仓 Releases 页面。

### 移除

- 删除没有任何界面入口的 OpenCode 集成，以及只会生成占位 Markdown、并未调用翻译引擎的 reflectionTranslation 路径。

### 安全与分发

- App 和 DMG 均使用 Developer ID Application 签名并分别提交 Apple 公证；公证票据已装订，发布前会挂载最终 DMG，重新执行签名、Gatekeeper、stapler 与镜像完整性验证。
- 加密的 PKCS#12 证书只在 GitHub Actions 的临时钥匙串中导入并于任务结束后删除。Apple App 专用密码可通过 macOS 隐藏输入弹窗写入 GitHub Secret，不进入命令参数、终端历史或日志。

## EN

The previous public release was 1.13.0 (2026-07-26). Version 1.14.0 is the first BiblioSmith Launcher release signed with Developer ID, notarized by Apple, and stapled for distribution. It also includes the pipeline interaction and reliability work merged since the previous release.

### Added

- **A held book can be re-routed in place.** When a dirty text layer or missing credential blocks a book, you can choose MinerU, PaddleOCR, or automatic routing again without deleting and re-queueing it. The choice persists across restarts.
- **One book can be removed from a collection batch.** A tombstone keeps the original membership snapshot intact while hiding only the chosen book; the whole batch is deleted only when its final book is removed.
- **Reader-device evidence now has a complete UI.** The Artifacts tab records the reader, version, verdict, and checked artifact. Records are bound to the artifact hash and are visibly invalidated when the artifact changes.
- **Source-file cleanup approval is reachable and auditable.** Settings lists candidates and each required check. Approval is allowed only for validated reading artifacts with matching hashes, is stored as a structured record, and is checked by the relevant manual cleanup scripts before a source PDF is deleted.

### Fixed and improved

- **Long pipeline work no longer occupies the app main thread.** Backend commands are scheduled asynchronously, and translation, OCR, and conversion children have explicit but generous limits. A hung child returns a recognizable timeout error.
- **Talkative children are no longer mistaken for hung ones.** stdout and stderr are drained while the process runs, so a long book can emit a large final report without filling a pipe and being killed as a timeout.
- **Live Zotero discovery shows the complete handoff preview.** When translation handoff is required, the wizard now reports that step instead of understating the work to come.
- **Reading artifacts now live consistently under `output/reading/`.** HTML, monolingual EPUB, bilingual EPUB, and Digest outputs are no longer scattered across legacy locations. Local-reading projects without the old `book.yaml` read source, language, and title metadata from the canonical `metadata/source_manifest.json` contract.
- **State from the previous book or project no longer flashes into the current one.** Sample excerpts, model selection, guide documents, and deletion confirmation are rebuilt around the active item; React hooks checking now uses the complete preset.
- **The Updates page no longer claims to perform an online check that did not exist.** The placeholder control is replaced by a direct link to this repository's Releases page.

### Removed

- Removed the unreachable OpenCode integration and the reflectionTranslation path that created placeholder Markdown without ever calling the translation engine.

### Security and distribution

- Both the app and DMG are signed with Developer ID Application and submitted separately for Apple notarization. Tickets are stapled, and the final DMG is mounted before publication so signing, Gatekeeper, stapler, and image integrity are checked against the artifact users receive.
- The encrypted PKCS#12 certificate is imported only into a temporary GitHub Actions keychain and deleted when the job finishes. The Apple app-specific password can be written to GitHub Secrets through a hidden macOS prompt without entering process arguments, shell history, or logs.

## JA

前回の公開リリースは 1.13.0（2026-07-26）でした。1.14.0 は、Developer ID で正式に署名し、Apple の公証を受け、チケットを staple した最初の BiblioSmith Launcher です。前回以降に統合されたパイプライン操作と信頼性の改善も含みます。

### 追加

- **保留された書籍をその場で再ルーティングできます。** 汚れたテキストレイヤーや資格情報不足で停止した場合、削除して再投入せずに MinerU、PaddleOCR、または自動判定を選び直せます。選択は再起動後も保持されます。
- **コレクションのバッチから 1 冊だけを取り除けます。** 墓石記録により元のメンバーシップのスナップショットを保ったまま選んだ書籍だけを非表示にし、最後の 1 冊を取り除いたときだけバッチ全体を削除します。
- **リーダー実機の検証記録に完全な UI が付きました。** 成果物タブでリーダー、バージョン、判定、検査した成果物を記録できます。記録は成果物のハッシュに結び付けられ、成果物が変わると明示的に無効と表示されます。
- **ソースファイル削除の承認が操作可能かつ監査可能になりました。** 設定画面に候補と各証拠を表示し、検証済みの reading 成果物とハッシュが一致する場合だけ承認できます。承認は構造化記録として永続化され、該当する手動クリーンアップスクリプトはソース PDF を削除する前に確認します。

### 修正と改善

- **長時間のパイプライン処理がアプリのメインスレッドを占有しなくなりました。** バックエンドコマンドを非同期に実行し、翻訳・OCR・変換の子プロセスに明示的で十分に長い上限を設けました。停止したプロセスは識別可能なタイムアウトエラーを返します。
- **出力の多い子プロセスを停止中と誤判定しなくなりました。** 実行中に stdout と stderr を継続して読み取るため、長い書籍が大きな最終レポートを出してもパイプが詰まってタイムアウト扱いされません。
- **Zotero のライブ検出で交接プレビューが完全になりました。** 翻訳への引き渡しが必要な場合、ウィザードがその手順を表示し、後続作業を少なく見積もることがなくなりました。
- **閲覧成果物を `output/reading/` 配下へ統一しました。** HTML、単言語 EPUB、対訳 EPUB、Digest が旧来の場所に分散しなくなりました。旧 `book.yaml` のないローカル閲覧プロジェクトでは、正式な `metadata/source_manifest.json` 契約からソース、言語、タイトルの情報を読み取ります。
- **前の書籍やプロジェクトの状態が現在の画面に一瞬混ざることがなくなりました。** サンプル抜粋、モデル選択、ガイド文書、削除確認は現在の対象に合わせて再構築され、React hooks の検査も完全なプリセットへ更新しました。
- **存在しないオンライン更新確認を行ったと表示しなくなりました。** 更新画面のプレースホルダー操作を、このリポジトリの Releases ページへの直接リンクに置き換えました。

### 削除

- UI から到達できなかった OpenCode 連携と、翻訳エンジンを呼ばずにプレースホルダー Markdown だけを作っていた reflectionTranslation 経路を削除しました。

### セキュリティと配布

- App と DMG の両方を Developer ID Application で署名し、それぞれ Apple 公証へ提出します。公証チケットを staple し、公開前に最終 DMG をマウントして、ユーザーが受け取る成果物そのものに対して署名、Gatekeeper、stapler、イメージ整合性を再検証します。
- 暗号化された PKCS#12 証明書は GitHub Actions の一時キーチェーンにだけ読み込み、ジョブ終了時に削除します。Apple のアプリ用パスワードは macOS の非表示入力ダイアログから GitHub Secret に保存でき、プロセス引数、シェル履歴、ログには入りません。
