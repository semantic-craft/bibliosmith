# Qwen / Doubao Responses API 迁移依据（2026-07-30）

本文只回答 BiblioSmith 当前使用的 Qwen、Doubao 文本生成能力如何迁移到 Responses API。Embedding 与 Rerank 仍应使用各自的专用接口，不属于 Responses API 的替代范围。

## 结论

- 当前模型目录里的 4 个 Qwen 模型都在百炼官方 Responses 支持清单中：`qwen3.7-max`、`qwen3.7-plus`、`qwen3.6-plus`、`qwen3.6-flash`。
- 当前 Doubao 模型中的 `doubao-seed-2-1-pro-260628`、`doubao-seed-2-1-turbo-260628`、`doubao-seed-evolving` 均可接 Responses。方舟迁移文档规定 250615 及之后的大语言模型默认支持 Responses，并直接以 Seed 2.1 Pro 展示 Responses 调用；模型列表和 Seed-Evolving 专页也把这些当前模型列在 Responses 能力下。
- 两家都采用 `POST .../responses`、`input` 和 `output[]` 结构；原始 HTTP 响应不能再读取 `choices[0].message.content`。
- 两家的 `store` 默认值都是 `true`。本项目逐翻译单元独立调用，且处理本地书稿，应显式发送 `"store": false`。
- Doubao 明确支持 `max_output_tokens`；Qwen 当前参数参考没有列出该字段，并声明未列出的 OpenAI 参数会被忽略。不要声称 Qwen 迁移后仍保留了原来的输出 token 上限。

## Context7 覆盖情况

按项目规则先执行了以下 Context7 路由：

1. `ctx7 library "阿里云百炼 Qwen" ...` 解析到 `/websites/help_aliyun_zh_model-studio`。
2. 对该库分别查询 Qwen Responses 合同和用户指定的 `compatibility-with-openai-responses-api`。Context7 只返回百炼首页的 Chat Completions 示例、Batch 页面和旧的 Chat 兼容参数页，没有命中两个当前 Responses 页面。
3. 初次用产品名查找时选到了 `/websites/volcengine_doubao`，只返回产品页片段；改用方舟文档库 `/websites/volcengine_82379` 后，Context7 命中了迁移指南、创建 Response 和 Seed-Evolving 等官方页面。
4. Context7 返回了 `client.responses.create`、`input`、`max_output_tokens`，以及先含 `reasoning`、后含 `message/output_text` 的 Seed 2.1 原始响应示例；这些内容足以覆盖本项目的 Doubao 迁移合同。

所以 Doubao 的关键合同由 Context7 命中的厂商一手中文文档覆盖；Qwen 的当前 Responses 页面仍未被 Context7 命中，改用用户指定的阿里云官方迁移指南和参数参考直接核对。

## Qwen（阿里云百炼）

主迁移文档是[OpenAI Responses 接口兼容及迁移指南](https://help.aliyun.com/zh/model-studio/compatibility-with-openai-responses-api)，参数合同以[创建响应 / Responses API 参数参考](https://help.aliyun.com/zh/model-studio/qwen-api-via-openai-responses)为准。

### Endpoint 与模型

- 北京推荐 Base URL：`https://{WorkspaceId}.cn-beijing.maas.aliyuncs.com/compatible-mode/v1`
- 北京 HTTP Endpoint：`POST https://{WorkspaceId}.cn-beijing.maas.aliyuncs.com/compatible-mode/v1/responses`
- 现有 `https://dashscope.aliyuncs.com` 仍可使用，但官方建议迁移到业务空间专属域名。
- 旧路径 `/api/v2/apps/protocols/compatible-mode/v1/responses` 即将停止维护；应使用 `/compatible-mode/v1/responses`。
- 当前项目目录中的 4 个 Qwen 模型均出现在迁移指南和参数参考的北京支持清单中。

项目以共享 DashScope 域名作为无配置默认值；用户可在 Launcher 中选填北京 Workspace ID，由程序生成 `https://{WorkspaceId}.cn-beijing.maas.aliyuncs.com/compatible-mode/v1`，并同时用于连接测试和实际翻译。Workspace ID 属于用户配置，不写入公共 provider 注册表。

### 请求与响应合同

适合本项目的最小请求是：

```json
{
  "model": "qwen3.7-max",
  "input": [
    {"role": "system", "content": "<translation system instruction>"},
    {"role": "user", "content": "<source text>"}
  ],
  "store": false
}
```

官方同时支持把系统指令放入 `instructions`、把待译文本作为字符串 `input`；消息数组更接近当前实现，语义也受官方迁移示例直接覆盖。

SDK 可读取 `response.output_text`。原始 HTTP 响应需要遍历：

```text
output[] where type == "message"
  -> content[] where type == "output_text"
  -> concatenate text
```

不能假定 `output[0]` 是最终文本，因为开启思考时 `output` 可先出现 `reasoning` 项。

### Qwen 特有边界

- `store` 默认 `true`；本项目必须显式设为 `false`。
- `temperature`、`top_p` 在参数参考中有定义。
- 参数参考声明只处理文档明确列出的参数，其他 OpenAI 参数会被忽略；该页当前未列出 `max_output_tokens` 或 `max_tokens`。若项目未来需要严格限制 Qwen 输出长度，应先取得百炼官方新增合同或做真实 API 验证，不能把“请求中发了字段”当作“限制已生效”。
- `background` 当前不支持，未列出的兼容字段也不能默认认为可用。

## Doubao（火山方舟）

主迁移依据是[迁移至 Responses API](https://www.volcengine.com/docs/82379/1585128?lang=zh)，完整字段合同见[创建 Response](https://www.volcengine.com/docs/82379/1569618?lang=zh)。模型能力用[模型列表](https://www.volcengine.com/docs/82379/1330310?lang=zh)和[最新模型：Seed-Evolving](https://www.volcengine.com/docs/82379/2549861?lang=zh)交叉验证。

### Endpoint 与模型

- Base URL：`https://ark.cn-beijing.volces.com/api/v3`
- HTTP Endpoint：`POST https://ark.cn-beijing.volces.com/api/v3/responses`
- 官方迁移规则：250615 及之后的大语言模型，如无特殊说明，默认支持 Responses；`doubao-1-5-pro-32k-character-250715` 是文档列出的例外。
- `doubao-seed-2-1-pro-260628`：迁移文档直接给出 Responses 请求和完整响应。
- `doubao-seed-2-1-turbo-260628`：符合 250615 之后的规则，并在当前模型列表的 Responses 能力中列出。
- `doubao-seed-evolving`：官方专页明确说明该模型使用 Responses 的 `previous_response_id` 回传上下文、支持 Responses 缓存；当前模型列表也将其列入 Responses。

### 请求与响应合同

适合本项目的最小请求与 Qwen 相同：

```json
{
  "model": "doubao-seed-2-1-pro-260628",
  "input": [
    {"role": "system", "content": "<translation system instruction>"},
    {"role": "user", "content": "<source text>"}
  ],
  "store": false
}
```

Doubao 还明确支持：

- `instructions`：系统或开发者指令；与 `previous_response_id` 并用时不会继承上一轮指令。
- `max_output_tokens`：回答与思维链合计的最大输出 token 数。它是 Chat API `max_completion_tokens` 的 Responses 对应字段。
- `temperature`：默认 `1.0`。
- `store`：默认 `true`；存储默认保留 3 天，最多 7 天。

原始 HTTP 文本提取结构同 Qwen：扫描 `output[]` 中的 `message`，再扫描其 `content[]` 中的 `output_text.text`。官方 Seed 2.1 响应示例先返回 `reasoning`，后返回 `message`，进一步说明不能固定读取 `output[0]`。

## 对当前实现的直接要求

| 项目 | Qwen | Doubao |
|---|---|---|
| Provider transport | `openai-responses` | `openai-responses` |
| URL | `{base_url}/responses` | `{base_url}/responses` |
| Prompt | `input` 消息数组，或 `instructions` + 字符串 `input` | 同左 |
| 隐私 | 显式 `store: false` | 显式 `store: false` |
| 文本解析 | 扫描 `message -> output_text -> text` | 同左 |
| 最大输出 | 当前官方合同未列；不要虚报已生效 | 可用 `max_output_tokens` |
| 当前内置模型 | 4 个均支持 | Evolving、2.1 Pro、2.1 Turbo 均支持 |

连接测试也必须跟随 provider transport：Responses 槽位请求 `/responses` 并发送 `input`、`store: false`；仍使用 Chat Completions 的其他厂商继续请求 `/chat/completions`。

最后，`text-embedding-v4` 与 `qwen3-rerank` 仍分别走 `/embeddings`、`/reranks`。它们不是文本生成 Responses 接口，保持专用调用才是“能接 Responses 的全升级”，而不是把所有 Qwen 名称的能力机械改到 `/responses`。
