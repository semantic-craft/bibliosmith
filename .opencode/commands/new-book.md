---
description: Create or plan a local reading project from a user-provided file.
agent: book-runner
---

根据参数 `$ARGUMENTS` 创建或规划一个新的本地阅读工程。

执行前必须：

1. 读取 `AGENTS.md`。
2. 读取 `skills/local-book-reading-pipeline/SKILL.md`。
3. 从参数确认书名/作者 slug、本地 `--source-file`、源语言和目标语言。

边界：

- 必须使用 `tools/create_local_book_project.py`。
- 新工程路径必须是 `books/local/{target}/{number}_{title_author}/`。
- 只接受用户已经提供的本地文件，不替用户搜索书籍全文。
- 书源、译文、QA 和生成产物不得提交到 Git。

如果 `$ARGUMENTS` 不足以创建工程，只输出缺少的具体参数和推荐命令，不要猜测。
