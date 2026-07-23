# 本地阅读翻译工作台

BiblioSmith 本地阅读/翻译工作台。

默认用途不是发布公版书，而是处理你电脑上已有的 EPUB/PDF：抽取、拆章、翻译、审校、生成 Markdown/HTML/EPUB。

启动新书：

```bash
cd bibliosmith
python3 tools/create_local_book_project.py "书名_作者" --source-file "/path/to/book.epub"
```

然后进入生成的目录，让 agent 使用：

```text
Use skills/local-book-reading-pipeline/SKILL.md to process this book.
```

项目级技能入口在 `.agents/skills/`；源文件在 `skills/`。Claude 通过 `.claude/skills` 读同一份白名单。

默认产物目录：

```text
books/local/zh-Hans/001_书名_作者/output/reading/
```

本改造版不做公版搜索、版权状态判断、private-use 声明或 GitHub release。上游公版流水线仍保留在 `template/epub_pipeline/`，仅作参考。
