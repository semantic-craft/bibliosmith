# ローカル読書・翻訳ワークスペース

BiblioSmith は、手元にある EPUB、PDF、論文、原稿を Markdown、翻訳文、HTML、EPUB、対訳 EPUB に整え、個人の読書や研究に使うためのローカル優先ツールです。

## 対象範囲

- 新しいプロジェクトは `books/local/{target}/{number}_{title_author}/` に作成されます。
- 原本、翻訳、QA 記録、生成物はローカルに置かれ、Git には追加されません。
- 読書用の最終成果物は `output/reading/` に保存されます。
- 書籍本文の検索、DRM やアクセス制御の回避、公開可否の判断は行いません。

## クイックスタート

```sh
python3 tools/create_local_book_project.py "書名_著者" \
  --source-file "/path/to/book.epub" \
  --source-language ja \
  --target-language zh-Hans
```

続いて [ローカル読書ガイド](../docs/guides/how-to-use-local-reading.ja.md) に従い、抽出、翻訳、QA、読書用成果物の作成を進めてください。

簡体字中国語版は [README.zh-CN.md](../README.zh-CN.md)、英語版は [README.md](../README.md) を参照してください。
