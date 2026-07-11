# Contract: Provider 透传与镜像错误

**Feature**: 001-gateway-foundation | 公共数据面对外契约（FR-001~008、FR-035~038）。SDK 仅改 base URL / token / 模型名即可工作（SC-001）。

## 入口与协议识别（FR-001/FR-006）

公共入口保持 SDK 友好的 provider-native / compatibility-native 路径，**不**用 `/openai/...` 前缀做主路径。识别依据（见 research R3）：

| 协议 | 触发特征 | 模型来源 |
| ---- | ---- | ---- |
| OpenAI-compatible Chat Completions | `POST /v1/chat/completions` | body `model` |
| Anthropic Messages | `POST /v1/messages`（`x-api-key` + `anthropic-version`） | body `model` |
| Gemini generateContent | `POST /v1beta/models/{model}:generateContent`（及 `:streamGenerateContent`） | path `{model}` |

识别失败（协议不可知）→ 回退 PingoGate 原生错误体（FR-036）。

## 透传行为（FR-002~005, FR-008）

1. 客户端持**网关 Key** 鉴权；网关 Key 在转发前被剥离，由核心注入上游认证（FR-010/FR-011/FR-012）。
2. 非流式：请求 body 原样转发，响应 body/headers 原样回传。
3. 流式（`stream:true` / `:streamGenerateContent`）：SSE 事件按上游顺序逐块透传至流结束，网关不缓冲整流、不重组事件（FR-005、research R2）。
4. 上游成功响应与上游错误（4xx/5xx）**原样透传**，不改写状态码与错误体（FR-008/FR-037）。

## 上游认证注入矩阵（FR-012）

| Provider kind | 注入 |
| ---- | ---- |
| openai-compatible | `Authorization: Bearer <key>` |
| anthropic | `x-api-key: <key>` + `anthropic-version: <ver>` |
| gemini | URL query `?key=<key>` |

## 镜像错误体（FR-035/FR-036/FR-038）

网关**自身**产生的错误（鉴权失败、无路由、能力不支持、上游超时/不可用），在协议已识别时镜像目标 Provider 的原生错误体结构，使 SDK 错误解析无需改动（SC-002）：

| Provider | 镜像错误体形状（示意） |
| ---- | ---- |
| OpenAI-compatible | `{ "error": { "message": "...", "type": "...", "code": "..." } }` |
| Anthropic | `{ "type": "error", "error": { "type": "...", "message": "..." } }` |
| Gemini | `{ "error": { "code": <int>, "message": "...", "status": "..." } }` |
| 协议未识别 | PingoGate 原生：`{ "error": { "message": "...", "kind": "pingogate.<reason>" } }` |

错误场景 → 状态码语义：

| 场景 | 状态码语义 | FR |
| ---- | ---- | ---- |
| 缺失/无效网关 Key | 401（镜像形状） | FR-013 |
| 无匹配路由 / 上游不可用 | 404/502（镜像形状） | FR-016 |
| 能力族不支持 | 400/404 显式「不支持」 | FR-007/SC-010 |
| 上游超时（可配置，FR-038） | 类 504，记入指标 | FR-038 |

## 契约测试（先写并失败）

1. 三协议各一条：官方 SDK 仅改 base URL/token/模型名 → 非流式成功，响应与直连一致，且响应/日志不含上游 Provider Key（SC-001/SC-002）。
2. 三协议各一条流式：SSE 事件顺序透传至结束；中途断开记录断开原因（Edge Case）。
3. 无效网关 Key → 镜像错误体（按目标协议形状）+ 未触达上游（FR-013）。
4. 未声明模型别名 → 「无路由」镜像错误（FR-016）。
5. 请求不支持能力族（如 Responses/Realtime/Batch）→ 显式「不支持」错误（FR-007）。
6. 上游错误（构造 4xx/5xx）→ 原样透传，状态码/错误体不被改写（FR-008/FR-037）。
7. 上游超时（配置短 timeout + 慢上游桩）→ 类 504 镜像错误 + 指标 +1（FR-038）。
8. 协议未识别请求 → PingoGate 原生错误体（FR-036）。
