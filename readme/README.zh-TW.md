# 本機閱讀與翻譯工作區

BiblioSmith 是本機優先的閱讀與翻譯工具，可將你已經擁有的 EPUB、PDF、論文或書稿整理成 Markdown、譯文、HTML、EPUB 或雙語 EPUB，供個人閱讀與研究使用。

## 使用範圍

- 新專案建立在 `books/local/{target}/{number}_{title_author}/`。
- 原始檔、譯文、QA 紀錄與產物只保留在本機，不會加入 Git。
- 最終閱讀產物寫入 `output/reading/`。
- 本專案不搜尋書籍全文、不移除 DRM、不繞過存取控制，也不判斷作品能否公開發布。

## 快速開始

```sh
python3 tools/create_local_book_project.py "書名_作者" \
  --source-file "/path/to/book.epub" \
  --source-language en \
  --target-language zh-Hans
```

接著依照[本機閱讀使用說明](../docs/guides/how-to-use-local-reading.zh-TW.md)，完成擷取、翻譯、QA 與閱讀產物建置。

簡體中文版請見 [README.zh-CN.md](../README.zh-CN.md)，英文版請見 [README.md](../README.md)。
