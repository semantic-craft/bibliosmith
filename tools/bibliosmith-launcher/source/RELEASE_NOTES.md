# BiblioSmith Launcher 1.16.2

## ZH

BiblioSmith Launcher 1.16.2 将翻译提示词、出版结构和独立运行环境三条链路合并为一套可复核的本地成书流程。本版同时纳入最新安全依赖更新和 macOS 随包运行时签名修复。

### 翻译提示词方案

- 新增统一的翻译提示词方案库，内置“结构保真翻译”“四维反思精修”“语境回溯精译”和“全流程审校闭环”四套按功能命名的方案。
- 内置方案保持只读；可复制为本地方案并以不可变、追加式修订继续编辑。可按执行器设置默认方案、按书覆盖，并显式采用新的内置修订。
- 同时提供方案模板与本次实际提示词预览。样章、全书执行、专家交接和审批均绑定准确的方案 ID、修订与内容哈希。
- 实际提示词只在运行时编译，不写入日志或持久化作业状态；旧 `customInstructions` 路径已移除，专家代理结果须通过证据链校验。

### 出版结构与阅读输出

- 将出版结构与内部翻译分块彻底分离。PDF 与 EPUB 的来源证据会统一编译为可审计、可持久化的 Publication Map。
- `build_reading` 现在按出版章节和小节生成语义化 HTML/EPUB，内部翻译块不会再泄漏到目录或读者导航。
- 学术书的注释与回跳关系贯穿来源、翻译和 EPUB 成书链路，并保留结构化语义。
- 将包合法性、结构可读性和可选的真实阅读器验收拆成独立证据层，能够区分“文件可打开”与“成书结构正确”。
- 来源图、出版图、提示词预览、审批、提升、成书和验证现在按精确哈希绑定，防止预览竞争或陈旧结构产物被误用。

### 独立运行、设置与安全

- 分离只读应用资源、可变应用状态与用户书库工作区；正式 App 不再依赖开发仓库路径，并打包经过校验的 Node、uv 与 Python 运行时输入。
- 修复阅读输出打开流程；书架可批量移除条目而保留本地项目文件。
- 翻译模型和 OCR 模型设置恢复为下拉选择器，一次只显示当前配置；OCR 凭据继续只存于 macOS Keychain。
- 将 `cryptography` 升级到 50.0.0，纳入上游安全修复。
- 随包的 Playwright Chromium、FFmpeg 与动态库现在逐一使用 Developer ID 和 Hardened Runtime 签名，仅浏览器主程序获得最小 JIT 权限；发布验收会启动包内 Chromium 并实际执行 JavaScript。
- 本版仍仅提供 macOS Apple Silicon DMG。安装包使用 Developer ID 签名、Apple 公证与 stapling，并通过 Gatekeeper 验证；请从本 Release 手动下载升级。

## EN

BiblioSmith Launcher 1.16.2 brings prompt management, publication structure, and a self-contained runtime into one auditable local book-production workflow. It also includes the latest security dependency update and a macOS bundled-runtime signing fix.

### Translation prompt packs

- Added a unified prompt-pack library with four functionally named built-ins: Structure-Faithful Translation, Four-Dimension Reflection, Context-Retrieval Translation, and Full-Process Quality Loop.
- Built-in packs remain read-only. They can be copied into local packs with immutable, append-only revisions, selected as executor defaults, overridden per book, and explicitly advanced to a newer built-in revision.
- Added both template and actual-prompt previews. Samples, full-book runs, expert handoffs, and approvals bind the exact pack ID, revision, and content hash.
- Actual prompts are compiled only at runtime and are not persisted in logs or job state. The legacy `customInstructions` path is removed, and expert-agent results require evidence-chain validation.

### Publication structure and reading output

- Fully separated publication structure from internal translation chunks. PDF and EPUB producer evidence is compiled into one durable, auditable Publication Map.
- `build_reading` now generates semantic HTML and EPUB from publication chapters and sections, keeping internal translation chunks out of the table of contents and reader navigation.
- Academic notes and backlinks retain structured semantics from source through translation to the final EPUB.
- Package validity, structural readability, and optional real-reader acceptance are now separate evidence layers, distinguishing “opens successfully” from “is structured correctly.”
- Source maps, publication maps, prompt previews, approvals, promotion, builds, and validation are bound by exact hashes, preventing preview races and stale structure artifacts.

### Self-contained runtime, settings, and security

- Separated read-only app resources, mutable app state, and the user's library workspace. The packaged app no longer depends on a development checkout and carries validated Node, uv, and Python runtime inputs.
- Fixed opening reading outputs and added bulk shelf removal without deleting local project files.
- Restored translation-model and OCR-model dropdowns with one active configuration at a time. OCR credentials remain stored only in the macOS Keychain.
- Upgraded `cryptography` to 50.0.0, incorporating upstream security fixes.
- Bundled Playwright Chromium, FFmpeg, and dynamic libraries are now individually Developer ID signed with Hardened Runtime. Only the browser executable receives the minimum JIT entitlement, and release acceptance launches the packaged Chromium to execute real JavaScript.
- This release remains a macOS Apple Silicon DMG. It is Developer ID signed, Apple notarized and stapled, and Gatekeeper verified. Download the DMG from this Release to upgrade manually.

## JA

BiblioSmith Launcher 1.16.2 は、翻訳プロンプト管理、出版構造、自己完結型ランタイムを、一つの監査可能なローカル製本フローへ統合します。最新のセキュリティ依存関係更新と macOS 同梱ランタイム署名の修正も含みます。

### 翻訳プロンプトパック

- 機能名で整理した四つの内蔵方式「構造忠実翻訳」「四次元リフレクション」「文脈回溯翻訳」「全工程品質ループ」を備える統合プロンプトパックライブラリを追加しました。
- 内蔵パックは読み取り専用です。ローカルパックへ複製し、不変かつ追記型のリビジョンとして編集できます。実行系の既定値、書籍別上書き、新しい内蔵リビジョンの明示採用にも対応します。
- テンプレートと今回の実プロンプトをそれぞれプレビューできます。サンプル、全書実行、専門家への引き継ぎ、承認は、正確なパック ID、リビジョン、内容ハッシュに結合されます。
- 実プロンプトは実行時にのみコンパイルし、ログやジョブ状態には保存しません。旧 `customInstructions` 経路を撤去し、専門家エージェントの結果には証拠チェーン検証を要求します。

### 出版構造と読書用成果物

- 出版構造と内部翻訳チャンクを完全に分離しました。PDF／EPUB の生成元証拠を、永続的で監査可能な一つの Publication Map に統合します。
- `build_reading` は出版上の章・節から意味構造を持つ HTML／EPUB を生成し、内部翻訳チャンクを目次や読者ナビゲーションへ露出しません。
- 学術書の注釈と戻りリンクは、原文から翻訳、最終 EPUB まで構造化された意味関係を保持します。
- パッケージ妥当性、構造上の可読性、任意の実読書アプリ受入れを別々の証拠層に分け、「開ける」と「正しい本構造」を区別します。
- Source Map、Publication Map、プロンプトプレビュー、承認、昇格、製本、検証を正確なハッシュで結合し、プレビュー競合や古い構造成果物の誤用を防ぎます。

### 自己完結型ランタイム、設定、セキュリティ

- 読み取り専用アプリ資源、可変アプリ状態、ユーザーの書庫ワークスペースを分離しました。配布 App は開発用チェックアウトに依存せず、検証済みの Node、uv、Python ランタイム入力を同梱します。
- 読書成果物を開く処理を修正し、ローカルのプロジェクトファイルを残したまま書棚から一括削除できるようにしました。
- 翻訳モデルと OCR モデルの設定をドロップダウンへ戻し、一度に一つの設定だけを表示します。OCR 認証情報は引き続き macOS Keychain のみに保存します。
- `cryptography` を 50.0.0 へ更新し、上流のセキュリティ修正を取り込みました。
- 同梱する Playwright Chromium、FFmpeg、動的ライブラリを Developer ID と Hardened Runtime で個別に署名します。JIT 権限はブラウザー本体だけに最小限付与し、リリース受入れでは同梱 Chromium を起動して実際に JavaScript を実行します。
- 本リリースも macOS Apple Silicon 向け DMG のみです。Developer ID 署名、Apple 公証、stapling、Gatekeeper 検証を行います。この Release から DMG を手動でダウンロードして更新してください。
