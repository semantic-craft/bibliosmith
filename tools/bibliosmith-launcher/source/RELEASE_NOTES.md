# BiblioSmith Launcher 1.12.1

## ZH

- 用标题搜索发车不再排出五个演示假条目。此前从标题搜索或其他 Zotero 查询来源发车时，队列里排的是内置的 `-DIRECT` / `-SCAN` 等五条演示数据，附件 key 是假的，运行阶段必然失败。现在发车与向导预检走同一条 worker dry-run 发现路径，排进队列的子任务带的是真实附件 key。
- 你在向导里改过的路由覆写会真正生效。之前这些覆写只在预检里体现，发车时被静默丢弃。
- 发现结果为空时，给出一条明确的阻塞行说明原因，而不是拿兜底假数据把队列填满。
- 浏览器演示模式与桌面端后端对齐：按来源自带的发现证据逐条算路由，凭据、脏文本层、已转换的判定完全一致，没有证据时同样只返回一条阻塞行。

## EN

- Launching from a title search no longer queues five demo entries. Until now, queueing a title search or any other Zotero query source enqueued the five built-in `-DIRECT` / `-SCAN` fixtures, whose attachment keys were fabricated, so the run stage always failed. Queueing now goes through the same worker dry-run discovery the wizard preview uses, and the queued children carry real attachment keys.
- Route overrides set in the wizard now take effect. They were previously honoured in the preview and silently dropped when the job was queued.
- When discovery finds nothing, you get one explicit blocked row explaining why, instead of a queue filled with fallback fixtures.
- The browser demo mode matches the desktop backend: routes are computed per discovery evidence carried by the source, using the same credential, dirty-text-layer, and already-converted policy, and a source without evidence yields the same single blocked row.

## JA

- タイトル検索からの投入で、5 件のデモ項目が並ぶことがなくなりました。これまではタイトル検索やその他の Zotero 検索を投入すると、組み込みの `-DIRECT` / `-SCAN` などの見本データが 5 件そのままキューに入り、添付キーが実在しないため実行段階で必ず失敗していました。投入もウィザードのプレビューと同じワーカーのドライラン探索を通るようになり、キューに入る子ジョブは実在する添付キーを持ちます。
- ウィザードで指定した経路の上書きが実際に反映されます。以前はプレビューにだけ反映され、投入時には黙って捨てられていました。
- 探索結果が空のときは、見本データでキューを埋める代わりに、理由を示すブロック行が 1 行だけ返ります。
- ブラウザのデモモードもデスクトップ側と揃えました。ソースに付随する探索エビデンスごとに経路を算出し、認証情報・汚れたテキストレイヤー・変換済みの判定も同一で、エビデンスがない場合は同じくブロック行を 1 行返します。
