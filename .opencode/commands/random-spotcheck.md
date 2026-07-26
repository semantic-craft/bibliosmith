---
description: Review representative local-reading samples against their source.
agent: book-runner
---

在 `$ARGUMENTS` 指定的本地书籍工程内执行或规划代表性抽检。

执行前必须读取：

- `AGENTS.md`
- `skills/local-book-reading-pipeline/SKILL.md`
- `skills/expert-translation-quality/SKILL.md`
- 书籍工程内的 `metadata/source_manifest.json`、`chapters/src/`、`chapters/final/` 和 `qa/status.md`

注意：

- 样本至少覆盖开头、中段、结尾，以及脚注、表格、长段落或高术语密度等风险单元。
- 每个样本必须能回到对应源文，检查漏译、误译、结构、注释和术语。
- 发现的问题先在同类范围内扩查，修复后重新抽检。
- 结果写入 `qa/`；不要把抽检写成公开发布门禁。
