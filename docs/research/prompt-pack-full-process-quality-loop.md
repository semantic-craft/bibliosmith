# “全流程审校闭环”提示词方案的机制与授权边界（#171）

> 研究对象：[`SaberOnGo/public-domain-books-translation`](https://github.com/SaberOnGo/public-domain-books-translation)。
>
> 固定上游：[`219f452311817a09cfbe59d7e345a1ac2a81faa8`](https://github.com/SaberOnGo/public-domain-books-translation/commit/219f452311817a09cfbe59d7e345a1ac2a81faa8)，提交时间 2026-07-13 08:03:29 +08:00，标题 `Publish release artifacts and strengthen coverage gates`。2026-08-05 通过 GitHub API 与 `git ls-remote` 核验，该提交仍是 `main` 的 HEAD。
>
> 方法：在固定提交上逐文件阅读主控、英语到简体中文阶段提示词、通用质量门禁、两个翻译质量 skill 和仓库许可证；只归纳机制，不复制上游长篇 prompt。法律部分是工程边界建议，不是针对具体司法辖区的法律意见。

## 结论

“全流程审校闭环”不应实现成一段可编辑的巨型提示词，也不应把 LifeBook 的公开出版流水线整体搬进 BiblioSmith。上游的可复用核心是一个**有状态、多角色、可回退的专家代理阶段图**：

1. 译前建立本书研究、文体画像、试译结论和受控术语；
2. 分章翻译调用保持精简，只注入当前原文、必要上下文、少量相关规则和命中的术语；
3. 每章先做目标语独立顺读，再回到原文校准，并分别完成忠实度、可读性/意象、术语审校；
4. 任何修复轮都不能直接算 PASS，必须追加一次没有新问题、没有新修复的整章复查；
5. 章节证据齐全后才可进入终稿；全书再交给相互独立的内容评审者；
6. 抽检发现一处问题时，把它视为可能的问题族，先全书找同类、修复确认命中、记录例外并定点关闭，再用新的检查轮证明闭环；
7. 评审失败按问题类型回到研究、试译、翻译、术语或章节审校，而不是在最终成品上解释或局部粉饰。

应明确排除上游的版权/公版判断、在线找书与抓取、公开出版、封面与发行元数据、release/versioning、对外发布措辞，以及运行过程中自动修改全局模板或内置方案。BiblioSmith 的本地材料边界和输出/隐私规则由产品与执行器掌管，不能由 Prompt Pack 覆盖。

授权上，最保守的判断是：上游具体 prompt、skill 与说明文字属于“非代码创作内容”，按 `CC BY-NC-SA 4.0` 处理；不能因为仓库中的脚本是 MIT 就把提示词也当作 MIT。若复制或改写其具体表达，内置方案必须作为单独标识的 CC 内容资产保存署名、许可证、源提交、逐文件来源和改编说明，并且不能进入商业用途，除非取得另行许可。若要避免把产品发行受限于该非商业许可证，应只吸收可核验的流程机制，使用 BiblioSmith 原创措辞重新实现，并保留“机制参考”来源记录；商业发布前仍应做法律复核或取得许可。

## 1. 固定来源与材料分层

### 1.1 这不是一个 prompt

上游英语模板的主控按固定顺序串联 21 个步骤，从 ingest、研究、试译、术语、翻译、章节审校，一直到 EPUB、独立评审、release 和复盘；顺序直接写在 [`00_orchestrator_zh_en.md`](https://github.com/SaberOnGo/public-domain-books-translation/blob/219f452311817a09cfbe59d7e345a1ac2a81faa8/template/epub_pipeline/English-to-Simplified-Chinese/prompts/00_orchestrator_zh_en.md#L40-L64)。真正构成行为的材料分成四层：

| 层 | 上游代表文件 | 实际作用 | Prompt Pack 中的处理 |
| --- | --- | --- | --- |
| 主控与状态机 | [`MASTER_PROMPT.md`](https://github.com/SaberOnGo/public-domain-books-translation/blob/219f452311817a09cfbe59d7e345a1ac2a81faa8/template/epub_pipeline/English-to-Simplified-Chinese/MASTER_PROMPT.md#L20-L57)、[`00_orchestrator_zh_en.md`](https://github.com/SaberOnGo/public-domain-books-translation/blob/219f452311817a09cfbe59d7e345a1ac2a81faa8/template/epub_pipeline/English-to-Simplified-Chinese/prompts/00_orchestrator_zh_en.md#L40-L64)、[`PIPELINE_SPEC.md`](https://github.com/SaberOnGo/public-domain-books-translation/blob/219f452311817a09cfbe59d7e345a1ac2a81faa8/template/epub_pipeline/common/PIPELINE_SPEC.md#L26-L64) | 定义顺序、状态、产物和回退 | 转成阶段图与执行器契约，不作为一段模型输入 |
| 阶段提示词 | `03`–`11`、`16`、`16a`、`17` | 给某一角色一项有边界的任务 | 形成只读内置方案的阶段模板 |
| 质量技能 | [`expert-translation-quality`](https://github.com/SaberOnGo/public-domain-books-translation/blob/219f452311817a09cfbe59d7e345a1ac2a81faa8/skills/expert-translation-quality/SKILL.md#L19-L68)、[`translation-quality-defect-families`](https://github.com/SaberOnGo/public-domain-books-translation/blob/219f452311817a09cfbe59d7e345a1ac2a81faa8/skills/translation-quality-defect-families/SKILL.md#L19-L39) | 提供跨阶段的专家判义和问题族闭环方法 | 引用 BiblioSmith 已有对应 skill，不在每次请求中重复整篇注入 |
| 发布与模板治理 | `12`–`15`、`18`、`18a`、`19`，rights/release references | 做 EPUB、公开发布、版本与模板演进 | 从该翻译提示词方案中排除 |

固定提交包含 11 个“源语言 → 简体中文”模板目录；各目录都使用同一套 `00`–`19` 加 `08a` 的阶段骨架，仅源语言规则和少量附加节点不同（[固定提交的模板树](https://github.com/SaberOnGo/public-domain-books-translation/tree/219f452311817a09cfbe59d7e345a1ac2a81faa8/template/epub_pipeline)）。这支持“**通用阶段图 + 源语言覆盖层**”，不支持把英语模板原文冒充为“任意源语”方案。

### 1.2 固定修订的来源清单

首个内置修订需要至少把下列上游文件登记为来源，而不是只记录仓库首页：

- 主控事实：`template/epub_pipeline/English-to-Simplified-Chinese/{MASTER_PROMPT.md,SKILL.md,AGENTS.md}`、`prompts/00_orchestrator_zh_en.md`、`common/{PIPELINE_SPEC.md,automation_contract.md}`；
- 译前阶段：`prompts/03_global_translation_research_zh_en.md`、`04_book_specific_research_zh_en.md`、`05_pretranslation_trials_zh_en.md`、`06_glossary_style_zh_en.md`；
- 翻译与章内审校：`07_translate_chapters_zh_en.md`、`08a_chapter_post_translation_control_zh_en.md`、`08_review_fidelity_zh_en.md`、`09_review_readability_imagery_zh_en.md`、`10_review_terminology_zh_en.md`、`11_chapter_quality_gate_zh_en.md`；
- 独立评审与闭环：`16_independent_review_agents_zh_en.md`、`17_revision_routing_zh_en.md`、`common/prompts/16a_stratified_random_spotcheck.md`；
- 横切技能：`skills/expert-translation-quality/SKILL.md`、`skills/translation-quality-defect-families/SKILL.md`；
- 授权：`license/LICENSE.en.md`、`license/CONTRIBUTING.en.md`、`license/COMMERCIAL_LICENSE.en.md`。

不要把 `books/**` 中的具体书稿、译文、QA、封面或 EPUB 当作方案来源或测试资产。

## 2. 真正构成“全流程审校闭环”的机制

### 2.1 译前研究与校准

译前能力不是一篇百科研究报告，而是四类可供后续阶段消费的约束产物：

| 阶段 | 上游事实 | 可复用机制 |
| --- | --- | --- |
| 通用质量基线 | `03` 读取通用研究与质量标准，并产生本次采用规则的确认记录（[`03` 第 10–30 行](https://github.com/SaberOnGo/public-domain-books-translation/blob/219f452311817a09cfbe59d7e345a1ac2a81faa8/template/epub_pipeline/English-to-Simplified-Chinese/prompts/03_global_translation_research_zh_en.md#L10-L30)） | 将方案自己的稳定质量原则解析为运行上下文；不要求每本书重新上网研究 |
| 本书研究与文体画像 | `04` 要求识别作者/时代/目的/读者、难点、关键词层级、句法、意象边界，并生成 book research 与 style profile（[`04` 第 13–53 行](https://github.com/SaberOnGo/public-domain-books-translation/blob/219f452311817a09cfbe59d7e345a1ac2a81faa8/template/epub_pipeline/English-to-Simplified-Chinese/prompts/04_book_specific_research_zh_en.md#L13-L53)） | 产出结构化的书级上下文；只基于用户本地材料和用户允许的研究来源 |
| 试译校准 | `05` 从开篇、动作、术语/历史敏感、修辞/结尾等位置选 3–5 个样本，保留失败并在全部通过后放行批量翻译（[`05` 第 13–71 行](https://github.com/SaberOnGo/public-domain-books-translation/blob/219f452311817a09cfbe59d7e345a1ac2a81faa8/template/epub_pipeline/English-to-Simplified-Chinese/prompts/05_pretranslation_trials_zh_en.md#L13-L71)） | 把“风格是否可执行”变成先验证后运行的校准门禁 |
| 术语与风格锁定 | `06` 根据研究与试译更新术语表、专名表、style guide 与 style profile，并区分 locked/preferred/avoid/note-only（[`06` 第 9–40 行](https://github.com/SaberOnGo/public-domain-books-translation/blob/219f452311817a09cfbe59d7e345a1ac2a81faa8/template/epub_pipeline/English-to-Simplified-Chinese/prompts/06_glossary_style_zh_en.md#L9-L40)） | 以相关术语行和少量核心规则供后续按需注入，避免把整份规则塞入每个请求 |

其中 `03` 的“全局研究”在上游更像读取静态规则后的确认节点，不应被产品误标为每次都需要外部搜索的模型提示词。

### 2.2 分章翻译

`07` 的关键价值不在文学示例，而在输入卫生和角色隔离：

- 只有预翻译报告 PASS 才启动；输出写到对应章节译稿（[`07` 第 14–24 行](https://github.com/SaberOnGo/public-domain-books-translation/blob/219f452311817a09cfbe59d7e345a1ac2a81faa8/template/epub_pipeline/English-to-Simplified-Chinese/prompts/07_translate_chapters_zh_en.md#L14-L24)）。
- 一次翻译调用只给当前章节/段落组、文体画像中最关键的 5–8 条规则、实际命中的术语和必要上下文；release、版权、EPUB、QA 路径等不进入翻译请求（[`07` 第 26–41 行](https://github.com/SaberOnGo/public-domain-books-translation/blob/219f452311817a09cfbe59d7e345a1ac2a81faa8/template/epub_pipeline/English-to-Simplified-Chinese/prompts/07_translate_chapters_zh_en.md#L26-L41)）。
- 翻译阶段只输出译文；审校、解释和流程记录由后续角色生成，避免让同一次调用同时扮演译者、审校员和书记员（[`07` 第 20–24 行](https://github.com/SaberOnGo/public-domain-books-translation/blob/219f452311817a09cfbe59d7e345a1ac2a81faa8/template/epub_pipeline/English-to-Simplified-Chinese/prompts/07_translate_chapters_zh_en.md#L20-L24)）。
- 多义词由翻译阶段先负责；无法可靠消歧时才保留不错误收窄的表达并进入后文回看（[`07` 第 85–87 行](https://github.com/SaberOnGo/public-domain-books-translation/blob/219f452311817a09cfbe59d7e345a1ac2a81faa8/template/epub_pipeline/English-to-Simplified-Chinese/prompts/07_translate_chapters_zh_en.md#L85-L87)；对应专家 skill 的三窗口回看见[第 36–68 行](https://github.com/SaberOnGo/public-domain-books-translation/blob/219f452311817a09cfbe59d7e345a1ac2a81faa8/skills/expert-translation-quality/SKILL.md#L36-L68)）。

因此，方案保存的是阶段模板和所需输入槽位；原文、相邻上下文、术语行和书级画像由执行器在本机即时注入，不属于内置方案内容。

### 2.3 整章控制与多维审校

上游不是“翻译后再润色一次”，而是把不同证据职责拆开：

| 节点 | 只回答什么问题 | 主要证据 |
| --- | --- | --- |
| 整章译后控制 `08a` | 当前章作为中文书是否自然，并在回到原文后仍忠实？是否存在术语、注释、读者可见接口问题？ | 先目标语独立阅读，再源文校准；完整范围见 [`08a` 第 52–84 行](https://github.com/SaberOnGo/public-domain-books-translation/blob/219f452311817a09cfbe59d7e345a1ac2a81faa8/template/epub_pipeline/English-to-Simplified-Chinese/prompts/08a_chapter_post_translation_control_zh_en.md#L52-L84) |
| 忠实度 `08` | 是否漏译、误译，人物/数字/时间/因果/语气是否偏移？ | 源章 + 译章 + 术语表；[`08` 第 9–24 行](https://github.com/SaberOnGo/public-domain-books-translation/blob/219f452311817a09cfbe59d7e345a1ac2a81faa8/template/epub_pipeline/English-to-Simplified-Chinese/prompts/08_review_fidelity_zh_en.md#L9-L24) |
| 可读性/意象 `09` | 目标语独立阅读是否成立；润色后有无越界、压缩或事实漂移？ | 两段式审校；[`09` 第 10–40 行](https://github.com/SaberOnGo/public-domain-books-translation/blob/219f452311817a09cfbe59d7e345a1ac2a81faa8/template/epub_pipeline/English-to-Simplified-Chinese/prompts/09_review_readability_imagery_zh_en.md#L10-L40) |
| 术语 `10` | 专名、术语、历史称谓、注号和禁用写法是否按书级规则一致？ | 译章 + 术语/专名表 + style profile；[`10` 第 12–36 行](https://github.com/SaberOnGo/public-domain-books-translation/blob/219f452311817a09cfbe59d7e345a1ac2a81faa8/template/epub_pipeline/English-to-Simplified-Chinese/prompts/10_review_terminology_zh_en.md#L12-L36) |
| 章节门禁 `11` | 上述证据是否足以把译稿提升为终稿？ | control + 三类审校；缺失或非零问题 PASS 时直接阻断，见 [`11` 第 17–23 行](https://github.com/SaberOnGo/public-domain-books-translation/blob/219f452311817a09cfbe59d7e345a1ac2a81faa8/template/epub_pipeline/English-to-Simplified-Chinese/prompts/11_chapter_quality_gate_zh_en.md#L17-L23) |

最重要的闭环不变量是：**发现并修复问题的那一轮只能记为 `FIXED_RECHECK_REQUIRED`；随后必须再有一轮整章检查，且该轮 `issues_found=0`、`fixes_applied=0`、`unresolved_blocking_issues=0` 才可 PASS。**上游在 `08a` 的返工规则和 PASS 条件中明确重复这一点（[`08a` 第 101–120 行](https://github.com/SaberOnGo/public-domain-books-translation/blob/219f452311817a09cfbe59d7e345a1ac2a81faa8/template/epub_pipeline/English-to-Simplified-Chinese/prompts/08a_chapter_post_translation_control_zh_en.md#L101-L120)、[第 141–157 行](https://github.com/SaberOnGo/public-domain-books-translation/blob/219f452311817a09cfbe59d7e345a1ac2a81faa8/template/epub_pipeline/English-to-Simplified-Chinese/prompts/08a_chapter_post_translation_control_zh_en.md#L141-L157)）。这应是执行器验证的状态规则，而不是依赖模型自称已经复查。

### 2.4 独立评审

上游要求主执行者不能自证完成，至少派生两个互不参考的评审角色：Agent A 负责内容与翻译，Agent B 负责 EPUB 工程、排版、metadata、封面与可读性（[`16` 第 3–16 行](https://github.com/SaberOnGo/public-domain-books-translation/blob/219f452311817a09cfbe59d7e345a1ac2a81faa8/template/epub_pipeline/English-to-Simplified-Chinese/prompts/16_independent_review_agents_zh_en.md#L3-L16)）。

BiblioSmith Prompt Pack 应保留：

- 独立于主译者的内容/翻译评审角色；
- 评审者不能读取其他评审者的结论；
- 对每个被审单元给出问题、严重级别、证据与返工位置；
- 多义词、术语、忠实度与中文顺读均不能被平均分掩盖。

应排除 Agent B 中封面、公开发布 metadata、release 文案等出版职责。表格、公式、图片文字接口如确实影响译文理解，可以作为翻译质量审校输入；OPF、spine、封面像素、发布目录等留给布局/产物验证组件。

### 2.5 抽检发现与问题族全书闭环

抽检在上游只负责发现盲点，不是主要润色引擎。真正可复用的是“从一个样本扩展到全书同类”的闭环：

1. 抽检样本由确定性工具生成并记录 seed；评审者不能由主执行者挑选“看起来没问题”的段落（[`16` 第 47–72 行](https://github.com/SaberOnGo/public-domain-books-translation/blob/219f452311817a09cfbe59d7e345a1ac2a81faa8/template/epub_pipeline/English-to-Simplified-Chinese/prompts/16_independent_review_agents_zh_en.md#L47-L72)）。
2. 任一问题先归纳为潜在问题族；使用术语表、禁用写法、标题映射、`rg` 和小上下文源译对照等低成本证据收集候选（[`translation-quality-defect-families` 第 19–39 行](https://github.com/SaberOnGo/public-domain-books-translation/blob/219f452311817a09cfbe59d7e345a1ac2a81faa8/skills/translation-quality-defect-families/SKILL.md#L19-L39)、[第 81–92 行](https://github.com/SaberOnGo/public-domain-books-translation/blob/219f452311817a09cfbe59d7e345a1ac2a81faa8/skills/translation-quality-defect-families/SKILL.md#L81-L92)）。
3. 只把候选及其邻近源文交给评审角色确认；修复所有确认命中，并记录合理例外。
4. 在旧轮次中记录检索范围、命中、修复和定点关闭；随后用新的 seed/新检查轮复核，旧 PASS 不能替代当前运行（[`16a` 第 121–135 行](https://github.com/SaberOnGo/public-domain-books-translation/blob/219f452311817a09cfbe59d7e345a1ac2a81faa8/template/epub_pipeline/common/prompts/16a_stratified_random_spotcheck.md#L121-L135)）。

上游还要求把可复用教训自动回填到全局 `translation-quality-defect-families` skill（[`16a` 第 63–67 行](https://github.com/SaberOnGo/public-domain-books-translation/blob/219f452311817a09cfbe59d7e345a1ac2a81faa8/template/epub_pipeline/common/prompts/16a_stratified_random_spotcheck.md#L63-L67)）。BiblioSmith 应只保留**书内问题族闭环**；运行一个不可变内置方案不得自动修改该方案或仓库 skill。跨书经验应成为显式、另行批准的方案新修订或 skill 维护工作。

### 2.6 回退路由

`17` 按问题类型把工作退回到研究、试译、单章翻译、书级规则、术语、章节门禁或制作阶段，并要求记录严重级别、修改文件和重新验证项（[`17` 第 13–44 行](https://github.com/SaberOnGo/public-domain-books-translation/blob/219f452311817a09cfbe59d7e345a1ac2a81faa8/template/epub_pipeline/English-to-Simplified-Chinese/prompts/17_revision_routing_zh_en.md#L13-L44)）。对 Prompt Pack 有用的是翻译相关路由：

- 原则/整书共性失败 → 回本书研究或 style profile；
- 校准失败 → 回试译；
- 单章问题 → 回分章翻译或整章控制；
- 术语问题 → 回术语表与术语审校；
- 证据缺失 → 回对应审校，不得直接提升为终稿。

版权、来源、封面、EPUB 工程和 release 路由不属于该方案。

## 3. BiblioSmith 必须排除的上游内容

BiblioSmith 的根 README 明确：它只处理用户电脑上已有材料，仓库没有发现书源、版权审查或出版流水线，也不判断一本书能否出版（[`README.md`](https://github.com/semantic-craft/bibliosmith/blob/a9ab294e32f8aa1f471864eb848eece311c5652e/README.md#L1-L14)）。项目指令进一步禁止搜索书籍全文、绕过访问控制以及自行作出版/授权判断（[`AGENTS.md`](https://github.com/semantic-craft/bibliosmith/blob/a9ab294e32f8aa1f471864eb848eece311c5652e/AGENTS.md#L14-L19)）。因此，下列内容不能进入内置方案的可编辑或可执行部分：

| 排除项 | 上游位置/表现 | 原因与替代归属 |
| --- | --- | --- |
| 公版/版权/授权判断 | `01_ingest_clean`、rights checklist、public/private publication mode | 产品只核对本地文件身份；任何法律判断均不由 Prompt Pack 生成 |
| 在线找书、下载和来源 URL | `SOURCE_URL`、public-domain source evidence | 本地阅读边界禁止用方案扩张为找全文工具 |
| “私人自用即可合法”的结论或声明模板 | `modes/private_use`、private-use declaration | Prompt 不能把用途声明当成权利判断；隐私与存储边界由产品强制 |
| EPUB 生产与出版设计 | `12`–`15`、封面、字体、OPF、spine、publication lint | 这些是布局/构建组件职责，不是翻译提示词特征；只保留影响译文理解的文字接口检查 |
| 公开发行与版本化 release | `18`、`18a`、`output/release/`、release notes | BiblioSmith 写 `output/reading/`，不把本地阅读产物自动升级为公开发布物 |
| 发布/商业授权文案 | public-domain notices、commercial license management | 方案只记录自身来源许可证，不替用户作品作授权陈述 |
| 运行时全局模板治理 | `19_retrospective_template_update`、自动回填全局 skill/template | 内置修订不可变；任何升级只能产生经审批的新修订 |
| 上游具体书籍资产 | `books/**` 的原文、译文、QA、封面、EPUB | 非方案机制，且 BiblioSmith 明令不得打包继承的书籍内容 |

仍可由 BiblioSmith 自己的输出管线执行 EPUBCheck 或布局检查，但它们不应被标作“全流程审校闭环”方案的 prompt 阶段，也不能被自定义方案关闭。

## 4. 给统一 Prompt Pack 阶段模型的事实清单

以下不是最终数据结构决定，而是后续设计不能丢失的能力事实。

### 4.1 最小阶段图

| 建议阶段类别 | 角色 | 必要输入 | 产出/门禁 |
| --- | --- | --- | --- |
| `book_research` | 研究/定向 Agent | 本地书级材料、方案质量基线 | book research、style profile |
| `calibration_trials` | 试译 Agent | 风险分层样本、style profile | 试译记录；全部通过才放行 |
| `glossary_style` | 术语 Agent | 研究、试译、源章 | terms/proper nouns/style revision |
| `translate_unit` | 译者 | 当前源文、必要邻近上下文、5–8 条核心规则、命中术语 | 只输出译文草稿 |
| `chapter_full_control` | 章级审校者 | 整章源译、书级规则、术语 | 目标语先行 + 源文校准；修复轮非 PASS |
| `fidelity_review` | 忠实度审校者 | 整章源译 | 逐项问题与证据 |
| `readability_review` | 目标语编辑 | 译章；之后才给源章 | 中文顺读/节奏问题及校准结果 |
| `terminology_review` | 术语审校者 | 译章、术语/专名规则 | 漂移、禁用写法、例外 |
| `chapter_gate` | 聚合门禁 | control + 三类审校 | 只在最新零问题复查后提升终稿 |
| `independent_content_review` | 与主译者隔离的评审者 | 终稿样本、源文小上下文、相关规则 | 独立问题表与返工决定 |
| `defect_family_closure` | 主控 + 专项评审 | 抽检发现、全书候选、源译对照 | 问题族、全书审计、修复、例外、干净复查 |
| `revision_route` | 主控 | 所有未关闭问题 | 精确回退阶段和重新验证项 |
| `final_translation_gate` | 聚合门禁 | 全部章门禁、独立评审、问题族关闭证据 | 翻译质量完成；不代表 release/出版完成 |

### 4.2 每个阶段需要显式声明的属性

- `stage_id`、功能名称、角色，以及兼容的 `expert-agent` 执行器；
- 支持的源/目标语言标签；通用阶段与源语言覆盖层分开；
- 必需输入槽、可选输入槽、上下文窗口策略和最大注入规则数；
- 输出种类：纯译文、问题表、结构化研究、门禁记录或路由决定；
- 是否允许修改译稿，修改后要求哪种重新验证；
- PASS/FAIL 的机器可检字段，尤其“修复轮不能 PASS”；
- 失败时可回退到哪些阶段；
- 是否要求与主译者隔离、是否允许读取其他评审结论；
- 隐私与持久化类别：模板可保存，实际原文/译文注入只在本地运行上下文中存在；
- 来源清单、许可证、改编说明、修订号和内容哈希。

### 4.3 Prompt Pack 与执行器的边界

Prompt Pack 可以定义角色、任务、质量维度、输入槽、输出约定和门禁语义；执行器必须拥有并强制：

- 本地项目路径、原文读取和私密文本的注入；
- 章节顺序、检查点、重试、并发和状态持久化；
- 结构/占位符、格式、文件写入和输出目录安全；
- 固定方案修订、作业审批绑定和内容哈希；
- 确定性抽样、seed、评审隔离与门禁字段校验；
- BiblioSmith 的隐私、许可证和本地阅读硬边界。

自定义方案不得用 prompt 文本覆盖这些执行器约束。

### 4.4 可直接复用现有 BiblioSmith 能力

BiblioSmith 已有 [`expert-translation-quality`](https://github.com/semantic-craft/bibliosmith/blob/a9ab294e32f8aa1f471864eb848eece311c5652e/skills/expert-translation-quality/SKILL.md) 与 [`translation-quality-defect-families`](https://github.com/semantic-craft/bibliosmith/blob/a9ab294e32f8aa1f471864eb848eece311c5652e/skills/translation-quality-defect-families/SKILL.md)，项目规则也已要求在忠实度、术语、文风、多义词和复发问题场景使用它们（[`AGENTS.md`](https://github.com/semantic-craft/bibliosmith/blob/a9ab294e32f8aa1f471864eb848eece311c5652e/AGENTS.md#L40-L47)）。首版方案应引用这些本仓 skill 的固定修订，而不是再复制一份 LifeBook skill 进方案正文。这样既减少重复，也能让方案修订明确绑定“哪一版 skill”。

## 5. 许可证、署名与改编边界

### 5.1 上游许可证的保守分类

上游仓库采用分层许可证：

- 原始公版文本保持各自权利状态；
- 翻译、注释、前言、编辑文字、封面、版式以及“其他非代码创作内容”默认 `CC BY-NC-SA 4.0`；
- 脚本、工具代码和自动化代码默认 MIT（[`license/LICENSE.en.md` 第 12–49 行](https://github.com/SaberOnGo/public-domain-books-translation/blob/219f452311817a09cfbe59d7e345a1ac2a81faa8/license/LICENSE.en.md#L12-L49)）。

相关 prompt 和 skill 文件没有文件级许可证头。贡献规则把 style guidance、notes、review records 等列为贡献，并明确非代码贡献可以 `CC BY-NC-SA 4.0` 发布（[`CONTRIBUTING.en.md` 第 12–28 行](https://github.com/SaberOnGo/public-domain-books-translation/blob/219f452311817a09cfbe59d7e345a1ac2a81faa8/license/CONTRIBUTING.en.md#L12-L28)）。因此，在没有上游书面澄清前，应把具体提示词、skill 说明和质量规则的表达**保守地视为 CC BY-NC-SA 非代码内容**，而不是 MIT 代码。

上游另行声明第三方对非代码创作内容的商业使用需要 LifeBook Shufang 和相关权利人许可，并把付费、广告支持、机构采购、打包分发等列为商业使用情形（[`COMMERCIAL_LICENSE.en.md` 第 12–22 行](https://github.com/SaberOnGo/public-domain-books-translation/blob/219f452311817a09cfbe59d7e345a1ac2a81faa8/license/COMMERCIAL_LICENSE.en.md#L12-L22)）。

### 5.2 若复制或改编具体提示词

CC 官方法律文本允许为非商业目的复制、分享和制作改编材料，但要求署名；分享改编材料时还要求使用相同许可要素，并禁止施加额外下游限制（[CC BY-NC-SA 4.0 Legal Code §2(a)、§3](https://creativecommons.org/licenses/by-nc-sa/4.0/legalcode.en)）。内置方案至少要做到：

1. 将方案文字作为与 AGPL 程序代码可区分的 `CC BY-NC-SA 4.0` 内容资产；
2. 显示原项目名、`SaberOnGo/public-domain-books-translation` URL、`LifeBook Shufang and contributors`、固定提交和逐文件永久链接；
3. 提供许可证名称和链接，保留免责声明入口，不暗示上游背书 BiblioSmith；
4. 明确写出修改：例如“改为 BiblioSmith 本地阅读；移除版权判断、在线取书、公开出版、release 和自动模板回填；重组为通用阶段图和源语言覆盖层”；
5. 将改编后的提示词资产继续以 `CC BY-NC-SA 4.0` 提供；
6. 不给该内容加与 CC 权利冲突的额外技术或合同限制；
7. 在任何付费、广告支持、企业打包、机构采购或其他可能属于商业用途的发行前，先取得单独许可或法律意见。

BiblioSmith 自己的非代码内容层也声明为 `CC BY-NC-SA 4.0`（[`license/LICENSE.en.md`](https://github.com/semantic-craft/bibliosmith/blob/a9ab294e32f8aa1f471864eb848eece311c5652e/license/LICENSE.en.md#L12-L30)），所以许可证要素相同；这不替代上游署名，也不自动授予商业使用权。

### 5.3 更适合产品长期演进的路径：机制重实现

美国版权局的官方指引区分了方法/流程与其具体文字表达：版权不保护 idea、procedure、process、system 或 method of operation，但可以保护描述这些方法的原创表达（[U.S. Copyright Office, Circular 33](https://www.copyright.gov/circs/circ33.pdf)）。据此可采用更低耦合的工程路径：

- 从本报告中的阶段事实出发，重新设计 BiblioSmith 的阶段 ID、输入/输出 schema、门禁字段和原创中文指令；
- 不复制上游句段、例子、评分文案、目录结构或长列表，不做逐句近义改写；
- 仍记录“机制参考”仓库、固定提交和查阅文件，避免把来源抹去；
- 对确实想沿用的独特 prompt 表达，单独按 CC 资产处理或取得许可；
- 商业发行前不要仅依赖“方法不受版权保护”这一条美国法来源作全球结论，仍需复核目标司法辖区和实际文本相似度。

### 5.4 每个只读内置修订的最低来源记录

```text
pack_id
revision_id
content_sha256
display_name = 全流程审校闭环
upstream_repository
upstream_commit
upstream_commit_date
source_files[] = {path, permanent_url, role, copied_or_mechanism_only}
upstream_attribution
source_license
adapter_license
license_url
modifications_summary
commercial_use_status
no_endorsement_notice
imported_at
```

若 `copied_or_mechanism_only = copied|adapted`，该阶段文字必须进入 CC 资产清单；若为 `mechanism_only`，应保存原创实现说明和人工审查结果，不能只靠字段自称“未复制”。

## 6. 后续票据可据此锁定的决定

本研究已经给出后续阶段模型的事实边界，但仍有几项属于产品/架构决定，不在研究票据中擅自替人拍板：

- 首版是否保留上游“3–5 个试译样本”、双独立评审者、具体抽样预算与分数阈值，还是把它们做成只读方案修订中的参数；
- 全书抽检由 Prompt Pack 声明需求，还是由专家执行器统一实现；
- 原创机制重实现与 CC 改编资产二选一，或同一内置方案中逐阶段混合；
- 方案详情页如何同时展示功能来源、逐文件永久链接、许可证和改编说明；
- 内置方案引用现有 skill 时，是绑定 skill 内容哈希，还是在方案修订中固化依赖版本。

无论这些决定如何选择，都不应改变三条底线：内置修订不可被运行时静默改写；本地书稿不进入方案资产或持久化 prompt 日志；翻译质量 PASS 不等于版权、出版或 release PASS。
