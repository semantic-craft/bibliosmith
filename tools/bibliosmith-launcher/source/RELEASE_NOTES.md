# BiblioSmith Launcher 1.15.0

## ZH

BiblioSmith Launcher 1.15.0 是继 2026-07-29 发布的 1.14.0 之后的一次阶段性版本。本版完成了 Launcher 岛式界面、PDF 智能路由、OCR 对照与 MinerU 精准处理、EPUB 与 BabelDOC 输入链路，以及多项成书和任务恢复修复。

### 界面与工作流

- 将主流程收束为“书架 + 输入岛 + 书籍抽屉 + 设置浮层”。输入岛只保留空闲与处理中两种状态，处理中可直接查看阶段、进度与阻塞原因。
- 书籍抽屉集中展示当前项目、产物与后续动作；移除旧侧边栏、多页向导及与新流程重复的页面。
- 加强多书连续处理与交接：多本书的状态和交接证据不再互相覆盖。

### PDF、OCR 与输入能力

- 新增按书检测 PDF 文本层的智能路由：原生文本 PDF 直接提取；扫描件或文本层质量不足的 PDF 进入 OCR；仍可手动强制指定路线，减少不必要的付费 OCR。
- 新增 OCR 小样对照，可从同一 PDF 的内页抽取样本，比较 PaddleOCR 与 MinerU 结果后再选择整书路线。
- 完成 MinerU v4 精准单书与批量流程；超出 200 页或 200 MB 的输入可分片处理并重组，同时保留明确的进度信息。
- 支持 EPUB 转 Markdown，并新增 BabelDOC 版式保留型双语 PDF 输入链路。
- 扩充按量付费模型支持，包括 Qwen 与 Doubao Responses 兼容接口；补充 Qwen 联网检索、翻译进度与检查点衔接。

### 成书正确性

- 改进 PDF 文本提取、标题归一化、页锚点和页眉页脚清理；不会再用虚构的 “Page N” 条目伪造目录。
- 修复 PaddleOCR、MinerU 与仅翻译交接中的资源保留、文件名冲突、临时产物泄漏和多书串扰。
- 单语与双语 EPUB 现在会保留 fenced code block；内部页锚点继续服务于回溯，但不会出现在可见正文中。
- 修复 PaddleOCR 与 MinerU 边车完成态识别，包括 MinerU 裸整数页码数组；无效或混合页表会整体拒绝，避免误记完整状态或重复 OCR。
- Launcher 测试现在显式隔离本地阅读项目根，不再向真实工作树的 `books/local` 写入测试项目。

### 可靠性与分发

- 强化任务持久化、截止时间、失败恢复、重试与源文件漂移检测；测试中的重试不再依赖真实等待。
- 新建任务不再支持 `conversion_only` 模式；已有旧检查点仍可打开，新任务统一使用受支持的提取或翻译轨道。
- 本版仍仅提供 macOS Apple Silicon DMG。安装包继续使用 Developer ID 签名、Apple 公证与 stapling，并通过 Gatekeeper 验证。
- Launcher 暂不提供应用内自动更新；请从本版本的 GitHub Release 下载 DMG 手动升级。

## EN

BiblioSmith Launcher 1.15.0 is a milestone release following 1.14.0, published on 2026-07-29. It completes the Launcher island interface, smart PDF routing, OCR comparison and precise MinerU processing, EPUB and BabelDOC inputs, and a broad set of book-production and recovery fixes.

### Interface and workflow

- Consolidated the main experience into a shelf, a two-state input island, a book drawer, and a settings overlay. While work is running, the island shows the current phase, progress, and blocking reason.
- Centralized project artifacts and next actions in the book drawer, retiring the former sidebar, multi-page wizard, and duplicate legacy views.
- Improved consecutive multi-book runs and handoff. One book can no longer overwrite another book's state or handoff evidence.

### PDF, OCR, and input support

- Added per-book PDF text-layer probing. Native PDFs use direct extraction, while scans and low-quality text layers are routed to OCR. A route can still be forced explicitly, avoiding unnecessary paid OCR.
- Added an OCR sample comparison that evaluates PaddleOCR and MinerU on the same interior pages before committing to a full-book route.
- Completed precise MinerU v4 single-book and batch workflows. Inputs over 200 pages or 200 MB can be split and reassembled with explicit progress reporting.
- Added EPUB-to-Markdown input and a BabelDOC path for layout-preserving bilingual PDFs.
- Expanded pay-as-you-go provider support with Qwen and Doubao Responses-compatible endpoints, plus Qwen web search, translation progress, and checkpoint integration.

### Book-production correctness

- Improved PDF text extraction, heading normalization, page anchors, and running-header cleanup. The pipeline no longer fabricates “Page N” entries as a table of contents.
- Fixed asset preservation, filename collisions, scratch-artifact leakage, and cross-book interference across PaddleOCR, MinerU, and translate-only handoffs.
- Monolingual and bilingual EPUBs now preserve fenced code blocks. Internal page anchors remain available for traceability without appearing in visible prose.
- Fixed completed-state detection for PaddleOCR and MinerU sidecars, including MinerU's bare integer page arrays. Invalid or mixed page tables are rejected as a whole, preventing false completion and repeat OCR.
- Launcher tests now isolate the local reading project root explicitly and no longer write test projects into the real worktree's `books/local` directory.

### Reliability and distribution

- Strengthened durable task progress, deadlines, failure recovery, retries, and source-drift detection. Retry tests no longer require real sleeping.
- Retired `conversion_only` for new jobs. Existing legacy checkpoints remain readable; new jobs use the supported extraction or translation tracks.
- This release remains a macOS Apple Silicon DMG. The package is Developer ID signed, Apple notarized and stapled, and verified with Gatekeeper.
- In-app updating is not yet available. Download the DMG from this GitHub Release to upgrade manually.

## JA

BiblioSmith Launcher 1.15.0 は、2026-07-29 公開の 1.14.0 に続く節目のリリースです。Launcher のアイランド型 UI、PDF のスマートルーティング、OCR 比較と MinerU の高精度処理、EPUB／BabelDOC 入力、および製本・タスク復旧に関する一連の修正をまとめています。

### インターフェースとワークフロー

- メイン操作を「書棚、2 状態の入力アイランド、ブックドロワー、設定オーバーレイ」に整理しました。処理中は現在の段階、進捗、停止理由を入力アイランドで確認できます。
- プロジェクトの成果物と次の操作をブックドロワーに集約し、従来のサイドバー、複数ページのウィザード、重複していた旧画面を廃止しました。
- 複数書籍の連続処理と引き継ぎを改善しました。別の書籍の状態や引き継ぎ証跡を上書きしません。

### PDF、OCR、入力形式

- 書籍ごとに PDF のテキストレイヤーを検査するスマートルーティングを追加しました。ネイティブ PDF は直接抽出し、スキャン PDF や品質の低いテキストレイヤーは OCR に送ります。ルートの明示指定も可能で、不要な有料 OCR を避けられます。
- 同じ PDF の本文ページを使って PaddleOCR と MinerU を比較し、全書処理の前にルートを選べる OCR サンプル比較を追加しました。
- MinerU v4 の高精度な単書・バッチ処理を完成させました。200 ページまたは 200 MB を超える入力は分割して処理し、進捗を示しながら再結合できます。
- EPUB から Markdown への入力と、レイアウトを維持する BabelDOC の対訳 PDF ルートを追加しました。
- 従量課金プロバイダーとして Qwen と Doubao の Responses 互換エンドポイントに対応し、Qwen のウェブ検索、翻訳進捗、チェックポイント連携も追加しました。

### 製本結果の正確性

- PDF テキスト抽出、見出しの正規化、ページアンカー、柱の除去を改善しました。存在しない「Page N」を目次として生成することはありません。
- PaddleOCR、MinerU、翻訳のみの引き継ぎにおける素材の欠落、ファイル名衝突、一時成果物の混入、書籍間の干渉を修正しました。
- 単言語・対訳 EPUB で fenced code block を保持します。内部ページアンカーは追跡に利用できますが、可視本文には表示されません。
- MinerU の整数だけで構成されたページ配列を含め、PaddleOCR と MinerU のサイドカー完了状態判定を修正しました。不正または混在したページ表は全体を拒否し、誤った完了判定や OCR の再実行を防ぎます。
- Launcher のテストはローカル読書プロジェクトのルートを明示的に隔離し、実際のワークツリーにある `books/local` へテスト用プロジェクトを書き込まなくなりました。

### 信頼性と配布

- タスク進捗の永続化、期限、失敗復旧、再試行、入力元の変化検出を強化しました。再試行テストでは実時間の待機を行いません。
- 新規ジョブでは `conversion_only` モードを廃止しました。既存の旧チェックポイントは引き続き開けますが、新規ジョブは対応済みの抽出または翻訳トラックを使用します。
- 本リリースも macOS Apple Silicon 向け DMG のみです。Developer ID 署名、Apple の公証と stapling を行い、Gatekeeper で検証します。
- アプリ内自動更新にはまだ対応していません。この GitHub Release から DMG をダウンロードして手動で更新してください。
