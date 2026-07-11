# Phase 0 Research: PingoGate 网关地基

**Feature**: 001-gateway-foundation | **Date**: 2026-06-29

本文件解决 plan 的 Technical Context 中的未知项与关键技术取舍。每条记录决策、理由与被否方案。Pingora 相关结论以 context7 查得的 `pingora-proxy` 0.8.0 文档为锚（实现期须按宪法 XVII 再次 `context7` 核对当前版本 API）。

---

## R1. 请求管线如何映射到 Pingora `ProxyHttp`

**Decision**：实现单个 `ProxyHttp`（在 `pipeline` crate），宪法 VI 管线阶段映射到其 filter 回调：

| 管线阶段（宪法 VI） | Pingora 回调 | 职责 |
| ---- | ---- | ---- |
| 入口 + 协议识别 | `request_filter` | 由路径/方法/头识别目标协议（openai-compatible / anthropic / gemini）写入 `CTX` |
| 鉴权授权 | `request_filter` | 校验网关 Key → `Principal`；失败时短路返回镜像错误 |
| 能力族判定 | `request_filter` | 不支持的能力族（Responses/Realtime/Batch…）短路返回显式「不支持」 |
| 路由 | `upstream_peer` | 模型别名 → Provider/上游，构造 `HttpPeer`（含上游 TLS） |
| 请求转换（仅认证注入） | `upstream_request_filter` | 注入上游认证、剥离网关 Key；本阶段不改 body |
| 上游派发 | Pingora 默认代理 | —— |
| 响应转换/透传 | `response_filter` + `upstream_response_body_filter` | 头透传；body 流式逐块转发（SSE 不缓冲） |
| 可观测性 | `logging` | trace id、指标、延迟、状态码、token 计数 |
| 错误 | `fail_to_proxy` / `error_while_proxy` / `fail_to_connect` | 上游超时/不可用 → 镜像目标 Provider 错误体 |

**Rationale**：`request_filter` 返回 `Ok(true)` 即可在不连上游的情况下向下游写响应，天然承载 FR-013/FR-007/FR-016 的短路错误；`upstream_request_filter` 是注入上游认证的唯一位置，使 adapter 不接触 body 与连接，满足宪法 VI「adapter 不拥有网关生命周期」与 spec FR-011。

**Alternatives considered**：自建 hyper/axum 代理——被否，违反宪法 III「Pingora MUST 用于热路径」。在 `upstream_peer` 里做鉴权——被否，鉴权应在更早的 `request_filter` 以便短路、避免无谓建连。

---

## R2. SSE 流式透传如何零拷贝、不缓冲

**Decision**：流式响应经 `upstream_response_body_filter` / `response_body_filter` 逐 `Bytes` 块原样转发，不在网关侧聚合整流；不解析 SSE 事件语义，仅透传字节并保持顺序直至上游流结束（FR-005）。`CTX` 标记该响应为流式（依据上游 `content-type: text/event-stream` 或请求 `stream:true`）用于观测，但不改变转发逻辑。

**Rationale**：满足宪法 XXI 零拷贝/无阻塞与 SC-003 延迟预算；body filter 是 Pingora 暴露的逐块钩子，按块返回即流式。

**Alternatives considered**：缓冲整个响应再转发——被否，破坏 SSE 增量语义与延迟预算。网关侧重组 SSE 事件——被否，本阶段是原生透传，无需理解事件（事件桥接属 Out of Scope 的 Realtime/Tools）。

---

## R3. 协议识别策略

**Decision**：在 `request_filter` 按「请求特征」识别协议，不把 `/openai/...` 之类前缀作为主路径（FR-006）：
- **Anthropic Messages**：`POST /v1/messages`，常带 `x-api-key` + `anthropic-version`。
- **Gemini generateContent**：路径含 `:generateContent` / `:streamGenerateContent`（`/v1beta/models/{model}:generateContent`）。
- **OpenAI-compatible Chat Completions**：`POST /v1/chat/completions`。
识别结果与「兼容表面 vs 原生」标记写入 `RequestContext`；无法识别 → 在协议确定前失败，回退 PingoGate 原生错误体（FR-036）。模型名从 body（OpenAI/Anthropic 的 `model`）或路径（Gemini 的 `{model}`）提取，用于路由。

**Rationale**：基于各 Provider 官方 SDK 实际使用的 path/header，保证 SDK 仅改 base URL/token/模型名即可（SC-001）。

**Alternatives considered**：统一前缀路由（`/openai`、`/anthropic`）——被否，违反 FR-006 SDK 友好原则。仅凭 body 探测——被否，Gemini 模型在路径中、且需尽早识别以选错误体形状。

---

## R4. 网关自身错误如何镜像目标 Provider 错误体

**Decision**：在 `core` 定义 `GatewayError`（鉴权失败/无路由/能力不支持/上游超时/上游不可用等）与「错误体渲染器」；每个 provider adapter 声明其原生错误体形状（OpenAI `{"error":{message,type,code}}`、Anthropic `{"type":"error","error":{type,message}}`、Gemini `{"error":{code,message,status}}`）。管线在已识别协议时用对应渲染器输出镜像错误（FR-035），未识别协议时输出 PingoGate 原生错误体（FR-036）；上游自身返回的错误原样透传，不改写（FR-008/FR-037）。上游超时采用类 504 语义并记入指标（FR-038）。

**Rationale**：现有 SDK 的错误解析逻辑无需改动即可工作（SC-002/FR-035）。把错误体形状归到 adapter，符合宪法 XI「adapter 拥有 Provider 特定转换」。

**Alternatives considered**：统一 PingoGate 错误体——被否，SDK 解析会失败。在每个 endpoint handler 内联错误 JSON——被否，违反宪法 IX「HTTP 映射只在边界」与 DRY。

---

## R5. 上游认证注入

**Decision**：认证由核心在 `upstream_request_filter` 注入，adapter 只声明「认证方式描述符」，不接触明文 Key（FR-011/FR-012、宪法 XX）：
- OpenAI-compatible：`Authorization: Bearer <key>`
- Anthropic：`x-api-key: <key>` + `anthropic-version: <ver>`
- Gemini：URL query `?key=<key>`
明文网关 Key 在注入上游认证前从上游请求剥离。Provider Key 来自配置中的**密钥引用**（本阶段 `env:VAR_NAME`），由 `storage` 的解析器在 snapshot 构建时解析并以受保护形式驻留，永不进日志/指标（FR-022/FR-034）。

**Rationale**：单点注入符合 architecture-discussion 的认证设计与宪法 X/XX。

**Alternatives considered**：adapter 自行注入——被否，会让 adapter 接触 Key，违反 spec FR-011。Azure `api-key`、AWS SigV4、GCP Vertex OAuth——本阶段范围外（spec Out of Scope），认证描述符设计保留扩展位但不实现。

---

## R6. 配置热重载与 `RuntimeSnapshot`

**Decision**：`config` crate 持有 `Arc<ArcSwap<RuntimeSnapshot>>`；`ProxyHttp` 每请求 `.load()` 取当前 snapshot 并在 `CTX` 绑定整个请求生命周期（FR-021/FR-030）。重载流程（FR-019、宪法 XII）：加载候选 `pingogate.yaml` → serde schema 校验 → 语义校验（路由引用的 Provider 存在、密钥引用可解析）→ 构建不可变 `RuntimeSnapshot` → `ArcSwap::store` 原子切换 → 记录版本与回滚目标。校验失败在切换前返回错误，活跃 snapshot 不变（FR-020）。触发源：`SIGHUP`（tokio signal 任务）、Admin API reload、`notify` 文件监听（FR-018）。并发重载用串行化（mutex/单 reload 任务）保证最终一致（Edge Case）。

**Rationale**：ArcSwap 是宪法 X/XII 明示的快照原子切换机制；每请求 load + 绑定保证在途请求零中断（SC-004）。

**Alternatives considered**：`RwLock<Config>`——被否，写锁会阻塞热路径读、违反宪法 XXI。进程级重启重载（Pingora 零停机升级）——被否，过重，且不满足「在途请求继续用旧 snapshot」的同进程语义。

---

## R7. Admin API 承载方式

**Decision**：Admin API 用 Pingora 的 `ServeHttp` 简单 HTTP 服务承载于**独立 admin listener**（与公共代理端口分离），不引入 axum/独立 HTTP 框架。端点：`GET /healthz`、`GET /readyz`、`POST /config/validate`、`POST /reload`、`GET /reload/status`。每个请求经 `admin` crate 的 `Principal`/`AuthContext`/`authorize(action, resource)` 边界鉴权，bootstrap 模式用静态管理员凭据但仍走同一边界（FR-024/FR-025、宪法 XX）。

**Rationale**：Admin API 非热路径，但用 Pingora 自带 `ServeHttp` 可避免新增 HTTP 框架依赖（宪法 III 依赖最小化）；独立 listener 隔离管理面与数据面（宪法 X）。

**Alternatives considered**：与公共入口共用端口按路径分流——被否，混淆数据面/管理面、增大误暴露风险。引入 axum——被否，额外重依赖，`ServeHttp` 已够。

---

## R8. 吞吐/并发目标（spec 遗留 Outstanding 项）

**Decision**：本阶段不设契约性吞吐 SC，仅设**初始基准目标**供 plan 后基准验证：4 核节点 ≥ 2000 req/s 非流式透传、≥ 1000 并发 SSE 长连接，网关 CPU 不先于上游成为瓶颈。契约性的是 SC-003 延迟预算。吞吐基准在 `pingogate` 集成测试/bench 中度量，回归阻止合入（宪法 XXI）。

**Rationale**：spec clarify 阶段将该项延后到 plan；透传网关的吞吐主要受上游与连接池约束，设硬性 SC 易失真，故定为可修订基准而非验收门槛。

**Alternatives considered**：写成 SC-011 硬指标——被否，缺真实硬件基线，违反 SC「可验证」要求。完全不提——被否，宪法 XXI 要求每功能定义资源预算。

---

## R9. 可观测性落地

**Decision**：`tracing` 输出结构化日志，每请求在 `request_filter` 生成并贯穿管线的 trace/请求 ID（FR-031）；Prometheus 指标端点（独立或 admin listener 暴露）覆盖请求量、状态码、延迟、上游延迟、token 计数（输入/输出 + 上游可提供时 reasoning/cache），按 Provider + 能力族打标签，MAY 按网关 Key principal id 归因（FR-032/FR-033）。密钥/token 经统一脱敏层后才输出（FR-034）。OpenTelemetry 作为可选 exporter 预留。

**Rationale**：对齐宪法 XIX 与 spec US4；标签维度与宪法 XIX「必需观测信号」一致。

**Alternatives considered**：仅日志无指标——被否，违反 SC-008/宪法 XIX。

---

## 未决项归零确认

spec 中无残留 `[NEEDS CLARIFICATION]`；clarify 阶段唯一 Outstanding 项（吞吐/并发）已由 R8 解决为初始基准目标。Phase 0 完成。
