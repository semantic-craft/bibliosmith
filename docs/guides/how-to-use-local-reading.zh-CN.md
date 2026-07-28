# 本地阅读使用说明

1. 在 Launcher 中选择或创建一个 BiblioSmith 仓库目录。
2. 加入你已经保存在本机的 EPUB、PDF、Markdown、文本文件或支持的 Zotero 附件。
3. 选择转换、翻译和输出格式。
4. 先查看样张，再批准与当前参数绑定的完整翻译方案。
5. 依次完成翻译、QA、定稿、EPUB 构建和校验。
6. 从书籍工程的 `output/reading/` 打开所需产物。

主要产物位于：

- Markdown：`output/reading/book.md`
- HTML：`output/reading/html/`
- EPUB：`output/reading/book.epub`
- 双语 EPUB：`output/reading/book_bilingual.epub`
- Digest EPUB：`output/reading/book_digest.epub`
- Digest 附属文件：`output/reading/digest/`

如需速读版，在 Launcher 中明确勾选 BiblioSmith Digest。手动运行时先在书籍工程根目录
写入 `digest.config.json`，再执行：

```sh
python -m digest.bibliosmith_digest --book-root books/local/{target}/{number}_{title_author}
```

输出仍然是标准 EPUB。

每本书放在 `books/local/{target}/{number}_{title_author}/`。书源、译文、QA 证据和阅读
产物都留在本机，并由 Git 忽略。

项目不搜索书籍全文，不移除 DRM，不绕过访问控制，也不判断作品能否公开发布。
