# Phase 1 Data Model: PingoGate 网关地基

**Feature**: 001-gateway-foundation | **Date**: 2026-06-29

本文件从 spec 的 Key Entities 与 FR 推导本阶段的领域类型、字段、关系、校验规则与状态流转。命名为 Rust 视角的概念模型（实现期标识符用英文，宪法 XXII），不锁定具体字段类型细节。所有类型归属见 [plan.md](./plan.md) 的 crate 划分。

## 实体关系总览

```text
pingogate.yaml ──(parse+validate)──► RuntimeSnapshot (immutable, versioned)
                                          │
        ┌─────────────┬──────────────────┼───────────────┬──────────────┐
        ▼             ▼                   ▼               ▼              ▼
  ListenerSet   GatewayKey[]        Provider[]      ModelRoute[]   ObservabilityCfg
                  (principal)      (+capability      (alias→        (labels)
                                    family, auth)     provider)
RequestContext ──binds──► RuntimeSnapshot (per-request, whole lifecycle)
   │
   ├─ identified ProtocolKind
   ├─ resolved Principal (from GatewayKey)
   └─ routing decision (ModelRoute → Provider)

AuthContext { Principal } ──authorize(action,resource)──► Admin 操作
ReloadStatus { version, result, rollback_target }  ◄── Control Plane State
```

## 核心实体

### RuntimeSnapshot（运行时快照）

由配置构建的**不可变**运行时视图，热路径只读、经 `ArcSwap` 原子切换（FR-019/FR-030、宪法 X/XII）。

| 字段 | 说明 | 来源/校验 |
| ---- | ---- | ---- |
| `version` | 单调递增或内容哈希版本标识 | 构建时生成 |
| `listeners` | 公共入口 + admin 入口监听配置（地址、TLS） | `pingogate.yaml` |
| `gateway_keys` | 具名网关 Key 集合（见 GatewayKey） | 配置；至少 0 个（空则全部请求被拒） |
| `providers` | 已声明上游集合 | 配置；类型 ∈ {openai-compatible, anthropic, gemini} |
| `routes` | 模型别名 → Provider/上游模型 映射 | 配置；每条 `provider` MUST 存在于 `providers` |
| `observability` | 指标/日志配置（标签开关） | 配置 |
| `rollback_target` | 上一个有效 snapshot 的版本引用 | 重载时记录 |

**不变式**：构建成功后字段不可变；任何变更产生新 snapshot。校验失败不产生 snapshot（FR-020）。

### GatewayKey（网关 Key）

客户端访问 PingoGate 的凭据；本阶段为配置中声明的**一组具名 Key**，每个是可识别主体（FR-009）。

| 字段 | 说明 | 校验 |
| ---- | ---- | ---- |
| `name` / `id` | 主体标识（principal id），用于日志/指标归因 | 唯一、非空 |
| `secret_ref` | Key 值或其引用（比对用） | 非空；明文 MUST NOT 出现在日志/指标（FR-034） |
| `enabled` | 是否启用 | 默认 true |

**关系**：鉴权成功 → 映射为 `Principal`。与 `ProviderKey` 严格隔离（FR-010）。

### ProviderKey（上游密钥）

PingoGate 访问上游的凭据；以**引用**声明，明文不外泄（FR-011/FR-022、宪法 XX）。

| 字段 | 说明 | 校验 |
| ---- | ---- | ---- |
| `reference` | 密钥引用，本阶段 `env:VAR_NAME` | 引用 MUST 在 snapshot 构建时可解析，否则校验失败（Edge Case） |
| `resolved` | 解析后的受保护值（驻内存，不序列化、不日志） | 仅内部使用 |

### Provider（上游声明）

| 字段 | 说明 | 校验 |
| ---- | ---- | ---- |
| `name` | Provider 实例名（路由引用目标） | 唯一、非空 |
| `kind` | `openai-compatible` / `anthropic` / `gemini` | 枚举 |
| `base_url` | 上游基础 URL | 合法 URL；https 默认校验 TLS |
| `auth` | 认证方式描述符（见 AuthMethod） + `key`(ProviderKey) | 与 `kind` 匹配 |
| `capability_families` | 声明支持的能力族 | 本阶段固定含 `generation.stateless`（FR-029、宪法 XI） |
| `anthropic_version` | 仅 anthropic：版本头值 | kind=anthropic 时必填 |

### AuthMethod（认证方式描述符）

不含明文 Key，仅描述如何注入（R5、FR-012）。

| 变体 | 注入位置 |
| ---- | ---- |
| `BearerToken` | `Authorization: Bearer <key>` |
| `ApiKeyHeader { header, version_header? }` | `x-api-key: <key>` + `anthropic-version` |
| `QueryKey { param }` | URL query `?key=<key>` |

> 扩展位：Azure `api-key`、AWS SigV4、GCP Vertex OAuth 不在本阶段实现（spec Out of Scope）。

### ModelRoute（模型路由 / 别名）

| 字段 | 说明 | 校验 |
| ---- | ---- | ---- |
| `alias` | 客户端可见模型名 | 唯一、非空 |
| `provider` | 目标 Provider name | MUST 存在于 `providers`（语义校验） |
| `upstream_model` | 上游真实模型名 | 非空 |

**关系**：路由查找无匹配 → 返回明确「无可用路由」错误（FR-016、镜像错误体）。

### CapabilityFamily（能力族）

按能力而非 endpoint 建模（宪法 XI）。本阶段枚举仅 `generation.stateless` 有效；其余族（`generation.stateful`/`realtime.live`/`embedding`/`batch`/…）已知但**显式不支持**，命中即返回「不支持」错误（FR-007/FR-029/SC-010）。用于路由判定、指标标签、不支持判定。

### ProtocolKind（协议类型）

`request_filter` 识别结果（R3）：`OpenAiCompatibleChat` / `AnthropicMessages` / `GeminiGenerateContent`（含 streaming 变体标记）/ `Unidentified`。决定路由解析、错误体形状与 SSE 标记。

### Principal / AuthContext

管理与归因的身份边界（FR-024、宪法 XX）。

| 类型 | 字段 | 说明 |
| ---- | ---- | ---- |
| `Principal` | `id`, `kind`(gateway-key / bootstrap-admin), `display_name` | 鉴权产物 |
| `AuthContext` | `principal`, ... | 承载 `authorize(action, resource)` 判定的上下文 |

**规则**：每个 Admin 请求构造 `AuthContext` 并经 `authorize` 判定；禁止「全局 admin token 等值判断」式硬编码（FR-025）。

### RequestContext（请求上下文）

贯穿管线的请求级状态（Pingora `CTX`），向前流动不回退（宪法 VI）。

| 字段 | 说明 |
| ---- | ---- |
| `trace_id` | 贯穿管线的请求/追踪 ID（FR-031） |
| `snapshot` | 本请求绑定的 `RuntimeSnapshot`（Arc 克隆，FR-021） |
| `protocol` | 识别出的 `ProtocolKind` |
| `principal` | 鉴权得到的 `Principal` |
| `route_decision` | 命中的 `ModelRoute` / Provider |
| `is_streaming` | 是否 SSE 流式（观测用） |
| `metrics_labels` | provider、capability family、(opt) principal id |

### ReloadStatus（重载状态）— Control Plane State

| 字段 | 说明 |
| ---- | ---- |
| `last_result` | success / rejected(原因) |
| `active_version` | 当前生效 snapshot 版本 |
| `rollback_target` | 回滚目标版本 |
| `timestamp` | 最近一次重载时间 |

由 Admin API `reload status` 查询（FR-027）。驻内存（file+memory），重载产生但**不被运行时配置重载所改写其语义**（宪法 X 分离规则）。

## 状态流转

### 配置重载状态机（FR-019/FR-020）

```text
Active(snapshot_v) 
   │  reload trigger (SIGHUP / admin / file-watch)
   ▼
Loading candidate ──► Schema validate ──fail──► Rejected (Active 不变, 记录原因)
   │ ok                                            ▲
   ▼                                               │
Semantic validate ──fail───────────────────────────┘
   │ ok
   ▼
Build immutable snapshot_v+1
   │
   ▼
ArcSwap::store(v+1)  ──► Active(snapshot_v+1)  (记录 version + rollback_target=v)
```

并发重载：经串行化（单 reload 任务/互斥），后到者基于最新 Active 重新校验（Edge Case「并发重载」）。

### 请求生命周期状态（宪法 VI 管线）

```text
Recv → IdentifyProtocol → AuthGatewayKey ─fail→ MirrorError(短路)
  → CapabilityCheck ─unsupported→ MirrorError(短路)
  → Route ─no match→ MirrorError(短路)
  → InjectUpstreamAuth → Dispatch
       ├─ upstream ok → StreamResponse(透传/SSE) → Log/Metrics
       └─ upstream timeout/unavailable → MirrorError(类504) → Log/Metrics
```

## 校验规则汇总

- 路由的 `provider` 必须存在；密钥引用必须可解析——否则重载/校验在切换前失败（FR-016/FR-022、Edge Cases）。
- `kind=anthropic` 必须有 `anthropic_version`。
- 网关 Key `name` 与 Provider `name` 与路由 `alias` 各自唯一。
- 任何含明文密钥的字段 MUST NOT 进入序列化输出、日志、指标（FR-034、宪法 XX）。
