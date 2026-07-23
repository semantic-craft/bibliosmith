# TBL 与 translation-agent 翻译机制研读报告（#44）

> 研读对象：
> - **TranslateBooksWithLLMs**（下称 TBL，hydropix/TranslateBooksWithLLMs，AGPL-3.0）——只提炼机制规格，**不含任何代码复制**；我们将原生重实现。引用格式 `src/xxx.py::符号`，均指 TBL 仓库路径。
> - **translation-agent**（andrewyng/translation-agent，MIT）——吴恩达 reflection 工作流，全量通读。
>
> 研读日期：2026-07-14。基于两仓当日 main 分支浅克隆。本文只摆机制事实与影响面，不替 #45/#48 下结论。

---

## 第一部分：TBL 机制规格

TBL 是一个 Python 全栈工具（CLI `translate.py` + Flask/WebSocket Web UI），核心翻译逻辑在 `src/core/`。总体架构：格式适配器（EPUB/TXT/SRT/DOCX）→ 占位符保护 → token 分块 → LLM 请求（多后端）→ 校验/修复/降级 → 检查点持久化。

### 1. Token 分块的参数化

**核心类**：`src/core/chunking/token_chunker.py::TokenChunker`

- **计数器**：tiktoken `cl100k_base`（硬编码 encoding，构造函数 `get_encoding("cl100k_base")`）。
- **参数**：`max_tokens`（硬上限）+ `soft_limit_ratio`（默认 0.8）。软限 = `max_tokens * 0.8`：累积超过软限后遇到下一个会突破硬限的单元就切块。
- **全局默认**：`src/config.py` 中 `MAX_TOKENS_PER_CHUNK = 450`（env 可覆盖），`SOFT_LIMIT_RATIO = 0.8`。TokenChunker 类内部默认 800（被上层配置覆盖）。
- **边界层级**（`TokenChunker._chunk_units`）：
  1. 段落边界：`re.split(r'\n\s*\n', text)` 按双换行切段；
  2. 单段超过 `max_tokens` → 按句子终止符切句（`SENTENCE_TERMINATORS` 配置集，最长匹配优先），句子块用空格拼接、递归走同一算法；
  3. 小块合并：小于 `max_tokens * 0.25` 的残块不单独成块，前缀合并进下一个句子块（合并后仍需 ≤ max_tokens）。
- **纯文本（TXT）入口**：`src/core/text_processor.py::split_text_into_chunks` 薄封装 TokenChunker，输出结构化 chunk 字典：`{context_before, main_content, context_after}`——前块最后一段、本块内容、后块第一段。
- **EPUB 的 HTML 感知分块**：`src/core/epub/html_chunker.py::HtmlChunker.chunk_html_with_placeholders`。输入是**已被占位符替换后的文本**（见第 4 节），流程：
  1. `_find_safe_split_points`：只在「块级闭合占位符后紧跟块级开启占位符」的位置切（保证块内占位符配平，无孤儿标签）。切点带优先级（`src/core/epub/tag_classifier.py::TagClassifier.get_split_priority`）：1=章节标题（h1-h3 闭合后）、2=次级章节（h4-h6/section/article）、3=段落（p/div/blockquote）、4=其他块（li/tr/td/th）；
  2. `_merge_segments_into_chunks`：段落级片段贪心合并到 `max_tokens`（EPUB 默认 450）；
  3. 超大片段用 `src/core/epub/text_splitter.py::TextSplitter.split_oversized_segment` 分层降级切：句子 → 标点 → 换行 → 词边界强切；
  4. `_finalize_chunk` 调 `src/core/epub/placeholder_renumberer.py` 把每块占位符**本地重编号为 0,1,2…**，同时记录 `global_indices` 映射（LLM 只看到小数字，降低破坏率；译完映射回全局号）。
- **Plain Text Mode（EPUB/DOCX 的纯文本管线）**：`src/core/common/plain_text_pipeline.py::build_plain_segments`。段落按索引分组进 token 预算段（`{'indices': [...], 'text', 'partial'}`）；超大段拆句后所有片段共享同一段落索引（`partial=True`），译完按索引写回原槽位——空段落（纯图片块）不发 LLM、槽位保留，避免计数漂移。

### 2. 跨块上下文延续

**携带什么**：上一块**译文**的最后 25 个词（不是原文）。生成点：
- TXT 管线：`src/core/translator.py::_make_llm_request_with_adaptive_context`（335 行附近）与 `src/core/common/plain_text_pipeline.py::translate_paragraphs_plain`（314-318 行）：`words = cleaned.split(); previous_translation_context = " ".join(words[-25:])`（不足 25 词取全部）。
- 同时携带 `context_before` / `context_after`（源文邻块首尾段），但注意：**当前的提示词模板实际上没有把 context_before/context_after 拼进 prompt**——`src/prompts/prompts.py::generate_translation_prompt` 只消费 `previous_translation_context`，context_before/after 参数被接受但未渲染（遗留接口）。
- **拼进提示词的哪个位置**：user prompt 的最前部，作为独立小节 `# CONTEXT - Previous Paragraph`，位于词汇表块和 `# TEXT TO TRANSLATE` 之前（`prompts.py` 276-283 行）。system prompt 保持稳定不变（利于 KV cache）。
- **重要边界**：
  - **EPUB 占位符管线不带跨块上下文**：`src/core/epub/xhtml_translator.py::translate_chunk_with_fallback` 调用翻译时 `previous_translation_context=""`（453 行），注释明言这使 EPUB 块可安全并行（`_translate_all_chunks_with_checkpoint` 763-767 行）。
  - 并行度 > 1 时纯文本管线也放弃上下文链（`plain_text_pipeline.py` 255 行：`sequential` 才读）。
  - refine 过程有独立的 `previous_refined_context`（同样是上一块精修后的最后 25 词，`translator.py::refine_chunks` 854-858 行）。

### 3. Checkpoint 文件格式与断点续传

双层持久化：**SQLite 作业库 + 文件系统旁挂**。

- **管理器**：`src/persistence/checkpoint_manager.py::CheckpointManager`。SQLite 位于 `data/jobs.db`（`src/persistence/database.py`），上传/中间产物在 `data/uploads/<translation_id>/`。
- **作业层**：`start_job` 建作业行（translation_id、file_type、config JSON）。**API key 在持久化时被剥离**（issue #213 注释）：resume 从 .env 或恢复请求重新解析，绝不落库。输入文件复制保全到 `uploads/<id>/`（`_preserve_input_file`，config 记 `preserved_input_path`）。
- **块层（TXT/SRT）**：`save_checkpoint` 每块写一行（chunk_index、original_text、translated_text、chunk_data JSON、status completed/failed）+ 更新作业进度（current_chunk_index/total/completed/failed）+ 可选 translation_context（延续上下文也入库）。调用点：`src/core/adapters/generic_translator.py`（285、359 行）。
- **EPUB 双粒度**：
  - 文件级：整个 XHTML 译完 → `save_epub_file` 把译后文件字节存到 `uploads/<id>/translated_files/<href>`，然后 `src/core/epub/translator.py::_save_checkpoint` 记录「最后完成的文件索引」。
  - **块级（XHTML 内部）**：`save_xhtml_partial_state` 每 5 块（`xhtml_translator.py` 中 `CHECKPOINT_FREQUENCY = 5`）写一个 JSON 到 `uploads/<id>/xhtml_states/<safe_href>.json`。Schema 即 `src/core/epub/xhtml_translation_state.py::XHTMLTranslationState.to_dict`：全部 chunks（含每块 text/local_tag_map/global_indices）、global_tag_map、placeholder_format、translated_chunks（已完成前缀）、current_chunk_index（下一个待译块）、stats、prompt_options、bilingual/original_chunks、时间戳、global_stats。**load 后有 `validate()` 一致性检查**（translated_chunks 长度必须等于 current_chunk_index 等），失败则整个 partial state 作废重来。文件译完成功保存后才删 partial state（原子性保证，`translator.py::_save_checkpoint` 中先 save 后 delete）。
- **续传语义**：`load_checkpoint` 统一约定 current_chunk_index = 最后完成单元，resume = +1；带 `resume_index_semantics='completed'` 标记区分旧格式（EPUB 旧格式存的是下一个文件号）。失败块索引单独列出（`failed_chunk_indices`），支持「partial」状态只重试失败块。
- **中断路径**：翻译循环把 `translated_chunks` 维持为连续前缀（`src/core/common/parallel.py::iter_ordered_concurrent` 保证按序交付），中断/限流时以 `len(translated_chunks)` 为续点存档后抛出或返回 `was_interrupted=True`。服务器重启时 `reset_running_jobs_on_startup` 按 server_session_id 把孤儿 running 作业改为 interrupted。

### 4. EPUB 标签保留与校验（重点）

**总思路：LLM 永远不看到 HTML 标签。** 标签在发给 LLM 前被替换为语义占位符 `[idN]`，译后按 map 还原。这是「结构安全」的第一性来源，提示词层的保留指令只是辅助。

- **占位符生成**：`src/core/epub/tag_preservation.py::TagPreserver.preserve_tags`：
  1. `re.split(r'(<[^>]+>)', text)` 把 body HTML 切成 标签/非标签 序列；
  2. **相邻的标签 + 不可译内容合并成单个占位符**：`is_non_translatable`（同文件 19-52 行）判定空白/不可见 Unicode/纯数字编号/罗马数字为不可译，与标签一起归组。例如 `</span></p><p><span>3.</span>` 整体一个 `[idN]`——大幅减少占位符数量（= 降低 LLM 破坏概率 + 省 token）；
  3. 产出 `(text_with_placeholders, tag_map)`；`tag_map: {"[id0]": "<p class=…><span>", …}`。
  4. 技术内容保护（`preserve_tags_and_technical_content`，protect_technical=True 时）：多行代码块/公式（```…```、$$…$$）先提取为原子块，行内代码/LaTeX/度量值单独占位——技术内容**藏进占位符而非靠提示词嘱咐**（prompts.py 127-132 行注释明确说旧的提示词段已废弃）。
- **占位符格式**：`src/common/placeholder_format.py::PlaceholderFormat`，统一 `[id` + N + `]`（`src/config.py` 489-495 行）。修复提示词里保留了 4 种历史格式（`/N`、`$N$`、`[[N]]`、`[N]`）的说明以兼容。
- **三层校验**（`src/core/epub/placeholder_validator.py::PlaceholderValidator`）：
  - `validate_basic`：所有期望占位符出现即可；
  - `validate_strict`（译后主校验，`xhtml_translator.py::validate_placeholders` 调用）：① 数量精确相等；② 索引集合 = {0..n-1} 无缺失无多余；③ 每个具体占位符字符串在场。注意 strict 检查的「顺序」实际是索引集合完整性（indices 排序后 == range），**不校验占位符在文中的相对位置**——位置错误靠修复提示词或人查。
  - 兜底：还原后 `xhtml_translator.py::_replace_body`（1290-1305 行）再扫一遍剩余未还原占位符并告警。
- **错误诊断**：`xhtml_translator.py::build_specific_error_details` 生成结构化错误清单（缺失/重复/乱序/计数不符/错误索引），供修复提示词使用。
- **修复提示词**：`src/prompts/prompts.py::generate_placeholder_correction_prompt`。角色设定「占位符修复专家」，输入原文（占位符正确）+ 错误译文 + 错误清单，要求**只动占位符位置、不改译文**，输出包在 `<CORRECTED_TAG_IN>…<CORRECTED_TAG_OUT>` 里；`xhtml_translator.py::attempt_placeholder_correction` 提取后再走 strict 校验，不过关就放弃修复。注意：**当前 EPUB 主流程实际未接线这个修复调用**（`translate_chunk_with_fallback` 里 Phase 1 失败直接进 Phase 2；correction 函数与 `MAX_PLACEHOLDER_CORRECTION_ATTEMPTS`（config 337 行 =2、502 行被重定义为 0）在主循环中无调用点）——机制存在但被停用，重实现时可作为可选层。
- **译文提取安全阀**：所有翻译输出必须包在 `<TRANSLATION>…</TRANSLATION>` 里（`src/config.py` 471-472），`src/core/llm/utils/extraction.py::TranslationExtractor` 提取时先剥 `<think>` 块和 markdown 代码围栏。**EPUB 模式下提取失败 = 硬失败**（`translator.py::_make_llm_request_with_adaptive_context` 305-311 行：不许拿原始响应兜底，避免 `<TRANSLATION>` 标签混进 HTML）；纯文本模式才允许 raw-response 兜底。
- **XML 注入防线**（还原前的消毒，`xhtml_translator.py`）：
  - `_escape_stray_ampersands`（1077-1110 行）：XML 只认 5 个预定义实体；HTML 命名实体（&nbsp; 等）替换为字面字符，未知 `&` 一律 `&amp;` 化——因为 lxml recover 模式会**静默删除**非法实体连同邻近文本（issue #202）；
  - `_escape_stray_angle_brackets`（1113-1120 行）：还原标签前把译文中所有裸 `<`/`>` 实体化（韩国网文 `<技能名>` 这类字面尖括号会变成幽灵元素破坏文档）。真实标签此时都在 tag_map 里，不受影响。
- **还原**：`TagPreserver.restore_tags` 按索引倒序替换（先 [id10] 后 [id1]，防前缀误替换）。

### 5. 失败块重试 / 修复 / 降级（重点）

EPUB 管线是明确的**三阶段降级**（`xhtml_translator.py::translate_chunk_with_fallback`，文件头注释 + 439-576 行）：

- **Phase 1 — 正常翻译 + 重试**：至多 `max_retries` 次（config `MAX_TRANSLATION_ATTEMPTS` 默认 2）。每次译后跑 strict 占位符校验；失败计 `stats.placeholder_errors` 并重试。首试/重试成功分别计数（`successful_first_try` / `successful_after_retry`）。
- **Phase 2 — Token 对齐兜底**（`EPUB_TOKEN_ALIGNMENT_ENABLED` 默认 true）：
  1. 剥掉全部占位符得到纯文本；
  2. 以 `has_placeholders=False` 重译（不含占位符指令，必然「成功」）；
  3. `src/core/epub/token_alignment_fallback.py::TokenAlignmentFallback.align_and_insert_placeholders` 用**比例位置对齐**把占位符插回译文：原文中每个占位符的相对位置（去占位符后的字符偏移 / 总长）映射到译文长度，再吸附到最近词边界（空格/标点/CJK 标点，`_find_nearest_word_boundary`）；同位置多个占位符按原索引序插入、从尾向头插保持偏移有效；
  4. 内部再有两级兜底：比例重插（html_utils 版本）→ 首尾夹放 + 均匀分布（`_fallback_proportional` 349-366 行）。**算法保证 100% 占位符数量完整**，代价是位置可能有轻微版式偏差（日志明确警告用户）。
- **Phase 3 — 原文回退**：Phase 2 也失败（或被禁用）时返回**未翻译的原块**（恢复全局索引后原样进书），计 `stats.fallback_used`，日志警告「该块保留源语言」。**结构完整性永远优先于翻译完整度**——宁可留原文，绝不出坏 EPUB。
- **健康检查**：`stats.check_quality_warning()` 在占位符失败率过高时向日志发一次性大警告（`_translate_all_chunks_with_checkpoint` 852-857 行），提示换更强模型。
- **纯文本管线的失败语义**（`plain_text_pipeline.py` 287-307 行）：块失败（异常或返回 None）→ 保留原文进槽位、计 failed_chunks，不再重试（重试在 provider 层）。段落数不匹配的软修复：`_reconcile_paragraph_counts`——少了补空串、多了并进最后一格，爆炸半径限制在段内。
- **Provider 层重试**（以 `src/core/llm/providers/openai.py::generate` 为代表）：`MAX_TRANSLATION_ATTEMPTS`（默认 2）内循环。超时/JSON 解析错/未知异常 → sleep 2s 重试；HTTP 4xx（非 429）**快速失败不重试**（`rate_limit_handler.py::is_retryable_http_status`）；429 走独立预算的 key 轮换（见第 6 节）；报错文本命中 context 关键词 → 抛 `ContextOverflowError` 交上层。
- **上下文自适应重试**（Ollama/本地为主，`src/core/context_optimizer.py::AdaptiveContextManager`）：从 2048（thinking 模型 6144）起步，响应被截断或用量 ≥95% 时 +2048 重试（上限 131072，最多 10 次重试）；连续 5 块都能装进更小窗口则自动缩容（留 20% 余量防振荡）。`RepetitionLoopError`（本地小模型复读检测，流式期间即可触发，config 310-313 行阈值）→ 双倍步长扩容重试。`ContextOverflowError` 且无法扩容 → 按 0.6 因子在句边界切半重试（`translator.py::split_chunk_for_retry`，最多 3 次，先译前半、余下拼回队列）。
- **另有一套通用重试基建**：`src/core/adapters/retry_manager.py::RetryManager`——按异常类型配置退避策略（指数/线性/立即/不重试）+ 熔断器（5 次失败开路 60s，半开态 2 次成功恢复）。主要服务适配器层。
- **限流自动暂停**：`RateLimitError` 一路上抛到任务层，作业转 paused 状态存档（`AUTO_PAUSE_ON_RATE_LIMIT` 默认 true；false 则等待后自动续传）。

### 6. API key 轮换

- **数据结构**：`src/core/llm/key_pool.py::KeyPool`——round-robin 游标 + 每 key `throttled_until`（monotonic 时间戳）。`acquire()` 返回下一个未限流 key；全限流时返回最早恢复者（不阻塞）。asyncio.Lock 保护，支持并行调度。
- **key 输入**：`src/core/llm/base.py::normalize_api_keys`——单 key、逗号/换行分隔串、可迭代对象皆可；去空去重保序。三个配置通道（.env / Web UI / CLI）同格式（docs/API_KEY_ROTATION.md）。所有云 provider（Gemini/OpenRouter/OpenAI/Mistral/DeepSeek/Poe/NIM）共享此机制；Ollama 无 key。
- **429 处理**：`src/core/llm/rate_limit_handler.py::handle_rate_limit`：
  1. 等待时长：`Retry-After` 头 → `X-RateLimit-Reset`（OpenRouter 用，ms 时间戳）→ 指数退避 `min(2^(n+2), 60)`；
  2. 标记失败 key 限流至该时刻（取 max 不缩短已有限流）；
  3. 有空闲 key → **零等待轮换**；
  4. 全忙 → sleep 到最早恢复点；
  5. 预算耗尽 → 抛 `RateLimitError` 触发上游自动暂停。
- **关键设计（issue #217）**：429 事件有**独立预算** `rate_limit_budget = pool_size + max_attempts - 1`，与 provider 的瞬态重试计数分离——轮换到备用 key 不消耗重试次数，否则大 key 池会在试遍所有 key 前就把重试额度烧光。调用形态见 `providers/openai.py::generate`（116-223 行）：`attempt` 与 `rate_limit_events` 两个计数器分开走。

### 7. --text-cleanup 与 --refine 的提示词策略

**--text-cleanup**（`translate.py` 54 行 CLI flag → `prompt_options['text_cleanup']`）：
- 实现为**主翻译 system prompt 的可选小节**，非独立 pass：`src/prompts/prompts.py::TEXT_CLEANUP_SECTION`（95-106 行），经 `_build_optional_prompt_sections` 拼入。
- 意图：源文有 OCR/排版缺陷时「**翻译过程中顺手修复**」——断词连回（trans-\nlation → translation）、双空格/标点后缺空格、错标点、误切段落合并；明令禁止增删内容或改作者文风。零额外 token 成本（只加了一段指令）。

**--refine**（`translate.py` 55-56 行；`--refine-only` 跳过翻译只精修）：
- **独立的第二遍 LLM pass**，输入是**译文草稿（不含原文）**。核心提示词 `src/prompts/prompts.py::generate_refinement_prompt`（381-552 行）：
  - system：角色「精英 {target} 文学编辑与文体家」；明确「你不是在翻译，是在用 {TARGET} **重写**」；输入定性为「业余、直译、生硬的草稿」；
  - 优先级序：自然流畅 → 地道习语 → 优雅措辞 → 节奏韵律 → **保义**（第 5 位）；
  - 修什么：直译腔、词汇重复/同根词撞车（明确举例换同义词）、语序不自然；保什么：事实内容、人名专名、术语；
  - EPUB 变体带占位符时附加「占位符保留绝对关键」大警告；
  - 可选 `refinement_instructions`（用户自定义精修指令）与词汇表块（对**草稿**过滤，因为草稿已是目标语言——`translator.py::_make_refinement_request` 517-523 行注释）。
- **执行形态**：
  - TXT：`src/core/translator.py::refine_chunks`——逐块顺序精修，携带 `previous_refined_context`（上一块精修结果尾 25 词）；失败保留一遍译文（`refinement_chunk_failed`）；精修起始上下文窗口至少 4096（比翻译大，因为 prompt 装着整块译文）。
  - EPUB：`src/core/epub/xhtml_translator.py::_refine_epub_chunks`——带占位符精修：全局索引→本地索引→LLM→strict 校验→**校验不过直接弃用精修结果、保留一遍译文**（1497-1502 行）→ 还原全局索引。上下文取相邻译块全文（非 25 词）。
  - refine-only 模式：`src/core/refine/txt_refiner.py` / `epub_refiner.py` / `docx_refiner.py` / `srt_refiner.py`——对已译文件重新分块后走同一精修函数；EPUB refine-only 无断点续传（v1 注明）。
- **成本形态**：精修是全量第二遍（每块一次 LLM 调用，prompt 含草稿全文 + 上下文），约使总 token 成本翻倍再多一点。

### 8. 词汇表 / NER

**词汇表注入**（`src/core/glossary/`）：
- **加载**：CLI `--glossary` 接 JSON/CSV（`cli_loader.py`，需 source/target 列，可选 category）；Web UI 走 SQLite `store.py`。
- **按块过滤**（`filter.py::filter_glossary`）：每块只注入**实际出现在该块源文里的词条**。拉丁词用 `\b` 词边界匹配（Fan 不匹配 Fantasy），CJK 用子串匹配（无词边界概念）；源词可用 `|` 声明屈折变体（俄语等），任一变体命中即注入、计数合并。按最长变体降序排（长词优先处理重叠）。**上限 50 条/块**（`models.py::GlossaryConfig.max_entries`），超限按出现频次保留（频次同则长者优先），仍按长度序输出保持稳定；命中上限记一次性警告。
- **渲染与位置**（`injector.py::build_glossary_block`）：`# GLOSSARY - REQUIRED TRANSLATIONS` 小节，MANDATORY 语气 + 逐行 `source -> target [category]`（category 作消歧提示）。**注入 user prompt**（`prompts.py` 286-288 行注释：因其逐块变化，放 user 侧保 system prompt 稳定可缓存）。
- **与 refine 的组合**：精修请求同样注入词汇表，但过滤对象是**草稿译文**（匹配 target 语言词条），意图是「一遍译对的术语在精修中保持稳定」（`translator.py` 517-523 行）。

**NER 候选抽取**（`src/core/glossary/ner.py`，Phase 2）：
- `suggest_terms`：取源文样本（默认截 6000 字符）发一次 LLM 调用；提示词 `prompts.py::generate_ner_extraction_prompt`——角色「文学实体抽取器」，6 类标签（character/location/organization/item/title/other），要求只抽可能复现的专名、每实体给一个规范目标译名、去重、不确定则略去；输出 JSON 数组包在 `<NER_JSON>…</NER_JSON>`。
- **宽容解析**（`parse_ner_response`）：标签内容 → markdown 围栏 → 首个平衡 `[...]` / `{...}`（含字符串感知的括号配平扫描 `_find_balanced`）→ 尾逗号修复；对象包裹数组时自动解包（entities/terms/candidates/items/results 字段）。剥 `<think>` 块。
- **人审闸门**：候选**不自动入库**，用户确认后才进词汇表（模块 docstring 明示）。

### 9. 多 LLM 后端抽象

- **基类**：`src/core/llm/base.py::LLMProvider`——抽象方法只有一个 `async generate(prompt, timeout, system_prompt) -> LLMResponse | None`。基类统一提供：KeyPool 构建、httpx.AsyncClient 连接池（keepalive 5 / max 10）、`extract_translation`（`<TRANSLATION>` 标签提取 + `<think>`/围栏剥离）。
- **统一响应**：`LLMResponse` dataclass：content + prompt_tokens/completion_tokens/context_used/context_limit + `was_truncated`（自适应上下文的学习信号）+ `was_fallback`。
- **工厂**：`src/core/llm/factory.py::create_llm_provider(provider_type, **kwargs)`——8 个内建 provider（ollama/openai/gemini/openrouter/mistral/deepseek/poe/nim）+ litellm 逃生口（信用走各家原生 env var）。模型名以 `gemini` 开头时自动切 Gemini。key 解析顺序：kwargs → 专名 kwargs → env → config 默认；工厂前置 `_require_key` 报清晰错误。
- **Ollama**（`providers/ollama.py`，674 行，最重）：流式请求、`options.num_ctx` 由 AdaptiveContextManager 动态设定；**流中复读检测**（可提前中断生成抛 `RepetitionLoopError`）；thinking 模型探测（`detect_thinking_model`，探测请求 + 已知模型名单）与 thinking token 估算（eval_count 可能不含思考 token，按 len/3 估算取 max，435 行附近以 `context_used >= 0.95 * window` 判 `was_truncated`）。
- **OpenAI 兼容层**（`providers/openai.py`）：单个 `/v1/chat/completions` POST（非流式），端点归一化（自动补 `/chat/completions`）；本地端点（localhost）额外发 3 个禁思考参数（`thinking:false`、`enable_thinking:false`、`chat_template_kwargs`）；官方 OpenAI 不发。usage 字段直读 token 数，`was_truncated` 恒 False（API 不提供）。上下文探测 `get_model_context_size` 走 `ContextDetector`。NIM 复用此类只换端点与 provider_name。
- **调用侧**：核心管线不直接持 provider，经 `src/core/llm_client.py::LLMClient/create_llm_client` 薄包装（make_request/generate + extract_translation）。

---

## 第二部分：translation-agent 机制

全部实现在单文件 `src/translation_agent/utils.py`（约 680 行，MIT），同步、顺序、零依赖状态。入口 `utils.py::translate(source_lang, target_lang, source_text, country, max_tokens=1000)`。

### 1. 三步工作流

**单块路径**（全文 < 1000 token，`one_chunk_translate_text`）——恰好 3 次 LLM 调用：

| 步骤 | 函数 | system 角色 | user prompt 结构 | 输出 |
|---|---|---|---|---|
| ① initial_translation | `one_chunk_initial_translation` | 「专家语言学家，专精 {src}→{tgt} 翻译」 | 一句任务描述 + `{src}: 原文` + `{tgt}:` 引导补全；「除译文外不输出任何东西」 | 直译初稿 |
| ② reflect | `one_chunk_reflect_on_translation` | 同上 + 「你将收到源文与译文，目标是改进翻译」 | `<SOURCE_TEXT>` + `<TRANSLATION>` 包裹两输入；要求按**四维度**给出改进建议：(i) 准确性（增译/误译/漏译/未译）(ii) 流畅性（语法/拼写/标点/冗余重复）(iii) 风格（贴源文风格与文化语境）(iv) 术语（一致性、领域惯用、习语等价）；「每条建议针对一处具体位置；只输出建议列表」 | 批评建议清单（自由文本，无结构化 schema） |
| ③ improve | `one_chunk_improve_translation` | 「专家语言学家，专精**翻译编辑**」 | `<SOURCE_TEXT>` + `<TRANSLATION>` + `<EXPERT_SUGGESTIONS>` 三段输入；要求「参考专家建议编辑译文」，复述同一四维度 +(v) 其他错误；「只输出新译文」 | 终稿 |

### 2. 长文本分块与逐块 reflection

- **触发**：`translate` 用 tiktoken `cl100k_base` 计数（`num_tokens_in_string`），≥ `MAX_TOKENS_PER_CHUNK`（模块常量 1000）走多块路径。
- **均分块**：`calculate_chunk_size(token_count, token_limit)` 先算需要几块（向上取整），再把总量均分——避免「999 + 1」这种尾块畸形（docstring 例：2242/500 → 496）。
- **切分器**：LangChain `RecursiveCharacterTextSplitter.from_tiktoken_encoder(model_name="gpt-4", chunk_size=token_size, chunk_overlap=0)`——递归按段落/句子/词降级切，**零重叠**。
- **逐块三步**（`multichunk_translation` = `multichunk_initial_translation` → `multichunk_reflect_on_translation` → `multichunk_improve_translation`，三个函数分别对所有块跑完第①/②/③步，非交错）。**关键机制**：每一步的每一次调用都发送**整个文档**——`tagged_text = 前文块拼接 + <TRANSLATE_THIS>当前块</TRANSLATE_THIS> + 后文块拼接`，指令是「其余部分仅作上下文，只翻译/只批评/只改进标记段」。
- **成本形态（重要）**：N 块 → **3N 次调用，每次 prompt ≈ 全文长度** → prompt token 成本 **O(3·N·全文) ≈ O(全文²) 量级**（相对文档长度呈平方增长）；reflect/improve 的 prompt 还要再叠上初稿与建议。对书籍级文本不加改造不可用（一本 15 万 token 的书 ≈ 150 块 × 3 × 15 万 ≈ 67M+ prompt token）。上下文窗口也必须装下全文，否则直接溢出。
- **拼装**：`"".join(translation_2_chunks)` 直接连接，无校验、无修复、无对账。

### 3. country 定制点

- 唯一的地域化参数，**只作用于 reflect 步**：country 非空时 reflection prompt 增加一句「最终风格与语调应匹配 {country} 地区口语化的 {target_lang}」（`one_chunk_reflect_on_translation` 124-146 行、`multichunk_reflect_on_translation` 371 行起的分支）。initial/improve 两步不感知 country——地域约束靠 reflect 建议间接传导到 improve。

### 4. 迭代次数与工程形态

- **迭代恰好一轮**：translate→reflect→improve 固定管线，无循环、无收敛判据、无质量门（README 提到「可迭代多轮」是未来方向，代码没有）。
- `get_completion`：默认 `gpt-4-turbo`、`temperature=0.3`、`top_p=1`，同步 OpenAI SDK。**无重试、无限流处理、无断点、无日志、无输出校验**（连「模型输出了解释文字」都不防）。
- 后端可换性靠 `app/patch.py`（Gradio demo 用）monkey-patch `get_completion`：OpenAI/Groq/TogetherAI/Ollama/CUSTOM base_url 皆可（openai 兼容接口），带简单 RPM 节流。核心库本身只认 `OPENAI_API_KEY`。
- 测试 `tests/test_agent.py` 为调用示例性质（需真 key），无 mock 断言。

---

## 第三部分：机制对照表——TBL `--refine` vs translation-agent reflection

> 供 #45（引擎架构）与 #48（词汇表/refine 分工）拍板。只摆机制事实与影响面。

### 逐维对照

| 维度 | TBL `--refine` | translation-agent reflection |
|---|---|---|
| **输入** | 仅**译文草稿**（+ 相邻译块上下文 + 前块精修尾 25 词）。**不带源文** | **源文 + 初稿**（reflect），**源文 + 初稿 + 建议清单**（improve）。多块模式下源文 = 整个文档 |
| **提示词策略** | 单步「重写」：角色 = 目标语文学编辑；把输入定性为烂稿，直接产出润色稿。优化目标偏**目标语流畅度/文学性**，保义列第 5 位 | 两步「批评→编辑」：先产出显式的按四维度（准确/流畅/风格/术语）定位到具体位置的建议清单，再据此编辑。优化目标含**对源文的忠实度**（增译/误译/漏译在第一维度） |
| **每块 LLM 调用数** | 翻译 1 + 精修 1 = **2** | 初译 1 + 反思 1 + 改进 1 = **3** |
| **每次调用 prompt 规模** | O(块) ——草稿块 + 邻块上下文 + 指令 | 单块模式 O(全文)；多块模式 **O(全文)**（tagged_text 全文入 prompt），且 improve 步再叠初稿 + 建议 |
| **总 token 成本形态** | **O(全文) 线性**，系数约 2.2-2.5×单遍翻译 | 单块（<1000 tok）约 3-4×；多块 **约 O(全文²/1000)**，书籍级不可直接用，须改造（如滑动窗口上下文替代全文上下文） |
| **能否修忠实度问题** | 机制上不能（看不到源文，只能改善表达；误译/漏译会被通顺地保留甚至强化） | 机制上能（reflect 第一维度专查 addition/mistranslation/omission/untranslated） |
| **与词汇表可组合性** | 已内建：refine 请求注入词汇表块，但对**草稿（目标语）**过滤词条，管「保持一遍译名稳定」 | 无词汇表机制。但 reflect 的 (iv) 术语维度是天然挂点：词汇表可注入 reflect 提示词作为「术语裁判标准」，或注入 improve 作为硬约束——均需自行实现 |
| **与上下文延续可组合性** | 已内建：previous_refined_context（前块精修尾 25 词）+ 相邻块上下文 | 内建即「全文上下文」（这正是成本爆炸源）；改造后需自行设计窗口 |
| **与标签/占位符管线可组合性** | 已内建：EPUB 精修带占位符 + strict 校验，坏了就弃用精修结果保底 | 完全无标签概念（纯文本）。若套占位符管线，reflect/improve 两步都要过校验，失败面 ×2 |
| **失败语义** | 每块独立降级：精修失败/校验失败 → 保留一遍译文，永不倒退 | 无失败处理：任何一步异常即崩溃；improve 输出无校验直接采信 |
| **确定性/可断点** | 精修循环支持中断（TXT 中断后余块原样输出）；EPUB refine-only 暂无 resume | 无断点、无状态，重跑即全量重跑 |
| **中间产物** | 无（草稿直接被替换；日志可见前后对比） | 有显式 reflection 文本——**可存档、可人审、可作为质检报告复用** |

### 三种归宿在机制层面意味着什么

**A. 合并（reflection 作为 refine 的实现替换/升级）**
- 把 TBL 式 refine 的「单步重写」换成「reflect(源文+译稿) → improve」两步，接入我们自己的分块/占位符/校验/断点框架。
- 机制代价：每块从 2 次调用变 3 次（+50% 调用数）；prompt 需同时装源块 + 译块（+建议清单），块预算（TBL 默认 450 token）下上下文窗口压力可控，但比纯 refine 每块贵约 2-3×。
- 机制收益：获得修忠实度的能力（TBL refine 结构性缺失）；reflection 文本可入 QA 流水线。
- 必须自行解决：多块 reflection 的上下文供给（translation-agent 的全文方案不可搬，需换成邻块窗口——TBL 的 context_before/after 基建正好是现成挂点）；占位符校验在 improve 步之后再挂一次。

**B. 并存（两种二遍 pass 并列为可选项）**
- 机制上零冲突：两者都是「块级第二遍」，输入不同（草稿 vs 源+稿）、成本不同（2x vs 3x）、能力不同（表达 vs 忠实+表达）。可按场景选择：OCR 烂源/文学润色用 refine，精度敏感/术语密集用 reflection。
- 代价：两套提示词 + 两套校验路径要长期维护；#48 的词汇表分工要定义两遍（refine 对草稿过滤 vs reflection 对源文过滤，语义不同）。
- 注意一个机制陷阱：refine 与 reflection 串联使用（三遍 pass）时，refine 在前会先把误译「写通顺」，增加 reflect 识别误译的难度；反序则 refine 可能破坏 improve 修好的忠实性。并存 ≠ 可自由串联。

**C. 取代（reflection 完全替换 refine）**
- 机制上即方案 A 去掉纯 refine 路径：所有二遍需求都付 3 次调用成本。
- 丢失的能力：refine 的「无源文轻量润色」模式（refine-only 对已译文件的独立精修，此场景**没有源文可用**，reflection 无法运作——`src/core/refine/txt_refiner.py` 的输入就是纯译文）。若我们需要「对既有译本做纯润色」的产品能力，reflection 无法覆盖，取代即砍掉该场景。
- 简化的部分：单一二遍语义，词汇表注入点唯一（#48 的分工讨论简化为「词汇表如何进 reflect/improve」）。

### 与 #45/#48 直接相关的机制事实清单

1. **TBL refine 看不到源文**——它是目标语内的文体重写，忠实度问题（误译/漏译）机制上不可修复；translation-agent reflection 的第一维度恰好补这个洞。两者不是同类物的强弱版本，而是**不同轴**上的二遍。
2. **translation-agent 的全文上下文是成本核弹**：多块模式每次调用送全文，书籍级为 O(n²)。任何采纳都必须先把 tagged_text 换成窗口上下文；换掉之后它剩下的核心资产其实就是**那三段提示词与四维度批评框架**。
3. **TBL 的结构安全与二遍 pass 是正交层**：占位符保护/strict 校验/三阶段降级对任何二遍（refine 或 reflection）同样适用——EPUB 精修已经示范了「二遍输出必须过占位符校验、不过就弃用」的模式（`_refine_epub_chunks` 1497-1502 行），reflection 接入可直接复用该闸门。
4. **词汇表的注入位置语义不同**：翻译遍对源文过滤、refine 遍对草稿（目标语）过滤（TBL 现状）；若接 reflection，天然第三个挂点是 reflect 的术语维度（作为批评标准而非硬替换）。#48 需要决定的是这三个挂点各自开不开、注什么。
5. **成本基线**（每块调用数 × prompt 规模）：单遍翻译 1×O(块)；+refine 2×O(块)；+reflection（窗口化改造后）3×O(块)。EPUB 占位符失败重试与 Phase 2 兜底会在此之上再加不定次数调用。
6. **上下文延续与并行互斥**（TBL 实测结论）：EPUB 管线为并行放弃了跨块上下文，纯文本管线并行 >1 时同样放弃。reflection 若要带「前块终稿」上下文，同样面临顺序化 vs 并行的取舍。

---

## 附：本报告引用的关键文件索引

**TBL**：
`src/core/chunking/token_chunker.py` · `src/core/epub/html_chunker.py` · `src/core/epub/text_splitter.py` · `src/core/epub/tag_classifier.py` · `src/core/epub/tag_preservation.py` · `src/core/epub/placeholder_validator.py` · `src/core/epub/placeholder_renumberer.py` · `src/core/epub/token_alignment_fallback.py` · `src/core/epub/xhtml_translator.py` · `src/core/epub/xhtml_translation_state.py` · `src/core/epub/translator.py` · `src/common/placeholder_format.py` · `src/core/common/plain_text_pipeline.py` · `src/core/common/parallel.py` · `src/core/translator.py` · `src/core/text_processor.py` · `src/core/context_optimizer.py` · `src/core/llm/base.py` · `src/core/llm/factory.py` · `src/core/llm/key_pool.py` · `src/core/llm/rate_limit_handler.py` · `src/core/llm/providers/ollama.py` · `src/core/llm/providers/openai.py` · `src/core/llm/utils/extraction.py` · `src/core/glossary/{models,filter,injector,ner,cli_loader}.py` · `src/core/refine/{txt_refiner,epub_refiner}.py` · `src/core/adapters/{generic_translator,retry_manager}.py` · `src/persistence/checkpoint_manager.py` · `src/prompts/prompts.py` · `src/prompts/examples/{helpers,placeholder_examples}.py` · `src/config.py` · `translate.py` · `docs/API_KEY_ROTATION.md`；测试佐证：`tests/test_tag_preservation_extended.py`、`tests/test_checkpoint_manager_xhtml.py`、`tests/test_xhtml_chunk_interruption.py` 等。

**translation-agent**：
`src/translation_agent/utils.py`（全部核心）· `app/patch.py`（多后端 monkey-patch）· `examples/example_script.py`。
