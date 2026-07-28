# 本機閱讀使用說明

1. 在 Launcher 中選擇或建立 BiblioSmith 儲存庫資料夾。
2. 加入你已經保存在本機的 EPUB、PDF、Markdown、文字檔或支援的 Zotero 附件。
3. 選擇轉換、翻譯與輸出格式。
4. 先查看樣張，再核准與目前參數綁定的翻譯方案。
5. 依序完成翻譯、QA、定稿、閱讀產物建置與 EPUBCheck。
6. 從書籍專案的 `output/reading/` 開啟產物。

主要產物位於：

- Markdown：`output/reading/book.md`
- HTML：`output/reading/html/`
- EPUB：`output/reading/book.epub`
- 雙語 EPUB：`output/reading/book_bilingual.epub`
- Digest EPUB：`output/reading/book_digest.epub`
- Digest 附屬檔案：`output/reading/digest/`

如需精簡閱讀版，請在 Launcher 中明確啟用 BiblioSmith Digest。手動執行時，先在書籍專案根目錄建立 `digest.config.json`，再執行：

```sh
python -m digest.bibliosmith_digest --book-root books/local/{target}/{number}_{title_author}
```

原始檔、譯文、QA 紀錄與閱讀產物只保留在本機，不會加入 Git。本專案不搜尋書籍全文、不移除 DRM、不繞過存取控制，也不判斷作品能否公開發布。
