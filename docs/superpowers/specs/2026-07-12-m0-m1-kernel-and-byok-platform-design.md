# M0+M1 内核与 BYOK 平台 设计（Design）

> **Feature**：M0+M1 内核透传 + BYOK 用户 key 管理平台（合并 spec，多 plan 分期实现）
> **日期**：2026-07-12
> **状态**：Draft（待 review）
> **路线图定位**：`docs/01-platform-roadmap.md` 里程碑 M0（内核单机网关）+ M1（BYOK 密钥平台）合并
> **上位约束**：`.claude/rules/constitution.md`（双语言内核架构总纲 + I~XXII）
> **调研依据**：`docs/02-provider-schemas.md`（五接口 schema + 流式/上下文/usage 差异）

## 1. 目标与范围

### 1.1 目标

交付 PingoGate 的**首个完整闭环**：用户能在平台上管理自己的 BYOK Provider Key（加密保管，管理员不可见明文），并用虚拟 Key 经 Rust 内核透传调用上游模型。即"能存 key 也能用 key"。

合并 M0（内核单机网关）+ M1（BYOK 密钥平台）：Rust 内核做六接口同态透传，Go 控制面做用户身份 + BYOK key 管理，两者经 gRPC 协作（Go 推快照 / Go 调 Rust KeyVault 加解密 / Rust 推 usage 给 Go）。

本 spec 覆盖完整设计；实现分三个 plan（S1/S2/S3，见 §11），每个 plan 独立可验证。

### 1.2 包含（In Scope）

**Rust 内核（`core-rs/`，`pingogate-core` 二进制）**：
- Cargo workspace 搭建，按宪法 IV 重组 crate（`core` / `storage` / `snapshot` / `pipeline` / `router` / `transform` / `provider` / `listener` / `keyvault` / `pingogate-core`）。
- Pingora 数据面管线：协议识别 -> Key 鉴权 -> 模型路由 -> 上游认证注入 -> 透传（含 SSE 零拷贝）-> 错误镜像 -> 可观测。
- **六接口同态透传**：OpenAI Chat Completions / OpenAI Responses / Gemini generateContent / Gemini streamGenerateContent / Gemini Interactions / Anthropic Messages，含 SSE。同态透传：客户端协议 = 上游协议，原样转发，`previous_response_id` / `previous_interaction_id` 原样转发不处理，PingoGate 不存会话状态。
- **协议识别按路径结构后缀匹配，不硬编码版本前缀**（`{ver}` provider 特定且可变，见 `docs/02-provider-schemas.md` §0）：识别 `/chat/completions` / `/responses` / `:generateContent` / `:streamGenerateContent` / `interactions` / `/messages` 后缀模式，版本前缀通配。
- **KeyVault**（安全核心）：Provider Key 加解密唯一入口，AES-GCM，主密钥 `env:PINGO_MKEK`。明文只在 `KeyVault::decrypt()` 返回的受保护 `SecretString` 中存活，MUST NOT 进日志 / 指标 / 任何对外输出。Go 非内核只接触密文。
- `RuntimeSnapshot` + `ArcSwap` 不可变快照热重载（SIGHUP / 文件监视 / Admin API / Go gRPC 推送四触发同一编排器）。
- `SnapshotSource` trait：file 实现（单机模式）+ gRPC 实现（平台模式，接收 Go 推送）。
- `KeyAuth` trait：`StaticKeyAuth`（单机模式静态 key）+ `VirtualKeyAuth`（平台模式虚拟 key）。
- **用量提取**：只提取 provider 响应中现成的 usage 字段（OpenAI `include_usage` / Anthropic `message_delta` / Gemini `usageMetadata`），增量记录 + 终态对账，推 Go 落库（平台模式）或记 log/指标（单机模式）。**Rust 不带 tokenizer、不做估算、零 DB**。
- 最小管理端点：`/healthz` / `/readyz` / `/metrics` / `/reload` / `/reload/status` / `/config/validate`，经 `Principal` / `authorize` 边界鉴权。
- 可观测性：tracing 结构化日志 + trace id；Prometheus 指标（按 provider + 能力族打标签）；密钥脱敏。

**Go 非内核（`ctrl-go/`，`pingogate-ctrl` 二进制）**：
- Go module 搭建，按宪法 IV 组织（`cmd/pingogate-ctrl` + `internal/{identity,keymgmt,vkey,snapshot,usage,admin,storage,proto}`）。
- **用户身份（L0）**：User 实体（扁平，无 Tenant/Project，多租户后置 L4）+ bootstrap admin + `Principal`/`authorize` 边界。OAuth 后置（L6），M0+M1 用 bootstrap 静态 admin + 用户 CRUD。
- **BYOK Provider Key 管理（L1）**：`UserProviderKey` 实体（owner_user_id / provider_type / encrypted_key / base_url / created_by / created_at / last_used_at）。用户录入 key -> Go 转发 Rust KeyVault 加密 -> Go 存 DB（密文 + 元数据）。
- **1Password 可见性**：明文可见性绑定 `created_by`，与 RBAC 正交。只有 `created_by` 能查看明文（Go 校验 created_by 后调 Rust KeyVault 解密，一次性返回），其余任何人（任何角色 / admin）只见末位 + 元数据。BYOK key 的 `created_by` = owner user，故管理员不可见用户明文 key。
- **虚拟 Key（L2）**：`VirtualKey` 实体（owner / scope: provider/model/IP/过期/并发配额 / 哈希存 / 状态机 active/revoked）。Go 签发（明文返用户一次 + 哈希存 DB）/ 撤销 / 列表，推快照给 Rust 热路径校验。
- **gRPC 通信 + proto**：`proto/` 定义跨语言类型（User / VirtualKey / UserProviderKey 密文 / 快照 / KeyVault 加解密请求响应）。Go -> Rust 快照推送（gRPC stream）；Go -> Rust KeyVault 加解密（gRPC unary）；Rust -> Go usage 事件推送（gRPC stream）。
- **控制面 API**：REST（net/http + chi）+ sqlx（SQLite 起步，Postgres 可切）。用户 CRUD / provider key CRUD / virtual key CRUD / 管理端点。
- **用量落库**：收 Rust 推来的 usage 事件（数字或 body），无 usage / 失败时用 tokenizer 估算（Go 带 tiktoken-go），聚合落 DB。
- 管理面：健康 / 就绪 / reload 编排（触发 Rust 热重载）。

### 1.3 不包含（Out of Scope，后置）

以下**完全不做**，后置到对应能力层。注意：usage 提取与落库、Go token 估算（无 usage/失败时）、虚拟 key、KeyVault 加密、1Password 可见性均**在 Scope**（见 §1.2），不在此列。

- **多租户 Tenant/Project/RBAC**（L4）：M0+M1 扁平 User，无 Tenant/Project 层级、无角色枚举、无组织 key 共享。组织 key（成员共享）后置 L4；M0+M1 只有 BYOK key（用户私有）。
- **OAuth/OIDC/SSO 登录**（L6）：M0+M1 用 bootstrap 静态 admin + API token 鉴权（用户/管理操作均走 authorize 边界），OAuth 第三方登录后置 L6。
- **计费 / 配额 / 限流**（L5/L6）：M0+M1 提取 usage 落库（In Scope），但**不做**计费（model ratio / 成本 / 账单）、配额扣减（预算扣减落账）、限流（运行时配额计数 + 超限拒绝）。这些后置 L5/L6（配额运行时计数在 Rust 内存，L5 引入）。
- **跨协议桥接 / 上下文虚拟化**（L3.5）：M0+M1 只做同态透传（客户端协议 = 上游协议），不做跨协议桥（Responses->Chat Completions 等）。`previous_response_id` / `previous_interaction_id` 原样转发，PingoGate 不查历史、不 materialize。跨协议桥需 Go 上下文桥（ConversationTimeline + ContextMaterializer），后置 L3.5。
- **marketplace schema 转换规则配置**（L6）：M0+M1 转换规则硬编码（仅 OpenAI `include_usage` 注入），不做 marketplace 可视化 schema 匹配（Go+React），后置 L6。
- **嵌入式控制台**（L6）：M0+M1 仅机器可读 API，无 React 控制台，后置 L6。
- **多协议桥 / Realtime / Live / 文件媒体 / Batch Jobs**（远期，蓝图 10/12/13/14）：后置，架构仅留能力族位置（宪法 XI）。
- **网关基础设施高级项**（L3+）：M0+M1 仅上游 TLS 默认校验 + 基础重试；HTTPS 终止证书管理 / ACME 自动化 / mTLS / CORS / IP allow-deny / 熔断 / 健康检查 / region-aware 路由等后置。


### 1.4 数据面复用 001

Rust 内核数据面**移植 001-gateway-foundation 已验证实现**（redesign 分支 `app/`），而非重写。001 数据面范式正确（仅控制面范式错误），代码宪法合规。移植做适配（见 §6）。

## 2. 架构

### 2.1 整体形态（平台模式，M0+M1 主形态）

```text
客户端（OpenAI/Anthropic/Gemini SDK，持虚拟 key）
  │ 仅改 base URL / token / 模型名
  ▼
pingogate-core 二进制（Rust 内核）
  ├─ public listener（Pingora HttpProxy）
  │   └─ 数据面管线（pipeline crate）
  │       协议识别 -> 虚拟Key鉴权(查快照) -> 模型路由 -> 上游认证注入(KeyVault解密)
  │                -> 透传/SSE -> 错误镜像 -> 可观测 -> 提取usage推Go
  │       │
  │       └─ 读 RuntimeSnapshot（ArcSwap，Go 推送）
  │              ├─ providers（含密文 provider key，KeyVault 热路径解密）
  │              ├─ routes（模型别名 -> provider/upstream_model）
  │              └─ virtual_keys（哈希 -> {owner, scope, provider_key_ref}）
  │
  ├─ admin listener（Pingora ServeHttp）：healthz/readyz/metrics/reload（经 authorize）
  │
  └─ gRPC server：接收 Go 推快照 / 响应 Go KeyVault 加解密 / 推 usage 给 Go
       ↑↓ gRPC
pingogate-ctrl 二进制（Go 非内核）
  ├─ 控制面 API（chi）：用户 CRUD / provider key CRUD / virtual key CRUD
  ├─ KeyVault 客户端：录入时转发明文给 Rust 加密、查看明文时调 Rust 解密（校验 created_by）
  ├─ 快照推送：DB 变更 -> 构建快照（含密文 key + 虚拟 key 哈希）-> gRPC 推 Rust
  ├─ 用量落库：收 Rust usage 事件 -> 估算(无usage时) -> 聚合落 DB
  └─ DB（sqlx/SQLite）：users / user_provider_keys(密文) / virtual_keys(哈希) / usage
```

### 2.2 双运行模式

Rust 内核同一二进制支持两种形态（宪法总纲）：

- **单机模式（standalone）**：内核单独跑，从 `pingogate-core.yaml` 加载路由 / 上游 / 静态网关 key，provider key 用 `env:VAR` 密钥引用（不加密，单用户无隔离），无 Go、无 DB、无 KeyVault。轻量 LLM 网关。`SnapshotSource::File` + `StaticKeyAuth`。
- **平台模式（platform）**：内核 + Go 非内核，gRPC 推快照（含虚拟 key / 加密 provider key），KeyVault 解密，多用户 BYOK 隔离 + 用量落库。完整 SaaS 平台。`SnapshotSource::Grpc` + `VirtualKeyAuth`。

M0+M1 两种模式都实现（单机模式复用 001，平台模式新建 Go + gRPC）。两模式共享数据面管线 / 路由 / 转换引擎，仅快照来源与鉴权模式不同。

### 2.3 请求分流（平台模式）

```text
无上下文请求（无 previous_id，六接口同态）：
  客户端 -> Rust 内核（鉴权+路由+透传）-> 上游    Go 不介数据路径

带上下文请求（有 previous_response_id/previous_interaction_id，同态透传）：
  M0+M1：客户端 -> Rust 内核（原样转发 previous_id）-> 上游
  （L3.5 后：客户端 -> Go(查timeline+materialize) -> Rust -> 上游，跨协议桥）
```

M0+M1 同态透传不区分有无 previous_id，都直入 Rust 原样转发（上游服务端自管状态）。L3.5 跨协议桥才需 Go materialize。

## 3. 组件与 crate 划分

### 3.1 Rust 内核（`core-rs/`）

按宪法 IV，依赖自底向上无环：

| crate | 职责 | 001 对应 |
|---|---|---|
| `core` | 共享域类型：`AppError`、`Principal`/`AuthContext`/`authorize`、`SecretString`、`ProtocolKind`/`ProviderKind`/`CapabilityFamily`/`AuthMethod`、`TraceId` | `core/`（移植） |
| `storage` | `SecretResolver` + `EnvSecretResolver`；`SnapshotSource` trait + `FileSnapshotSource` + `GrpcSnapshotSource` | `storage/secret.rs`（移植）+ 新增 |
| `snapshot` | `RuntimeSnapshot` + `SnapshotHolder`（ArcSwap）；语义校验；密钥预解析（单机）/ 密文持有（平台，KeyVault 热路径解密） | `config/snapshot.rs` + `validate.rs`（移植重组） |
| `provider` | `ProviderAdapter` trait + 六接口 adapter；能力族声明；错误体形状 | `provider/`（移植扩展） |
| `pipeline` | `GatewayProxy`（ProxyHttp）；协议识别；上游认证注入；SSE 流式；模型路由调用；usage 提取；可观测 | `pipeline/`（移植扩展） |
| `router` | 模型别名提取 -> 路由解析 | `pipeline/router.rs`（拆出） |
| `transform` | `Transform` trait（请求/响应转换）；M0+M1 最小实现（OpenAI `include_usage` 注入） | 新增 |
| `listener` | Pingora public / admin service 装配 | `listener/`（移植） |
| `keyvault` | KeyVault 加解密 gRPC 服务端；AES-GCM；主密钥加载 | 新增 |
| `pingogate-core` | 主二进制：bootstrap + 信号 + reload 编排 + gRPC server 装配 | `pingogate/`（移植改名） |

### 3.2 Go 非内核（`ctrl-go/`）

按宪法 IV + Go 约定：

| package | 职责 |
|---|---|
| `cmd/pingogate-ctrl` | 主二进制：bootstrap + 装配 |
| `internal/identity` | User 实体 + CRUD + bootstrap admin + Principal/authorize |
| `internal/keymgmt` | UserProviderKey CRUD + 1Password 可见性（created_by 校验）+ KeyVault gRPC 客户端 |
| `internal/vkey` | VirtualKey 签发/撤销/列表 + 哈希存储 |
| `internal/snapshot` | 快照构建 + gRPC 推送给 Rust |
| `internal/usage` | 收 Rust usage 事件 + 估算（tiktoken-go）+ 聚合落库 |
| `internal/admin` | 管理面 API + 健康就绪 + reload 编排 |
| `internal/storage` | sqlx + 迁移（users/user_provider_keys/virtual_keys/usage 表） |
| `internal/proto` | protobuf codegen |

### 3.3 跨语言（`proto/`）

gRPC 接口定义（内部通信安全见 §12A：mTLS + 内部 token + 仅 127.0.0.1）：
- `SnapshotService.PushSnapshot`（Go -> Rust，stream）：推 RuntimeSnapshot（providers/routes/virtual_keys 哈希/加密 provider key），带 version，Rust 回 ack（见 §12C 同步语义）。
- `KeyVaultService.Encrypt` / `Decrypt`（Go -> Rust，unary）：加解密 provider key。Decrypt 请求含 `requester_user_id` + `key_id`，Go 校验 created_by 后调用，Rust 记审计日志（双层防御，见 §12A）。
- `UsageService.ReportUsage`（Rust -> Go，stream）：Rust 推 usage 事件（数字或 body）。
- `HealthService`（双向，unary）：心跳 + Rust 报告当前快照 version，Go 据此判断是否需重推全量（见 §12C 恢复）。

跨语言类型经 protobuf codegen 同步 Rust / Go。

## 4. 数据流

### 4.1 请求处理（平台模式热路径）

```text
1. Pingora 接收请求，new_ctx() 加载当前 RuntimeSnapshot（ArcSwap::load）
2. request_filter():
   a. 协议识别（路径后缀模式，版本前缀通配）-> ProtocolKind + streaming 标记
      - 识别失败 -> PingoGate 原生错误；不支持能力 -> 镜像 Provider 错误
   b. 虚拟 Key 鉴权（VirtualKeyAuth）：提取 key -> 哈希比对查快照 -> Principal{owner, scope}
      - 无效/撤销/过期 -> 镜像 Provider 鉴权错误，不触达上游
   c. scope 校验：验 model/provider/IP/过期/并发配额（内存计数器）
   d. 模型路由：提取别名 -> 解析 provider + upstream_model
   e. authorize(Proxy, route) -- RBAC 边界
3. upstream_peer(): HttpPeer（含上游 TLS 默认校验）
4. upstream_request_filter():
   a. 剥离客户端 key 头
   b. KeyVault 解密 provider key（平台模式，密文来自快照）-> SecretString
   c. 注入上游认证（Bearer/x-api-key/query_key，按 provider）
   d. transform: OpenAI Chat Completions 流式注入 include_usage（若配置）
5. Pingora 透传：响应头透传；body 逐 Bytes 块零拷贝（SSE 不缓冲）
6. usage 提取（旁路 observer）：解析 SSE 末块 usage（各接口位置不同）-> 推 Go
7. 错误路径：上游超时/不可用 -> 镜像 Provider 错误；上游自身错误原样透传
8. 可观测：trace id/protocol/route/status/latency；metrics 计数
```

### 4.2 BYOK key 录入流程

```text
用户 -> Go API（录入 provider key 明文）
  -> Go internal/keymgmt 校验身份
  -> Go 调 Rust KeyVault.Encrypt(明文) -> 密文
  -> Go 存 DB（密文 + 元数据 + created_by = 当前用户）
  -> Go 构建新快照（含密文 key）-> gRPC 推 Rust
```

### 4.3 BYOK key 查看明文流程（1Password 可见性）

```text
created_by 用户 -> Go API（请求看明文）
  -> Go 校验 created_by == 请求者（1Password 规则，Go 决定"谁能看"）
  -> Go 调 Rust KeyVault.Decrypt(密文) -> 明文
  -> Go 返回明文给用户（一次性，不持久化）
非 created_by 用户 -> Go 只返回末位 + 元数据，不调解密
```

### 4.4 热重载

```text
触发：SIGHUP | 文件监视 | Admin reload | Go gRPC 推送
  ↓
Reloader（串行化）：
  单机模式：SnapshotSource::File 读 pingogate-core.yaml -> 校验 -> 解析密钥 -> 构建
  平台模式：SnapshotSource::Grpc 接收 Go 推送 -> 校验 -> 构建（密文，KeyVault 热路径解密）
  -> SnapshotHolder::store（ArcSwap 原子切换）
  -> 记录版本/状态/回滚目标
失败：不改活跃 snapshot；进行中请求继续用旧 snapshot（零中断）
```

## 5. 配置 schema

### 5.1 单机模式 `pingogate-core.yaml`

```yaml
listeners:
  public: { address: "0.0.0.0:8080" }
  admin:  { address: "127.0.0.1:9090" }
gateway_keys:
  - { name: "team-alpha", secret_ref: "env:PINGO_KEY_ALPHA", enabled: true }
providers:
  - name: "openai-main"
    kind: "openai-compatible"
    base_url: "https://api.openai.com"
    auth: { method: "bearer", key_ref: "env:OPENAI_API_KEY" }
    capability_families: ["generation.stateless"]
routes:
  - { alias: "gpt-4o", provider: "openai-main", upstream_model: "gpt-4o" }
upstream: { timeout_ms: 60000 }
```

### 5.2 平台模式

Rust 内核配置仅含 listener / gRPC 监听 / 主密钥引用；providers/routes/virtual_keys/加密 provider key 全部由 Go 推送（不经文件）。

```yaml
listeners:
  public: { address: "0.0.0.0:8080" }
  admin:  { address: "127.0.0.1:9090" }
grpc:    { address: "127.0.0.1:9091" }
keyvault:
  master_key_ref: "env:PINGO_MKEK"   # AES-GCM 主密钥
mode: platform                      # standalone | platform
```

## 6. 从 001 移植的适配

1. **依赖结构重构**：001 crate `pingo_*` -> `pingogate_*`；`config` crate 拆 `snapshot`；新增 `router`（从 pipeline 拆）/ `transform` / `keyvault`。
2. **SnapshotSource 双模式**：001 只文件构建；M0+M1 抽象 trait，file + gRPC 两实现。
3. **KeyAuth 双模式**：001 内联静态 key 校验；M0+M1 抽象 trait，StaticKeyAuth + VirtualKeyAuth。
4. **协议识别扩展**：001 精确路径匹配 `/v1/chat/completions`；M0+M1 改路径后缀模式匹配（版本前缀通配），加 Responses/Interactions/streamGenerateContent。
5. **KeyVault 新增**：001 无加密；M0+M1 新增 keyvault crate（平台模式）。
6. **usage 提取新增**：001 无用量；M0+M1 新增 usage 提取（各接口位置）+ 推 Go。

## 7. 错误处理（宪法 IX）

- `AppError`（Domain/Application/Infrastructure/Validation），HTTP 码仅边界映射。
- **L3+ 数据面（Rust）**：网关自身错误镜像目标 Provider 原生错误体（OpenAI/Anthropic/Gemini 各形状）；协议识别前失败回退 PingoGate 原生错误体；上游自身错误原样透传。
- **L0-L2 控制面（Go）**：标准 REST 错误体（错误码 + 消息 + 可选详情），经 authorize 边界返回。
- 密钥明文 MUST NOT 进错误信息 / 日志 / 指标。

## 8. 安全（宪法 XX）

- 100% Safe Rust（内核）；Go 遵循 Go 安全实践。
- **BYOK 可见性（1Password 模型）**：明文可见性绑定 `created_by`，与 RBAC 正交。只有 `created_by` 看明文，其余只见末位 + 元数据。明文只在 Rust `KeyVault::decrypt()` 返回的 `SecretString` 存活，Go 只持密文。
- **KeyVault 跨边协作**：Go 持密文 + 元数据 + created_by 校验权；Rust 持加解密能力 + 主密钥。Go 决定"谁能看"，Rust 决定"怎么解"。录入时 Go 短暂接触明文（转发 Rust 加密），可接受。
- 上游 Provider Key 不由 adapter 直接接触（pipeline 单点注入）。
- Admin / 所有特权操作经 `Principal`/`authorize` 边界；无全局 admin token 等值。
- 上游 TLS 默认校验。
- 不持久化完整 prompt/response（M0+M1 无上下文桥，同态透传不存内容）。

## 9. 可观测性（宪法 XIX）

- **Go 管理面**：slog 结构化日志，管理操作审计（who/what/when），密钥脱敏。
- **Rust 热路径**：tracing 带 trace/请求 ID；Prometheus 指标按 provider + 能力族打标签，MAY 按 principal id 归因。
- **用量（Rust 提取 + Go 落库）**：Rust 提取现成 usage（各接口位置）推 Go；Go 估算（无 usage/失败时）+ 聚合落库。必需信号：请求量/状态码/延迟/上游延迟/token 计数。
- 密钥 / token 经统一脱敏层后才输出（双语言）。

## 10. 性能（宪法 XXI）

- 延迟预算：无状态透传 p50 < 5 ms / p95 < 20 ms（Pingora 级，001 基准 p50≈1.5ms/p95≈1.7ms）。
- 零拷贝流式（SSE 不缓冲）；异步无阻塞；显式背压。
- **热路径零 DB、零跨进程调用**：Rust 热路径只读内存快照；Go 推快照不阻塞 Rust 热路径。
- Go 控制面按人速 QPS 设计，异步无阻塞。

## 11. 实现分期（三个 plan）

spec 覆盖完整设计，实现分三 plan，每 plan 独立可验证。具体步骤由 writing-plans 出。

### S1：双语言基建 + Rust 内核透传
- `proto/` 定义 + Rust/Go codegen 骨架
- Rust 内核从 001 移植：六接口同态透传 + 协议识别（路径后缀模式）+ 路由 + 上游认证注入 + 热重载
- `SnapshotSource`/`KeyAuth` trait + File/Static 实现（单机模式可跑）
- gRPC 骨架（Rust server + Go client，先通心跳/推空快照）
- KeyVault 空壳（trait 占位，不实现加密）
- **证伪**：Rust 内核单机模式六接口透传跑通；Go 能 gRPC 连 Rust（推空快照）；平台模式位置预留编译通过

### S2：用户身份 + Go 控制面 + KeyVault 加密
- Go `internal/identity`：User CRUD + bootstrap admin + authorize 边界
- Go `internal/keymgmt`：UserProviderKey CRUD + KeyVault gRPC 客户端
- Rust `keyvault` crate：AES-GCM 加解密实现 + 主密钥加载 + gRPC 服务端
- Go `internal/snapshot`：DB 变更 -> 构建快照（密文 key）-> gRPC 推 Rust
- Rust `SnapshotSource::Grpc` + `KeyAuth` 扩展（接收 Go 推送）
- **证伪**：用户能 CRUD；录入 provider key 经 Rust 加密存 DB（密文）；Go 推快照 Rust 接收；单机+平台双模式切换

### S3：1Password 可见性 + 虚拟 key + 闭环
- Go 1Password 可见性：created_by 校验 + 末位脱敏 + 解密查看明文
- Go `internal/vkey`：VirtualKey 签发/撤销/哈希存储 + 推快照
- Rust `VirtualKeyAuth`：虚拟 key 哈希比对 + scope 校验 + 并发配额计数（内存）
- Rust usage 提取（各接口位置）+ `UsageService.ReportUsage` 推 Go
- Go `internal/usage`：收事件 + 估算（tiktoken-go）+ 聚合落库
- OpenAI `include_usage` 注入（transform 最小实现）
- **证伪（M0+M1 完整闭环）**：用户用虚拟 key 经 Rust 透传调上游，用自己的 BYOK key（KeyVault 解密注入），admin 不可见明文 key；usage 提取落库；同态透传响应与直连一致

## 12. 成功标准（Success Criteria）

- **SC-1**：客户端仅改 base URL / token / 模型名，六接口（含 SSE）经网关调通上游，响应与直连一致。
- **SC-2**：用户能录入 BYOK provider key（加密存 DB），只有 created_by 能看明文，admin 不可见。
- **SC-3**：用户用虚拟 key 经网关调上游，网关用该用户的 BYOK key（KeyVault 解密注入），上游 key 不下发客户端、不进日志/指标。
- **SC-4**：虚拟 key 无效/撤销/过期/scope 超限在鉴权阶段被拒，不触达上游。
- **SC-5**：热重载成功切换；非法配置被拒且活跃 snapshot 100% 继续服务；在途请求零中断。
- **SC-6**：100% Admin API 请求经 authorize 边界；缺失/无效凭据被拒。
- **SC-7**：无状态透传 p50 < 5 ms / p95 < 20 ms。
- **SC-8**：不支持的能力族返回显式「不支持」错误，不畸形透传。
- **SC-9**：Rust 内核零 DB（热路径零 DB，平台模式 Go 推快照）；Rust 不带 tokenizer。
- **SC-10**：usage 提取落库（有 usage 的接口），无 usage 的接口 Go 估算。
- **SC-11**：单机模式（无 Go）六接口透传可跑（Rust 独立可用）；平台模式（Go+Rust）完整闭环。
- **SC-12**：`SnapshotSource`/`KeyAuth` trait 双实现，单机/平台模式切换。

## 12A. 内部 gRPC 安全（宪法 XX 补充）

Rust 内核与 Go 非内核间的 gRPC（快照推送 / KeyVault 加解密 / usage 上报）是内部通信，但**必须防本地任意进程调用**（否则本地恶意进程可调 `KeyVault.Decrypt` 解密所有 key）。

- **传输**：本地 127.0.0.1 监听；默认 mTLS（Rust 与 Go 互验证书），单机模式无 gRPC 不涉及。开发环境可降级为明文（显式 `grpc.tls: false`，仅本地）。
- **鉴权**：gRPC 元数据携带共享内部 token（`env:PINGO_INTERNAL_TOKEN`，启动时 Rust/Go 各自加载）；Rust 校验调用方 token，拒绝无 token / 错 token 的调用。
- **KeyVault.Decrypt 专门约束**：即使 gRPC 鉴权通过，Rust 侧不单独信任 Go 的 created_by 校验--Rust 在 Decrypt 请求里要求 Go 传 `requester_user_id` + `key_id`，Rust 记审计日志（谁解密了谁的 key）。Go 侧 created_by 校验是第一道，Rust 审计是第二道（事后追责），双层防御。
- **不暴露公网**：gRPC listener 仅 bind 127.0.0.1，不对外。

## 12B. Bootstrap 流程

平台模式首次启动的"鸡生蛋"问题：没有 admin 就无法创建用户/admin。

- **bootstrap admin**：环境变量 `PINGO_BOOTSTRAP_ADMIN_TOKEN`（首次启动 Go 读取）。Go 启动时若 DB 无任何 admin，用此 token 创建首个 admin（Principal kind=Admin），之后该 env 可撤销。
- **bootstrap admin 走 authorize 边界**：即使是 bootstrap，也经 `Principal::admin` + `authorize`，不绕过边界（宪法 XX）。
- **主密钥**：`env:PINGO_MKEK`（KeyVault AES-GCM 主密钥），Go 启动时校验存在，缺失即启动失败（不给"明文存 key"的降级路径）。
- **gRPC 内部 token**：`env:PINGO_INTERNAL_TOKEN`，Rust/Go 启动时各自加载，缺失即启动失败。
- **Rust 首次全量同步**：Rust 启动后内存快照空，Go 检测到 Rust 连接（gRPC 建连）后立即推全量快照（providers/routes/virtual_keys/加密 key）；Rust 在收到首个全量快照前 `readyz` 返回 not-ready（拒绝流量）。

## 12C. 快照同步与恢复

平台模式 Rust 内存快照是 Go DB 状态的缓存副本，需保证一致与可恢复。

- **权威源 = Go DB**：Rust 内存快照非权威，崩溃丢失无碍（Go 重推）。
- **版本号**：每个快照带单调递增 `version`（Go 维护）。Rust 收到快照记 version；Go 推送时若 Rust 报告的 version 落后，推增量或全量。
- **推送语义**：gRPC stream，Go 推 `Snapshot` 消息（全量；增量优化后置）。Rust 收到 -> 校验 -> ArcSwap 原子切换 -> 回 ack（含 version）。Go 未收 ack 不更新"已同步 version"。
- **Rust 崩溃恢复**：Rust 重启 -> 内存空 -> Go 检测重连 -> 推全量 -> Rust ready。恢复期间 Rust not-ready。
- **Go 崩溃恢复**：Go 重启 -> 读 DB -> 重建内存状态 -> 向 Rust 推当前全量快照。Rust 用旧快照继续服务（Go 崩溃期间 Rust 热路径不中断，因快照在 Rust 内存）。
- **并发推送串行化**：Go 侧 snapshot 推送经 mutex 串行（同一时刻一个推送），避免乱序；Rust 侧 ArcSwap 原子切换保证一致性。
- **最终一致**：短暂不一致可接受（Go DB 已改、Rust 未同步期间，Rust 用旧快照服务）。权威是 DB，对账后修正。

## 12D. 测试策略（宪法 XIII）

契约 / 集成 / 单元测试先行。

- **单元**：
  - Rust：协议识别（六接口 + 后缀模式 + 版本前缀通配 + 不支持能力 + 未识别）；上游认证注入（三方式）；模型别名提取；snapshot 构建与校验；KeyVault 加解密（AES-GCM 正确性）；StaticKeyAuth/VirtualKeyAuth；authorize 边界；usage 提取（各接口位置）；密钥脱敏。
  - Go：User/ProviderKey/VirtualKey CRUD；1Password 可见性（created_by 校验，非创建者只见末位）；哈希存储；快照构建。
- **集成**：
  - Rust：管线阶段往返；六接口透传（非流式 + SSE，mock 上游）；热重载（成功切换 / 失败不改活跃 / 在途零中断）；单机/平台双模式切换。
  - Go：控制面 API 往返；gRPC 推快照 Rust 接收；KeyVault 加解密 gRPC 往返。
- **契约**：
  - gRPC 接口双侧契约（proto 定义为契约，Rust/Go 各自生成 + 测试）：SnapshotService / KeyVaultService / UsageService。
  - Admin API 契约（六端点 + 鉴权边界）。
  - 六接口透传 + 镜像错误契约。
- **安全测试（端到端）**：
  - 1Password 可见性端到端：非 created_by 用户（含 admin）调看明文 API，断言只返回末位；created_by 调，断言返回明文但 Rust 审计日志记录。
  - 密钥不泄漏：抓取所有日志/指标/错误体，断言 0 明文 key 出现（抽样审计）。
  - gRPC 鉴权：无 token / 错 token 调 KeyVault.Decrypt，断言拒绝。
- **基准**：无状态透传延迟基准（默认 `#[ignore]`，显式运行），回归阻止合入（p50 < 5ms / p95 < 20ms）。

## 13. 风险与未决

- **R1 未决**：单机模式 admin 鉴权用单 token 还是一组具名 admin key？倾向单 token，S1 plan 定。
- **R2 未决**：`router` 是否 S1 独立 crate？倾向独立，S1 plan 定。
- **R3 风险**：001 移植时 Pingora 版本可能需升级（001 用 pingora-proxy 0.8.0），API 变更风险。S1 实现期用 context7 核对（宪法 XVII）。
- **R4 风险**：crate 重组后 001 测试需重新组织，可能暴露隐藏耦合。S1 移植时逐 crate 验证。
- **R5 未决**：OpenAI Chat Completions 流式 `include_usage` 注入是否 S3 实现？倾向是（否则该接口流式无 usage）。S3 plan 定。
- **R6 风险**：KeyVault 热路径解密性能--平台模式每请求解密 provider key 是否缓存？倾向 S3 先不缓存（正确性优先），性能优化后置。
- **R7 风险**：gRPC 快照推送一致性--Go 推送中途 Rust 用旧快照，需保证最终一致 + 版本号。S2 设计。
- **R8 未决**：Gemini Interactions 完整 schema（`docs/02-provider-schemas.md` §8 待补充），S1 协议识别可能需实现期核对 API ref。
- **R9 未决**：DB 迁移工具选型（sqlx migrate vs golang-migrate vs 手写）？倾向 sqlx migrate（与 sqlx 一体），S2 plan 定。
- **R10 未决**：gRPC 内部 mTLS 证书怎么签发（自签 CA？启动时生成？）？倾向首次启动 Go 生成自签 CA + Rust/Go 各证书，S1 plan 定。
- **R11 风险**：Rust 首次全量同步前 not-ready，若 Go 启动慢则 Rust 长时间拒流量；需 readiness 探针编排（K8s 部署时）。S3 部署配置考虑。
