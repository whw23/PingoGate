# Provider Schema 调研补充要点

> 调研五接口时必须特别确认/强调的点（workflow 调研结果回来后逐项核对，缺失则补充）。

## 1. 流式接口形态差异（关键）

**只有 Gemini 把流式与非流式拆成两个独立接口（不同 path action）；OpenAI/Anthropic 都是单接口 + body `stream:true` 切换。**

| Provider | 接口 | 流式与非流式 | 流式触发 |
|---|---|---|---|
| OpenAI Chat Completions | `POST /v1/chat/completions` | 同一接口 | body `"stream": true` |
| OpenAI Responses | `POST /v1/responses` | 同一接口 | body `"stream": true` |
| Anthropic Messages | `POST /v1/messages` | 同一接口 | body `"stream": true` |
| Gemini generateContent | `POST /v1beta/models/{model}:generateContent` | 独立接口（非流式） | - |
| Gemini streamGenerateContent | `POST /v1beta/models/{model}:streamGenerateContent` | 独立接口（流式） | 本身就是流式 |
| Gemini Interactions | `:interact` 等 | 独立（状态化） | 另算 |

**影响**：
- 协议识别（宪法 VI）：Gemini 靠 path 的 `:streamGenerateContent` vs `:generateContent` 判定流式；OpenAI/Anthropic 靠 body `stream` 字段。001 已正确处理（`streaming_by_path` 标记），M0 移植复用。
- 路由：Gemini 的两个接口是同一模型的不同 action，路由层要识别 `:action` 后缀统一路由到该模型。

## 2. Gemini `?alt=sse` 陷阱

Gemini `:streamGenerateContent` 默认返回**分块 JSON 数组**（非标准 SSE），只有加 `?alt=sse` 才返回标准 `text/event-stream`（`data:` 行）。

- Google 官方 SDK 通常自动带 `?alt=sse`。
- 裸 curl 不带会拿到 JSON 数组流。
- Rust 内核透传：M0 不解析（透传字节），但 L5 提取 usage 要区分两种格式（usage 在末块 `usageMetadata`，但 chunk 格式不同）。
- content-type 区分：`text/event-stream`（SSE）vs `application/json`（JSON 数组流）。

## 3. usage 位置差异（影响 L5 Rust 提取）

| Provider/接口 | usage 位置 | 流式如何拿 | 是否默认给 |
|---|---|---|---|
| OpenAI Chat Completions（流式） | 末个 chunk `usage` | **需 `stream_options.include_usage:true`** | ❌ 默认不给 |
| OpenAI Responses（流式） | `response.completed` 事件 | 默认 | ✅ |
| Anthropic Messages（流式） | `message_start`（input）+ `message_delta`（output） | 默认，分两处 | ✅ |
| Gemini generateContent（非流式） | body `usageMetadata` | 直接取 | ✅ |
| Gemini streamGenerateContent（SSE） | 末个 SSE chunk `usageMetadata` | 末块取 | ✅ |
| Gemini streamGenerateContent（JSON 数组） | 末个 JSON 对象 `usageMetadata` | 末对象取 | ✅ |

**关键**：OpenAI Chat Completions 流式默认不给 usage，需 Rust 转换引擎注入 `stream_options.include_usage:true`（无状态 body 改写，宪法 VI 归 Rust）。其他 provider 默认给。

## 4. 上下文机制差异（影响 Go 上下文桥）

| Provider/接口 | 上下文机制 | 状态性 |
|---|---|---|
| OpenAI Chat Completions | 完整 messages 数组 | 无状态 |
| OpenAI Responses | `previous_response_id`（服务端存会话） | **有状态** |
| Anthropic Messages | 完整 messages + system | 无状态 |
| Gemini generateContent | 完整 contents 数组 | 无状态 |
| Gemini Interactions | `previous_interaction_id`（服务端存状态） | **有状态** |

**影响**：只有 OpenAI Responses 和 Gemini Interactions 是有状态协议（带 previous_id），需经 Go 上下文桥 materialize。其他三个无状态，直入 Rust 透传。

## 5. 待 workflow 核对的项

workflow 调研结果回来后，逐个接口核对其 `streaming` / `usage_field` / `context_mechanism` 字段是否覆盖上述差异。缺失则补充到最终调研文档。
