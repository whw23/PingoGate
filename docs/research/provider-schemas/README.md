# Provider 接口 Schema 调研

> **日期**：2026-07-12
> **状态**：调研文档（基于官方文档 + 已知 API 知识；Gemini Interactions 经 WebFetch 查证）
> **范围**：5 个核心接口的请求/响应/失败 schema + header + body + 模型差异 + 上下文机制 + 流式形态
> **用途**：为 Rust 内核协议识别/转换引擎、Go 上下文桥 materialize、marketplace schema 转换规则提供依据

## 0. 关键差异速查

| 维度 | OpenAI Chat Completions | OpenAI Responses | Gemini generateContent | Gemini streamGenerateContent | Gemini Interactions | Anthropic Messages |
|---|---|---|---|---|---|---|
| 端点 | `POST /v1/chat/completions` | `POST /v1/responses` | `POST /v1beta/models/{m}:generateContent` | `POST /v1beta/models/{m}:streamGenerateContent` | `interactions.create`（REST） | `POST /v1/messages` |
| 流式接口 | 单接口 + `stream:true` | 单接口 + `stream:true` | 独立接口（非流式） | **独立接口（流式）** | 单接口 | 单接口 + `stream:true` |
| 认证 | `Authorization: Bearer` | `Authorization: Bearer` | `x-goog-api-key` 或 `?key=` | 同左 | API key | `x-api-key` + `anthropic-version` |
| 上下文机制 | 完整 messages（无状态） | `previous_response_id`（**有状态**） | 完整 contents（无状态） | 同左 | `previous_interaction_id`（**有状态**） | 完整 messages（无状态） |
| usage 位置 | 末 chunk（需 `include_usage`） | `response.completed` | body `usageMetadata` | 末 chunk `usageMetadata` | 末 chunk | `message_start`+`message_delta` |
| 官方 $schema | OpenAPI 有 | OpenAPI 有 | Discovery 文档 | 同左 | API ref 页 | 部分有 |

**关键**：
1. **只有 Gemini 把流式/非流式拆成两个独立接口**（`:generateContent` vs `:streamGenerateContent`）；OpenAI/Anthropic 都是单接口 + body `stream:true`。
2. **只有 OpenAI Responses 和 Gemini Interactions 是有状态协议**（带 `previous_id`），需经 Go 上下文桥 materialize。
3. **OpenAI Chat Completions 流式默认不给 usage**，需 Rust 转换引擎注入 `stream_options.include_usage:true`。

---

## 1. OpenAI Chat Completions

### 官方 schema
- OpenAI 维护 [openai-openapi](https://github.com/openai/openai-openapi) GitHub 仓库，含完整 OpenAPI 3.0 规范（请求/响应/错误 schema）。官方 $schema 存在。

### 请求
- **端点**：`POST /v1/chat/completions`
- **Headers**：
  - `Authorization: Bearer <key>`（必需）
  - `Content-Type: application/json`（必需）
  - `OpenAI-Organization` / `OpenAI-Project`（可选，多组织/项目）
- **Body 顶层字段**：
  - `model`（string，必需）
  - `messages`（array，必需）：`[{role, content}]`，完整上下文每次传
  - `stream`（bool，可选）：流式开关
  - `stream_options: {include_usage: true}`（可选）：**流式时返回 usage，默认 false 不返回**
  - `temperature` / `top_p` / `max_tokens` / `max_completion_tokens`
  - `tools` / `tool_choice`（function calling）
  - `response_format`（JSON mode）
  - `logprobs` / `n` / `stop` / `seed`
  - `reasoning_effort`（仅 reasoning 模型 o1/o3/o4）
- **流式**：SSE，`data: {chunk}\n\n`，末尾 `data: [DONE]`
- **上下文机制**：**无状态**，完整 `messages` 数组每次传，无 previous_id

### 响应
- **成功 body**：`{id, object:"chat.completion", created, model, choices:[{index, message:{role,content,tool_calls}, finish_reason}], usage:{prompt_tokens, completion_tokens, total_tokens}}`
- **流式 events**：`chat.completion.chunk`（每 chunk）；usage 在末个 chunk（仅 `include_usage:true` 时）
- **usage 位置**：非流式在 body `usage`；流式在末个 chunk `usage`（需 `include_usage:true`）

### 失败
- **错误体**：`{"error":{"message","type","param","code"}}`
- **状态码**：401（鉴权）/ 400（请求错）/ 404（模型不存在）/ 429（限流）/ 500/503（上游）

### 模型差异
- **reasoning 模型**（o1/o3/o4-mini）：有 `reasoning_effort`（low/medium/high），无 `temperature`/`top_p`/`logprobs`，`max_completion_tokens` 替代 `max_tokens`
- **gpt-4o 系列**：支持 `response_format` JSON schema、vision（`content` 含 `image_url`）
- **gpt-4o-mini**：轻量，参数集同 gpt-4o
- 部分模型不支持 `logprobs` / `n`

### 上下文缓存
- OpenAI 自动 prompt caching（长 prompt 自动缓存，无需客户端配置），缓存命中 token 折扣计费

---

## 2. OpenAI Responses

### 官方 schema
- 同 [openai-openapi](https://github.com/openai/openai-openapi) 仓库，Responses 端点含完整 OpenAPI 规范。

### 请求
- **端点**：`POST /v1/responses`
- **Headers**：同 Chat Completions（`Authorization: Bearer`、`Content-Type`、可选 org/project）
- **Body 顶层字段**：
  - `model`（必需）
  - `input`（必需）：input items（**不同于 Chat Completions 的 messages**），可含文本/图片/文件/工具调用历史
  - `instructions`（可选，system 指令）
  - `previous_response_id`（可选）：**有状态上下文链接**，服务端存会话
  - `store`（bool，默认 true）：服务端是否存 response
  - `stream`（bool）
  - `tools` / `tool_choice`
  - `reasoning`（reasoning 模型，含 `effort`）
  - `temperature` / `max_output_tokens` / `top_p`
- **流式**：SSE，事件类型化（见下）
- **上下文机制**：**有状态**，`previous_response_id` 链式续接，服务端存会话历史

### 响应
- **成功 body**：`{id, object:"response", status, model, output:[{type, ...}], usage:{input_tokens, output_tokens, total_tokens}, previous_response_id}`
- **流式 events**：`response.created` / `response.in_progress` / `response.output_item.added` / `response.content_part.added` / `response.output_text.delta` / `response.output_text.done` / `response.completed`
- **usage 位置**：`response.completed` 事件的 `response.usage`（默认给）

### 失败
- 同 Chat Completions 错误体结构

### 模型差异
- **reasoning 模型**（o1/o3/o4-mini）：`reasoning.effort`，有 reasoning summary 事件
- **GPT-5 系列**：Responses 为主要接口
- 并非所有模型支持 Responses（gpt-4o 等也支持，但语义略异）

### 上下文缓存
- `store=true` 默认服务端存状态；`previous_response_id` 链式续接，无需重传历史

---

## 3. Gemini generateContent

### 官方 schema
- Google 通过 [Discovery API](https://ai.google.dev/api) 提供 API 规范（非标准 JSON Schema，但 Discovery 文档含字段定义）。无独立 $schema 文件，Discovery 文档为准。

### 请求
- **端点**：`POST /v1beta/models/{model}:generateContent`（model 在 path）
- **Headers**：
  - `x-goog-api-key: <key>`（或 URL `?key=<key>` query）
  - `Content-Type: application/json`
- **Body 顶层字段**：
  - `contents`（array，必需）：`[{role:"user"/"model", parts:[{text}|{inline_data}|{fileData}]}]`，完整上下文每次传
  - `systemInstruction`（可选）
  - `generationConfig`（可选）：`temperature`/`topP`/`maxOutputTokens`/`thinkingConfig`（2.5+）/`responseMimeType`/`responseSchema`
  - `safetySettings`（可选）
  - `tools`（function calling / Google Search / code execution）
- **流式**：**非流式接口**，无 stream 参数
- **上下文机制**：**无状态**，完整 `contents` 数组每次传

### 响应
- **成功 body**：`{candidates:[{content:{parts,role}, finishReason, index, safetyRatings}], promptFeedback, usageMetadata:{promptTokenCount, candidatesTokenCount, totalTokenCount, thoughtsTokenCount?}}`
- **usage 位置**：body `usageMetadata`

### 失败
- **错误体**：`{"error":{"code","message","status"}}`
- **状态码**：400/401/403/404/429/500

### 模型差异
- **Gemini 2.5 系列**：`thinkingConfig`（thinking budget），reasoning token
- **Gemini 2.0/1.5**：无 thinking
- **flash vs pro**：参数集同，能力差异
- **多模态**：`inline_data`（base64）/ `fileData`（文件引用）
- **图像生成模型**（gemini-3-pro-image）：响应含图像

### 上下文缓存
- Gemini explicit context caching：`cachedContent` 引用预缓存内容，缓存命中 token 折扣

---

## 4. Gemini streamGenerateContent

### 官方 schema
- 同 generateContent，Discovery 文档。

### 请求
- **端点**：`POST /v1beta/models/{model}:streamGenerateContent`（**独立接口**，与 generateContent 不同 path action）
- **`?alt=sse` 陷阱**：
  - 不带 `?alt=sse`：返回**分块 JSON 数组**（非标准 SSE，`[{chunk1},{chunk2},...]` 流式）
  - 带 `?alt=sse`：返回标准 SSE（`data: {chunk}\n\n`）
  - Google 官方 SDK 通常自动带 `?alt=sse`；裸 curl 不带会拿 JSON 数组流
- **Headers / Body**：同 generateContent
- **上下文机制**：无状态

### 响应
- **SSE 模式**（`alt=sse`）：`data: {candidate chunk}\n\n`，末个 chunk 含 `usageMetadata`
- **JSON 数组模式**：`[{chunk},...,{chunk with usageMetadata}]`，末对象含 usage
- **content-type 区分**：`text/event-stream`（SSE）vs `application/json`（JSON 数组）
- **usage 位置**：末个 chunk/对象的 `usageMetadata`

### 失败
- 同 generateContent

### 模型差异
- 同 generateContent

---

## 5. Gemini Interactions

### 官方 schema
- API reference 页 `/api/interactions-api`（overview 页未给完整 schema）。GA as of June 2026。schema 较新，可能随版本变。

### 请求
- **端点**：`interactions.create`（REST，具体 URL 在 API ref 页，overview 未显示；模型在 path 或 body）
- **Body 顶层字段**（overview 页确认）：
  - `previous_interaction_id`（可选）：**有状态上下文链接**，服务端存 Interaction 历史
  - `store`（bool，默认 true）：服务端是否存
  - `background`（bool）：长任务后台执行（与 `store=false` 互斥）
  - `tools` / `system_instruction` / `generation_config`（含 `thinking_level`/`temperature`）：**每轮须重新指定**，不跨 turn 保留
  - `user_input`（input）
- **状态性**：**有状态**（`store=true` 默认），服务端存 Interaction 对象（thoughts/tool calls/results/model_output/user_input 的时序序列）
- **上下文机制**：`previous_interaction_id` 链式续接，服务端取历史，只保留 inputs/outputs 跨 turn
- **流式**：overview 未明示（API ref 待查；推测 SSE）
- **数据留存**：Paid 55 天 / Free 1 天，可配 7/14/28/55 天，可删

### 响应
- **Interaction 对象**：时序执行步骤（thoughts / tool calls / tool results / model_output / user_input）
- **usage 位置**：末个 chunk（具体字段待 API ref 确认）

### 失败
- 同 Gemini 错误体（`{"error":{"code","message","status"}}`）

### 模型差异
- 支持 `gemini-3.5-flash` / `gemini-3.1-pro-preview` / `gemini-3-flash-preview` / `gemini-2.5-pro/flash/flash-lite` / 图像 / TTS / Gemma 系列
- 支持 agents：`deep-research-preview` / `antigravity-preview`
- **不支持**（vs generateContent）：`video_metadata` / Batch API / 自动 function calling / explicit caching / custom safety settings

### 上下文缓存
- `store=true` 服务端存 Interaction；`previous_interaction_id` 链式；非 generateContent 的 cachedContent 机制

---

## 6. Anthropic Messages

### 官方 schema
- Anthropic 文档（platform.claude.com/docs）含字段定义；无独立 $schema 文件，文档为准。

### 请求
- **端点**：`POST /v1/messages`
- **Headers**：
  - `x-api-key: <key>`（必需）
  - `anthropic-version: 2023-06-01`（必需）
  - `Content-Type: application/json`（必需）
  - `anthropic-beta`（可选，beta 功能）
- **Body 顶层字段**：
  - `model`（必需）
  - `messages`（array，必需）：`[{role:"user"/"assistant", content}]`，**完整上下文每次传**
  - `max_tokens`（必需）
  - `system`（可选，system prompt，可含 cache_control）
  - `stream`（bool）
  - `tools` / `tool_choice`
  - `thinking: {type:"enabled", budget_tokens}`（extended thinking 模型）
  - `temperature` / `top_p` / `stop_sequences`
- **流式**：SSE，事件类型化
- **上下文机制**：**无状态**，完整 `messages` + `system` 每次传，**无 previous_id 等价物**

### 响应
- **成功 body**：`{id, type:"message", role:"assistant", content:[{type:"text"/"tool_use", ...}], model, stop_reason, stop_sequence, usage:{input_tokens, output_tokens, cache_creation_input_tokens?, cache_read_input_tokens?}}`
- **流式 events**：`message_start`（含 input usage）/ `content_block_start` / `content_block_delta` / `content_block_stop` / `message_delta`（含 output usage + stop_reason）/ `message_stop`
- **usage 位置**：**两处**--`message_start` 的 input usage + `message_delta` 的 output usage（流式需合并）

### 失败
- **错误体**：`{"type":"error","error":{"type","message"}}`
- **状态码**：400/401/403/404/413/429/500/529

### 模型差异
- **Claude 4.x / 3.5**：Opus/Sonnet/Haiku 层级
- **extended thinking 模型**：`thinking` 字段 + `budget_tokens`，thinking token 计费
- **不同 max output**：Opus > Sonnet > Haiku
- **prompt caching**：`cache_control` 在 content blocks / system 上，显式客户端提示缓存

### 上下文缓存
- Anthropic **explicit prompt caching**：客户端在 content block 加 `cache_control: {type:"ephemeral"}`，服务端缓存命中 `cache_read_input_tokens` 折扣计费。非服务端状态化，是客户端提示的缓存。

---

## 7. 对内核/控制面设计的影响

### 协议识别（Rust 内核，宪法 VI）
- Gemini 靠 path action 区分流式：`:streamGenerateContent` vs `:generateContent`（`streaming_by_path` 标记）
- OpenAI/Anthropic 靠 body `stream:true` 区分
- 001 已正确处理，M0 移植复用

### usage 提取（Rust 内核，只提取现成 usage）
| 接口 | Rust 提取方式 |
|---|---|
| OpenAI Chat Completions 流式 | 需 Rust 转换引擎注入 `stream_options.include_usage:true`，再提取末 chunk usage |
| OpenAI Responses 流式 | 提取 `response.completed` 事件 usage |
| Gemini generateContent | 提取 body `usageMetadata` |
| Gemini streamGenerateContent | 提取末 chunk `usageMetadata`（区分 SSE vs JSON 数组） |
| Gemini Interactions | 提取末 chunk usage（字段待 API ref 确认） |
| Anthropic Messages 流式 | 合并 `message_start`（input）+ `message_delta`（output） |

### 上下文桥（Go，§3B/L3.5）
- **有状态协议**（OpenAI Responses 的 `previous_response_id` / Gemini Interactions 的 `previous_interaction_id`）：经 Go materialize
  - 同态透传（Responses->Responses）：只存 id 映射
  - 跨协议转化（Responses->Chat Completions）：存完整 ConversationTimeline，materialize 成完整 messages
- **无状态协议**（Chat Completions / Messages / generateContent）：直入 Rust 透传，不经 Go

### marketplace schema 转换（Go+React，§3B）
- 模型 schema 经常变（Gemini Interactions 尤新），marketplace 存转换规则
- Rust 无状态转换引擎 + Go materialize 都按 marketplace 规则执行
- 声明式 TransformPlan，Rust 编译执行（宪法 VI）

---

## 8. 待补充

- Gemini Interactions 完整 body schema（API ref 页 `/api/interactions-api`，overview 不全）
- Gemini Interactions 流式机制（overview 未明示）
- 各接口官方 $schema 文件精确 URL（OpenAPI/Discovery 已确认存在，精确链接实现期核对）
- marketplace schema 转换规则的具体格式（L6 spec 时定）
