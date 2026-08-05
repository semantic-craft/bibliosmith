# Island 风格最简界面重设计：书架 · OCR · 翻译

Status: **功能层已锁定**（2026-08-01 与用户逐项拍板）· UI 视觉细节讨论中
Origin: 2026-08-01 设计会话，基于三路并行代码盘点（UI 面 / OCR 与转换链路 / Zotero 集成）

> 本文是功能层的收口结论，供并行会话引用。UI 视觉规范（圆角/留白/配色/动效）
> 另起小节追加，未定稿前以本文「悬而未决」小节为准。
> 内核（15 stage 域模型、双审批门、断点续传、翻译引擎）**不动**，本轮只动呈现层
> 与少量后端命令。

## 目标与非目标

**目标**

- 界面收敛为 Trae 式 island 风格：统一底色上浮圆角卡片，模块靠留白分隔，
  中央一个突出的「添加书」输入岛。
- 整个应用只保留三个模块：书架、OCR、翻译。其余页面/面板全部砍掉或后台化。
- 转换与翻译一体化：`conversion_only` 模式退役，单一流水线，两条成书轨。
- 成品一键挂回 Zotero（`imported_file`）。

**非目标**

- 不重写书架（`Shelf.tsx` 封面网格保留）、不动 `model.ts` 域模型与 stage 顺序。
- 不改翻译引擎（reflect/improve、断点续传、占位符保护原样）。
- 不做新目标语言；仍是外文 → 中文。

## 已锁定的决策（2026-08-01，用户拍板）

| # | 决策 | 内容 |
| --- | --- | --- |
| 1 | 纯转换彻底剥离 | 「只转不翻」从 UI 消失（CLI 脚本保留应急）。同时**接入 BabelDOC 作为保版式双语 PDF 轨**：文字版 PDF 可选直出保留原版式的双语 PDF（AGPL-3.0 与本仓同许可证；翻译后端接现有模型槽的 OpenAI 兼容端点；无审批门，一次过；扫描件不适用） |
| 2 | EPUB 输入 v1 就做 | 上传 EPUB 免 OCR 直进翻译轨；需新写 EPUB→Markdown 章节抽取器（一条新转换路由） |
| 3 | OCR 对比内建化 | 每本书预检时抽 N 页 PaddleOCR 与 MinerU 各跑一次，抽屉里并排对比、点选定路由；照抄翻译 sample-compare 的 manifest→报告→并排 UI 模式 |
| 4 | 挂回用 imported_file | 走现成 `zsearch add file <path> --parent <KEY>`，成品进 Zotero 云存储；回写 attachment key 防重复挂 |

附带默认砍（用户未反对即定）：digest 勾选框、源清理面板（UI 砍，脚本路径保留）。

## 盘点结论：三个结构性事实

1. **「三模块」现状是一个标签页加两块设置。** 六个标签（总览/更新/书架/教程/
   设置/日志）里唯一工作面是「书架」（pipeline tab）；模型与 OCR 藏在设置十块
   面板中的两块。精简的本质是砍五个页面 + 把两块设置提为一级，而不是重写书架。
2. **EPUB 焊死在翻译后面。** 两个 EPUB builder
   （`tools/bibliosmith-launcher/source/scripts/build_epub.cjs`、
   `build_bilingual_epub.py`）都只吃 `chapters/final/`（翻译定稿后才存在）。
   「PDF 不翻译直出 EPUB」没有代码路径；而向导默认模式恰是 `conversion_only`
   ——只出 HTML、Markdown 不落盘的半吊子路径（`pdf_to_html_paddleocr.py` 的
   `full_md` 只存在于内存变量）。决策 1 砍的正是它。
   另注意：`conversion_only` 在 `contract.rs` 里**没有常量**，是「不属于另两个
   模式」的兜底空洞，退役时要把这个隐式分支清干净
   （`book_pipeline.rs` `should_handoff_after_run`）。
3. **挂回 Zotero 的地基是现成的。** `zsearch add file` 已实现完整四步上传
   （`packages/zotero-cli/src/zotero_cli/zotero_api.py` `add_imported_file()`，
   EPUB MIME 已配好）；每本书产物已持久化源条目 `parentItemKey`
   （`artifact.source_refs`）。缺的只是：launcher 一条 attach 命令（现在唯一的
   Zotero 命令是只读的 discover）、EPUB/HTML 产物回写 `zotero_key`（现在只有
   markdown 产物有）、Zotero 写凭据注入（launcher 目前只注入 OCR 凭据）。

## 目标界面：整个应用只剩三个面

### 面 1：书架主页（唯一页面）

```
┌──────────────────────────────────────────────┐
│ ●●●  BiblioSmith                       ⚙     │  ← 极简标题栏：品牌 + 设置入口
│                                              │
│   ╭──────────────────────────────────────╮   │
│   │ 📚 拖入 PDF / EPUB，或输入书名搜 Zotero │   │  ← 「添加书」输入岛
│   │  文字版 PDF → [重排 EPUB ⇄ 双语 PDF]   │   │     （对应 Trae 的中央命令岛）
│   ╰──────────────────────────────────────╯   │
│                                              │
│   ╭────╮  ╭────╮  ╭────╮  ╭ ┄┄ ╮            │
│   │封面│  │封面│  │封面│  │ ＋ │            │  ← 现有书架网格保留
│   │▓▓░░│  │待审│  │完成│  ╰ ┄┄ ╯            │
│   ╰────╯  ╰────╯  ╰────╯                    │
└──────────────────────────────────────────────┘
```

现在的三步向导（来源→预检→确认）压扁进输入岛：拖文件或选中 Zotero 条目后，
路由预检结果就地展开，一个按钮入队。不再有独立向导、不再有模式选择器。
轨道二选一只在文字版 PDF 时出现，其他输入自动定轨。

### 面 2：书籍抽屉（点封面滑出，现有 `BookDrawer.tsx` 瘦身）

```
╭─ 书名 · 来源: Zotero ────────────────╮
│  ① 转换 ────── ② 翻译 ────── ③ 成书  │  ← 内部 15 stage 折叠成 3 个用户阶段
│  ▓▓▓▓▓▓▓▓▓▓    ▓▓▓░░░░       ░░░    │     （BabelDOC 轨只显示单段进度）
│                                      │
│  [状态卡：随阶段变脸]                  │
│   · OCR 抽样对比: Paddle ⇆ MinerU 并排 │
│   · 翻译审批: 样本对比 + 换模型 + 放行  │
│   · 完成: 打开 EPUB / HTML / 双语 PDF  │
│          [⤴ 挂回 Zotero]              │
│                                      │
│  ▸ 高级（阶段时间线 / 产物 / 日志）     │
╰──────────────────────────────────────╯
```

用户阶段 → 内部 stage 的折叠映射：
①转换 = route/extract/index/handoff/split；②翻译 = prepare/approve_translation/
translate/expert_qa/approve_promotion/promote；③成书 = build_reading/
validate_reading（digest 退役）。审批门照旧停驻，呈现为状态卡。

### 面 3：设置（页面降级为弹层，十块砍成三块）

- **模型**：8 个供应商槽原样保留（`ModelsSettingsPanel.tsx`）
- **OCR**：PaddleOCR / MinerU 密钥（`OcrSettingsPanel.tsx`）
- **通用**：语言 + 代理（两家 OCR 都是境内远程 API，代理必须留）+ 开机自启

## 流水线：一个入口，两条成书轨

```
                    ┌─ 文字版 PDF ──┬─ 重排轨（默认）──────────────────────┐
选书 ──路由预检──┤               └─ 保版式轨: BabelDOC → 双语 PDF ——┐   │
（拖入/Zotero）    ├─ 扫描 PDF → OCR 抽样对比(Paddle⇆MinerU) ──→ 重排轨 │   │
                    └─ EPUB → 抽取器（新）───────────────────→ 重排轨 │   │
                                                                      │   │
重排轨 = Markdown → 分章 → 审批门 → 翻译 → QA → 审批门 → EPUB/双语EPUB/HTML
                                                                      ↓   ↓
                                                          ⤴ 挂回 Zotero（统一出口）
```

分工：重排轨保留全部质量机关（双审批门、反思二遍、术语表、EPUBCheck）；
保版式轨是无门禁的轻通道，只对文字版 PDF 开放，适合图表公式多的学术书
（学术著作留 PDF 的既定原则）。脏文字层守卫（`blocked_dirty_text_layer`）照旧。

## 砍掉清单

| 砍什么 | 说明 |
| --- | --- |
| 总览、更新、教程、全局日志四个页面 | 更新页整条摘掉是 2026-07-27 已定决策；每本书日志在抽屉「高级」里仍有 |
| 标题栏状态 chips + Quick Actions | 仓库管理时代遗留 |
| 向导模式选择器（三模式） | 一体化后只剩一条流；`conversion_only` 正式退役 |
| 设置七块：嵌入索引、项目路径、运行时、依赖安装、源清理、诊断日志、教程配套 | 运行时/依赖引导转后台静默，失败才浮条提示 |
| 抽屉内诊断导出、读者证据表单 | 支持型功能，非核心 |
| Markdown 源卡片、Zotero collection 卡、`external_adapter`/`fake` 死源 | collection 卡运行时本就不支持（`book_pipeline.rs` 明确 unsupported）；输入收敛为「文件 + Zotero 单条目」 |
| digest 勾选框、`build_digest` 的 UI 出口 | 不在三模块内 |
| 窗口/托盘三条死命令的前端接线欠账 | `minimize_main_window` 等本就无前端调用方，随重构一并清理 |

`App.tsx`（2077 行）里约 1200 行引导/同步/代理/教程状态随四页一并删除，
是本轮最大的删码红利。

## 新做清单（功能级，供拆票）

| # | 事项 | 量级 | 关键落点 |
| --- | --- | --- | --- |
| 1 | UI 收敛：砍四页 + 设置降弹层 + 向导压进输入岛 | 大 | `App.tsx`、`shell/`、`pages/`、`NewJobWizard.tsx` |
| 2 | 流程收敛：删模式选择器，`conversion_only` 显式退役（补常量清隐式分支） | 小 | `contract.rs`、`book_pipeline.rs`、`model.ts` |
| 3 | OCR 抽样对比：双引擎跑 N 页 + 报告 + 并排 UI + 选定路由 | 中 | 照抄 `sample.py`/`sample_cli.py`/`run_book_pipeline_translation_sample` 模式；全仓目前唯一没有的对比能力 |
| 4 | EPUB→Markdown 章节抽取器（新转换路由） | 中 | `packages/ocr/` 新脚本 + 路由 kind |
| 5 | BabelDOC 保版式轨：pip 依赖 + 子进程路由 + 双语 PDF 产物 kind | 中 | 翻译后端接模型槽 OpenAI 兼容端点；无门禁、单段进度 |
| 6 | Paddle 本地路径落盘 Markdown | 一行级 | `pdf_to_html_paddleocr.py` 把内存里的 `full_md` 写盘，打通本地 PDF→翻译的缝 |
| 7 | 挂回 Zotero：Tauri attach 命令 + key 回写 + 写凭据注入 + 本地源书的 Zotero 条目选择器 | 中 | 统一走 `zsearch add file`（注意给它补 429/Retry-After 重试——EPUB 比 Markdown 大得多；worker 里的 `upload_file()` 已有此处理可参考）；选择器复用 `ZoteroTitleSearch` |

已知坑（实现时注意）：

- 引擎并行撞车缝：prompt 措辞被测试替身按前缀分发，unit 遍历是线程池，
  新增 unit 级写入必须 per-unit 路径。
- 改审批门抽样前先读 `translation_approval_binding` 的 `sampleEvidence`。
- 前端测试基线：`BookDrawer`/`NewJobWizard`/`PipelineWorkbench`/`model` 的
  `.test.*` 与 `src/test/fixtures.ts` 都要随动；判据是 0 failed，条数会漂。
- 提交信息三语门禁：commit body 需 ZH/EN/JA 三段各 ≥40 字，PR 上查整条分支史。

## UI 视觉规范（教科书式 island，2026-08-01 研究定稿）

用户拍板：**冷白灰** + **输入岛两态**（空书架 hero 居中 / 有书收缩为顶条），
并要求「最典型最教科书式的 island 风格」。经调研，island 风格唯一有官方实现
可考的教科书是 **JetBrains Islands 主题**（2025-09 官宣、2025.3 起全系 IDE
默认外观），其原始 token 在开源仓
`JetBrains/intellij-community` 的
`platform/platform-resources/src/themes/islands/ManyIslands{Light,Dark}.theme.json`。
以下数值直接取自该实现：

**七条硬规则**

1. **三层背景体系**（分离靠层差，不靠线条）：
   - Light：画布 `#E9EAEE`（gray-150）→ 岛 `#FFFFFF` → 岛内子面 `#F7F8F9`（gray-160）
   - Dark（**刻意反转**，岛比画布更暗、内容区最暗以护眼）：
     画布 `#26282C`（gray-30）→ 岛 `#191A1C`（gray-10）→ 子面 `#212326`（gray-20）
2. **岛无边框、无投影**。投影只给弹层（抽屉/弹窗/菜单）。
3. **圆角分级**：岛 10px（`Island.arc=20`，arc 为直径）、卡片/子面 8px、
   控件 4–6px（`Component.arc=8`）、弹层 12px。不是大圆角风格，克制。
4. **岛间隙 6px**（`Island.borderWidth=6`，紧凑 4px），含岛与窗口边缘的留距；
   岛永不贴窗口边。间隙即边界。
5. **非活动岛淡出**至 α≈0.56（`Island.inactiveAlpha`）——抽屉打开时背后
   岛组整体降透明度。
6. **标题栏是画布的一部分**：无底色、无 `border-bottom`，红绿灯与齿轮直接
   坐在画布上。
7. **单强调色 + 成对语义色**（强色 + 同色系 ~10% 透明软底），沿用现有
   amber=审批 / info=运行 / err=失败 / jade=完成 的语义映射不变。

**BiblioSmith 具体映射**（mockup v2 已按此实现）

- 岛的划分：输入岛、书架岛两个结构岛；标题栏在画布层；封面卡是书架岛
  **内**的内容（子面层），不是独立岛。
- 抽屉是弹层：距窗口边缘 6px 悬浮、12px 圆角、带投影，打开时结构岛淡至 0.56。
- 现 `styles.css` 的「纸墨+玉」暖色 token 全套换为上述冷灰体系；三层
  token 架构（`:root` / `prefers-color-scheme` / `data-theme` 覆盖）保留。

**审美终决（2026-08-01，用户授权代决）**

- **强调色：玉色留任**（light `#12897A` / dark `#43C9B0`），不换蓝紫。
  依据：紫/蓝是 AI 工具的同质化重灾区（Tailwind `bg-indigo-500` 默认值污染
  训练数据 → 「所有 AI 生成界面都是靛蓝色」已成公认现象，Trae 自己也是
  紫蓝强调色）；青玉 + 冷灰是公认的「像蓝一样可信但更新鲜」的搭配
  （Chakra teal.500 即其吉祥物色）。玉色同时保住品牌连续性与现有
  jade/amber/info/err 语义 token 体系，也贴合书籍文献工具的「玉」气质。
- **控件统一 6px 小圆角，不用 pill 胶囊形**。全局只有一套圆角语言
  （岛 10 / 卡 8 / 控件 6 / 弹层 12），与教科书一致；混入 pill 会引入第二
  套 UI 方言，也是「AI 味」清单上的典型症状。
- **悬停不加投影不位移**（岛无投影的推论）：hover = 背景提亮至子面层色，
  选中态 = 强调色 2px 外圈（mockup 的 `.pane.picked` 模式）。
- **动效 150–250ms ease-out**；抽屉 240ms 滑入，背后岛同步淡至 0.56。
- **封面卡保留彩色渐变**（按书名派生色相的现有 Shelf 行为）：内容层的
  色彩活力衬在中性镶边上，正如 Trae 白底上的彩色模板卡。

mockup v2（`island-mockup-v2.html`，会话 scratchpad）即最终视觉方向。

## 相关文档

- `docs/planning/prd-monorepo-dual-mode-pipeline.md` —— 双模式流水线总纲
  （本文的模式收敛是对其 story 面的收窄）
- `docs/book-pipeline-capability-matrix.md` —— 注意其中「本地文件夹产物
  normalized as Markdown/HTML/EPUB」的表述与实情不符（Paddle 路径无 Markdown、
  EPUB 无路径），随新做清单 #6 一并修正
- `docs/adr/0002-progress-and-terminal-notifications.md` —— 逐阶段显式推进、
  闸门停驻的既定裁决，本设计不改
- `docs/bilingual-epub-builder.md` —— 双语构建契约
- `packages/ocr/docs/adr/0002-remote-paddleocr-only.md` —— 远程 OCR only 的
  既定裁决，抽样对比在其之内（仍是远程调用）
