# “四维反思精修”原始提示词与本地改编边界

> Wayfinder 研究票据：[界定“四维反思精修”的原始提示词与本地改编边界](https://github.com/semantic-craft/bibliosmith/issues/165)
> 核验日期：2026-08-05
> 上游固定提交：[`andrewyng/translation-agent@e0fc605acbb5d78cb7a58a98bc8bd8f0056df49c`](https://github.com/andrewyng/translation-agent/commit/e0fc605acbb5d78cb7a58a98bc8bd8f0056df49c)
> BiblioSmith 对照基线：[`semantic-craft/bibliosmith@a9ab294e32f8aa1f471864eb848eece311c5652e`](https://github.com/semantic-craft/bibliosmith/commit/a9ab294e32f8aa1f471864eb848eece311c5652e)

## 结论

“四维反思精修”应登记为**一个程序化引擎方案**：它的语义工作流改编自 Andrew Ng 的 `translation-agent`，本地可执行版本则由 BiblioSmith 加上中文目标语约束、局部上下文、结构保护、术语表、失败处理、断点续传和 QA 证据。不能把“吴恩达原版”和“BiblioSmith 四维版”登记成两个功能相同的方案；来源提交与本地修订应作为同一方案修订的出处及改编元数据。

这不妨碍保留独立的“结构保真翻译”：后者只有初译一遍，调用链和成本均不同；“四维反思精修”则明确包含初译、反思、改进三阶段。

## 1. 固定上游的真实机制

### 1.1 三阶段与变量

上游 README 将流程明确写成三步：初译、对初译提出改进建议、依据建议改译，而非一段可脱离执行器单独运行的万能提示词。[README 第 1–17 行](https://github.com/andrewyng/translation-agent/blob/e0fc605acbb5d78cb7a58a98bc8bd8f0056df49c/README.md#L1-L17)

| 阶段 | 实际输入变量 | 输出契约 | 机制特征 |
| --- | --- | --- | --- |
| 初译 | `source_lang`、`target_lang`、`source_text` | 只返回译文 | system 角色是指定语对的语言专家；user 内容指定翻译方向和原文，不要求解释。[单块实现第 72–97 行](https://github.com/andrewyng/translation-agent/blob/e0fc605acbb5d78cb7a58a98bc8bd8f0056df49c/src/translation_agent/utils.py#L72-L97) |
| 四维反思 | 上述三项、`translation_1`，以及可选 `country` | 只返回逐项、具体的改进建议 | 反思检查准确性（增译、误译、漏译、未译）、流畅性（语法、拼写、标点、重复）、风格（原文风格与文化语境）、术语（领域一致性与目标语习语）；`country` 只用于约束地区性语体。[第 100–172 行](https://github.com/andrewyng/translation-agent/blob/e0fc605acbb5d78cb7a58a98bc8bd8f0056df49c/src/translation_agent/utils.py#L100-L172) |
| 改进 | `source_lang`、`target_lang`、`source_text`、`translation_1`、`reflection` | 只返回新译文 | 编辑器同时看到原文、初译和建议，再按准确性、流畅性、风格、术语及其他错误重写。[第 175–228 行](https://github.com/andrewyng/translation-agent/blob/e0fc605acbb5d78cb7a58a98bc8bd8f0056df49c/src/translation_agent/utils.py#L175-L228) |

单块入口依次调用以上三个阶段并只返回改进稿。[第 231–260 行](https://github.com/andrewyng/translation-agent/blob/e0fc605acbb5d78cb7a58a98bc8bd8f0056df49c/src/translation_agent/utils.py#L231-L260) 默认传输是 OpenAI Chat Completions，模型 `gpt-4-turbo`、temperature 0.3、top_p 1，每次请求各有一条 system 和 user 消息；代码没有设置输出 token 上限。[第 20–69 行](https://github.com/andrewyng/translation-agent/blob/e0fc605acbb5d78cb7a58a98bc8bd8f0056df49c/src/translation_agent/utils.py#L20-L69)

上游提示词中没有实际的 glossary 输入变量。README 把在提示词中加入 glossary 作为可定制示例，并把“怎样建立、注入 glossary”列为尚待探索的扩展方向；不能把该说明误记成原版已有的术语表执行机制。[README 第 8–13 行](https://github.com/andrewyng/translation-agent/blob/e0fc605acbb5d78cb7a58a98bc8bd8f0056df49c/README.md#L8-L13) [第 56–64 行](https://github.com/andrewyng/translation-agent/blob/e0fc605acbb5d78cb7a58a98bc8bd8f0056df49c/README.md#L56-L64)

### 1.2 长文本行为

默认阈值是 1000 个 `cl100k_base` token。[默认值第 14–17 行](https://github.com/andrewyng/translation-agent/blob/e0fc605acbb5d78cb7a58a98bc8bd8f0056df49c/src/translation_agent/utils.py#L14-L17) 低于阈值走单块；达到或超过阈值时，代码先算出较均匀的块大小，再用 LangChain 的 `RecursiveCharacterTextSplitter.from_tiktoken_encoder(model_name="gpt-4", chunk_overlap=0)` 切分，最后直接拼接各块改进稿。[token 计数第 263–285 行](https://github.com/andrewyng/translation-agent/blob/e0fc605acbb5d78cb7a58a98bc8bd8f0056df49c/src/translation_agent/utils.py#L263-L285) [分块入口第 594–678 行](https://github.com/andrewyng/translation-agent/blob/e0fc605acbb5d78cb7a58a98bc8bd8f0056df49c/src/translation_agent/utils.py#L594-L678)

多块模板并不只发送相邻上下文：每处理一个块，三个阶段都会重新拼出**完整源文**，用 `<TRANSLATE_THIS>` 标出当前块，并在提示词中再次放入当前块；反思另带当前初译，改进另带当前初译和当前建议。[初译第 288–344 行](https://github.com/andrewyng/translation-agent/blob/e0fc605acbb5d78cb7a58a98bc8bd8f0056df49c/src/translation_agent/utils.py#L288-L344) [反思第 347–465 行](https://github.com/andrewyng/translation-agent/blob/e0fc605acbb5d78cb7a58a98bc8bd8f0056df49c/src/translation_agent/utils.py#L347-L465) [改进第 468–551 行](https://github.com/andrewyng/translation-agent/blob/e0fc605acbb5d78cb7a58a98bc8bd8f0056df49c/src/translation_agent/utils.py#L468-L551)

由此得到的成本形态是：短文本固定 3 次模型调用；长文本若切成 `N` 块则固定 `3N` 次。更重要的是，每次都重复完整源文，输入量约随 `N × 全文长度` 增长；在块上限固定时，这是随全文长度近似二次增长的输入成本，也会继续承受整篇源文的上下文窗口压力。这是从固定提交源码得出的推论，不是上游给出的性能承诺。上游自己也明确提示该演示并非成熟软件，而且传统端到端翻译通常更快、更便宜。[README 第 15–19 行](https://github.com/andrewyng/translation-agent/blob/e0fc605acbb5d78cb7a58a98bc8bd8f0056df49c/README.md#L15-L19)

固定提交的核心入口没有块级检查点、占位符/段落校验、失败块降级或输出重试；三个阶段按列表顺序完成后才拼接结果。[阶段串联第 554–591 行](https://github.com/andrewyng/translation-agent/blob/e0fc605acbb5d78cb7a58a98bc8bd8f0056df49c/src/translation_agent/utils.py#L554-L591) [入口第 635–678 行](https://github.com/andrewyng/translation-agent/blob/e0fc605acbb5d78cb7a58a98bc8bd8f0056df49c/src/translation_agent/utils.py#L635-L678)

### 1.3 许可证边界

上游采用 MIT License，版权声明为 `Copyright (c) 2024 Andrew Ng`；许可证允许使用、修改与分发，但复制软件或其实质部分时必须保留版权及许可声明，并明确不提供担保。[LICENSE 第 1–9 行](https://github.com/andrewyng/translation-agent/blob/e0fc605acbb5d78cb7a58a98bc8bd8f0056df49c/LICENSE#L1-L9)

因此，内置方案修订至少应保存：仓库 URL、完整 commit、`MIT`、版权声明和“本地改编”说明；若实现中收入上游提示词的实质文本，发行物还须保留 MIT 通知。功能名仍用“四维反思精修”，出处只放在详情/版本元数据中。

## 2. BiblioSmith 的来源继承与实质改编

### 2.1 为什么仍是同一来源方案

BiblioSmith 的 `WindowedReflectionSecondPass` 逐项保留了上游反思的准确性、流畅性、风格、术语四个维度，并在改进阶段继续按这四维及“其他错误”编辑初译；阶段顺序仍是初译后反思、再依据反思改进。[本地 `pipeline.py` 第 61–192 行](https://github.com/semantic-craft/bibliosmith/blob/a9ab294e32f8aa1f471864eb848eece311c5652e/packages/translation-engine/src/translation_engine/pipeline.py#L61-L192) 这是同一工作流的本地工程化，而不是第二种独立翻译方法。

### 2.2 本地安全修订清单

| 方面 | 上游固定提交 | BiblioSmith 当前行为 | 边界判断 |
| --- | --- | --- | --- |
| 语言与基础指令 | 任意人工填写的源/目标语，可选 `country` | 执行器目前只接受 `zh-Hans`；基础指令增加只输出译文、禁止重复、专名与引注可追溯、真实换行及占位符示例。[`profiles.py` 第 65–94 行](https://github.com/semantic-craft/bibliosmith/blob/a9ab294e32f8aa1f471864eb848eece311c5652e/packages/translation-engine/src/translation_engine/profiles.py#L65-L94) | 本地目标语和安全包络，不是新来源方案。 |
| 长文上下文 | 每块重复整篇源文 | 初译只带上一块译文末尾 25 个词/中文字符单元作连续性参考；反思只看前一、当前、后一块的源文和初译；改进只接收当前源文、初译与反思。[`engine.py` 初译第 683–751 行](https://github.com/semantic-craft/bibliosmith/blob/a9ab294e32f8aa1f471864eb848eece311c5652e/packages/translation-engine/src/translation_engine/engine.py#L683-L751) [邻块装配第 838–895 行](https://github.com/semantic-craft/bibliosmith/blob/a9ab294e32f8aa1f471864eb848eece311c5652e/packages/translation-engine/src/translation_engine/engine.py#L838-L895) [尾部提取第 1652–1669 行](https://github.com/semantic-craft/bibliosmith/blob/a9ab294e32f8aa1f471864eb848eece311c5652e/packages/translation-engine/src/translation_engine/engine.py#L1652-L1669) [`pipeline.py` 第 102–192 行](https://github.com/semantic-craft/bibliosmith/blob/a9ab294e32f8aa1f471864eb848eece311c5652e/packages/translation-engine/src/translation_engine/pipeline.py#L102-L192) | 用有界局部上下文替代整篇重复，是成本与串块风险修订。 |
| 结构与占位符 | 无结构保护/校验 | 在分块前保护 front matter、围栏/行内代码、数学、链接 URL、脚注；块内再保护标题前缀、段落分隔和 HTML 标签，并要求每个占位符恰好一次且顺序不变。[`placeholders.py` 第 5–103 行](https://github.com/semantic-craft/bibliosmith/blob/a9ab294e32f8aa1f471864eb848eece311c5652e/packages/translation-engine/src/translation_engine/placeholders.py#L5-L103) 候选还需保持标题形状、内容块数，不得新增重复或大段照抄源文。[`engine.py` 第 1257–1296 行](https://github.com/semantic-craft/bibliosmith/blob/a9ab294e32f8aa1f471864eb848eece311c5652e/packages/translation-engine/src/translation_engine/engine.py#L1257-L1296) | 引擎强制安全层；自定义方案也不得覆盖。 |
| 术语 | 反思只提出术语一致性目标；没有 glossary 参数 | 每块只注入该块命中的书级术语，最多 50 项；译后检查缺失的必用译名并生成带源文/译文摘录的 QA 信号，但不据此自动判失败。[`glossary.py` 第 66–117、145–162 行](https://github.com/semantic-craft/bibliosmith/blob/a9ab294e32f8aa1f471864eb848eece311c5652e/packages/translation-engine/src/translation_engine/glossary.py#L66-L117) [`engine.py` 第 1022–1061 行](https://github.com/semantic-craft/bibliosmith/blob/a9ab294e32f8aa1f471864eb848eece311c5652e/packages/translation-engine/src/translation_engine/engine.py#L1022-L1061) | 本地可执行术语约束及证据层。 |
| 初译失败处理 | 无候选重试、结构降级 | 候选先重试和可选修复；仍失败则去占位符翻译并按相对位置重插；再失败才保留原文。限流/服务不可用不会被伪装成原文降级。[`pipeline.py` 第 246–317 行](https://github.com/semantic-craft/bibliosmith/blob/a9ab294e32f8aa1f471864eb848eece311c5652e/packages/translation-engine/src/translation_engine/pipeline.py#L246-L317) 原文降级会使单元失败，并阻止该单元继续第二遍。[`engine.py` 第 752–779 行](https://github.com/semantic-craft/bibliosmith/blob/a9ab294e32f8aa1f471864eb848eece311c5652e/packages/translation-engine/src/translation_engine/engine.py#L752-L779) [第 919–1018 行](https://github.com/semantic-craft/bibliosmith/blob/a9ab294e32f8aa1f471864eb848eece311c5652e/packages/translation-engine/src/translation_engine/engine.py#L919-L1018) | 结构优先的失败语义。 |
| 改进失败处理 | 无校验或草稿回退 | 每轮重新生成反思和改进稿；改进稿通过占位符及结构校验才接受。符合有限安全条件时保留已经验证的初译，否则抛出可重试结构错误。[`pipeline.py` 第 195–234 行](https://github.com/semantic-craft/bibliosmith/blob/a9ab294e32f8aa1f471864eb848eece311c5652e/packages/translation-engine/src/translation_engine/pipeline.py#L195-L234) | 不让“精修”破坏一个已安全的草稿。 |
| 断点与复现 | 无断点 | 初译与反思分别保存连续前缀；key 绑定任务清单哈希、provider profile/config、策略和 pass，当前自定义指令进入 pass digest；完成态还校验模型、块大小和产物哈希。[`checkpoint.py` key 第 15–38 行](https://github.com/semantic-craft/bibliosmith/blob/a9ab294e32f8aa1f471864eb848eece311c5652e/packages/translation-engine/src/translation_engine/checkpoint.py#L15-L38) [存取第 118–217 行](https://github.com/semantic-craft/bibliosmith/blob/a9ab294e32f8aa1f471864eb848eece311c5652e/packages/translation-engine/src/translation_engine/checkpoint.py#L118-L217) [`engine.py` 完成态绑定第 379–470 行](https://github.com/semantic-craft/bibliosmith/blob/a9ab294e32f8aa1f471864eb848eece311c5652e/packages/translation-engine/src/translation_engine/engine.py#L379-L470) [反思续传第 780–918 行](https://github.com/semantic-craft/bibliosmith/blob/a9ab294e32f8aa1f471864eb848eece311c5652e/packages/translation-engine/src/translation_engine/engine.py#L780-L918) | 本地执行状态，不属于上游 prompt 文本。 |
| QA 证据 | 库入口只返回最终稿 | 完成的二遍单元保存初译、分块反思和修订稿三种带 SHA-256 的可寻址产物，并报告降级、初译调用、续传和术语指标。[`engine.py` 第 913–1000 行](https://github.com/semantic-craft/bibliosmith/blob/a9ab294e32f8aa1f471864eb848eece311c5652e/packages/translation-engine/src/translation_engine/engine.py#L913-L1000) | 本地可审计证据层。 |

自定义 `translation` 与 `reflection` 指令目前各自限长 2000 字符并进入对应 pass hash；引擎在用户指令之后重新声明结构要求优先。[`engine.py` 第 1177–1212 行](https://github.com/semantic-craft/bibliosmith/blob/a9ab294e32f8aa1f471864eb848eece311c5652e/packages/translation-engine/src/translation_engine/engine.py#L1177-L1212) [`profiles.py` 第 42–62 行](https://github.com/semantic-craft/bibliosmith/blob/a9ab294e32f8aa1f471864eb848eece311c5652e/packages/translation-engine/src/translation_engine/profiles.py#L42-L62)

### 2.3 本地成本形态

设一个翻译单元切成 `N` 块、`placeholderRetries = R`：

- 健康的一遍“结构保真翻译”是 `N` 次 provider 调用。
- 健康的“四维反思精修”是初译 `N` 次，加每块反思和改进各一次，共 `3N` 次 provider 调用；与上游调用次数同阶。
- 初译候选若一直不合格，每块最多先调用 `R + 1` 次，再调用一次无占位符的对齐翻译；第二遍每次失败会把“反思 + 改进”整组重做，最多 `2(R + 1)` 次。因此在对齐成功且继续第二遍的极端路径上，引擎层最多约 `(3R + 4)N` 次 provider 调用，尚未计 provider 内部的网络级瞬态重试。
- provider 默认最多进行 3 次瞬态传输尝试；429 使用独立的凭据池预算，超过短等待阈值会向上抛并依赖断点续跑。[`providers.py` 第 247–280 行](https://github.com/semantic-craft/bibliosmith/blob/a9ab294e32f8aa1f471864eb848eece311c5652e/packages/translation-engine/src/translation_engine/providers.py#L247-L280) [第 718–757 行](https://github.com/semantic-craft/bibliosmith/blob/a9ab294e32f8aa1f471864eb848eece311c5652e/packages/translation-engine/src/translation_engine/providers.py#L718-L757)

与上游不同，本地反思只带三块窗口、改进只带当前块，初译只带 25 单元的前译文尾部；固定块大小下，模型输入量随 `N` 近似线性增长。具体货币成本仍由 provider、模型、实际重试和输出长度决定，方案元数据不应承诺固定价格。

## 3. 方案登记与展示边界

### 3.1 只登记一个可执行方案

建议内置记录采用以下语义，而非另建一个“吴恩达原版”记录：

```yaml
name: 四维反思精修
executor: programmatic-engine
stages: [initial-translation, four-axis-reflection, improvement]
language:
  source: auto
  target: zh-Hans
origin:
  repository: https://github.com/andrewyng/translation-agent
  commit: e0fc605acbb5d78cb7a58a98bc8bd8f0056df49c
  license: MIT
  copyright: Copyright (c) 2024 Andrew Ng
derivation: adapted
```

同一修订详情再列本地改编标签：`bounded-adjacent-context`、`protected-structure`、`chunk-glossary`、`validated-degradation`、`resumable-checkpoints`、`qa-evidence`。这些标签解释本地行为，不产生第二个方案 ID。

### 3.2 可编辑内容与引擎强制内容分层

| 层 | 模板视图 | 自定义副本能否修改 | 实际提示词预览 |
| --- | --- | --- | --- |
| 方案语义 | 初译任务、四维反思标准、依据建议改进 | 可以；改动产生新修订 | 显示编译后的该层 |
| 来源与版本 | 上游 URL、commit、MIT、版权、本地改编说明 | 不得伪造；派生副本继承 provenance | 显示元数据，不发送给模型 |
| 引擎安全包络 | 占位符顺序、段落/标题结构、禁止重复、失败降级 | 不可以 | 必须显示其最终插入位置 |
| 运行时注入 | 当前源文/初译、前后窗口、上一译文尾部、命中的 glossary | 由所选书和样本块生成 | 只在本机临时预览，不当作模板保存 |

这样既能让用户看到“提示词实际是什么”，又不会把书稿、局部上下文或术语命中误存成方案内容。

### 3.3 后续规格必须补齐的绑定

当前检查点已经绑定自定义指令 digest 和翻译策略，但还没有“提示词方案 ID + 不可变修订 + 内容哈希”这一等对象。实现方案库时，应把三者加入：

1. 预览/批准绑定；
2. 初译与反思 checkpoint key；
3. completion/report 与三种 QA 产物；
4. 运行详情中的来源 commit 和本地改编版本。

否则，只改内置模板而不改现有 `translationPolicyVersion` 时，旧批准或完成缓存仍可能被误认作同一次运行。该缺口应由后续架构票据解决，不应通过复制出一个“原版方案”规避。

## 4. 决策摘要

1. “四维反思精修”的原始语义固定到 `andrewyng/translation-agent@e0fc605acbb5d78cb7a58a98bc8bd8f0056df49c`。
2. 原始工作流是初译、四维反思、依据反思改进；短文本 3 次调用，长文本 `3N` 次且每块重复全文。
3. BiblioSmith 的三阶段和四维标准继承该来源；邻块窗口、结构保护、术语注入、失败降级、断点与 QA 是同一方案的本地安全修订。
4. 方案库只登记一个“四维反思精修”可执行方案；来源、许可证和本地改编列入不可变修订元数据。
5. 语义模板允许复制后编辑，引擎安全包络不可覆盖；运行时书稿及注入内容只进本地实际提示词预览。
