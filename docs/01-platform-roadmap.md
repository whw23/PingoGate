# PingoGate SaaS 平台路线图

> **状态**：讨论稿 / 架构分解
> **日期**：2026-07-12
> **定位**：本文档把 PingoGate 的 SaaS 平台目标分解为有依赖序的能力层，作为后续每个 feature spec 的编排依据。它不是某个 feature 的 spec，而是**平台级路线图**。

## 1. 目标与定位

PingoGate 的目标是**多用户、BYOK 优先的 LLM 流量治理 SaaS 平台**。这不是一个单次交付的 v1，而是一个分阶段长起来的平台。

核心理念（源自蓝图，与宪法对齐）：

- **用户优先、路由后置**：身份与密钥是承重墙，路由是后来长上去的。001（路由优先单管理员代理）的范式错误在此纠正。
- **BYOK 为基座**：用户自带 Provider Key，完全私有，管理员不可见明文（蓝图 16A）。
- **SaaS 平台 ≠ 一个 spec 装下所有**：成熟 SaaS 平台分阶段长起来，每阶段是独立可交付的闭环。平台是愿景，分解是路径。
- **层间只预留位置，不预留框架**（宪法 II YAGNI）：每层只实现自己，对上层留接口位置而非实现框架。

### 双语言内核架构

PingoGate 采用**内核 Rust + 非内核 Go** 双语言架构（详见宪法「双语言内核架构」总纲）：

- **Rust 内核**（`core-rs/`，`pingogate-core` 二进制）：**无状态**请求处理核心引擎 + 安全核心 + 用量采集。Pingora 数据面管线（协议识别 / 鉴权 / 请求路由 / 请求转换 / 透传 / SSE）+ 路由引擎 + 转换引擎（无状态 body 改写 / 协议桥，不含 materialize）+ KeyVault（密钥加解密）+ 快照引擎（内存）+ 用量采集（内存通道，推 Go 落库）。**零 DB、零持久状态**。性能 / 正确性敏感、稳定少变。
- **Go 非内核**（`ctrl-go/`，`pingogate-ctrl` 二进制）：**所有状态** + 业务面 + 平台治理。控制面 API（用户 / 组织 / RBAC / 虚拟 key CRUD）+ OAuth / 会话 + **上下文桥（ConversationTimeline 存储 + ContextMaterializer）** + 用量聚合 / 落库 / 计费 + 审计 + 控制台 BFF + DB。迭代频繁、生态依赖。
- **通信**：Go 经 gRPC 把配置 / 密钥 / 虚拟 key 作为快照推给 Rust 内核；Rust KeyVault 解密后存内存快照，热路径零 DB 零解密。Rust 用量采集到内存后批量推 Go 落库。**带上下文请求**（previous_response_id / previous_interaction_id）由 Go materialize 后推 Rust 透传；**无上下文请求**直入 Rust。跨语言类型经 `proto/` codegen 同步。
- **分界原则**：Rust 内核 = 无状态计算（透传 / 路由 / 无状态转换 / 安全 / 采集，零 DB）；Go 非内核 = 所有状态（配置 / 用户 / 上下文 timeline / 用量落库 / 计费，管 DB）。
- **部署**：双二进制，可单 Docker 镜像。单仓库（`proto/` / `core-rs/` / `ctrl-go/` / `console/` / `deploy/` / `docs/` / `.claude/`），proto 为 Rust / Go 共享根。
- **双运行模式**：Rust 内核同一二进制支持两种形态：
  - **单机模式（standalone）**：内核单独跑，从 `pingogate-core.yaml` 加载路由 / 上游 / 静态网关 key，provider key 用密钥引用（`env:VAR`，不加密），无 Go、无 DB。轻量 LLM 网关（蓝图 3.1 单二进制优先）。KeyVault 不启用。
  - **平台模式（platform）**：内核 + Go 非内核，gRPC 推快照（虚拟 key / 加密 provider key / 计费规则），KeyVault 解密，多用户 BYOK 隔离 + 用量计量 + 计费。完整 SaaS 平台。
  - 单机模式是内核最小可用形态；平台模式是增强。两模式共享数据面管线 / 路由 / 转换引擎，仅快照来源与鉴权模式不同。

**分界原则**：请求从进到出的整条处理链路（含路由 / 转换）+ 安全核心 = Rust；围绕这条链路的配置 / 治理 / 业务 = Go。

各能力层的主语言：L0-L2 控制面 = Go（L1 的 KeyVault 加解密在 Rust 内核，Go 经 gRPC 调）；L3 数据面 = Rust；L4-L6 治理 / 用量 / 计费 = Go（用量事件由 Rust 热路径上报，Go 聚合）。

## 2. 能力层分解

平台按依赖序拆为 L0-L6 七层。每层是一个独立 sub-spec（各自 spec → plan → 实现 → 验证）。**每层独立可交付、可验证、可用**。

```text
L0 地基：身份与边界
  User + Principal/authorize + bootstrap admin
  ↓ 所有上层都依赖身份
L1 BYOK 密钥：Provider Key 加密保管 + 1Password 可见性
  用户存 key -> 只有 owner 看明文 -> admin 不可见
  ↓ 有了 key 才能路由
L2 虚拟 key：签发 / scoped / 可撤销（≈ SAS token）
  ↓ 有凭据才能调数据面
L3 数据面骨架：Pingora + 单协议透传（虚拟key -> provider key -> 上游）
  ↓ 能跑流量才有用量
L3.5 上下文桥：ConversationTimeline + ContextMaterializer（Go）
  带 previous_response_id / previous_interaction_id 的有状态请求 -> Go 查 timeline + materialize -> Rust 透传
  依赖：L3 数据面 + provider schema（marketplace 驱动 materialize 规则）
  ↓ 有上下文桥才能桥接有状态协议到无状态上游
L4 多租户：Tenant -> Project + RBAC + 组织 key 共享
  ↓ 有组织模型才有治理
L5 可观测 + 用量：UsageSink 流式计量 + 指标
  ↓ 有用量才有计费
L6 计费配额 + 控制台 + 多协议 + 规模化（Redis / Kafka）
```

### 里程碑

| 里程碑 | 包含层 | 价值 |
|---|---|---|
| **M0**：内核单机网关 | L3（单机模式） | Rust 内核单独跑，文件配置 + 静态 key，无 Go 无 DB，一个可用的轻量 LLM 网关。**数据面复用 001 已验证实现**（Pingora 管线/协议识别/认证注入/热重载，移植到 `core-rs/`，做双模式 + 虚拟key 适配） |
| **M1**：BYOK 密钥平台 | L0 + L1 | Go 控制面 + Rust KeyVault，能安全存 BYOK key 的多用户平台 |
| **M2**：BYOK 网关 | M1 + L2 + L3（平台模式） | 能调模型的 BYOK 网关（真正 MVP） |
| **M3**：多租户治理 | M2 + L4 + L5 | 有组织治理与用量观测的网关 |
| **M4**：完整 SaaS 平台 | M3 + L6 | 计费配额 + 控制台 + 多协议 + 规模化 |

## 3. 各层详述

### L0 — 地基：身份与边界

**职责**：建立平台一切特权的身份与授权边界。

- `User` 实体（扁平，无 Tenant/Project 层级——多租户是 L4）
- `Principal` / `AuthContext` / `authorize(action, resource)` 边界（宪法 XX）
- bootstrap admin（静态凭据但走 authorize 边界，非全局 token 等值）
- 最小存储（file+memory 或 SQLite 单档）
- 机器可读管理面 API（经 authorize 边界）

**预留位置（不实现）**：`Principal` 预留 RBAC 角色枚举扩展位（L4）；`authorize` 的 resource 预留 scope 字段。

**证伪目标**：身份边界成立——非授权请求被拒，bootstrap admin 也走 authorize。

### L1 — BYOK 密钥：Provider Key 加密保管

**职责**：用户录入自己的 Provider Key，加密存储，按 1Password 可见性规则暴露。

- `UserProviderKey` 实体（owner_user_id / provider_type / encrypted_key / base_url / created_by / created_at / last_used_at）
- `KeyVault`：provider key 加解密唯一入口（主密钥 `env:PINGO_MKEK` + AES-GCM），明文只活在受保护 `SecretString`（宪法 XX）
- 1Password 可见性：明文仅 `created_by` 可见，其余任何人（任何角色 / 任何 admin）只见末位 + 元数据；可见性与 RBAC 正交
- 管理面 CRUD API：录入 / 查看（owner 明文 / 他人末位）/ 启停 / 删除 / 轮换
- `ControlPlaneStore` trait + `KeyVault` trait（实现可换，为 L4/L5 预留）

**预留位置**：key 的 `scope` 字段（user/org，L4 才用 org）；`UserChannel`（路由用，L3 才实现）。

**证伪目标**：BYOK 安全模型在代码层成立——用户存 key → 只有 owner 看明文 → admin 全程不可见明文（端到端测试覆盖）。

### L2 — 虚拟 key：签发与撤销

**职责**：PingoGate 签发客户端凭据，可 scoped / 可撤销，哈希存储验证。

- `VirtualKey` 实体（owner / scoped: provider/model/IP/过期 / 哈希存 / 状态机：active/revoked）
- 签发 API（返回明文一次，之后哈希存）
- 撤销 / 列表 / 重建 API
- 借鉴 IoT Hub SAS token 模型（可撤销、可 scoped、中心签发、哈希验证）

**预留位置**：虚拟 key 的路由消费在 L3 才实现；scope 的并发配额字段为 L3/L6 预留。

**证伪目标**：虚拟 key 签发与撤销闭环，哈希验证成立，scoped 限制生效。

### L3 — 数据面骨架：Pingora + 单协议透传

**职责**：把 L0-L2 的控制面能力接入真正的请求管线，用用户的 key 打到上游。**这是 Pingora 首次进入**（宪法 III 热路径必须 Pingora 在此触发）。

- Pingora `ProxyHttp` 管线：虚拟 key 鉴权 → 识别 user → 取该 user 的 provider key（解密）→ 注入上游认证 → 透传
- 单协议起步（OpenAI-compatible Chat Completions）
- 不可变 `RuntimeSnapshot` + `ArcSwap`（宪法 XII）：虚拟 key 映射 + 解密后 provider key 全内存，热路径零 DB
- SSE 流式透传（零拷贝，不解析语义；宪法 XXI）
- 上游 TLS 默认校验
- 镜像目标 Provider 错误体（宪法 IX）

**预留位置**：partition 接口（L6）；用量 observer 旁路（L5，透传不解析但 observer 解析 SSE 提 usage）。

**证伪目标**：用户用虚拟 key 经网关调通上游模型，响应与直连一致，上游 key 不泄漏。

### L4 — 多租户：Tenant / Project / RBAC

**职责**：把扁平 User 升级为多租户组织模型，加 RBAC 治理。

- `Tenant`（组织 / 个人）→ `Project` → `User` 三层
- RBAC：角色（org admin / member / 个人 user）+ 权限矩阵
- 组织 key：组织 admin 管理，组织成员可用（scope: org）；BYOK key 仍 scope: user 仅 owner
- 1Password 可见性扩展到组织 key：明文仅 `created_by` 可见，组织 admin 也只见末位（除非 admin 就是创建者）
- 管理面 API 扩展：组织 CRUD / 成员管理 / 角色分配 / 组织 key 管理

**证伪目标**：多租户隔离成立——组织成员可用组织 key 但不可见明文（非创建者），跨组织隔离，RBAC 边界生效。

### L5 - 可观测 + 用量：流式计量与指标

**职责**：让流量可见、可计量，为计费铺路。**Rust 只提取现成 usage，估算全 Go，查询/计费在 Go**（宪法 XIX）。

- **Rust 内核用量提取**：只提取 provider 响应中**现成**的 usage 字段（OpenAI `stream_options.include_usage` / Anthropic `message_delta` / Gemini `usageMetadata`），增量记录 + 终态对账，推 Go。**Rust 不带 tokenizer、不做估算**（各 provider tokenizer 算法不同，维护繁琐，交 Go 生态）。Rust 零 DB。
- **Go 非内核估算**：响应无 usage 时由 Go 用 tokenizer 估算（Rust 推 body，复用上下文 materialize 的 body 或专门推）；**失败 / 中断的部分计量归 Go**（Rust 推已转发部分，input 仍计因请求已发上游，output 按已吐部分）；用量聚合 / 落库（可插拔 sink：SQL / 未来 Redis / Kafka）/ 查询 / 计费 / 配额规则（人速业务，RBAC）。
- **单机模式**：Rust 只提取现成 usage（记 log / 指标，不落 DB）；无 usage 不估算（尽力而为，因无 Go）。
- Prometheus 指标（按 provider + 能力族 + principal 打标签，宪法 XIX）；tracing 结构化日志 + 贯穿管线 trace id；密钥脱敏（宪法 XX）。

**预留位置**：partition 分片（L6）；计费消费（L6）。

**证伪目标**：用量准确计量（流式 + 终态对账），指标标签正确，日志零密钥泄漏。

### L6 — 计费配额 + 控制台 + 多协议 + 规模化

**职责**：把网关变成完整 SaaS 平台。

- 计费配额：预算 / 配额 / 限流 / 价格 / 成本 / 用量归因（蓝图 15）
- 嵌入式控制台：Vite + React + React Router + React Query + shadcn/ui + Tailwind，build 后嵌入 Go 非内核（embed.FS）由 chi 托管（宪法 III/IV）；管理员视图 + 用户视图分离（蓝图 18）
- 多协议：Anthropic Messages / Gemini generateContent 透传 + SSE（蓝图 8）
- 规模化：多节点 + Redis（协调锁 / 限流 / 共享配额）+ Kafka（用量事件多消费者 / 回放），按触发条件引入（见 §5）
- 协议桥 / 上下文虚拟化（蓝图 10/11，更远期）

**证伪目标**：完整 SaaS 平台闭环——可计费、有 UI、多协议、可规模化。

## 3A. 鉴权架构（横切）

鉴权分两个维度，严格分离：**inbound**（客户端 -> 网关，证明调用者身份）与 **outbound**（网关 -> 上游，注入 provider key）。核心分界原则：**Go 管理生命周期（CRUD / 签发 / 撤销 / scope 配置），Rust 执行热路径校验（查快照 / 鉴权 / 注入）**。Go 拥有鉴权数据写权，Rust 只读 Go 推来的快照。

### Inbound 鉴权

| 职责 | Go 管理 | Rust 执行 |
|---|---|---|
| 虚拟 key 签发（生成明文返用户一次 + 哈希存 DB） | ✅ | ❌ |
| 虚拟 key 撤销 / 过期（更新 DB 状态 -> 推快照） | ✅ | ❌ |
| 虚拟 key scope 配置（model / provider / IP / 过期 / 并发配额 / owner） | ✅ | ❌ |
| 虚拟 key 哈希存储 | ✅ | ❌ |
| scope 校验（热路径，验 model / provider / IP / 过期） | ❌ | ✅ 内存快照，零 DB |
| 并发配额计数（在途流计数） | ❌ | ✅ 内存计数器，零 DB |
| 虚拟 key 校验（哈希比对查快照 -> Principal） | ❌ | ✅ KeyAuth::authenticate |
| RBAC authorize 边界（authorize(Proxy, route)） | ❌ | ✅ 热路径判定 |
| 静态网关 key（单机模式，从 pingogate-core.yaml 加载） | ❌（无 Go） | ✅ |
| OIDC / SAML 登录（控制台用户身份） | ✅ OAuth 流 + 会话 + 用户映射 | ❌ |
| service account 管理（本质是 owner kind = SA 的虚拟 key） | ✅ | ❌ |

### Outbound 上游认证

| 职责 | Go 管理 | Rust 执行 |
|---|---|---|
| BYOK provider key 录入（Go 收明文 -> 转发 Rust 加密） | ✅ 触发 | ✅ KeyVault 加密 |
| provider key 加密存储（Go 持密文 + 元数据，明文不落 Go/DB） | ❌（只持密文） | ✅ KeyVault 加密 |
| provider key 解密（热路径，KeyVault::decrypt -> SecretString） | ❌ | ✅ 独占 |
| 上游认证注入（upstream_request_filter，Bearer / x-api-key / query_key） | ❌ | ✅ 单点（宪法 VI） |
| provider key 元数据 CRUD（name / type / 状态 / created_by） | ✅ | ❌ |
| 1Password 可见性（明文仅 created_by，Go 校验 created_by 后调 Rust 解密） | ✅ 校验 created_by | ✅ 解密 |
| provider key 轮换 | ✅ 触发 | ✅ 重新加密 |
| 静态 provider key（单机模式，env:VAR 引用） | ❌（无 Go） | ✅ |

### KeyVault 跨边协作（最微妙）

```text
BYOK provider key 录入：用户 -> Go API -> Rust KeyVault::encrypt(明文) -> 密文回 Go -> Go 存 DB
BYOK provider key 查看明文（仅 created_by）：created_by -> Go 校验 -> Rust KeyVault::decrypt(密文) -> 明文回 Go -> 回用户（一次性）
```

**Go 持密文 + 元数据 + created_by 校验权；Rust 持加解密能力 + 主密钥**。Go 决定「谁能看」（1Password 规则校验 created_by），Rust 决定「怎么解」。明文不落 Go、不落 DB。录入时 Go 短暂接触明文（转发给 Rust 加密），可接受（前端只连 Go，不能为加密让客户端直连 Rust）。

### 鉴权模式归并

蓝图 16.1 列 8 种鉴权模式，在平台架构归并：

| 归并后 | 蓝图原模式 | 实现层 | 何时 |
|---|---|---|---|
| 静态网关 key | no-auth + static gateway key | Rust 单机模式 | M0 |
| 虚拟 key（平台唯一 inbound 模式） | virtual API key + tenant/project scoped + service accounts + BYOK user-scoped | Go 签发 + Rust 校验 | L2 |
| 企业身份登录 | OIDC / SAML / LDAP | Go 控制台登录 | L6（宪法 XX 预留） |
| ~~upstream key pass-through~~ | upstream key pass-through | 不实现 | 砍（违 BYOK 可见性，客户端知 provider key 破坏隔离） |

**关键决策**：虚拟 key 是平台模式唯一 inbound 鉴权模式--scoped / tenant / service-account / BYOK 都是虚拟 key 的**属性**（scope / owner kind），不是独立模式。OIDC 只管控制台登录，不直接管 API 调用（登录后 Go 签发虚拟 key 给用户用）。

## 3B. 协议转换架构（横切）

PingoGate 的核心差异点（蓝图第 10/11 章）。三层分工，严格按 Go/Rust 分界：

### 三层分工

| 层 | 职责 | 归属 | 何时 |
|---|---|---|---|
| **无状态转换引擎** | body 改写（如 OpenAI `include_usage` 注入）、协议桥（Responses->Messages 语义转换）、Capability IR 转换 | Rust 内核（宪法 VI） | L3 起 |
| **有状态 materialize** | 上下文桥：查 ConversationTimeline + materialize 成完整 messages/contents/input items | Go 非内核 | L3.5 |
| **转换规则配置** | marketplace 可视化 schema 匹配，配置转换/materialize 规则 | Go + React | L6 |

### Capability IR 架构（蓝图 10 推荐）

不把每对协议做成独立大 converter，用中间 IR：

```text
Client Protocol Emulator -> ConversationTimeline -> Capability IR -> Upstream Materializer
```

- **Client Protocol Emulator**：识别客户端协议（OpenAI Responses / Gemini Interactions / 等），Rust。
- **ConversationTimeline**：会话历史（用户回合 / 工具调用 / 工具结果 / assistant 输出），Go 存储。
- **Capability IR**：协议无关的中间表示，Rust 转换引擎产出 / 消费。
- **Upstream Materializer**：IR -> 目标 provider 协议（messages / contents / input items），有状态 materialize 在 Go，无状态转换在 Rust。

### 桥接路径（蓝图 10.3，9 条，分阶段）

有状态桥（经 Go materialize，依赖 ConversationTimeline）：
- OpenAI Responses -> Chat Completions / Anthropic Messages / Gemini
- Google Interactions -> Chat Completions / generateContent / Anthropic

无状态桥（Rust 转换引擎）：
- Chat Completions -> Responses
- generateContent -> Interactions
- Claude-compatible -> OpenAI-compatible / Gemini-compatible

### 与 marketplace 的关系

模型 schema 经常变（蓝图第 23 章已确认）。marketplace 存 schema 转换规则（Go+React 可视化匹配），驱动：
- Rust 无状态转换引擎（规则经快照推 Rust 执行）
- Go materialize（规则直接用）

Go 不硬编码协议，按 marketplace 配置的规则转换 / materialize。

## 3C. 计费与配额架构（横切）

SaaS 平台商业闭环。用量链路、计费模型、配额机制分清。

### 用量链路（Rust 提取 -> Go 估算/落库/计费）

```text
Rust 内核（热路径）
  ├─ 提取 provider 响应现成 usage（OpenAI include_usage / Anthropic message_delta / Gemini usageMetadata）
  ├─ 无 usage 时推 body 给 Go 估算
  ├─ 失败/中断推已转发部分给 Go 估算
  └─ 推 usage 事件（数字或 body）给 Go
       ↓
Go 非内核
  ├─ 估算（无 usage / 失败时，用 tokenizer，各 provider 算法不同）
  ├─ 聚合 + 落库（可插拔 sink：SQL / 未来 Redis / Kafka）
  ├─ 计费（按 model ratio / completion ratio / cache billing 算成本）
  └─ 配额扣减（读 Rust 推来的用量，扣 DB 配额）
```

Rust 零 DB、不带 tokenizer；Go 管估算/落库/计费/配额。

### 计费模型（蓝图 15 + New API 20 对齐）

- **model ratio / completion ratio / group ratio pricing**：不同模型不同价比，input/output 分计，组别比率（New API 模式）。
- **cache billing**：prompt cache 命中如何计费（Anthropic prompt cache / Gemini cachedContent / OpenAI 自动缓存，命中 token 按折扣计）。
- **subscription / top-up billing**：订阅 + 充值双模式，payment integration 预留（L6 远期）。
- **成本归因**：按 user / project / provider / model 归因成本与用量。

### 配额机制（运行时 vs 配置分离）

| | Go | Rust |
|---|---|---|
| 配额配置（预算 / 限流上限 / 并发上限） | ✅ DB + 推快照 | ❌ |
| 配额计数（运行时，已用量 / 在途并发） | ❌ | ✅ 内存（热路径零 DB） |
| 配额扣减（落账） | ✅ 收 Rust 推来的用量扣 DB | ❌ |
| 限流判定（热路径，超限拒绝） | ❌ | ✅ 查内存计数器 |

**运行时配额计数在 Rust 内存（热路径零 DB），配置与扣减在 Go**。Rust 每请求查内存计数器判限流，Go 收用量事件扣 DB 配额。

### 分层可观测（蓝图 15 八子类）

请求指标 / token 指标 / 成本指标 / 缓存指标 / realtime 指标 / batch 指标 / 网关指标 / 指标 sink。L5 起逐步覆盖（realtime/batch 随其能力域）。

## 3D. 远期能力域预留（蓝图 12/13/14，架构留位置不急实现）

以下能力域蓝图标注「倾向延后但架构必须预留位置」。路线图留位置，不编排具体层，待核心（协议转换 / 计费 / 多租户）稳后独立 spec：

- **Realtime / Live**（蓝图 12）：OpenAI Realtime / Gemini Live，WebSocket / WebRTC 双向长连接，会话生命周期。需独立管线（非 request/response），realtime.live 能力族。
- **文件 / 媒体 / 对象存储**（蓝图 13）：PingoGateFileRef，跨 provider 文件 materialization。被 Responses tools / multimodal / batch 依赖。file.media 能力族。
- **Batch Jobs**（蓝图 14）：OpenAI Batches / Gemini Batch / Anthropic Message Batches，job model。batch 能力族。

## 3E. 网关基础设施（执行 Rust / 配置 Go 分界）

蓝图 17 的网关基础设施按「执行在 Rust 热路径 / 配置在 Go」分界（与鉴权 / 配额同模式）：

| 类型 | 内容 | 归属 |
|---|---|---|
| **数据面基础设施（执行）** | TLS 终止 / HTTP2 / SNI / 上游 TLS 校验 / 重试 / fallback / 熔断 / 健康检查 / 请求大小限制 | Rust 内核（Pingora 热路径，宪法 III） |
| **管理面基础设施（配置）** | 证书文件管理 / ACME 自动化 / IP allow-deny 规则 / mTLS CA 与策略 / CORS 规则 | Go 非内核（CRUD + 推快照） |

- TLS 终止 / HTTP2 / SNI：Pingora 自带，Rust 内核。
- 上游 TLS 校验：Rust（upstream_peer，001 已实现）。
- 重试 / fallback / 熔断 / 健康检查：Rust 路由引擎（热路径决策，内存）。
- 证书 / ACME / IP 规则 / CORS / mTLS 策略：Go 配置，Rust 执行（热路径 filter 读快照）。
- L3 起逐步覆盖；ACME / mTLS / region-aware 路由等后置。

## 3F. 非核心 Provider 接口后置

蓝图 8 列了每个 provider 的完整 API 家族（OpenAI Responses/Conversations/Realtime/Files/Embeddings/Batches/Fine-tuning/Evals/Assistants...；Gemini Interactions/Live/Batch/Files...；Anthropic Messages/Batches/Files/MCP...）。本路线图核心覆盖 5 个主接口（OpenAI Chat Completions + Responses / Gemini generateContent + Interactions / Anthropic Messages，见 `docs/02-provider-schemas.md`）。**其余非核心接口后置**，随能力族（§3D / 宪法 XI）逐步支持，不预先编排。

## 3G. 已确认决策日志（蓝图 23 迁移）

| 决策 | 结论 |
|---|---|
| 配置格式 | 用户主配置 + proxy profile 用 YAML；marketplace catalog / 锁文件 / schema 用 JSON；TOML 不作核心 profile 格式 |
| 脚本引擎 | 放弃 Rhai；主路径用声明式 TransformPlan + Rust 编译计划；复杂第三方逻辑未来优先签名 WASM |
| 仓库与分发 | 单仓库（`proto/` / `core-rs/` / `ctrl-go/` / `console/` / `deploy/`）；GitHub 承担源码协作 + release 分发 + proxy/provider marketplace |
| 控制台 | 嵌入式 TypeScript 控制台（React + Vite + React Router + React Query + shadcn/ui + Tailwind），build 后 embed.FS 嵌入 **Go 非内核**，chi 托管（非 Rust）。管理员视图 + 用户视图分离 |
| 嵌入式控制台时机 | L6 实现；L0-L5 仅机器可读 API + 身份边界 |
| 双语言架构 | 内核 Rust + 非内核 Go（对蓝图 3.2「纯 Rust」的有意偏离，理由见宪法总纲） |
| Responses/Interactions 状态模拟 | 跨协议转化时必须存 ConversationTimeline（见宪法 XX）；同态透传不存完整内容 |
| 跨协议转换 / 上下文桥 / realtime / batch / files | 倾向延后但架构预留位置（§3B/§3D） |

LiteLLM（蓝图 19）/ New API（蓝图 20）对齐清单作为产品覆盖度对标参考，非自研需求清单；New API 是 Go 项目可随时参照。

## 4. 横切约束（每层都适用）

| 约束 | 来源 | 说明 |
|---|---|---|
| 内核 Rust + 非内核 Go | 宪法总纲 | 双二进制；请求处理链路+安全核心=Rust，业务治理=Go |
| 热路径必须 Pingora | 宪法 III | L3 起触发（Rust 内核）；L0-L2 无热路径 |
| Go 非内核用 net/http+chi+sqlx | 宪法 III | L0 起控制面；SQLite/PG 可切；热路径零 DB |
| 跨语言 gRPC + proto | 宪法 III/IV | Go->Rust 快照推送；类型经 proto codegen 同步 |
| 状态三分 | 宪法 X | Capability Repo / Runtime Config / Control Plane State 分离 |
| 不可变快照 + ArcSwap | 宪法 XII | L3 起（Rust 内核）；热路径零 DB，仅读内存 |
| 密钥脱敏 + 加密 | 宪法 XX | 全程；KeyVault（Rust 内核）唯一解密入口，Go 只持密文 |
| authorize 边界 | 宪法 XX | 全程；无全局 token 等值 |
| YAGNI | 宪法 II | 层间只预留位置不预留框架 |
| TDD | 宪法 XIII | 每层契约 / 集成 / 单元测试先行；gRPC 接口双侧契约测试 |

## 5. 中间件引入触发条件（不预设时间表）

外部中间件按**触发条件**引入，而非按计划。避免过早引入违背蓝图 3.1（单二进制优先，外部依赖是增强项非强制）。

### Kafka

**触发条件（同时满足）**：
1. 多节点部署（单实例扛不住）
2. 用量事件要喂给多个独立消费者（计费服务 / 数据仓库 / 审计 / 实时分析）
3. 需要事件回放（重放历史用量重算账）

未同时满足前，用 Rust 原生 tokio 通道批量 sink。partition 思想 v1 就用（按 partition_key 内存分片），Kafka 本体是未来可插拔 sink 实现之一。

### Redis

**触发条件**：
- 多节点需共享限流配额 / 分布式锁 / 共享配额计数（单实例用内存即可）

### Postgres

**触发条件**：
- SQLite 单机写入瓶颈，或多实例需共享控制面状态（单实例 SQLite 足够，sea-orm 保证可切换）

## 6. IoT Hub 类比边界（设计参考）

控制面骨架与 IoT Hub 高度同构，可大量借鉴成熟模式：

| IoT Hub 模式 | 迁移到 PingoGate | 落点 |
|---|---|---|
| SAS token | 虚拟 key（可撤销 / scoped / 哈希存） | L2 |
| device twin | policy 影子（desired/reported 驱动运行时） | L3/L4 |
| telemetry pipeline | UsageSink 批量 sink 骨架 | L5 |
| 设备生命周期状态机 | key 生命周期（注册/启用/禁用/轮换/删除） | L1/L2 |
| 连接表 + 并发配额 | 每虚拟 key 最大并发 SSE 流 + 背压 | L3/L6 |

**不可照搬**（LLM 独有或不适用的 IoT 模式）：
- 设备在线 / 离线状态机（LLM 无状态请求不需要）
- 海量空闲长连接管理（LLM 流结束即断）
- 接入协议层转换 MQTT<->AMQP（LLM 转换在语义层，非接入层）
- 设备 fan-in 聚合（LLM 是 fan-out 到不同上游）
- **token 流式计量 + 终态对账**（IoT 按消息/字节边界清晰，LLM token 增量产生、终态确定、中断要部分记）——需独立设计
- **BYOK 双向身份**（IoT 设备是接入方、后端凭据中心统管；PingoGate 用户既是接入方又是后端凭据提供者）——IoT 无对应物，需独立设计

## 7. 与宪法的关系

- **产品愿景**：原蓝图（已删除）的内容已全部迁移至本路线图（L0-L6 + §3A-§3G）与宪法。本路线图是产品愿景的分阶段实施编排，能力一项不丢失——要么在某层实现，要么显式划为更远期。
- **宪法**（`.claude/rules/constitution.md`）：上位约束。本路线图每层的横切约束（§4）源自宪法，不偏离。
- **feature spec**：每个 L 层是一个独立 feature spec，走自己的 spec → plan → 实现 → 验证。本路线图是各 feature spec 的编排依据，不是 feature spec 本身。

## 8. 当前状态

- **当前轮次**：M0（内核单机网关）作为首个 feature spec，brainstorming / spec 编写中。数据面复用 001 已验证实现（移植到 `core-rs/`，做双模式 + 虚拟key 适配）。
- **已完成**：仓库重置到 `dev` 分支（orphan，仅设计资产）；宪法定局双语言内核架构（Rust 内核 + Go 非内核，双运行模式）；路线图 L0-L6 + M0-M4；001 资产评估（数据面可借鉴）。
- **归档**：001-gateway-foundation（路由优先单管理员代理）在 `redesign` 分支，仅作历史参考。
