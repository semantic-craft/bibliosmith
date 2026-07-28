# Wayfinder 地图:三仓合一 Monorepo 与 TBL 全功能双模式翻译流水线

Migrated: 2026-07-26 · Source: `semantic-craft/bibliosmith-private-archive#40`

> 本文迁自只读归档仓的 issue `#40`。地图已于 2026-07-19 到达自己声明的终点,是一份
> 决策记录而不是待办;归档仓整仓只读,所以它只能在本仓维护。选文档而非重开 issue 的
> 理由,以及归档编号到本仓编号的映射,见 `docs/planning/README.md`。
>
> 正文保持原文(中文)。**裸 `#NN` 一律是归档仓编号**;已重编到本仓的用 `本仓 #NN`
> 标注。原文里指向 `semantic-craft/books-translation` 与 `semantic-craft/bibliosmith`
> 的 issue 链接已全部改写为指向 `semantic-craft/bibliosmith-private-archive` —— 前者
> 现在跳转到归档仓,后者现在是开源重建后的另一个仓,两者都不再指向这些票。
>
> 与本图互为表里的 PRD 见 `docs/planning/prd-monorepo-dual-mode-pipeline.md`。

---

## Destination

books-translation 转为私有仓库并成为单一 monorepo:物理收编 book-ocr-conversion 与 zotero-cli-agent(对外保留 zsearch/zfulltext 等同名独立 CLI 入口),并锁定「原生重实现 TranslateBooksWithLLMs 全部功能、双模式翻译(程序化快速模式 + agent 专家模式)、expert_qa 闸门对两种输出通吃」所需的全部架构决策。地图完成 = 产出一套可直接施工的 implementation issues(取代 PRD 归档仓 #1/#10/#20);实现工作本身不在本图内。

## Notes

- 领域:本地书籍/文献「OCR/转换 → 入库/检索 → 翻译 → 阅读产物(HTML/EPUB/双语/digest)」完整流水线。
- 各会话按票型使用技能:/grilling、/domain-modeling(决策票)、/research(研读票)、/prototype(需要具象化对照时)。
- **标定决策**(建图 grilling 已锁定,约束所有票):
  1. 融合形态 = Monorepo 物理合并(非编排契约、非统一 CLI 门面)。
  2. zsearch/zfulltext 等 CLI 与每日 OCR 定时任务:收编进流水线的同时保留独立入口,两者兼得。
  3. TBL 功能全量纳入:程序化翻译引擎、多 LLM 后端、词汇表+refine、扩展格式与周边(SRT/DOCX/TTS/通知/进度 UI)。
  4. TBL 能力获取方式 = 原生重实现;不 vendor AGPL 代码(许可与架构双重原因)。
  5. 翻译分工 = 双模式可选:程序化快速模式与 agent 专家模式并存,每本书入队时选择;expert_qa 闸门对两种输出都生效。
  6. 仓库归宿 = 本仓库(semantic-craft/books-translation)转私有后作为 monorepo 宿主。
  7. 地图终点 = 规划到可施工(planning-only),实现走常规 ready-for-agent 流程。
  8. 旧票处置 = 地图接管规划;#37/#38/#39/#7/#8 挂起待重写,PRD 归档仓 #1/#10/#20 在地图完成时被新实施 issue 集取代并关闭。
- 关键参考:`docs/prototypes/unified-pipeline-job-model.md`(既有三仓融合契约设计,merge 的起点)、`docs/monorepo-migration-plan.md`(布局与迁移方案,已定稿)、`docs/book-pipeline-capability-matrix.md`(现状边界,2026-07-09 后部分过时)、本 issue 首条评论(三仓库 + TBL 四方侦察报告)。
- 基线状态(2026-07-14 起):宿主已是私有、非-fork 仓;本地实现线即 trunk,ahead/behind=0,可正常 pull/push。历史上的 fork 分叉已消解,细节见「调和本地 fork 与 origin 的分叉基线」票;旧公开 fork 已删除,其叙事存 `archive/origin-main-2026-07-13` 分支。
- 改名(2026-07-18,#80):仓库已改名 `semantic-craft/bibliosmith`。**【迁移时校订】** 早期条目里的 `books-translation` 链接已在本文中改写为指向 `semantic-craft/bibliosmith-private-archive`;`semantic-craft/bibliosmith` 现在是开源重建后的另一个仓,不再是这些票的宿主。

## Decisions so far

<!-- 票据解决后逐条登记:- [票名](链接) — 一句话答案 -->

- [转私有:books-translation 仓库私有化](https://github.com/semantic-craft/bibliosmith-private-archive/issues/41) — fork 无法直接转私有;以迁移达成:旧 fork 改名后删除,全新私有非-fork 宿主仓建立,67 票保号迁入。
- [调和本地 fork 与 origin 的分叉基线](https://github.com/semantic-craft/bibliosmith-private-archive/issues/42) — 本地实现线成为 truth/trunk(origin 唯一独有内容仅 4 行 gitignore,不吸收);分叉消解,ahead/behind=0。
- [定夺 monorepo 布局与迁移方案](https://github.com/semantic-craft/bibliosmith-private-archive/issues/43) — packages/{ocr,zotero-cli,translation-engine,digest} + tools/lifebook-launcher;Python 统一 uv workspace;zotero-cli filter-repo 保史 + ocr 快照;根单一 .env;方案文档 docs/monorepo-migration-plan.md。施工已切片:骨架+digest(#68)→ 搬包(#55/#56)→ launcher 接线(#69)/根 .env(#70)→ Windows 机收口(#71)。
- [研读 TranslateBooksWithLLMs 源码,提炼翻译引擎机制规格](https://github.com/semantic-craft/bibliosmith-private-archive/issues/44) — 机制报告落 `research/tbl-mechanisms` 分支(含吴恩达 translation-agent 对照):TBL 结构安全靠 `[idN]` 占位符隔离 HTML + 三阶段降级(重试→比例对齐插回→保留原文),校验只查数量/索引不查位置;--refine 只看译稿、机制上不可修误译;reflection(四维批评)可修忠实度但全文上下文 O(n²),不改造不可用;词汇表有翻译遍/refine 遍/reflect 维度三个语义不同挂点。refine 与 reflection 是不同轴而非强弱版——供引擎架构与词汇表/refine 分工两票拍板。
- [翻译引擎技术栈与总体架构](https://github.com/semantic-craft/bibliosmith-private-archive/issues/45) — Python(uv workspace 成员 packages/translation-engine);外部适配器 CLI 沿 RunnerCommandExecutor,每 translate 阶段一调(unit 清单进、逐单元状态出);核心吃占位符保护后的文本单元,Markdown 章节适配器首建、EPUB/DOCX/SRT 直译留适配器槽位;v2 作业状态唯一真相源,引擎块级断点私有旁挂(记幂等键、成功即删);契约语对通用,首期只发 zh-Hans 目标语 profile。#46/#47/#48 的缝分别留在 Provider 接口、translate 阶段调用契约、二遍 pass 插槽+目标语 profile。
- [多 LLM 后端抽象与凭证配置](https://github.com/semantic-craft/bibliosmith-private-archive/issues/46) — Provider 集合=OpenAI 兼容+Gemini 原生(无 Anthropic 原生/无 Ollama→本地串行子项消解);key 轮换=全量多-key KeyPool+429 独立预算(填 #58 CredentialPool seam);配置=非密注册表(profile_id+config_id→provider/base_url/model/timeout/并发/chunk/key_env,闸门绑 ID,密钥运行时从根 .env 解析);并发=章内串行带尾 25 词上下文+章间并行;失败=类型化错误(RateLimitError→轮换/TransientError→有界退避/FatalError→快速失败)无熔断器;凭证=根单一 .env、单变量逗号分隔多 key、绝不进作业记录/日志/git。
- [双模式 translate 阶段与闸门契约(#38 重写规格)](https://github.com/semantic-craft/bibliosmith-private-archive/issues/47) — 模式=v2 intent 枚举 fast|expert 入队写死(改模式=披露闸失效重审);专家模式 translate=同一 unit 产物契约两种生产者(引擎 CLI vs agent 会话),runner 只验收哈希/清单、advance 关单元;approve_translation 两模式统一(专家绑 agent profile+技能标识),晋升闸天然同构;快速模式 expert_qa=QA policy 分层(自动化全量+专家抽检+缺陷扩检,晋升闸绑 policy+抽检证据);EPUBCheck 必过、阅读器实测可选证据;重试/断点无新机制(v2+#45/#46 覆盖)。#38 重写规格即此记录,施工=#63→#64/#65(措辞已校准);#48 解锁。
- [词汇表/refine 与专家技能的分工界线](https://github.com/semantic-craft/bibliosmith-private-archive/issues/48) — 词汇表=书籍项目单一事实源(prepare 种子化哈希入闸门,四消费者同源:快速翻译遍注入/自动化QA术语检查/专家技能/NER 人审写回;跨书联动留缝);二遍 pass=合并为唯一的窗口化 reflection(reflect(源+稿)→improve,可修忠实度,书级可选;纯无源 refine 砍掉留缝;#8 关闭、#62 重写吸收);NER=管线外按需命令(人工触发=披露同意,解闸门悖论;#61 重写);缺陷家族=人工回灌进目标语 profile 版本升级反哺。雾区「reflection 归宿」落定。

- [Zotero 发现与抽取在 monorepo 下的接线(#37 重写规格)](https://github.com/semantic-craft/bibliosmith-private-archive/issues/49) — 流水线 discover 单认 zotero-cli 只读快照;冻结 collection membership 后逐附件持久推进 route→extract→index→handoff,索引失败独立重试;物理存储格局留给平行票。

- [状态存储格局:统一还是联邦](https://github.com/semantic-craft/bibliosmith-private-archive/issues/50) — 保持**联邦式**持久化(ADR `docs/adr/0001-federated-book-pipeline-state.md`,Accepted 2026-07-17):Book Pipeline 状态只作**编排状态**的单一事实源,各子系统(launcher 作业 JSON、OCR SQLite、每书项目文件、sqlite-vec 索引)各自是自己存储的唯一写入方;翻译 checkpoint 归引擎所有,编排层只记 checkpoint 身份、输入 hash、契约版本与完成摘要。跨 store 无分布式事务:组件先提交自身输出并返回证据记录,编排层校验证据后在自身状态原子提交 stage 转移;启动/重试/恢复时对存储引用做 reconciliation(解析引用 → 校验身份/契约版本/SHA-256/完成证据 → 全匹配才复用,否则显式 blocked/failed),**绝不凭文件名或组件行为推断完成**。编排状态只含不透明 ID、hash、计数、契约版本、安全相对引用与隐私类别。

- [扩展格式与周边的范围与排序](https://github.com/semantic-craft/bibliosmith-private-archive/issues/51) — ADR `docs/adr/0002-progress-and-terminal-notifications.md`(Accepted 2026-07-17)。**v1 范围只两项**:(1)Tauri launcher 内实时 stage/unit 进度——公开作业状态暴露聚合 stage 计数、百分比、活动 stage 及其 unit 摘要,前端轮询既有状态命令渲染,**不新建独立 web UI**;(2)每个终态一条 webhook——run/retry/advance 达到 completed/partial/failed/blocked/skipped 后派发,事件 ID 确定性生成并作 `Idempotency-Key`,载荷不含书名/路径/日志/错误详情/私有文本/凭证,`BOOK_PIPELINE_WEBHOOK_URL` 仅走环境,未配置则零通知副作用。**SRT 输入、DOCX 输入、TTS 音频显式延期出 v1**,各留待后续 input-adapter / reading-output 票——延期本身即本票的范围决定。

- [独立品牌:目录 + GitHub 仓改名 bibliosmith](https://github.com/semantic-craft/bibliosmith-private-archive/issues/80) — 仓名/目录/LAN 镜像/两个 remote 全部改为 bibliosmith(2026-07-18,旧名自动跳转);4 个 worktree 以 `git worktree repair` 修复。旧路径留软链 `~/Projects/local-reading-translations → bibliosmith` 作过渡,**拆链前须先重建 launcher**(已装二进制的 `CARGO_MANIFEST_DIR` 烤了旧路径,拆链后 OCR 根会静默回退 legacy)。**有意保留**旧名的四处:`docs/monorepo-migration-plan.md` 历史路径、`book_pipeline.rs` 报错字符串、pyproject 包名、launchd label(装饰性/历史性)。

## 地图终点:已达成(2026-07-19)

标定决策 7 定义「地图完成 = 产出一套可直接施工的 implementation issues」。该产出物现已齐备,分两批:

**第一批(2026-07-18 立票,现已全部落地并合入 main)** — #82 围栏代码块占位符保护、#83 textCleanup 提示词小节、#84 占位符保留 few-shot、本仓 #31 预飞样本 Sample & Compare、#86 每书自定义翻译指令。

**第二批(2026-07-19,按 PRD 34 条 user story 逐条盘点后切出)**:

| 票 | 覆盖 | 性质 |
|---|---|---|
| 归档仓 #88 | story 16 全链自动推进 | **已定性 B**(2026-07-26):改 PRD 措辞、不写代码;见 PRD story 16 |
| 本仓 #27 | story 26 有界自动重试 + 剩余次数可观测 | 编排层当前无任何自动重试 |
| 归档仓 #90 | story 17 digest UI 闸口 | **已消解**:`NewJobWizard.tsx` 现有勾选框 |
| 归档仓 #91 | 预飞样本未携带 textCleanup/customInstructions | **已消解**:参数透传已落地 |
| 本仓 #21 | story 27 清理审批结构化并绑定 validate_reading | 审批目前是会被截断的日志行 |
| 本仓 #33 | story 19 运行面失修 | 活故障:zsearch 同步任务 + PATH CLI |
| 本仓 #30 | #46 已决的章间并行 | `concurrency_limit` 目前是死配置 |
| 本仓 #28 | story 24 webhook 重复投递 | `updated_at` 折进 event id |
| 本仓 #29 | story 18 后半 阅读器实测证据槽 | 可选证据当前无处可填 |
| 本仓 #34 | story 21 Windows 机 runbook 引导路径失修 | 改名后未再验证 |
| 本仓 #32 | story 10 术语表输出侧校验 | 目前只有 prompt 注入 + 哈希绑定 |
| 本仓 #35 | story 17 双语构建脚本双份分叉 | **已消解**:旧手工发布构建器随旧流程移除，只保留 Launcher 构建器；见 `docs/bilingual-epub-builder.md` |

**盘点结论(34 条 user story)**:17 条已落地、11 条部分落地(均有对应施工票)、5 条按决定不做或改形、1 条仍未决。逐条状态见 PRD(`docs/planning/prd-monorepo-dual-mode-pipeline.md`)。

⚠️ 「按决定不做」这 5 条要与「欠账」区分开:story 8(Ollama)、11(NER 自动)、12(纯 refine)是地图票 #46/#48 推翻了 PRD 原措辞;story 22/23(SRT/DOCX/TTS)是 #51 显式延期出 v1;**story 20(每日自动 OCR)是用户 2026-07-18 明确停用**——4 个 launchd 任务已 unload 移入废纸篓,重建需用户重新拍板,任何票都不得顺手装回去。

## Not yet specified

- **语义检索(sqlite-vec)与翻译流水线的联动场景**(如跨书术语一致性检索)是否立项 —— 仍是雾区,对应 PRD story 34。sqlite-vec 索引本身已落地并双向接线(`packages/zotero-cli/src/zotero_cli/vector_store.py`,`test_item_index.py` 11 个测试),缺的只是「拿它反哺翻译质量」这个场景的立项判断。
- **story 16「全链自动推进」与 ADR 0002「审批不执行下一阶段」的措辞冲突** —— 已在归档仓 #88 内摆出 A(加 `advance_to_quiescence`)/ B(改 PRD 措辞)两个候选,**已于 2026-07-26 定性为 B** —— 改 PRD 措辞、不写代码,理由与改写后的措辞见 PRD story 16。
- **SRT / DOCX / TTS 的重启时机** —— 归档仓 #51 已决定延期出 v1 并留待后续 input-adapter / reading-output 票,但何时开这些票尚无判据。三者当前在树内零脚手架(唯一相关命中是 `packages/ocr/mineru.py:43` 把 `.docx` 当**抽取输入**压成 Markdown,不是结构保留翻译)。

原「未定」项中已消解、移出本节的:实现 issue 集的切分与排序(见上「地图终点」);TTS/webhook/进度 UI 的规格(#51 定,webhook 与进度 UI 均已落地);术语 NER 的技术方案(#48 定为管线外按需命令,已落地为 `translation-engine-ner`);#39 溯源/结构化脱敏/安全打开的重写规格(已落地,产物携带内容寻址 id、producer、input_hashes、source_refs、privacy,且 `validate_artifact_contract` 会拒绝不完整溯源);digest 与双语 EPUB 在自动化 runner 的接入点(已接入 build_reading/validate_reading,digest 仅差 UI 闸口 → #90)。

## Out of scope

- Vendor/收编 TranslateBooksWithLLMs 的 AGPL 代码——建图时已决策排除,仅原生重实现。
- 三个仓库之外其他项目(如 scholar-writing-assistant)的融合。
