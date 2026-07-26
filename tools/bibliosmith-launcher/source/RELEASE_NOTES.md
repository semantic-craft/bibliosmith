# BiblioSmith Launcher 1.13.0

## ZH

上一版公开发布是 1.12.0（2026-07-20）。这一版是本项目转为开源之后的第一次发布，累积的改动比版本号跨度看起来要多。

### 新增

- **发车前就告诉你哪些翻译模型没配 key。** 此前模型下拉不看配置状态，选一个从没输过 key 的槽位照样能发车，OCR 跑上十几分钟，最后才在翻译阶段挂在鉴权错误上。现在未配 key 的选项标注「未配置」，确认步显示所选模型并给红色提示，发车按钮置灰。选项仍可选中，因为设置里的活动槽位本身可能没配 key，禁用当前值会把下拉卡死。
- **快速模式新增「反思二遍（更慢，约两倍模型开销）」勾选框。** 这条能力一直全程接通但没有任何界面入口，于是每个快速模式任务都按默认跑第二遍，多出来的开销无从拒绝。
- **门禁样本对照两处改进。** 模型下拉按槽位列全 8 项（Qwen 与 MiMo 各有两种计费方式，第二个槽此前根本选不到，只能靠旁边的自由文本框硬敲），旁边那个文本框随之删除；跑一次样本不再无条件把整本书的正式模型改写成样本用的那个——「试一下再决定」不该替你做决定，采纳改由明确的「以此模型翻译本书」按钮完成。样张模型与全书正式模型不一致时卡片里会红字标出。
- **诊断包可以从应用里导出了。** 书籍抽屉的「高级详情」新增导出一节，三档脱敏可选，写盘后回报路径。默认档 public-issue 只含阶段、状态与错误代码，没有错误摘要、没有工件清单、没有任何路径，可以直接贴进公开 issue。
- **阅读器实测证据可以留底了。** 子作业新增记录：阅读器名称、版本、结论，以及被检产物的类型与 SHA-256。此前这类记录只能手写进 qa/status.md，而校验阶段每次重跑都会重写生成区并把它静默抹掉。
- **阶段失败后会按上限自动重试。** 此前编排层压根没有自动重试，而阶段列表却对每一次失败都无条件标着「可重试」，等于承诺了一个不存在的能力。现在可重试的失败会显示剩余次数与下次重试的倒计时，只重跑失败的那一个阶段；预算耗尽或本就不可重试时给出明确的放弃原因并停住。手动点推进仍然只得一次尝试，不会额外触发自动阶梯。

### 修复

- **从访达启动应用后，流水线各阶段起不来。** macOS 从图形界面启动的 app 只继承 `/usr/bin:/bin:/usr/sbin:/sbin`，Homebrew、uv、nvm、volta、rustup 装的东西都不在里面，于是 `uv` / `node` / `java` 一个都找不到——同样的命令在终端里却跑得好好的。这些程序现在起子进程时统一解析到绝对路径。
- **OCR 与 Zotero 抽取改到 uv 工作区里跑。** 这四个调用点原先用裸解释器跑脚本，而依赖只装在工作区的虚拟环境里，换一台机器必然报模块找不到。双语构建的 python 也改走同一套私有运行时契约，设置页重试运行时之后不会陈旧。
- **用「标题搜索」发车不再排出五个内置演示条目。** 这些假条目到运行阶段必然失败；现在发车和向导预检走同一条真实发现路径，队列里的子任务带的是真实附件 key，向导里设的路由覆写也不再被悄悄丢弃。一本都没搜到时给一条明确的阻塞说明，且该说明在中文界面里不再显示成未翻译的英文串。
- **抽取跑完但 handoff 起不来时，不再丢掉整轮产物。** 此前这一步失败会把刚跑完的转换/OCR 结果连同各子任务状态一起丢弃，磁盘上留下的还是上一次的「抽取中」，同一会话内既卡住又无法重试，重启后重试还得重跑一遍 OCR。现在产物先落盘，抽取停在已完成，只需重跑 handoff 这一步。
- **书架上的阶段进度与后端对齐。** 此前前端只认 12 个阶段，少了「索引」和「生成摘要」；Zotero 附件是唯一会跑逐条索引的路线，一旦索引阶段失败，四步圆圈一个都不变红，书架显示这本书一切正常，实际早就卡住了。
- **向导第二步点 PaddleOCR / MinerU 凭据芯片，不再把路由预览表清空、把「下一步」按钮变灰。** 此前碰一下就走进死胡同，只能退回上一步再进来才能恢复。
- **逐本审批抽屉里的模型服务商下拉列全六个档位**（Kimi，以及各带两个计费槽的 Qwen 和 MiMo），此前写死三个；**同一品牌内换计费槽也不再带着上一个槽位发车**——记忆化的回调此前不跟随计费槽变化。
- **终态通知每个终态只发一条。** 此前身份里掺了时间戳，同一个作业重试后再次失败会算出新的身份，于是投出第二条通知。
- **断点续跑修复。** 改过「翻译时修复段内 OCR / 排版瑕疵」再续跑，此前会重译全部分块，而反思阶段却接着已经不存在的草稿跑，输出混着两遍的结果。
- **项目文档里带查询串的链接**（形如 `?a=1&b=2`）**不再被二次转义**，此前这类链接一律打不开。
- **界面不再把占位值当成事实展示**：项目卡片上编造的「最后更新」时间戳、设置里那个 Windows 味的 `D:\BiblioSmith` 路径都已去掉，取不到真实值时统一显示破折号。

### 移除

- 三个永远只显示破折号的装饰控件已删除：工件表的「打开」整列、流水线页顶部的「今日 OCR 预算」胶囊，以及一条永远不渲染的全局进度分支。它们在暗示一个并不存在的能力，删掉比留着破折号诚实。

### 安全

- markdown 渲染进一步加固；npm 与 cargo 侧的依赖告警已处理；新增版本化的密钥泄漏防护钩子与 CI 密钥扫描。

## EN

The last public release was 1.12.0 (2026-07-20). This is the first release cut after the project went open source, and it carries more than the version span suggests.

### Added

- **The launcher now says which translation models have no key before you launch.** The model dropdown previously ignored configuration state, so a slot that had never been given a key could be selected and queued; OCR then ran for a quarter of an hour before the job died on a provider authentication error in the translate stage. Unconfigured options are now labelled, the confirm step names the chosen model and warns in red, and the launch button is disabled. Options stay selectable, because the active slot chosen in Settings may itself be unconfigured and disabling the current value would jam the dropdown.
- **Fast mode gains a "Reflection second pass (slower, roughly double the model spend)" checkbox.** The capability was wired end to end but had no control anywhere, so every fast-mode job ran the second pass at its default and the extra spend could not be declined.
- **Two improvements to the gate sample.** The model dropdown now lists all eight slots (Qwen and MiMo each have two billing arrangements, and the second slot could not be picked at all — it had to be typed into a free-text box beside it, which is now gone). And running a sample no longer rewrites the book's real provider to whatever the sample used: "try it before deciding" should not decide for you, so adopting is now an explicit "translate this book with this model" button. When the sample's model differs from the book's real one, the card says so in red.
- **The diagnostic bundle has a way out of the app.** The book drawer's advanced details gain an export section with three redaction profiles, reporting the path it wrote. The default public-issue profile carries only stages, statuses and error codes — no error summaries, no artifact listings, no paths — so it can be pasted into a public issue as-is.
- **Reader-device evidence can be recorded against a built book**: reader name, version, verdict, and the checked artifact's kind and SHA-256. Until now such a record could only be hand-written into qa/status.md, where the validation stage silently overwrote it on every re-run.
- **A failed stage is retried automatically, up to a budget.** The orchestrator had no automatic retry at all, while the stage list labelled every failure "retryable" regardless — promising a capability that did not exist. A retryable failure now shows the attempts left and a countdown to the next one, and re-runs only the stage that failed; when the budget is spent, or the failure was never retryable, it stops with an explicit reason for giving up. Advancing by hand still buys exactly one attempt and cannot start an automatic ladder.

### Fixed

- **Pipeline stages failed to start when the app was launched from Finder.** A macOS app started from the GUI inherits only `/usr/bin:/bin:/usr/sbin:/sbin`, which contains nothing Homebrew, uv, nvm, volta or rustup installs into, so `uv`, `node` and `java` did not resolve at all — even though the identical command works from a terminal. These programs are now resolved to an absolute path when the subprocess is spawned.
- **The OCR and Zotero extraction line now runs inside the uv workspace.** Those four call sites invoked their scripts through a bare interpreter while the dependencies live only in the workspace virtual environment, so any other machine hit a missing-module error. The bilingual build's python goes through the same private runtime contract and no longer goes stale after retrying the runtime from Settings.
- **Launching from "title search" no longer queues the five built-in demo entries.** Those fixtures were guaranteed to fail at run time; queueing now goes through the same real discovery path the wizard preview uses, so queued children carry real attachment keys and route overrides set in the wizard are no longer silently dropped. A search that matches nothing yields one explicit blocked row, and that row is no longer rendered as an untranslated English string in the Chinese interface.
- **A handoff that cannot start no longer discards the whole extraction.** The failure used to throw away the conversion or OCR output that had just finished along with every child's stage state, leaving "extracting" on disk from the previous save: stuck within the session, un-retryable, and needing a full OCR re-run after a restart. Output is now saved first, extraction stays completed, and only the handoff step needs re-running.
- **Stage progress on the shelf now matches the backend.** The frontend listed only 12 stages, missing "index" and "build digest"; Zotero attachments are the one route that runs a per-item index, and when that stage failed none of the four circles turned red, so the shelf reported the book as fine while it was stuck.
- **Clicking the PaddleOCR / MinerU credential chips on step 2 of the wizard no longer empties the route preview and greys out "Next"** — until now a single click was a dead end recoverable only by stepping back and forward again.
- **The provider dropdown in the per-book approval drawer lists all six configured entries** (Kimi, plus Qwen and MiMo with two billing slots each) instead of a hardcoded three, and **switching billing slots within one brand no longer queues under the previous slot** — the memoised callback did not follow that field.
- **Terminal notifications fire once per terminal state.** The identity used to fold in a timestamp, so a job that failed, was retried and failed again computed a fresh identity and delivered a second notification.
- **Resume fix.** Changing "Fix within-paragraph OCR / layout defects while translating" and resuming an interrupted run used to re-translate every chunk while the reflection pass resumed against drafts that no longer existed, blending two passes into the output.
- **Links with a query string in project documents** (`?a=1&b=2`) **are no longer escaped twice.** Every such link was broken.
- **The UI no longer presents placeholder values as facts**: the invented "last updated" timestamp on the project card and the Windows-flavoured `D:\BiblioSmith` path in Settings are gone, and an em-dash is shown when there is no real value to report.

### Removed

- Three decorative controls that only ever rendered an em-dash are gone: the artifact table's "open" column, the "OCR budget today" pill at the top of the pipeline page, and a global progress branch that never rendered. They implied a capability that does not exist, and removing them is more honest than leaving the dashes.

### Security

- Further hardening of markdown rendering; npm and cargo advisories addressed; versioned leak-prevention hooks and a CI secret scan added.

## JA

前回の公開リリースは 1.12.0（2026-07-20）でした。本リリースはプロジェクトをオープンソース化してから最初のもので、バージョン番号の差から想像されるより多くの変更を含みます。

### 追加

- **投入前に、どの翻訳モデルにキーが設定されていないかを表示するようになりました。** これまでモデルの選択欄は設定状況を見ておらず、キーを一度も入力していないスロットを選んだまま投入でき、OCR が十数分走った末に翻訳段階でプロバイダーの認証エラーとして失敗していました。未設定の選択肢には印が付き、確認手順では選択したモデルを表示して赤字で警告し、投入ボタンを無効にします。設定画面で選ばれている有効スロット自体が未設定である場合もあるため、選択肢自体は選べるままにしています。現在値を無効化すると選択欄が固まってしまうためです。
- **高速モードに「リフレクション 2 回目（より遅く、モデル費用は約 2 倍）」のチェックボックスを追加しました。** この機能は端から端まで配線済みでしたが操作 UI がなく、高速モードのジョブは既定のまま常に 2 回目を実行し、追加費用を断ることができませんでした。
- **ゲート用サンプルの 2 点を改善しました。** モデルの選択欄が 8 スロットすべてを列挙します（Qwen と MiMo は課金方式が 2 つあり、2 つ目のスロットはこれまで選択できず、隣の自由入力欄に手で書くしかありませんでした。その入力欄は削除しました）。またサンプルの実行が、その書籍の正式なプロバイダーをサンプル用のものへ無条件に書き換えることはなくなりました。「試してから決める」操作が代わりに決めてしまうべきではないためで、採用は「このモデルで本書を翻訳する」ボタンによる明示的な操作になりました。サンプルのモデルが本書の正式なモデルと異なる場合は、カード上に赤字で示されます。
- **診断バンドルをアプリから書き出せるようになりました。** 書籍ドロワーの詳細情報に書き出しの節を追加し、3 段階の秘匿レベルを選べます。書き出し後はパスを表示します。既定の public-issue は、ステージ・状態・エラーコードのみを含み、エラーの要約も成果物の一覧もパスも持たないため、そのまま公開 issue に貼れます。
- **リーダー実機での確認結果を記録できるようになりました。** リーダー名、バージョン、判定に加え、検査した成果物の種別と SHA-256 を残します。これまでは qa/status.md に手書きするしかなく、検証ステージの再実行のたびに生成領域が書き直されて静かに消えていました。
- **失敗したステージを上限付きで自動再試行するようになりました。** これまでオーケストレーション側に自動再試行は一切なく、それでいてステージ一覧はすべての失敗に「再試行可能」と表示しており、存在しない機能を約束していました。再試行可能な失敗では残り回数と次回までのカウントダウンを表示し、失敗したステージだけを再実行します。予算を使い切った場合や、そもそも再試行できない failure の場合は、断念の理由を明示して停止します。手動での前進は従来どおり 1 回の試行のみで、自動的な再試行の連鎖は始まりません。

### 修正

- **Finder からアプリを起動するとパイプラインの各ステージが開始できない問題を修正しました。** GUI から起動した macOS アプリは `/usr/bin:/bin:/usr/sbin:/sbin` しか引き継がず、Homebrew・uv・nvm・volta・rustup の導入先が含まれないため、`uv` / `node` / `java` がまったく解決できませんでした（同じコマンドがターミナルでは動作します）。これらはサブプロセス起動時に絶対パスへ解決するようになりました。
- **OCR と Zotero の抽出を uv のワークスペース内で実行するようにしました。** これら 4 か所は素のインタープリターでスクリプトを起動していましたが、依存はワークスペースの仮想環境にしかないため、別のマシンでは必ずモジュール未検出になっていました。二言語版の構築で使う python も同じ専用ランタイムの取り決めを通すようにし、設定画面でランタイムを再試行したあとも古い解決結果が残りません。
- **「タイトル検索」からの投入で、5 件の組み込みデモ項目が並ぶことはなくなりました。** これらのダミーは実行時に必ず失敗していました。現在はウィザードのプレビューと同じ実探索の経路を通るため、キューの子ジョブは実在する添付キーを持ち、ウィザードで指定した経路の上書きも失われません。1 件も見つからない場合は明示的なブロック行を返し、その行が中国語表示で未翻訳の英語のまま出ることもなくなりました。
- **抽出が終わったあとに handoff を開始できなかった場合でも、その回の成果物を丸ごと失うことはなくなりました。** 以前はこの失敗により、直前に終わった変換・OCR の成果物と各子ジョブの状態がまとめて破棄され、ディスク上には前回保存時の「抽出中」が残り、同一セッション内では停止したまま再試行もできず、再起動後の再試行でも OCR からやり直しになっていました。現在は先に成果物を保存し、抽出は完了のまま、handoff の手順だけを再実行すれば済みます。
- **本棚のステージ進行がバックエンドと一致するようになりました。** フロントエンドは 12 ステージしか認識せず「索引」と「ダイジェスト生成」が欠けていました。項目単位の索引を実行するのは Zotero 添付の経路だけで、そのステージが失敗しても 4 つの丸は赤くならず、実際には停止しているのに本棚は正常と表示していました。
- **ウィザードのステップ 2 で PaddleOCR / MinerU の資格情報チップを押しても、ルートプレビューが空になって「次へ」が無効化されることはなくなりました。** 従来は一度押すと行き止まりで、前の手順に戻ってから進み直すしか復帰方法がありませんでした。
- **書籍ごとの承認ドロワーのプロバイダー選択に、設定済みの 6 件すべて**（Kimi と、課金スロットを 2 つ持つ Qwen・MiMo）**が並ぶようになりました**。従来は 3 件が固定でした。あわせて、**同一ブランド内で課金スロットを切り替えても、以前のスロットのまま投入されることがなくなりました** —— 記憶化されたコールバックがその項目の変化に追随していませんでした。
- **終了状態の通知は、終了状態ごとに 1 通だけ送られます。** 以前は識別子に時刻が混ざっており、失敗して再試行し再び失敗したジョブが新しい識別子を算出して 2 通目を送っていました。
- **再開時の修正。**「翻訳時に段落内の OCR / レイアウトの不備を修正」を変更して中断したジョブを再開すると、全チャンクを翻訳し直す一方で、リフレクションは既に存在しない下書きに対して再開され、2 つのパスが混ざった出力になっていました。
- **プロジェクト文書内のクエリ文字列付きリンク**（`?a=1&b=2`）**が二重エスケープされることはなくなりました。** この形式のリンクはすべて開けませんでした。
- **プレースホルダーを事実として表示しなくなりました。** プロジェクトカードの作り物の「最終更新」時刻と、設定に出ていた Windows 風のパス `D:\BiblioSmith` を削除し、実際の値が取得できない場合はダッシュを表示します。

### 削除

- ダッシュしか表示しない装飾的なコントロール 3 つを削除しました。成果物一覧の「開く」列、パイプライン画面上部の「本日の OCR 予算」バッジ、そして描画されることのない全体進捗の分岐です。存在しない機能があるかのように見せていたため、ダッシュを残すより削除するほうが誠実だと判断しました。

### セキュリティ

- markdown レンダリングをさらに強化し、npm と cargo の脆弱性勧告に対応し、バージョン管理された漏洩防止フックと CI のシークレット走査を追加しました。
