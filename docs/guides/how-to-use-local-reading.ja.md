# ローカル読書の使い方

1. Launcher で BiblioSmith リポジトリのフォルダを選択するか、新規作成します。
2. 手元にある EPUB、PDF、Markdown、テキストファイル、または対応する Zotero 添付ファイルを追加します。
3. 変換、翻訳、出力形式を選びます。
4. サンプルを確認し、現在の設定にひも付いた翻訳プランを承認します。
5. 翻訳、QA、最終稿への昇格、読書用成果物の作成、EPUBCheck の順に進めます。
6. 書籍プロジェクトの `output/reading/` から成果物を開きます。

主な出力は次の場所に保存されます。

- Markdown: `output/reading/book.md`
- HTML: `output/reading/html/`
- EPUB: `output/reading/book.epub`
- 対訳 EPUB: `output/reading/book_bilingual.epub`
- Digest EPUB: `output/reading/book_digest.epub`
- Digest 付属ファイル: `output/reading/digest/`

短縮読書版が必要な場合は、Launcher で BiblioSmith Digest を明示的に有効にします。手動実行では書籍プロジェクト直下に `digest.config.json` を置き、次を実行します。

```sh
python -m digest.bibliosmith_digest --book-root books/local/{target}/{number}_{title_author}
```

原本、翻訳、QA 記録、読書用成果物はローカルに置かれ、Git には追加されません。本プロジェクトは書籍本文の検索、DRM やアクセス制御の回避、公開可否の判断を行いません。
