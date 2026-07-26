# v1.12.1

## ZH

- 用「标题搜索」发车不再排出五个内置演示条目（`-DIRECT` / `-SCAN` / `-MINERU` / `-DIRTY` / `-DONE`）。这些假条目到运行阶段必然失败；现在发车和向导预检走同一条真实发现路径，队列里的子任务带的是真实附件 key，向导里设的路由覆写也不再被悄悄丢弃，一本都没搜到时给出一条明确的阻塞说明而不是兜底假数据。
- 修复从访达启动应用后流水线各阶段起不来的问题。macOS 从图形界面启动的 app 只继承 `/usr/bin:/bin:/usr/sbin:/sbin`，Homebrew、uv、nvm、volta、rustup 装的东西都不在里面，于是 `uv` / `node` / `java` 一个都找不到——同样的命令在终端里却跑得好好的。现在这些程序在起子进程时统一解析到绝对路径。
- 书架上的阶段进度与后端对齐。此前前端只认 12 个阶段，少了「索引」和「生成摘要」；Zotero 附件是唯一会跑逐条索引的路线，一旦索引阶段失败，四步圆圈一个都不变红，书架显示这本书一切正常，实际早就卡住了。
- 新建任务向导第二步点 PaddleOCR / MinerU 凭据芯片，不再把路由预览表清空、把「下一步」按钮变灰——此前碰一下就走进死胡同，只能退回上一步再进来才能恢复。
- 逐本审批抽屉里的模型服务商下拉，现在列出设置里配好的全部六个档位（Kimi，以及各带两个计费槽的 Qwen 和 MiMo）。此前写死三个：用 Qwen 发的车能发不能改，切走之后就再也切不回来。
- 快速模式新增「反思二遍（更慢，约两倍模型开销）」勾选框。这条能力本来就全程接通，但没有任何界面入口，于是每个快速模式任务都按默认跑第二遍，多出来的开销无从拒绝。
- 项目文档里带查询串的链接（形如 `?a=1&b=2`）不再被二次转义成 `&amp;amp;`。此前这类链接一律打不开。
- 界面不再把占位值当成事实展示：项目卡片上编造的「最后更新」时间戳、设置里那个 Windows 味的 `D:\BiblioSmith` 路径都已去掉，取不到真实值时统一显示破折号。
- 断点续跑修复：改过「翻译时修复段内 OCR / 排版瑕疵」再续跑，此前会重译全部分块，而反思阶段却接着已经不存在的草稿跑，输出混着两遍的结果。
- 安全与依赖：markdown 渲染进一步加固，并处理了 npm 与 cargo 侧的依赖告警。

## EN

- Launching from "title search" no longer queues the five built-in demo entries (`-DIRECT` / `-SCAN` / `-MINERU` / `-DIRTY` / `-DONE`). Those fixtures were guaranteed to fail at run time; queueing now goes through the same real discovery path the wizard preview uses, so queued children carry real attachment keys, route overrides set in the wizard are no longer silently dropped, and a search that matches nothing yields one explicit blocked row instead of fabricated data.
- Fixed the pipeline stages failing to start when the app is launched from Finder. A macOS app started from the GUI inherits only `/usr/bin:/bin:/usr/sbin:/sbin`, which contains nothing Homebrew, uv, nvm, volta or rustup installs into, so `uv`, `node` and `java` did not resolve at all — even though the identical command works from a terminal. These programs are now resolved to an absolute path when the subprocess is spawned.
- Stage progress on the shelf now matches the backend. The frontend listed only 12 stages, missing "index" and "build digest"; Zotero attachments are the one route that runs a per-item index, and when that stage failed none of the four circles turned red, so the shelf reported the book as fine while it was stuck.
- Clicking the PaddleOCR / MinerU credential chips on step 2 of the new-job wizard no longer empties the route preview and greys out "Next" — until now a single click was a dead end recoverable only by stepping back and forward again.
- The provider dropdown in the per-book approval drawer now lists all six configured entries (Kimi, plus Qwen and MiMo with two billing slots each). It previously hardcoded three: a job queued with Qwen could be launched but not edited, and switching away from it lost the option to switch back.
- Fast mode gains a "Reflection second pass (slower, roughly double the model spend)" checkbox. The capability was wired end to end but had no control anywhere, so every fast-mode job ran the second pass at its default and the extra spend could not be declined.
- Links with a query string in project documents (`?a=1&b=2`) are no longer escaped twice into `&amp;amp;`. Every such link was broken.
- The UI no longer presents placeholder values as facts: the invented "last updated" timestamp on the project card and the Windows-flavoured `D:\BiblioSmith` path in Settings are gone, and an em-dash is shown when there is no real value to report.
- Resume fix: changing "Fix within-paragraph OCR / layout defects while translating" and resuming an interrupted run used to re-translate every chunk while the reflection pass resumed against drafts that no longer existed, blending two passes into the output.
- Security and dependencies: further hardening of markdown rendering, plus npm and cargo advisory updates.

## JA

- 「タイトル検索」からの投入で、5 件の組み込みデモ項目（`-DIRECT` / `-SCAN` / `-MINERU` / `-DIRTY` / `-DONE`）が並ぶことはなくなりました。これらのダミーは実行時に必ず失敗していました。現在はウィザードのプレビューと同じ実探索の経路を通るため、キューの子ジョブは実際の添付キーを持ち、ウィザードで指定したルート上書きも失われません。1 件も見つからない場合は、偽データではなく明示的なブロック行を 1 行返します。
- Finder からアプリを起動するとパイプラインの各ステージが開始できない問題を修正しました。GUI から起動した macOS アプリは `/usr/bin:/bin:/usr/sbin:/sbin` しか引き継がず、Homebrew・uv・nvm・volta・rustup の導入先が含まれないため、`uv` / `node` / `java` がまったく解決できませんでした（同じコマンドがターミナルでは動作します）。これらはサブプロセス起動時に絶対パスへ解決するようになりました。
- 本棚のステージ進行がバックエンドと一致するようになりました。フロントエンドは 12 ステージしか認識せず「索引」と「ダイジェスト生成」が欠けていました。項目単位の索引を実行するのは Zotero 添付の経路だけで、そのステージが失敗しても 4 つの丸は赤くならず、実際には停止しているのに本棚は正常と表示していました。
- 新規ジョブウィザードのステップ 2 で PaddleOCR / MinerU の資格情報チップを押しても、ルートプレビューが空になって「次へ」が無効化されることはなくなりました。従来は一度押すと行き止まりで、前の手順に戻ってから進み直すしか復帰方法がありませんでした。
- 書籍ごとの承認ドロワーのプロバイダー選択に、設定済みの 6 件すべて（Kimi と、課金スロットを 2 つ持つ Qwen・MiMo）が並ぶようになりました。従来は 3 件が固定で、Qwen で投入したジョブは変更できず、一度切り替えると戻せませんでした。
- 高速モードに「リフレクション 2 回目（より遅く、モデル費用は約 2 倍）」のチェックボックスを追加しました。この機能は端から端まで配線済みでしたが操作 UI がなく、高速モードのジョブは既定のまま常に 2 回目を実行し、追加費用を断ることができませんでした。
- プロジェクト文書内のクエリ文字列付きリンク（`?a=1&b=2`）が二重エスケープされて `&amp;amp;` になる問題を修正しました。この形式のリンクはすべて開けませんでした。
- プレースホルダーを事実として表示しなくなりました。プロジェクトカードの作り物の「最終更新」時刻と、設定に出ていた Windows 風のパス `D:\BiblioSmith` を削除し、実際の値が取得できない場合はダッシュを表示します。
- 再開時の修正: 「翻訳時に段落内の OCR / レイアウトの不備を修正」を変更して中断したジョブを再開すると、全チャンクを翻訳し直す一方で、リフレクションは既に存在しない下書きに対して再開され、2 つのパスが混ざった出力になっていました。
- セキュリティと依存関係: markdown レンダリングをさらに強化し、npm と cargo の脆弱性勧告に対応しました。
