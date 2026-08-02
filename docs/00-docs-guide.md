# PingoGate 项目文档导读

> **日期**：2026-07-27
> **定位**：本文档说明 `docs/` 下各文档的用途、内容、使用方式，帮助新成员快速理解项目架构与文档体系。

## 文档总览

```
docs/
├── 00-docs-guide.md              ← 本文件（导读）
├── 01-platform-roadmap.md         ← 平台路线图（L0-L6 能力层 + 里程碑 + 横切架构）
├── 02-provider-schemas.md         ← 五接口 schema 调研（协议/流式/上下文/usage 差异）
└── superpowers/
    └── specs/
        └── 2026-07-12-m0-m1-kernel-and-byok-platform-design.md  ← M0+M1 合并 spec
```

另有 `.claude/rules/constitution.md`（项目宪法，对所有 feature 具上位约束力）和 `docs/blueprint/`（已删除，内容迁移到路线图与宪法）。

---

## 01-platform-roadmap.md：平台路线图

### 是什么

PingoGate SaaS 平台的**顶层架构分解**。把"多用户 BYOK 优先的 LLM 流量治理 SaaS 平台"这个大目标，拆成 7 个有依赖序的能力层（L0-L6），每层是独立可交付的闭环。它是后续每个 feature spec 的编排依据。

### 怎么用

- **启动新 feature**：查路线图找当前该做哪层，开新 feature spec（走 spec -> plan -> 实现 -> 验证）
- **理解架构**：看 §双语言内核架构总纲（Rust 内核 vs Go 非内核分界）、§2 能力层分解（L0-L6 依赖序）、§3 各层详述
- **做架构决策**：查 §3A 鉴权架构（inbound/outbound 分离 + Go/Rust 职责表）、§3B 协议转换架构（三层分工）、§3C 计费与配额架构
- **判断中间件引入时机**：查 §5 中间件触发条件（Kafka/Redis/Postgres 按条件引入，不预设时间表）
- **借鉴 IoT Hub**：查 §6 IoT Hub 类比边界（可借鉴 vs 不可照搬）

### 核心内容

#### 双语言内核架构（总纲）

| | Rust 内核（`core-rs/`） | Go 非内核（`ctrl-go/`） |
|---|---|---|
| **定位** | 无状态请求处理核心引擎 + 安全核心 | 所有状态 + 业务面 + 平台治理 |
| **包含** | Pingora 数据面 / 路由 / 转换 / KeyVault 加解密 / 快照引擎 / 用量采集 | 控制面 API / 用户/RBAC / 上下文桥 / 用量落库 / 计费 / 控制台 BFF / DB |
| **状态** | 零 DB、零持久状态 | 管所有 DB |
| **通信** | ← gRPC 推快照（Go -> Rust） | → 推快照 / 调 KeyVault 加解密 |

#### L0-L6 能力层与里程碑

| 层 | 内容 | 里程碑 |
|---|---|---|
| L0 | 身份与边界（User + Principal/authorize） | M1 |
| L1 | BYOK 密钥（Provider Key 加密保管 + 1Password 可见性） | M1 |
| L2 | 虚拟 key（签发/scoped/可撤销） | M2 |
| L3 | 数据面骨架（Pingora + 六接口透传） | M0/M2 |
| L3.5 | 上下文桥（ConversationTimeline + ContextMaterializer） | 后置 |
| L4 | 多租户（Tenant/Project/RBAC + 组织 key 共享） | M3 |
| L5 | 可观测 + 用量（流式计量 + 指标） | M3 |
| L6 | 计费配额 + 控制台 + 多协议 + 规模化 | M4 |

**当前状态**：M0+M1 已完成（36 task，12/12 SC pass）。下一是 L4 多租户。

#### 横切架构（§3A/3B/3C）

- **§3A 鉴权架构**：inbound（客户端->网关）vs outbound（网关->上游）严格分离。Go 管生命周期（签发/撤销/配置），Rust 执行热路径校验（查快照/鉴权/注入）。KeyVault 跨边协作：Go 持密文+created_by 校验权，Rust 持加解密+主密钥。1Password 双防线（L1 Go created_by + L2 Rust owner 独立拦截 + L3 审计）。
- **§3B 协议转换架构**：三层分工--Rust 无状态转换引擎（body 改写/协议桥）、Go 有状态 materialize（上下文桥）、Go+React marketplace 规则配置。Capability IR 架构（不把每对协议做成独立 converter）。
- **§3C 计费与配额架构**：用量链路（Rust 提取现成 usage -> Go 估算/落库/计费）。配额运行时计数在 Rust 内存（热路径零 DB），配置与扣减在 Go。

---

## 02-provider-schemas.md：五接口 Schema 调研

### 是什么

五个 LLM provider 核心接口的**完整 schema 调研**：请求/响应/失败 schema + header + body + 模型差异 + 上下文机制 + 流式形态 + 官方 $schema 状态。基于官方文档 + 已知 API 知识，Gemini Interactions 经 WebFetch 查证。

### 怎么用

- **写 Rust 协议识别**：查 §0 关键差异速查表（路径后缀模式匹配，版本前缀通配）+ §0 关键点 4（版本前缀可变）
- **实现 usage 提取**：查各接口的 "usage 位置"（OpenAI 末块需 include_usage / Anthropic 两处合并 / Gemini 末块 usageMetadata）
- **设计上下文桥**：查各接口的"上下文机制"（只有 Responses/Interactions 有状态，需 Go materialize；其余无状态直入 Rust）
- **处理流式差异**：查 §0 关键点 1（Gemini 双接口 vs 其他单接口）+ §4 Gemini `?alt=sse` 陷阱
- **设计 marketplace schema 转换**：查各接口 body 字段 + 模型差异（不同模型参数集不同）

### 核心内容

#### 六接口速查（§0）

| 维度 | OpenAI Chat | OpenAI Responses | Gemini generateContent | Gemini streamGenerateContent | Gemini Interactions | Anthropic Messages |
|---|---|---|---|---|---|---|
| 端点 | `{ver}/chat/completions` | `{ver}/responses` | `{ver}/models/{m}:generateContent` | `{ver}/models/{m}:streamGenerateContent` | `interactions.create` | `{ver}/messages` |
| 流式 | 单接口+stream:true | 单接口+stream:true | 独立接口（非流式） | **独立接口（流式）** | 单接口 | 单接口+stream:true |
| 认证 | Bearer | Bearer | x-goog-api-key/?key= | 同左 | API key | x-api-key+anthropic-version |
| 上下文 | 无状态（完整 messages） | **有状态**（previous_response_id） | 无状态（完整 contents） | 同左 | **有状态**（previous_interaction_id） | 无状态（完整 messages） |
| usage | 末块（需 include_usage） | response.completed | body usageMetadata | 末块 usageMetadata | 末块 | message_start+message_delta |

#### 四个关键差异（影响内核设计）

1. **Gemini 双接口**：`:generateContent` vs `:streamGenerateContent` 是两个独立 path action；其他 provider 单接口 + body `stream:true`
2. **有状态协议**：只有 OpenAI Responses 和 Gemini Interactions 带 `previous_id`，需 Go 上下文桥 materialize
3. **OpenAI Chat 流式无 usage**：需 Rust 转换引擎注入 `stream_options.include_usage:true`
4. **版本前缀可变**：`{ver}` 非固定（OpenAI/Anthropic `/v1/`，Gemini `/v1beta/`），协议识别按路径后缀匹配，不硬编码版本前缀

#### 各接口详述（§1-6）

每个接口包含：
- **官方 $schema 状态**：OpenAI 有 OpenAPI 仓库；Gemini 有 Discovery 文档；Anthropic 文档为准；Interactions API ref 待补
- **请求**：端点 + headers + body 顶层字段 + 流式触发 + 上下文机制
- **响应**：成功 body 字段 + 流式 events + usage 位置
- **失败**：错误体形状 + 状态码
- **模型差异**：reasoning 模型（o1/o3 thinking Effort）、多模态、不同参数集
- **上下文缓存**：OpenAI 自动缓存 / Anthropic prompt_cache_control / Gemini cachedContent

#### 对内核设计的影响（§7）

- **协议识别**：路径后缀匹配 + 版本前缀通配（T7 实现）
- **usage 提取**：各接口位置不同（T27 inline 事件驱动解析，O(1) 内存）
- **上下文桥**：有状态协议经 Go materialize（L3.5），无状态直入 Rust
- **marketplace schema**：模型 schema 常变，marketplace 存转换规则驱动 Rust + Go

---

## .claude/rules/constitution.md：项目宪法

### 是什么

PingoGate 的**上位约束文档**。22 条原则（I~XXII）+ 双语言内核架构总纲。对所有 feature 的 spec / plan / 实现具上位约束力。从 001 specs 引用中提取重建（v1.6.0 转述），后据 BYOK SaaS 方向整理（分层标注 + 目录重构 + 补 BYOK 安全 + 双语言内核）。

### 怎么用

- **写 spec 前**：查宪法的适用层图例（横切 / L0+ / L3+热路径），确认哪些原则在当前层强制
- **做技术决策**：查 III 技术栈锁定（Rust 内核 Pingora / Go 非内核 net/http+chi+sqlx）、IV 目录结构（10 crate + Go package）、VI 请求管线（Pingora ProxyHttp filter 映射）
- **审查安全**：查 XX 安全/保密/隐私（BYOK 1Password 可见性 + KeyVault 跨边 + authorize 边界 + TLS）
- **判断性能预算**：查 XXI 性能（p50<5ms/p95<20ms + 零拷贝 SSE + 热路径零 DB）

---

## superpowers/specs/：Feature Spec

### 是什么

每个 feature 的设计文档（spec），走 spec -> plan -> 实现 -> 验证流程。当前有 M0+M1 合并 spec（已实现完成）。

### 怎么用

- **理解 M0+M1 实现**：读 spec 的 §1 范围 + §2 架构 + §11 实现分期 + §12 成功标准 + §14 实现偏差 + Final Review Conclusion
- **启动新 feature**：用 brainstorming skill 产出新 spec，保存到此目录
- **查实现偏差**：spec §14 记录了 7 个关键偏差（404 vs 403 / SHA-256 vs bcrypt / SecretString Drop / 平台数据面 wiring / Mutex->Semaphore / MKEK UTF-8 / 等）

---

## 文档间关系

```
宪法（.claude/rules/constitution.md）
  ↓ 上位约束
路线图（docs/01-platform-roadmap.md）
  ↓ 编排依据
Feature Spec（docs/superpowers/specs/）
  ↓ 调研依据
Provider Schema 调研（docs/02-provider-schemas.md）
  ↓ 实现依据
Plans（docs/superpowers/plans/）
  ↓ 执行
代码（core-rs/ + ctrl-go/ + proto/）
```

- **宪法**约束所有层（技术栈/目录/管线/安全/性能）
- **路线图**编排层序（L0-L6）+ 横切架构（鉴权/协议转换/计费）
- **Schema 调研**为 Rust 协议识别/usage 提取/上下文桥提供接口级依据
- **Spec** 定义单个 feature 的范围/架构/成功标准
- **Plan** 把 spec 拆成可执行的 TDD task

---

## 当前项目状态（2026-07-27）

- **M0+M1 已完成**：36 task，12/12 SC pass，dev 分支
- **下一**：L4 多租户（Tenant/Project/RBAC + 组织 key 共享）
- **代码规模**：184 文件，Rust 12,495 行 + Go 8,826 行
- **已知 follow-up**：Go snapshot builder 推 routes、gRPC liveness->readiness、T16 streaming test、跨 runtime tonic 集成测试
