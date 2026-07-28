---
description: Run the checks appropriate for one local reading output.
agent: book-runner
---

在 `$ARGUMENTS` 指定的本地书籍工程目录内运行或规划验证。

执行前必须：

1. 读取 `AGENTS.md`。
2. 读取 `skills/local-book-reading-pipeline/SKILL.md`。
3. 核对 `metadata/source_manifest.json`、章节目录、`qa/status.md` 和请求的 `output/reading/` 产物。
4. 若已有旧 EPUB 或 staging 输出，重新构建以免旧 XHTML、链接或资源污染结果。

按产物选择最小有效验证：Markdown/HTML 做结构与抽样阅读，EPUB 跑 EPUBCheck，译文对照
`chapters/src/` 检查完整性和术语一致性。把命令、结果和剩余风险记入 `qa/status.md`。

不要把命令写死到某个本机绝对路径；所有路径必须从当前书籍工程或仓库根目录解析。
