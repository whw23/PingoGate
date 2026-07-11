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

- **Rust 内核**（`core-rs/`，`pingogate-core` 二进制）：请求处理核心引擎 + 安全核心。Pingora 数据面管线（协议识别 / 鉴权 / 请求路由 / 请求转换 / 透传 / SSE）+ 路由引擎 + 转换引擎 + KeyVault（密钥加解密）+ 快照引擎。性能 / 正确性敏感、稳定少变。
- **Go 非内核**（`ctrl-go/`，`pingogate-ctrl` 二进制）：业务面 + 平台治理。控制面 API（用户 / 组织 / RBAC / 虚拟 key CRUD）+ OAuth / 会话 + 计费 / 用量 / 审计 + 控制台 BFF + DB。迭代频繁、生态依赖。
- **通信**：Go 经 gRPC 把配置 / 密钥 / 虚拟 key 作为快照推给 Rust 内核；Rust KeyVault 解密后存内存快照，热路径零 DB 零解密。跨语言类型经 `proto/` codegen 同步。
- **部署**：双二进制，可单 Docker 镜像。

**分界原则**：请求从进到出的整条处理链路（含路由 / 转换）+ 安全核心 = Rust；围绕这条链路的配置 / 治理 / 业务 = Go。

各能力层的主语言：L0-L2 控制面 = Go（L1 的 KeyVault 加解密在 Rust 内核，Go 经 gRPC 调）；L3 数据面 = Rust；L4-L6 治理 / 用量 / 计费 = Go（用量事件由 Rust 热路径上报，Go 聚合）。

## 2. 能力层分解

平台按依赖序拆为 L0-L6 七层。每层是一个独立 sub-spec（各自 spec → plan → 实现 → 验证）。**每层独立可交付、可验证、可用**。

```
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
L4 多租户：Tenant -> Project + RBAC + 组织 key 共享
  ↓ 有组织模型才有治理
L5 可观测 + 用量：UsageSink 流式计量 + 指标
  ↓ 有用量才有计费
L6 计费配额 + 控制台 + 多协议 + 规模化（Redis / Kafka）
```

### 里程碑

| 里程碑 | 包含层 | 价值 |
|---|---|---|
| **M1**：BYOK 密钥平台 | L0 + L1 | 一个能安全存 BYOK key 的多用户平台（已可用） |
| **M2**：BYOK 网关 | L0 + L1 + L2 + L3 | 能调模型的 BYOK 网关（真正 MVP） |
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

### L5 — 可观测 + 用量：流式计量与指标

**职责**：让流量可见、可计量，为计费铺路。

- `UsageSink` trait：热路径写内存通道 → 后台批量 flush → 可插拔后端（file / SQL / 未来 Redis / Kafka）
- token 流式计量：跟随 SSE 流增量累计，处理中断部分计量，区分 input/output/reasoning/cache token，终态对账（IoT 没有的 LLM 独有挑战）
- Prometheus 指标（按 provider + 能力族 + principal 打标签，宪法 XIX）
- tracing 结构化日志 + 贯穿管线 trace id
- 密钥脱敏（宪法 XX）

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

## 7. 与蓝图、宪法的关系

- **蓝图**（`docs/blueprint/`）：完整产品愿景。本路线图是蓝图的**分阶段实施编排**，把蓝图的候选能力分组落到有序的 L0-L6。蓝图内容一项不丢失——要么在某层实现，要么显式划为更远期。
- **宪法**（`.claude/rules/constitution.md`）：上位约束。本路线图每层的横切约束（§4）源自宪法，不偏离。
- **feature spec**：每个 L 层是一个独立 feature spec，走自己的 spec → plan → 实现 → 验证。本路线图是各 feature spec 的编排依据，不是 feature spec 本身。

## 8. 当前状态

- **当前轮次**：L0 + L1（M1 里程碑）的 brainstorming / spec 编写中。
- **已完成**：仓库重置到 `dev` 分支（orphan，仅设计资产）；宪法 III 微调（承认管理面 axum + 控制面 sea-orm）。
- **归档**：001-gateway-foundation（路由优先单管理员代理）在 `redesign` 分支，仅作历史参考。
