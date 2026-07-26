# PRD:三仓合一 Monorepo 与双模式翻译流水线

Migrated: 2026-07-26 · Source: `semantic-craft/bibliosmith-private-archive#52`

> 本文迁自只读归档仓的 issue `#52`。那个仓库处于 GitHub 归档状态,整仓只读 ——
> 无法评论、无法修订、无法关闭,所以这份 spec 只能在本仓维护。为什么选文档而不是
> 重开 issue,以及归档编号到本仓编号的完整映射,见 `docs/planning/README.md`。
>
> 正文保持原文(中文),不做翻译,避免转述改变原意。**裸 `#NN` 一律是归档仓编号**;
> 已重编到本仓的用 `本仓 #NN` 标注。

## 迁移时的校订

原文写于 2026-07-19,以下几处此后已变,正文中保留原句并就地标注:

- **story 16「全链自动推进」已按归档仓 `#88` 的候选 **B** 定性**,改写为「逐阶段显式
  推进,闸门处停」,与 `docs/adr/0002-progress-and-terminal-notifications.md` 对齐。
  这是措辞变更,不写代码;当前的单阶段 `advance` 就是目标形态。归档仓 `#88` 关不掉,
  该决定在本仓留档。
- **归档仓 `#90`(digest 无 UI 闸口)与 `#91`(预飞样本未透传参数)已消解**,正文中
  已就地标注。
- **归档仓 `#99`(双语构建脚本双份分叉)的定性被推翻** —— 两份脚本是契约不同的两个
  工具,不是同一算法的分叉,详见 `docs/bilingual-epub-builders.md`。
- **测试基线数字已过时**:原文记 Rust 177 / 引擎 64。2026-07-26 实测 cargo test 209 通过。
- **目录名已变**:原文的 `tools/lifebook-launcher` 现为 `tools/bibliosmith-launcher`。
- **仓库拓扑已变**:开源重建后 `semantic-craft/bibliosmith` 是一个全新的公开仓,不再是
  本文这些票的宿主。

---

> 本 PRD 由 Wayfinder 地图 归档仓 #40 的建图共识综合而成,是三仓融合工作的目的地级总纲,方向上接替 PRD 归档仓 #1/#10/#20。地图开放票(#41–#51)逐张落地时,本 spec 对应小节以「Decisions so far」为准更新;标注 ⏳ 的条目为待决,以对应地图票为权威。
>
> **状态盘点(2026-07-19)**:地图票 #41–#51 已全部关闭,原「待决」小节 8 条全部回填(见下)。34 条 user story 的落地状态已逐条标注(17 已落地 / 11 部分落地 / 5 按决定不做或改形 / 1 未决),未落地部分均有对应施工票 #88–#99。
>
> ⛔ 标记要特别留意:**它表示「已决定不做」,不是「欠账」**。story 8/11/12 是地图票推翻了 PRD 原措辞,story 20/22/23 是用户或 ADR 明确停用/延期。不要把这六条当成待补的缺口。

## 状态图例

- ✅ **已落地** —— 实现 + 测试俱在,主路径可达
- 🚧 **部分落地** —— 有实现但不完整或不可达,附对应施工票
- ⛔ **按地图决定不做/改形** —— 地图票明确收窄或改变了形态,PRD 原措辞过时(保留原文以存史,后附实际决定)
- ⏳ **未决** —— 仍在雾区,无施工票

## Problem Statement

我(私人读者/文献研究者)的书籍与文献处理能力散落在三个仓库里:book-ocr-conversion 会把 PDF 变成干净 Markdown,zotero-cli-agent 会检索和管理文献库,本仓库会把干净 Markdown 变成中文译稿和阅读产物(HTML/EPUB/双语/digest)。三者之间靠隐式约定和手工 shell out 衔接:OCR 回传的 `.md` 恰好被 zsearch 索引到、launcher 恰好知道另一个仓库的脚本路径。任何一环变动都可能悄悄断链,没有一个地方能让我「丢进一本书/一个 Zotero 集合,拿到可读的中文版」。同时,handoff 之后的翻译—QA—出书全靠 agent 会话人工驱动,慢且不可批量;而 TranslateBooksWithLLMs 证明了程序化翻译(分块、上下文延续、断点续传、多 LLM 后端)完全可行,我的流水线却不具备。

## Solution

把三个仓库物理合并为一个私有 monorepo,以既有的统一作业模型(parent→child→stage→unit)为骨架,把「OCR/转换 → 入库/检索 → 翻译 → 阅读产物」接成一条显式契约的完整流水线;原生重实现 TBL 的全部能力作为新的程序化翻译引擎,与既有的 agent 专家翻译并列为双模式——每本书入队时选择快速模式或专家模式,expert_qa 闸门对两种输出一体生效。zsearch/zfulltext 与每日定时 OCR 等独立入口原样保留,流水线在内部以库/适配器方式复用同一套代码。

## User Stories

1. 🚧 As a 私人读者, I want 把一个本地 PDF 文件夹丢进流水线后直接拿到中文 EPUB, so that 我不需要理解中间任何一个环节。 —— 各阶段均已实现,但「直接」不成立:runner 每次调用只推进一个阶段,一本书需约 7 次操作(#88);digest 版无 UI 闸口(#90)。
2. ✅ As a 私人读者, I want 在入队每本书时选择「快速模式」或「专家模式」, so that 日常泛读的书便宜快出,重点书拿到出版级译文。 —— v2 intent 入队写死并持久化,改模式使披露闸失效并清理下游。
3. ✅ As a 文献研究者, I want 选中一个 Zotero 集合并让流水线逐附件发现、路由、抽取, so that 整个集合的文献自动变成可检索、可翻译的 Markdown。 —— collection membership 冻结为持久 child job,快照 SHA-256 绑定,同快照幂等。
4. ✅ As a 文献研究者, I want OCR 完成的 Markdown 自动回传 Zotero 并自动进入 zsearch 全文索引, so that 抽取和检索之间不再有需要手工触发的暗链。 —— 代码链完整(route→extract→index→handoff 一次 Run 推完,索引硬门于 `markdownAttachmentKey` 并重验 SHA,索引失败可独立重试)。**但本机运行面已失修 → 本仓 #33**。
5. ✅ As a 私人读者, I want 程序化翻译引擎按 token 分块并在块间携带上下文, so that 长书翻译不因分块而丢失人名、指代和语气的连续性。 —— 注:计数器是 UTF-8 字节保守上界而非真 tokenizer(为保持测试离线),实际块比 `maxTokens` 暗示的小。
6. ✅ As a 私人读者, I want 翻译作业可以断点续传, so that 中断(断电、限流、手动停止)后从最后一个 checkpoint 继续而不是重头翻。 —— 幂等键绑 task manifest SHA + provider profile/config + policy 版本 + passId;杀进程恢复有端到端测试。
7. 🚧 As a 私人读者, I want 翻译 EPUB 时保留原书结构与标签, so that 译本的目录、章节、强调、图表引用与原书一致。 —— 实际形态是 Markdown 章节进、EPUB 出:占位符保护(front matter/围栏/行内代码/数学/链接/脚注/标题前缀/段落边界)+ 结构校验已落地。**EPUB/DOCX/SRT 直译是 #45 留的适配器槽位,尚未建**。
8. ⛔ As a 编排者, I want 为翻译引擎配置多种 LLM 后端(Ollama、任意 OpenAI 兼容端点、云端 API), so that 我能按成本、隐私和质量在本地与云端模型间自由切换。 —— **#46 已收窄**:Provider 集合 = OpenAI 兼容 + Gemini 原生,Ollama/本地串行子项消解。两者均已落地;本地后端不再是目标。
9. ✅ As a 私人读者, I want 云端后端支持多 API key 轮换与 429 failover, so that 限流不会中断长时间批量翻译。 —— 线程安全轮转 KeyPool、按 key 节流窗口、`Retry-After` 解析、限流预算独立于瞬时错误预算,长节流上抛以便从 checkpoint 续。
10. 🚧 As a 私人读者, I want 术语表(glossary)在两种翻译模式下都强制生效, so that 同一本书内与系列书之间术语始终一致。 —— 注入 + 哈希绑定已落地,但**输出侧无校验**:LLM 无视术语表时管线没有任何信号 → 本仓 #32。
11. ⛔ As a 私人读者, I want 引擎自动从原文抽取候选术语(NER)供我确认, so that 术语表不需要我从零手建。 —— **#48 已改形**:NER 定为**管线外按需命令**(人工触发 = 披露同意,解闸门悖论),不是引擎自动。已落地为 `translation-engine-ner`。
12. ⛔ As a 私人读者, I want 快速模式提供可选的 refine 二次润色 pass, so that 便宜译文也能再提一档文学质量。 —— **#48 已改形**:纯无源 refine 砍掉留缝,合并为唯一的窗口化 reflection(reflect(源+稿)→improve,可修忠实度),书级可选。已落地。
13. ✅ As a 私人读者, I want 专家模式继续走 expert-translation-quality 等技能的专家级翻译与 QA, so that 已有的质量体系原样保留。
14. ✅ As a 私人读者, I want expert_qa 闸门对快速/专家两种模式的产出统一生效, so that 任何译文晋升 final 前都过同一道质量关。 —— QA policy 分层(自动化全量 + 专家抽检 + 缺陷扩检),晋升闸绑 policy 哈希 + 抽检证据。
15. ✅ As a 编排者, I want approve_translation 与 approve_promotion 两道人工闸门保留在自动化 runner 里, so that 自动化提速但关键节点仍由我拍板。 —— 两闸均重新推导绑定并在漂移时拒绝,promote 前再校验一次。
16. ✅ As a 编排者, I want handoff 之后的 split→prepare→translate→expert_qa→promote→build_reading→validate_reading **逐阶段显式推进,闸门处停**, so that 每一步的产物我都看得见,关键节点由我拍板而不是被自动跑过去。 —— **【2026-07-26 按归档仓 #88 定性 B 改写】** 原措辞是「全链自动推进」,与 `docs/adr/0002-progress-and-terminal-notifications.md` 的「审批记录一个当前哈希绑定的决定,但不执行下一阶段」直接冲突。ADR 0002 是更晚、更权威的决定,故认定 story 16 原措辞过时,改写本条以与 ADR 对齐,**不写代码**。当前实现(`advance_job_with_executor` 每次调用推进一个阶段)即为目标形态,不是缺口。归档仓 #88 无法关闭(整仓只读),此决定在本仓留档。
17. 🚧 As a 私人读者, I want 每本书产出 HTML、EPUB、双语 EPUB 与 digest 版, so that 不同阅读场景(通读、对照、速览)各有其形。 —— 四种产物后端均已落地并接进 runner,但 digest 在 UI 上不可达(归档仓 #90,**已消解**:`NewJobWizard.tsx` 现有勾选框),双语构建脚本有两份(本仓 #35;**定性已推翻**:不是分叉,是契约不同的两个工具,见 `docs/bilingual-epub-builders.md`)。默认集合为 `["md","html","epub"]`,双语与 digest 需显式勾选。
18. 🚧 As a 私人读者, I want 成品 EPUB 通过 EPUBCheck 与实际阅读器校验, so that 产物在我的设备上真正可读而非仅仅生成。 —— EPUBCheck 已落地(版本钉住,标准与双语 EPUB 都查,fatal/error 即失败)。**阅读器实测证据无处可记** → 本仓 #29。
19. 🚧 As a 文献研究者, I want zsearch/zfulltext 以同名 CLI 继续独立安装可用, so that 其他项目的 agent 检索文献的方式不因融合而改变。 —— 仓内 `[project.scripts]` 完好,但 **PATH 上那份是合库前的旧快照**(源目录 `~/Projects/lrt-zotero` 已不存在,缺 `collection-snapshot`/`index`/`profile`);另有一个已装载的 zsearch 同步定时任务指向已删除的 `~/Projects/zotero-cli-agent`,持续报错 → 本仓 #33。
20. ⛔ As a 编排者, I want 每日定时 OCR 任务在 monorepo 下继续运行, so that 文献库的增量附件持续自动转换,无需我记得手动跑。 —— **2026-07-18 用户决定停用自动 OCR**(原话:「自己决定要不要 OCR」),4 个 launchd 任务(`com.semantic-craft.books-translation.ocr.daily` + 3 个个人命名空间下的 `zotero-*-ocr`)全部 unload 并移入废纸篓,备份 plist 留存。安装脚本与目标路径仍然正确、随时可重建(`packages/ocr/scripts/install_launch_agent.sh`),但**重建需用户重新拍板**;手动 OCR 走 `packages/ocr/scripts/zotero_llm_worker.py --attachment-key <KEY>`。**本条不再是欠账,不要顺手装回去。**
21. 🚧 As a 编排者, I want Windows 双机运行面延续, so that 大批量 OCR 仍可分派到另一台机器。 —— 机制完整且 2026-07-17 端到端验证过,但改名发生在 7/18 之后,runbook 的 clone 地址与检出路径仍是旧名,自改名后未再验证 → 本仓 #34。
22. ⛔ As a 私人读者, I want 翻译 SRT 字幕时保持时间码同步、翻译 DOCX 时保留文档结构, so that 书籍之外的常见格式也走同一条流水线。 —— **#51/ADR 0002 显式延期出 v1**,留待后续 input-adapter 票。树内零脚手架。
23. ⛔ As a 私人读者, I want 可选的 TTS 音频输出, so that 译好的书能变成可听的版本。 —— **#51/ADR 0002 显式延期出 v1**,留待后续 reading-output 票。
24. ✅ As a 编排者, I want 长任务完成或失败时收到 webhook 通知, so that 我不用盯着进度也能及时知道结果。 —— 已落地并有隐私载荷测试。缺陷:同一(作业, 终态)可能重复投递 → 本仓 #28。
25. ✅ As a 私人读者, I want 在 launcher 里看到每本书/每个附件的实时阶段进度, so that 批量任务的健康状况一目了然。 —— schema v5 进度,前端 750ms 轮询。
26. 🚧 As a 编排者, I want 失败的块/附件/阶段自动重试且重试策略可观测, so that 偶发失败不需要人工兜底。 —— 重试**范围**裁剪精确且可观测,但编排层**没有任何自动重试**,也没有策略对象(无上限、无退避表、无放弃原因)→ 本仓 #27。
27. 🚧 As a 编排者, I want 源文件清理依旧走「验证产物后人工批准」的证据记录流程, so that 自动化程度提高但绝不静默删除源文件。 —— 「绝不静默删除」成立(runner 不删任何源文件)。但审批记录是会被日志截断挤掉的自由文本,且证据查的是转换阶段产物而非被校验过的阅读产物 → 本仓 #21。
28. ✅ As a 编排者, I want 所有凭证(OCR token、LLM key、Zotero key)只存在于 .env 且不进 git/日志/作业记录, so that 融合后安全红线不被稀释。 —— launcher 注入零凭证;webhook 端点仅在内存;仅 `.env.example` 入 git。
29. ✅ As a 编排者, I want 作业状态沿用版本化 schema(原子写、乐观锁、幂等键、哈希绑定审批), so that 并行会话与断点恢复不破坏状态一致性。 —— 有双线程屏障测试断言唯一写者获胜且 JSON 保持可解析。
30. ✅ As a 开发者, I want monorepo 内各包(ocr、zotero-cli、translation-engine、launcher、digest)边界清晰且可独立测试, so that 一处改动不需要全库回归。 —— 现基线:引擎 64、Rust runner 177、zotero-cli 62、ocr 11、digest 13,全绿。
31. ✅ As a 开发者, I want 外部工作全部经由既有适配器契约注入 fake, so that 不花一分钱 API 费也能全绿跑完流水线测试。
32. ✅ As a 开发者, I want 翻译引擎的 LLM 后端是可替换的 provider 接口, so that 新增模型后端只需实现接口而不动引擎核心。 —— `LLMProvider` 为 runtime_checkable Protocol;provider/target_profile/second_pass 三个工厂均为可注入缝。
33. ✅ As a 私人读者, I want 流水线记录产物的来源链(哪个源、哪次抽取、哪个模式翻译), so that 我能追溯任何一页译文的出身。 —— **#39 已落地**:内容寻址 artifact_id + producer + input_hashes + source_refs + privacy + validation,`validate_artifact_contract` 拒绝不完整溯源;结构化脱敏与安全打开(allowlist)同批落地。
34. ⏳ As a 编排者, I want 语义检索与翻译流水线联动(如跨书术语一致性查询), so that 文献库的知识反哺翻译质量。 —— 仍是雾区。sqlite-vec 索引本身已落地并双向接线,缺的是这个场景是否立项的判断。

## Implementation Decisions

已锁定(Wayfinder 地图 归档仓 #40 标定决策):

- **融合形态 = Monorepo 物理合并**:book-ocr-conversion 与 zotero-cli-agent 代码搬入本仓库,统一版本与发布;不采用纯编排契约或统一 CLI 门面方案。
- **仓库转私有**:semantic-craft/books-translation 先转私有再合并(#41),避免公开另外两仓代码。
- **独立入口保留**:monorepo 内部分包,zsearch/zfulltext 以同名 console script 继续发布;定时 OCR 改指新路径继续跑。流水线内部以库/适配器方式复用同一代码,不 fork 逻辑。
- **TBL 全功能、原生重实现**:程序化翻译引擎(分块/上下文延续/断点/标签保留/重试)、多 LLM 后端、词汇表+refine、扩展格式与周边全部纳入;不 vendor AGPL 代码,机制经研读票(#44)提炼后自行实现。
- **翻译双模式**:translate 阶段支持程序化快速模式与 agent 专家模式,入队时声明并持久化到作业状态;expert_qa 闸门与双人工审批(approve_translation/approve_promotion)对两种模式一体生效。
- **编排骨架不变**:沿用统一作业模型 parent→child→stage→unit 与既有阶段契约 discover→route→extract→handoff→split→prepare→approve_translation→translate→expert_qa→approve_promotion→promote→build_reading→validate_reading;翻译引擎对编排层表现为外部适配器命令(RunnerCommandExecutor 契约),与 OCR/Zotero worker 同构。
- **已有实现必须吸收**:本地已完成的 归档仓 #38 split+prepare 切片(advance 命令、确定性 split、prepare 产物、checkpoint、哈希幂等)是 runner 的既成起点,后续阶段在其上延伸而非重写。
- **状态分层沿用既有原则**:作业记录指向私有文本但绝不内嵌正文与凭证;每书项目状态(source_manifest、chapters src→translated→final 晋升、glossary、qa)保留。
- **尊重 ADR 0002**:OCR 继续走云端(PaddleOCR-VL/MinerU)异步作业,不引入本地 OCR 依赖。(注:此处指 `packages/ocr/docs/adr/0002-remote-paddleocr-only.md`;仓根另有一份同号 ADR `docs/adr/0002-progress-and-terminal-notifications.md`,两者不同。)
- **旧票处置**:#37/#38/#39/#7/#8 挂起,由地图票 归档仓 #49/#47 等产出重写规格;本 PRD 接替 归档仓 #1/#10/#20 的总纲角色。

原「待决」8 条已全部落定(2026-07-19 回填,详细答案见 #40「Decisions so far」):

- ✅ **引擎技术栈、进程边界、语言对范围**(#45)—— Python,uv workspace 成员 `packages/translation-engine`;外部适配器 CLI 沿 RunnerCommandExecutor,每 translate 阶段一调;核心吃占位符保护后的文本单元,Markdown 章节适配器首建,EPUB/DOCX/SRT 直译留槽位;v2 作业状态唯一真相源,引擎块级断点私有旁挂;首期只发 zh-Hans 目标语 profile。
- ✅ **provider 集合、配置面、key 轮换细节**(#46)—— OpenAI 兼容 + Gemini 原生(**无 Ollama、无 Anthropic 原生**);全量多-key KeyPool + 429 独立预算;非密注册表(profile_id + config_id → provider/base_url/model/timeout/并发/chunk/key_env),闸门绑 ID,密钥运行时从根 .env 解析;并发 = 章内串行带尾 25 词上下文 + 章间并行;类型化错误三分支,无熔断器。
- ✅ **双模式 runner 的阶段编排与闸门细则**(#47)—— 模式 = v2 intent 枚举 fast|expert 入队写死;专家模式与快速模式共用同一 unit 产物契约,runner 只验收哈希/清单;approve_translation 两模式统一;快速模式 expert_qa = QA policy 分层;EPUBCheck 必过、阅读器实测可选证据。
- ✅ **glossary/refine 与专家技能的分工**(#48)—— 词汇表 = 书籍项目单一事实源,四消费者同源;二遍 pass 合并为唯一的窗口化 reflection,纯无源 refine 砍掉;NER 移出管线为按需命令;缺陷家族人工回灌进目标语 profile。
- ✅ **Zotero discover/route/extract 显式接线**(#49)—— discover 单认 zotero-cli 只读快照;冻结 collection membership 后逐附件持久推进 route→extract→index→handoff,索引失败独立重试。
- ✅ **monorepo 布局、git 历史、路径/凭证/定时任务迁移**(#43)—— `packages/{ocr,zotero-cli,translation-engine,digest}` + `tools/lifebook-launcher`;Python 统一 uv workspace;zotero-cli filter-repo 保史 + ocr 快照;根单一 .env。
- ✅ **状态存储统一 vs 联邦**(#50)—— **联邦**(ADR 0001):Book Pipeline 状态只作编排状态的单一事实源,各子系统各自是自己存储的唯一写入方;无分布式事务,恢复时对存储引用做 reconciliation,绝不凭文件名推断完成。
- ✅ **SRT/DOCX/TTS/通知/进度 UI 的 v1 取舍**(#51)—— **v1 只做** launcher 内实时进度 + 每终态一条 webhook;**SRT 输入、DOCX 输入、TTS 音频显式延期出 v1**。

## Testing Decisions

- 只测外部行为:给定源(文件夹/集合/单书)与模式声明,断言阶段推进、产物记录、闸门状态与最终产物,不断言内部实现细节。
- **主缝(现有)= RunnerCommandExecutor**:编排层全部测试通过 Fake/Process 双态命令注入,零外部 API 依赖;新的翻译引擎调用与 runner 各阶段一律从此缝注入 fake。先例:book_pipeline 现有 53 个 fake-backed 测试(含 归档仓 #38 切片的 8 个)。**现基线已长到 Rust 177 个测试(其中 book_pipeline 126)**。
- **状态缝(现有)= BookPipelineStateStore**:内存 store 断言持久化语义(原子性、幂等、审批哈希绑定),先例同上。
- **唯一新缝 = 引擎内 LLM Provider 边界**:翻译引擎的分块、上下文携带、checkpoint、标签保留、重试逻辑用 fake provider 驱动测试;真实后端(Ollama/云端)只做手动冒烟。**现基线 64 个引擎测试,全离线**。
- 各包既有套件原样保留:zotero-cli 的 smoke/phrases 测试、OCR 的路由与质量守卫逻辑;monorepo 迁移的验收标准之一是这些套件不改语义地继续通过。**已达成:zotero-cli 62、ocr 11、digest 13,全绿**。
- EPUB 产物以 EPUBCheck 作为 validate_reading 阶段的机器校验先例。
- **CI 门(2026-07-19,#87 补齐)**:此前 `.github/workflows/` 只有发版 workflow,不跑任何测试。现已加 `ci.yml`,PR 与 main push 触发引擎 pytest、前端 tsc + startup-contract、后端 cargo test。

## Out of Scope

- 上游公有领域 LifeBook 工作流(版权核查、公有领域发现、release 发布)——fork 已剥离,不随 monorepo 回归。
- Vendor/收编 TranslateBooksWithLLMs 的 AGPL 代码。
- 三仓之外其他项目(如 scholar-writing-assistant)的融合。
- 本 PRD 不含实施排期;实施 issue 集由地图完成时统一切分(标定决策 7:规划到可施工)。**已完成:第一批 #82–#86(均已落地并合入 main),第二批 #88–#99**。

## Further Notes

- 治理关系:本 PRD 与 Wayfinder 地图 归档仓 #40 互为表里——地图管「还有哪些决策没做」,本 PRD 管「目的地长什么样」。⏳ 项落地后应回填对应小节。
- 三仓现状与 TBL 机制的详细侦察报告见 归档仓 #40 首条评论;既有融合契约设计见 docs/prototypes 下的统一作业模型文档。
- ~~风险提示:本地 fork 与 origin 分叉未调和(#42)前,任何基线操作(push/pull/迁移)都可能造成返工;book-ocr-conversion 工作树有大量未提交内容,搬入前必须先在原仓收编。~~ **已消解**:#42 分叉调和完成(本地实现线成 trunk,ahead/behind=0);#53 收编 book-ocr-conversion 未提交工作完成;#55/#56 搬包完成。
- **仓库已于 2026-07-18 改名 `semantic-craft/bibliosmith`(#80)**。~~旧名自动跳转,本文中的 `books-translation` 链接仍可用。~~ **【迁移时校订】** 这一句现在不成立:开源重建后 `semantic-craft/bibliosmith` 是一个全新的公开仓,`books-translation` 的跳转指向的是被归档的 `semantic-craft/bibliosmith-private-archive`。#80 有意保留四处旧名:`docs/monorepo-migration-plan.md` 历史路径、`book_pipeline.rs` 报错字符串、pyproject 包名、launchd label——**这些不是待清理的残留**。

