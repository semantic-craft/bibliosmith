# Qwen Responses API 联网搜索合同核对

日期：2026-07-30
范围：BiblioSmith 当前使用的 `qwen3.7-max`、`qwen3.7-plus`、`qwen3.6-plus`、`qwen3.6-flash`，华北 2（北京）百炼 Workspace 端点。
结论性质：文档核对，并包含一次经用户授权的最小真实 API 验证；不记录 Workspace ID 或 API Key。

## 结论

1. 四个模型均支持百炼 OpenAI 兼容 Responses API，也均在华北 2（北京）的联网搜索支持列表中。
2. Responses API 开启联网搜索的合同是：在 `tools` 数组加入 `{"type":"web_search"}`。不要再发送 Chat Completions / DashScope 使用的 `enable_search: true`。
3. `tool_choice` 默认是 `auto`。虽然 Responses 通用参数说明包含 `required`，但联网搜索功能对照表明确把“强制联网搜索”列为 Responses API 暂不支持；本项目不得用通用参数推断 `web_search` 可以强制调用。
4. 这四个模型都是混合思考模型，默认开启思考，但联网搜索本身不要求它们开启思考。Responses API 当前推荐用 `reasoning.effort` 控制思考强度；`enable_thinking` 是百炼扩展参数，官方已注明后续将不再支持。只有文档另行点名的 `qwen3-max` / `qwen3-max-2026-01-23` 在相关内置工具场景下要求开启思考，不应把该限制误套到本项目的四个模型。
5. 百炼 Responses 默认 `store: true`。BiblioSmith 处理私有书稿且当前调用不依赖 `previous_response_id`，请求应显式使用 `store: false`。
6. 最终文本与搜索来源位于不同输出项中：文本为 `output[].type == "message"` 下的 `content[].type == "output_text"`；来源为 `output[].type == "web_search_call"` 下的 `action.sources[]`。不能假定 `output[0]` 是最终文本。
7. Responses API 当前不支持 `enable_source`、`enable_citation`、`citation_format`，不会自动在回复中插入 `[1]` 角标。若界面需要来源，应自行读取 `web_search_call.action.sources` 并渲染来源列表。

## 真实调用证据

使用用户提供但未写入仓库的北京 Workspace ID 与本机现有 API Key，向 `qwen3.7-max` 发出一条 `store: false`、`tools: [{"type":"web_search"}]` 的非流式请求。结果为 HTTP 200，`output` 依次包含多组 `reasoning`、3 个 `web_search_call`，最后才是 `message`；`usage.x_tools.web_search.count` 为 3，原始来源共 70 条，最终文本可按 `message/output_text` 合同提取。该请求使用 15,467 个输入 Token 和 938 个输出 Token，说明联网搜索必须默认关闭并明确提示额外成本。

这次验证只证明上述模型、北京专属 Host 和当前请求合同实际可用；未把用户的 Workspace ID、API Key 或响应正文写入文档。

主要依据：

- [阿里云百炼：联网搜索](https://help.aliyun.com/zh/model-studio/web-search)
- [阿里云百炼：OpenAI 兼容 Responses——创建响应](https://help.aliyun.com/zh/model-studio/qwen-api-via-openai-responses)
- [阿里云百炼：深度思考](https://help.aliyun.com/zh/model-studio/deep-thinking)
- [阿里云百炼：网页抓取](https://help.aliyun.com/zh/model-studio/web-extractor)

## 用户提供的四个控制台页面

- [控制台文档 3016808](https://bailian.console.aliyun.com/cn-beijing?tab=api#/api/?type=model&url=3016808)
- [控制台文档 3033492](https://bailian.console.aliyun.com/cn-beijing?tab=api#/api/?type=model&url=3033492)
- [控制台文档 3033494](https://bailian.console.aliyun.com/cn-beijing?tab=api#/api/?type=model&url=3033494)
- [控制台文档 3033495](https://bailian.console.aliyun.com/cn-beijing?tab=api#/api/?type=model&url=3033495)

这些地址对应 Responses 的创建，以及已存储响应的读取、删除和输入项管理。对当前翻译引擎，只有“创建响应”属于运行链路：请求显式使用 `store: false`，所以后三类依赖已存储响应的管理接口不应接入。四个控制台地址仍是客户端路由，不适合作为模型支持清单；模型兼容性另以阿里云帮助中心的公开当前列表逐项核对。

## Context7 核对结果

按仓库规则查询了 Context7 库 `/websites/help_aliyun_zh_model-studio`。当前查询没有命中 Responses 联网搜索合同，而是返回了旧版 Chat Completions / DashScope 的 `enable_search` 示例和早期基础调用页面。因此以下当前合同以阿里云帮助中心的一手中文页面为准；不能把 Context7 返回的 `enable_search` 迁入 Responses 请求。

## 模型支持矩阵

| 模型 | Responses | Responses `web_search` | 华北 2（北京） | 思考模式 |
|---|---|---|---|---|
| `qwen3.7-max` | 支持 | 支持 | 支持 | 混合思考，默认开启；搜索本身不强制开启 |
| `qwen3.7-plus` | 支持 | 支持 | 支持 | 混合思考，默认开启；搜索本身不强制开启 |
| `qwen3.6-plus` | 支持 | 支持 | 支持 | 混合思考，默认开启；搜索本身不强制开启 |
| `qwen3.6-flash` | 支持 | 支持 | 支持 | 混合思考，默认开启；搜索本身不强制开启 |

证据链：

- [联网搜索的华北 2 支持列表](https://help.aliyun.com/zh/model-studio/web-search)逐项列出 Qwen3.7 Max、Qwen3.7 Plus、Qwen3.6 Plus、Qwen3.6 Flash，并说明 2025 年 7 月后发布的 Max / Plus / Flash 自动支持联网搜索。
- [Responses 的地域模型列表](https://help.aliyun.com/zh/model-studio/qwen-api-via-openai-responses)逐项列出上述四个模型。
- [深度思考模型列表](https://help.aliyun.com/zh/model-studio/deep-thinking)将上述四个模型列为“混合思考模式，默认开启思考模式”。

新加坡也列有这四个模型，但 Workspace ID、API Key 与 Host 必须和调用地域匹配。本项目当前用户提供的是北京 Host，格式为：

```text
https://{WorkspaceId}.cn-beijing.maas.aliyuncs.com/compatible-mode/v1
```

设置页应保存用户自己的 Workspace ID，并由程序构造 Host；不得把某个开发者的 Workspace ID 写入公共注册表。留空时是否退回共享域名属于产品策略，不改变本文的联网搜索请求合同。

## 请求合同

### 关闭联网搜索

最直接的做法是完全不发送 `tools`。若调用层始终保留工具列表，也可以设置 `tool_choice: "none"`。

```json
{
  "model": "qwen3.7-plus",
  "input": [
    {"role": "system", "content": "Translate faithfully."},
    {"role": "user", "content": "Source text"}
  ],
  "store": false,
  "reasoning": {"effort": "none"}
}
```

`reasoning.effort: "none"` 不是联网搜索的必需字段；这里用于关闭四个混合思考模型默认开启的思考，从而降低普通翻译的延迟与推理 Token 成本。也可以由产品单独提供思考强度设置。

### 允许模型按需联网

```json
{
  "model": "qwen3.7-plus",
  "input": [
    {"role": "system", "content": "Translate faithfully. Use web search only for current facts or terminology that cannot be resolved from the supplied context."},
    {"role": "user", "content": "Source text"}
  ],
  "tools": [{"type": "web_search"}],
  "store": false,
  "reasoning": {"effort": "none"}
}
```

`tool_choice` 省略时默认是 `auto`。本项目采用省略字段的最小请求，让模型按需调用搜索。除非百炼后续在联网搜索专页明确增加支持，不应发送 `required` 来宣称“强制联网”。

### HTTP 示例

```bash
curl -X POST \
  'https://{WorkspaceId}.cn-beijing.maas.aliyuncs.com/compatible-mode/v1/responses' \
  -H "Authorization: Bearer $DASHSCOPE_API_KEY" \
  -H 'Content-Type: application/json' \
  -d '{
    "model": "qwen3.7-plus",
    "input": "核实这个术语的近期官方译法并给出中文翻译：……",
    "tools": [{"type": "web_search"}],
    "reasoning": {"effort": "none"},
    "store": false
  }'
```

这段示例只使用环境变量占位符；本文没有读取或记录任何真实 API Key。

## 不应发送的旧接口参数

下列参数属于 Chat Completions / DashScope 的联网搜索合同，不是当前 Responses `web_search` 工具的启用方式：

```json
{
  "enable_search": true,
  "search_options": {
    "forced_search": true,
    "enable_source": true,
    "enable_citation": true,
    "citation_format": "[<number>]"
  }
}
```

Responses 应分别使用：

- 启用搜索：`tools: [{"type":"web_search"}]`
- 按需调用：省略 `tool_choice`，使用其默认值 `auto`
- 来源：解析 `web_search_call.action.sources`
- 角标：应用自行生成；Responses 暂无自动角标参数

## 思考参数

[Responses 参数参考](https://help.aliyun.com/zh/model-studio/qwen-api-via-openai-responses)给出的当前优先级为：

1. `reasoning.effort` 优先于 `enable_thinking`；
2. `reasoning.effort` 支持 `none`、`minimal`、`low`、`medium`、`high`、`xhigh`、`max`；
3. `xhigh`、`max` 只支持华北 2（北京）和新加坡；
4. `enable_thinking` 后续将不再支持。

因此新实现宜直接采用 `reasoning: {"effort": ...}`。这四个模型调用 `web_search` 时可使用 `none`，也可为更复杂的研究型翻译提供更高档位。不要因为官方综合示例使用了 `enable_thinking: true` 就把它误判为四个模型使用 `web_search` 的硬性要求。

## 响应解析与来源

非流式解析需要遍历整个 `output`：

```python
texts = []
sources = []

for item in response.get("output", []):
    if item.get("type") == "message":
        for part in item.get("content", []):
            if part.get("type") == "output_text":
                texts.append(part.get("text", ""))
    elif item.get("type") == "web_search_call":
        sources.extend(
            source.get("url")
            for source in item.get("action", {}).get("sources", [])
            if source.get("type") == "url" and source.get("url")
        )

translated_text = "".join(texts)
```

思考模式开启时，`reasoning` 项可能出现在最终 `message` 之前；搜索时还会出现 `web_search_call`。因此，解析器应按 `type` 查找，而不是固定读取 `output[0]`。

还可用以下字段核实工具调用：

- `usage.x_tools.web_search.count`
- `usage.plugins.web_search.count`（文档称其与 `x_tools` 内容相同）
- `output` 是否包含状态为 `completed` 的 `web_search_call`

`output_text.annotations` 在百炼合同中“通常为空数组”，不能把它当作搜索来源。官方明确要求从 `web_search_call.action.sources` 读取 URL。

## `store` 与隐私边界

`store` 默认值是 `true`。设为：

- `true`：响应被储存，可被 `previous_response_id` 和后续 API 使用；
- `false`：响应不储存，之后不能通过 `previous_response_id` 引用。

联网搜索不要求 `store: true`。对于一次性翻译请求，应显式使用 `false`。如果未来增加服务端多轮对话，应单独做隐私与保留期设计，而不是静默改回默认值。

## 计费、限流和地域

根据[联网搜索计费说明](https://help.aliyun.com/zh/model-studio/web-search)：

- 内置联网搜索没有免费调用额度；它与“联网搜索 MCP”是两种独立服务、独立计费。
- 费用包括模型调用费和搜索工具费。检索到的网页内容会进入提示上下文，增加输入 Token；思考输出也按输出 Token 计费。
- Responses API 的联网搜索工具按 `agent` 策略计费。当前华北 2（北京）标准为 4 元/千次搜索调用；新加坡文档列为 73.392381 元/千次。价格会变化，产品界面不应把数字写成永久承诺，应链接官方价格页。
- 联网搜索限流为 15 RPS，按阿里云主账号汇总所有 API Key、所有模型计算。超过后 API 不报错，但可能跳过搜索。因此成功的 HTTP 状态不等于一定执行了搜索，应检查 `web_search_call` / `usage.x_tools`。
- Workspace Host、API Key 和模型可用范围均与地域有关。北京 Workspace 必须使用 `cn-beijing.maas.aliyuncs.com`，不得只替换 Workspace ID 而保留其他地域后缀。

## 对实现的直接约束

1. 设置提供“联网搜索”开关即可：关闭时不发送工具；开启时发送单一 `web_search`，由默认的 `auto` 决定是否调用。当前文档不支持再提供“强制”档。
2. 该设置只对支持 Responses 内置工具的 Qwen 模型显示或生效；自定义模型 ID 应在运行时允许百炼返回不支持错误，界面不要宣称未知模型兼容。
3. 实际翻译与连接测试必须使用同一个由用户 Workspace ID 派生的 Host；Workspace ID 不得内置。
4. 连接测试若只发普通 `ping`，只能证明 Responses 端点和鉴权可用，不能证明联网搜索可用。搜索测试会产生费用，应由用户明确触发，并在 UI 标明可能收费。
5. 如果只需要译文，可继续忽略来源；若向用户展示“已联网”，则必须以 `web_search_call` 或 `usage.x_tools.web_search.count > 0` 为证据，不能只凭设置开关推断。
