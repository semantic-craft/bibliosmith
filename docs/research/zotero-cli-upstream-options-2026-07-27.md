# Zotero CLI 上游方案评估：可借鉴、可接入与不可替换的边界

**评估日期：2026-07-27**

**仓内复核：2026-08-04。** 本文对三个上游项目的代码判断均绑定到链接中的固定 commit，不应把“维护信号”段落当成不带日期的当前版本说明。复核时，本项目已经落地 `zotero-cli-agent-v1` 统一 envelope、机器可读 `schema`、稳定错误/退出码和 429 `Retry-After` 处理；下文优先级 A 因而是原始路线图，其中仍未落地的主要项是 `doctor`、通用 dry-run / NDJSON 批处理恢复、创建请求的 `Zotero-Write-Token`，以及普通本地检索的稳定快照读取。

## 结论

这三个仓库足以帮助我们把项目内 Zotero CLI 的**交互契约、错误处理、批处理体验和若干桌面端能力**明显向前推进，但不足以直接替换现有 `zsearch` / `zfulltext` / Book Pipeline，也不适合把三个工具同时安装后串起来。

更准确的定位是：

1. [`Agents365-ai/zotero-cli-cc`](https://github.com/Agents365-ai/zotero-cli-cc/tree/997edc9a10ed575a67afd8a7dbbcd1625ab137dc) 是最有价值的 **agent-first CLI 设计参照**：统一 JSON、错误码、schema discovery、dry-run、批处理进度、幂等缓存和更宽的命令面都值得吸收。
2. [`tnajdek/zotero-api-client`](https://github.com/tnajdek/zotero-api-client/tree/287bb69583bf55323eb3969dcde1c6d34f413993) 是最好的 **Zotero Web API 语义与测试参照**：适合拿来校验分页、版本控制、文件上传、响应包装和请求构造，不值得为了它给当前 Python 链路引入 Node 运行时。
3. [`PiaoyangGuohai1/cli-anything-zotero`](https://github.com/PiaoyangGuohai1/cli-anything-zotero/tree/f621952f3645546573d622440cbf707320f7a35f) 是 **桌面 Zotero 动作与文献工作流的能力素材库**：Find Full Text、开放获取 PDF 级联、DOCX 引文字段和审计思路有价值；其任意 JavaScript bridge、活库 `immutable=1` 读取和直接 SQLite 写入不能采用。

现有项目在最重要的流水线正确性上反而更成熟：稳定 SQLite 快照、冻结集合成员、逐附件状态、输入与产物哈希、显式 HITL 审批、阶段恢复和隐私分层都不是这三个仓库能替代的。优化应采用“**保留现有数据与编排契约，选择性移植上层接口模式和窄能力适配器**”的路线。

## 当前项目不可退让的基线

本项目的 Zotero 支持不是孤立的文献管理 CLI，而是 Book Pipeline 的输入边界：

- `zsearch` 承担本地库检索、集合发现和 Web API 写入，`zfulltext` 承担 item-scoped 的抽取、分块与索引；MCP 只暴露四个只读工具。参见 [`packages/zotero-cli/README.md`](../../packages/zotero-cli/README.md)。
- 集合执行使用 `zotero-collection-snapshot-v1`。实现会复制主库与 WAL/journal、执行 `quick_check`、规范化备份，并要求相邻两次规范化快照哈希一致，避免在 Zotero 活跃写入期间取得虚假的集合成员视图。参见 [`zotero_db.py`](../../packages/zotero-cli/src/zotero_cli/zotero_db.py)。
- Book Pipeline 已具备冻结后的父项/附件身份、逐附件恢复、阶段状态、审批绑定、产物哈希和原子状态写入；私有正文不能被复制进编排状态。参见 [`docs/book-pipeline-capability-matrix.md`](../book-pipeline-capability-matrix.md) 和 [ADR 0001](../adr/0001-federated-book-pipeline-state.md)。
- Web API 层已经显式发送 API v3，更新和删除会先读取最新对象版本，再使用 `If-Unmodified-Since-Version`，避免静默覆盖并发修改。参见 [`zotero_api.py`](../../packages/zotero-cli/src/zotero_cli/zotero_api.py)。

因此，任何上游方案若不能保持这些契约，都只能作为上层可选适配器，不能成为集合发现、流水线冻结或全文索引的新事实来源。

当前基线仍有一处内部债务：集合快照已经走稳定复制，但普通 `zsearch` 的 `iter_items()` 仍通过 `_connect_readonly()` 直接以 `mode=ro&immutable=1` 打开活库。原先 [issue #33](https://github.com/semantic-craft/bibliosmith/issues/33) 记录的同步定时任务已由用户决定保持退役，并于 2026-07-27 关闭；不得把本文当作重新装载它的授权。若未来另行决定恢复自动同步，应先让普通同步/检索改走稳定快照或官方 Local API，并补真实 WAL 并发回归；不能只修 launchd 路径后重新启用。

## 三个仓库的实质差异

| 维度 | `zotero-cli-cc` | `zotero-api-client` | `cli-anything-zotero` |
|---|---|---|---|
| 核心定位 | Python agent-first CLI、可选 MCP、PDF/工作区/RAG | JavaScript Web API SDK | Python CLI + Zotero Desktop 插件 |
| 读取路径 | 本地 SQLite；也通过 Web API 访问远端/群组 | 只走 Zotero Web API | SQLite、Local API、Connector API、插件 bridge |
| 写入路径 | 主要由 `pyzotero` 写 Web API；少量桌面动作走窄 bridge | 原始 POST/PUT/PATCH/DELETE 和文件上传 | Connector/bridge，另含实验性直接 SQLite 写 |
| 鉴权 | Web API key；本地读免鉴权 | Web API key | 不用 Web API key；依赖本机 Zotero 与无令牌 bridge |
| 结构化输出 | 最完整：统一 envelope、schema、错误码、自动 JSON、NDJSON | SDK 响应对象，不是 CLI 输出契约 | 新命令有结果契约，但尚未覆盖全部旧命令 |
| 离线/桌面依赖 | 本地读可离线；部分功能需 API、模型或插件 | 必须联网访问 Web API | Zotero Desktop 必须运行 |
| 对当前项目的最佳用途 | 移植 agent interface；评估少量能力适配器 | API conformance/reference | 研究桌面动作、DOCX、PDF 获取流程 |
| 是否可替换当前 CLI | 否 | 否，本身不是 CLI | 否 |

### 1. `Agents365-ai/zotero-cli-cc`

#### 值得吸收

它对 agent 调用面的设计最完整。其 [agent interface 文档](https://github.com/Agents365-ai/zotero-cli-cc/blob/997edc9a10ed575a67afd8a7dbbcd1625ab137dc/docs/agent-interface.md) 描述了统一 JSON envelope、稳定错误码与退出码、request ID、schema version、partial success、非 TTY 自动 JSON、NDJSON 进度，以及通过 `zot schema` 做机器可读的命令发现。其 [writer](https://github.com/Agents365-ai/zotero-cli-cc/blob/997edc9a10ed575a67afd8a7dbbcd1625ab137dc/src/zotero_cli_cc/core/writer.py) 和 [idempotency 模块](https://github.com/Agents365-ai/zotero-cli-cc/blob/997edc9a10ed575a67afd8a7dbbcd1625ab137dc/src/zotero_cli_cc/core/idempotency.py) 还提供 dry-run、安全等级和本地幂等缓存。

它的命令面也比现有实现更宽：标注、群组库、引用与表格、PDF 多提取器、预印本更新、重复项/孤儿项、工作区检索等。适合用来决定下一批真实用户价值，而不是照搬全部功能。

#### 不能原样采用

其 [SQLite reader](https://github.com/Agents365-ai/zotero-cli-cc/blob/997edc9a10ed575a67afd8a7dbbcd1625ab137dc/src/zotero_cli_cc/core/reader.py) 优先以 `mode=ro&immutable=1` 打开活库，只有遇到操作错误才退回一次性复制。这不能满足本项目“复制 WAL/journal、完整性检查、相邻稳定快照”的集合冻结要求。

其幂等机制是客户端本地缓存，不等于服务端 exactly-once：如果 Zotero 已成功创建对象，但本地缓存尚未落盘，重试仍可能重复创建。Zotero 官方为创建请求提供了 `Zotero-Write-Token`，成功 token 会被服务器缓存；我们应优先按官方语义补齐，而不是直接复制本地缓存设计。[官方写请求文档](https://www.zotero.org/support/dev/web_api/v3/write_requests)

认证配置还允许把 API key 写进用户配置文件，见其 [`config.py`](https://github.com/Agents365-ai/zotero-cli-cc/blob/997edc9a10ed575a67afd8a7dbbcd1625ab137dc/src/zotero_cli_cc/config.py)。保存路径没有在实现中显式收紧文件权限。这与本项目现有“根 `.env` / 进程环境优先、凭证不进仓库”的契约不同，不能顺势迁移。

其 48 个 MCP tools 虽然能力丰富，但写入和删除会直接执行，不能自动继承 CLI 层的 dry-run、幂等和确认语义。对本项目而言，这进一步证明 MCP 应继续保持只读；未来如开放写工具，需要独立的审批协议，而不是简单转发 writer。

许可证也需要先澄清。根 [`pyproject.toml`](https://github.com/Agents365-ai/zotero-cli-cc/blob/997edc9a10ed575a67afd8a7dbbcd1625ab137dc/pyproject.toml) 声明 AGPL-3.0-or-later 并提供商业许可，而 bridge 的 [`README`](https://github.com/Agents365-ai/zotero-cli-cc/blob/997edc9a10ed575a67afd8a7dbbcd1625ab137dc/extension/zot-cli-bridge/README.md) 又写 CC-BY-NC-4.0。未获得上游确认前，不应 vendoring 这部分代码。

#### 维护信号

仓库创建于 2026 年，尚年轻，但已有较大的测试集、GitHub Actions、持续发布和近期修复记录；[Actions](https://github.com/Agents365-ai/zotero-cli-cc/actions) 与 [v0.10.0](https://github.com/Agents365-ai/zotero-cli-cc/releases/tag/v0.10.0) 显示其当前仍活跃。依赖面明显重于本项目，包括 `pyzotero`、PDF 引擎、OpenAI 和可选 MCP/解析器，整包引入会扩大安装、升级与许可证表面。

### 2. `tnajdek/zotero-api-client`

#### 值得吸收

这是一个成熟的 JavaScript SDK，而不是 CLI。其 [`api.js`](https://github.com/tnajdek/zotero-api-client/blob/287bb69583bf55323eb3969dcde1c6d34f413993/src/api.js)、[`request.js`](https://github.com/tnajdek/zotero-api-client/blob/287bb69583bf55323eb3969dcde1c6d34f413993/src/request.js) 和 [`response.js`](https://github.com/tnajdek/zotero-api-client/blob/287bb69583bf55323eb3969dcde1c6d34f413993/src/response.js) 将 API key、用户/群组库、分页、版本条件、响应元数据、文件上传/下载、schema/template、full-text index status 等组合成可测试的请求对象。`pretend()` 只构造请求而不发送，尤其适合参考来实现可信 dry-run。

最适合的用法是建立**跨实现 conformance fixtures**：给定同一 Zotero API 场景，验证当前 Python `httpx` 层在 header、分页、版本冲突、文件上传三阶段和响应解析上是否与官方语义及这一成熟 SDK 一致。

#### 不宜引入为运行时依赖

当前 Zotero CLI 是 Python，Book Pipeline 核心是 Rust/Tauri。为一个现有 `httpx` 能覆盖的 API 客户端加入 Node 18、Babel/core-js 和第二套重试/错误模型，会增加而不是减少边界。

它也不会替我们解决本地 SQLite 一致性、Zotero Desktop 动作、全文抽取、HITL、流水线状态或私有正文边界。其 [请求实现](https://github.com/tnajdek/zotero-api-client/blob/287bb69583bf55323eb3969dcde1c6d34f413993/src/request.js) 的重试需要调用方主动开启，主要针对 408 与 5xx；超时支持仍有长期开放的 [issue #12](https://github.com/tnajdek/zotero-api-client/issues/12)。因此它是参照，不是托管可靠性的替代品。

#### 维护与许可证

仓库从 2017 年延续至今，CI 仍运行，近期提交与 v0.50.0 tag 表明仍在维护。参见 [Actions](https://github.com/tnajdek/zotero-api-client/actions)、[commits](https://github.com/tnajdek/zotero-api-client/commits/master/) 和 [`package.json`](https://github.com/tnajdek/zotero-api-client/blob/287bb69583bf55323eb3969dcde1c6d34f413993/package.json)。不过维护者较集中，而且 CI 的 lint 脚本容许 lint 失败，不能把“工作流绿色”解读成全部质量门禁严格通过。其许可证为 AGPL-3.0；仅将它用于黑盒/源码参照与测试语义的风险，远低于复制实现或分发其代码。

### 3. `PiaoyangGuohai1/cli-anything-zotero`

#### 值得研究

它将 SQLite、Local API、Connector API 和 Zotero 插件拼成统一 CLI，覆盖 DOI/arXiv/URL/文件摄取、开放获取 PDF 级联、批处理续跑、重复项合并预览、DOCX 静态/动态 Zotero 引文字段、审计日志和语义搜索。详见其 [README](https://github.com/PiaoyangGuohai1/cli-anything-zotero/blob/f621952f3645546573d622440cbf707320f7a35f/README.md) 与 [命令表](https://github.com/PiaoyangGuohai1/cli-anything-zotero/blob/f621952f3645546573d622440cbf707320f7a35f/docs/COMMANDS.md)。

高价值设计包括：

- `app doctor` 对 Zotero、数据库、插件和外部依赖做统一预检；
- PDF 批处理的 JSONL、限额与 resume；
- 重复合并默认 dry-run；
- 本地写操作审计；
- DOCX placeholder 到静态文本或 Zotero 动态域的分层工作流。

其新命令已经采用 `action / ok / status / code` 结果对象，见 [`results.py`](https://github.com/PiaoyangGuohai1/cli-anything-zotero/blob/f621952f3645546573d622440cbf707320f7a35f/cli_anything/zotero/core/results.py)，但 [roadmap](https://github.com/PiaoyangGuohai1/cli-anything-zotero/blob/f621952f3645546573d622440cbf707320f7a35f/docs/ROADMAP.md) 也明确反映统一契约仍未覆盖全部旧命令。

#### 两条硬性否决线

第一，插件注册无 token 的 `POST /cli-bridge/eval`，再把请求正文交给 JavaScript `eval()`，见 [`bootstrap.js`](https://github.com/PiaoyangGuohai1/cli-anything-zotero/blob/f621952f3645546573d622440cbf707320f7a35f/cli_anything/zotero/plugin/zotero-cli-bridge/bootstrap.js)。官方说明 Local API 本身无认证、仅应留在 loopback，而且浏览器页面可向内置 Connector HTTP server 发出某些 POST；这使通用 eval bridge 的权限面远超一个文献命令。[Local API](https://www.zotero.org/support/dev/web_api/v3/local_api)；[Connector HTTP server](https://www.zotero.org/support/dev/client_coding/connector_http_server)

第二，其 [`zotero_sqlite.py`](https://github.com/PiaoyangGuohai1/cli-anything-zotero/blob/f621952f3645546573d622440cbf707320f7a35f/cli_anything/zotero/utils/zotero_sqlite.py) 既会以 `immutable=1` 读取正在运行的 Zotero 活库，又包含直接更新 Zotero SQLite 表的实验性命令；备份只复制主数据库，没有复刻本项目的 WAL/journal 稳定快照协议。Zotero 官方把直接 SQLite 访问描述为比 JavaScript API 更脆弱；修改应通过受支持对象与事务 API 完成。[Zotero JavaScript API](https://www.zotero.org/support/dev/client_coding/javascript_api)

因此，不仅不能启用通用 `js` 命令，也不能把其 SQLite 层接到 Book Pipeline。

#### 维护信号

项目在 2026 年 4 月创建，7 月仍快速发布，[v1.2.0](https://github.com/PiaoyangGuohai1/cli-anything-zotero/releases/tag/v1.2.0) 较新；Apache-2.0 便于有选择地借鉴代码。但仓库尚无 GitHub Actions，维护者集中，接口和文档仍快速演化，[Actions](https://github.com/PiaoyangGuohai1/cli-anything-zotero/actions) 目前不能提供持续集成门禁证据。它适合试验和拆解，不适合成为流水线的核心依赖。

## 与现有流程相比，真正值得补的能力

### 优先级 A：立即提升效率，而且不扩大信任边界

1. **统一机器输出。** 为全部命令定义 `ZoteroCommandResult v1`：`ok`、`data`、`error.code`、`error.retryable`、`meta.request_id`、`meta.schema_version`、耗时、partial success；非 TTY 自动 JSON。
2. **稳定错误与退出码。** 明确输入错误、未找到、认证、并发冲突、限流、外部依赖、部分成功；让 Launcher 不再解析自由文本。
3. **`zsearch schema` 与 `doctor`。** schema 暴露命令、参数、安全等级和输出版本；doctor 只读检查数据库、WAL、API 权限、可选依赖与 bridge 状态。
4. **dry-run 与批处理 NDJSON。** 所有写命令先输出计划；批量流程按 item 逐条给出进度和最终汇总，支持从稳定 item key 恢复。
5. **Web API 强化。** 增加 key 权限预检、`Retry-After`、明确区分 412/428/429；创建请求使用官方 `Zotero-Write-Token`，更新继续保持版本条件。

这些改动可以在当前 Python 包内完成，不需要把任何上游工具加入运行时。

### 优先级 B：扩展文献工作能力，但保持 HITL

- 标注读取与导出；标注写入仍需显式审批。
- BibTeX/RIS/CSL JSON 导出、引用样式渲染和 Better BibTeX citekey 映射。
- 群组库与远端 Web API 只读 fallback，明确标识“本地快照”与“远端最新”两种证据来源。
- attachment/child 浏览、受控的文件导入与重命名。
- 可插拔 PDF 提取器与开放获取候选发现；下载、挂接和改动 Zotero 前显示来源、许可线索、目标 item 与计划。

### 优先级 C：只有真实需求出现才做的桌面端 bridge

如果用户确实需要 Zotero-native Find Full Text、桌面端重命名/导入或动态 DOCX 引文，可以设计一个窄 bridge：

- 每个动作独立 endpoint 和固定 schema；
- 无任意 JavaScript、无 SQL；
- 随机 token、loopback、最小权限、速率限制和审计；
- 默认关闭，启动时展示有效期和允许动作；
- 写动作由 Launcher 的 HITL 审批绑定 item key、参数和预期结果；
- bridge 完成后由 Web API/本地稳定快照二次核验，再推进流水线状态。

动态 DOCX 引文还依赖 LibreOffice/Zotero 插件，应该保持为独立工作流，不塞进核心书籍翻译路径。

## 分阶段实施建议

### Phase 0：契约先行，不引入依赖

- issue #33 已按用户决定以“同步定时任务保持退役”关闭；不要重新启用该任务，既有 OCR 定时任务也继续保持禁用。普通 `zsearch` 活库读取是独立的后续债务。
- 写出 `ZoteroCommandResult v1`、退出码、错误码、安全等级和 dry-run 规范。
- 以官方 Web API 文档为事实源，为创建、更新、删除、分页、限流、上传建立 conformance tests。[Web API Basics](https://www.zotero.org/support/dev/web_api/v3/basics)；[Write Requests](https://www.zotero.org/support/dev/web_api/v3/write_requests)；[File Uploads](https://www.zotero.org/support/dev/web_api/v3/file_upload)
- 固化三类传输边界：稳定本地快照只读、Web API 写、可选窄桌面 bridge；禁止通用 eval 与直接 SQLite 写。

### Phase 1：最高回报的内部改造

- 统一 JSON envelope、非 TTY 自动 JSON、稳定 exit/error code。
- 增加 `schema`、`doctor`、安全等级、dry-run 和 NDJSON batch progress。
- 补 key 权限验证、429/`Retry-After`、创建请求 write token，并保持现有并发版本保护。
- Launcher 继续调用本项目 `zsearch` / `zfulltext`，只把输出解析切到版本化契约。

### Phase 2：按用户价值增加能力

- 先做 annotations/export/citation/group library 等纯读或低风险能力。
- 文件导入、元数据修改、批量标签等写能力继续经过现有审批与证据绑定。
- 仅在真实需求和威胁模型明确后，试作一个窄 bridge；用真实 Zotero 沙盒库做跨平台 smoke，不碰用户主库。

### Phase 3：再决定是否保留外部适配器

只有当某个上游工具持续提供本项目不值得自研的独立能力，而且能通过许可证、安全、输出契约和恢复测试，才把它作为可选 adapter 固定版本接入。即使如此，集合快照、逐项流水线状态和 `zfulltext` 索引仍由本项目所有。

## 明确不建议

- 不把三个仓库都安装到核心路径，也不长期维护三套索引、配置和凭证约定。
- 不用任何上游 `immutable=1` 活库读取替换当前集合稳定快照。
- 不采用任何直接写 Zotero SQLite 的命令。
- 不安装或暴露 `cli-anything-zotero` 的通用 `/cli-bridge/eval`。
- 不在许可证矛盾澄清前复制或分发 `zotero-cli-cc` bridge。
- 不为了 `zotero-api-client` 给当前 Python CLI 引入 Node 运行时；把它用于语义与测试参照即可。
- 不用上游 workspace/RAG 替换 item-scoped `zfulltext`、artifact hash 和 Book Pipeline 的隐私边界。
- 不把写命令扩进现有只读 MCP 面，除非另立 ticket 明确鉴权、审批、审计、恢复和接受标准。
- 不把 API key 迁到新的明文配置文件，也不让任何错误、审计或进度输出包含凭证或私有正文。

## 回答原问题

**足够优化吗？** 足够。三者覆盖了 CLI 交互、Web API、桌面动作、批处理和 agent contract 的主要设计空间，足以制定并实施下一阶段优化。

**能否更优、更高效？** 能。最高回报不是换 CLI，而是统一结构化输出、schema/doctor、错误码、dry-run、NDJSON 恢复，以及补齐官方 API 的限流和幂等语义。这样会直接减少 Launcher glue code、自由文本解析和失败后人工判断。

**能否扩展能力边界？** 能，但需要分层。纯读的标注、引用、群组库和导出可以较快加入；Find Full Text、文件导入、动态 DOCX 引文等桌面动作应走可选窄 bridge 和 HITL。任意 eval、直接 SQLite 写和活库 `immutable=1` 读取不属于可接受的扩展。

**三者本身够不够做生产接入决策？** 不够。最终仍要以 Zotero 官方协议、本项目现有流水线契约、许可证确认、威胁模型和真实沙盒 smoke 为准。它们是高价值输入，不是生产正确性的共同背书。
