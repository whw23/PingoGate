# PingoGate 项目宪法（Project Constitution）

> **来源与保真说明**：本宪法从 `001-gateway-foundation` feature 的 `plan.md` / `research.md` / `spec.md` 对宪法的引用中提取重建，对应宪法原 v1.6.0；后据 BYOK SaaS 平台方向整理（分层标注 + 目录重构 + 补 BYOK 安全 + 双语言内核架构）。内容为原则转述，**非逐字原文**。如与宪法源文件有出入，以宪法源文件为准并回写本文件。
>
> 本文件作为项目级 rules 放置在 `.claude/rules/`，对 PingoGate 的所有 AI 协作与代码实现具有**上位约束力**。新 feature 的 spec / plan / 实现 MUST「以宪法为准」。

## 双语言内核架构（总纲）

PingoGate 采用**内核 Rust + 非内核 Go** 双语言架构，双二进制：

- **Rust 内核**（`pingogate-core` 二进制）：**无状态**请求处理核心引擎 + 安全核心 + 用量提取。性能 / 正确性敏感、稳定少变、不可替代。包括：Pingora 数据面管线（协议识别 / 鉴权 / 请求路由 / 请求转换 / 上游认证注入 / 响应转换 / SSE 零拷贝透传 / 错误镜像）、路由引擎（别名 / 权重 / fallback / 健康检查）、转换引擎（无状态 body 改写 / 协议桥，不含 materialize）、KeyVault（密钥加解密，安全核心）、快照引擎（RuntimeSnapshot + ArcSwap，内存）、**用量提取（只提取 provider 响应中现成的 usage 字段，不做估算、不带 tokenizer）**。**Rust 内核零 DB、零持久状态、不带 tokenizer**。
- **Go 非内核**（`pingogate-ctrl` 二进制）：**所有状态** + 业务面 + 平台治理。迭代频繁、生态依赖。包括：控制面 API（用户 / 组织 / RBAC / 虚拟 key CRUD）、OAuth / 会话、**上下文桥（ConversationTimeline 存储 + ContextMaterializer，处理带 previous_response_id / previous_interaction_id 的有状态请求）**、**用量估算（无 usage 时用 tokenizer 估算；失败 / 中断部分计量）+ 用量聚合 / 落库 / 查询 / 计费 / 配额 / 限流规则**、审计、控制台后端 BFF、管理面（健康 / 就绪 / reload 编排）、DB。
- **通信**：Go 非内核经 gRPC 把「配置 / 密钥 / 虚拟 key / 计费规则」作为快照推给 Rust 内核；Rust 内核 KeyVault 解密后存内存快照，热路径零 DB 零解密。Rust 提取现成 usage 数字推 Go；无 usage 时 Rust 推 body 给 Go 估算（复用上下文 materialize 的 body 或专门推）。带上下文请求由 Go materialize 后推给 Rust 透传。**Rust 内核零 DB 访问、不带 tokenizer**。跨语言类型经 protobuf codegen 同步。
- **部署**：双二进制，可打包进单个 Docker 镜像。单仓库（`proto/` / `core-rs/` / `ctrl-go/` / `console/` / `deploy/` / `docs/` / `.claude/` 共存），因 proto 为 Rust / Go 共享根，跨语言原子提交。
- **双运行模式**：Rust 内核支持两种部署形态，同一二进制渐进增强：
  - **单机模式（standalone）**：内核单独运行，从 `pingogate-core.yaml` 文件加载路由 / 上游 / 静态网关 key，provider key 用密钥引用（`env:VAR`，不加密，单用户无隔离需求），无 Go、无 DB。一个可用的轻量 LLM 网关（对标蓝图 3.1 单二进制优先）。KeyVault 不启用。
  - **平台模式（platform）**：内核 + Go 非内核。Go 经 gRPC 推快照（含虚拟 key / 加密 provider key / 计费规则），KeyVault 解密后存内存快照，多用户 BYOK 隔离 + 用量计量 + 计费。完整 SaaS 平台。
  - 单机模式是内核最小可用形态；平台模式是增强。两种模式共享同一数据面管线 / 路由引擎 / 转换引擎，仅快照来源与鉴权模式不同。

**分界原则**：**Rust 内核 = 无状态计算**（请求处理链路：透传 / 路由 / 无状态转换 / 安全 / 采集；零 DB 零持久状态）；**Go 非内核 = 所有状态**（配置 / 用户 / 上下文 ConversationTimeline / 用量落库 / 计费 / 审计；管 DB）。例外：单机模式（见下）Rust 从文件加载快照，仍无 DB。

**双语言决策理由（对蓝图 3.2「后端只用 Rust」的有意偏离）**：蓝图 3.2 原定后端纯 Rust，但经评估（详见决策记录），采用内核 Rust + 非内核 Go 更优：Rust 守「绝对不能错且性能敏感」的请求处理核心 + 安全核心（Pingora 热路径 / KeyVault 加解密 / 转换引擎，蓝图 3.5/10/11 本就把请求管线视为核心差异点），Go 拿「迭代频繁且生态依赖」的业务面 + 所有状态（SaaS 治理 / 计费 / 上下文 timeline / 控制台 BFF）。对标 New API/LiteLLM 级性能（Go 数据面够），IoT Hub 工业实践印证 GC 语言扛高吞吐中心化网关（Magistrala 纯 Go 生产级）。vibecoding 抹平语言熟练度差异。单二进制非硬约束，接受双进程。蓝图 3.2 的「纯 Rust」诉求由「内核 Rust」承担（核心仍 Rust），非内核 Go 不违背蓝图精神（蓝图 3.2 意在「敏感操作留 Rust」，已在内核满足）。

## 适用层图例

平台按 `docs/platform-roadmap.md` 的 L0-L6 能力层分阶段建设。每条原则标注适用层：

- **横切**：所有层强制（含 L0 控制面地基）。
- **L0+**：控制面层起强制（L0 身份 / L1 密钥 / L2 虚拟key / L4 多租户 / L5 用量 / L6 计费控制台）。
- **L3+热路径**：数据面热路径起强制（L3 Pingora 管线 / L5 用量计量 / L6 规模化）。L0-L2 无热路径，这些原则暂不触发，但实现时不得为后续埋坑。

---

## I. 复杂度层级判定【横切】
每个 feature 在 plan 阶段判定复杂度层级（简单 / 中等 / 复杂），并按层级强制对应下述原则的闸门要求。判定依据：新增模块数量、跨模块耦合、条件业务逻辑、是否涉及数据面管线 / 上下文虚拟化 / 多租户控制面 / 跨语言边界等。中等及以上强制全部适用原则；简单可降级部分原则的闸门要求。Complexity Tracking 记录所有违规与豁免理由。

## II. DRY / KISS / YAGNI【横切】
- 强制。范围严格限定在 spec 的 FR；不投机抽象；延后能力不预先建框架。
- **不为实现未排期能力预留框架，只预留「位置」**（trait / interface 边界、目录层、配置字段、枚举变体）。
- 路线图（`docs/platform-roadmap.md`）未到的能力层模块不建空壳；到了再建。

## III. 技术栈锁定【横切】
- **Rust 内核**：100% Safe Rust（`unsafe` 禁止除非显式评审）。热路径 MUST 使用 Pingora（`pingora` / `pingora-proxy` / `pingora-core`）。异步运行时 tokio；序列化 serde；日志 tracing；指标 Prometheus / OpenTelemetry。密钥加解密用 Rust 标准加密库（AES-GCM）。
- **Go 非内核**：Go（最新稳定版）。Web 用 net/http + chi（标准库兼容）；存储用 sqlx（薄 SQL 层，SQLite / Postgres 可切换，落宪法 X）；日志用 slog；指标用 prometheus/client_go；OAuth / 会话用成熟 crate。优先复用成熟库而非手搓。
- **跨语言通信**：gRPC（protobuf codegen 同步 Rust / Go 间的域类型）。Go -> Rust 快照推送用 gRPC stream。
- 依赖最小化：新增非锁定 crate / Go module 须在实现 PR 附理由。
- 前端仅 TypeScript（嵌入式控制台），所有权限 / 校验 / 策略 / 敏感操作留在 Rust 内核或 Go 非内核的后端，前端只是客户端。

## IV. 目录结构【横切】
顶层目录：
- `core-rs/`：Rust 内核 Cargo workspace（`pingogate-core` 二进制）。
- `ctrl-go/`：Go 非内核 module（`pingogate-ctrl` 二进制）。
- `proto/`：跨语言 protobuf 定义（Rust / Go 共享域类型 codegen）。
- `console/`：嵌入式控制台前端（TypeScript / React，L6 实现，L0-L5 仅预留空目录）。
- `deploy/`：部署配置（Dockerfile / K8s，双二进制打包）。
- `scripts/`：辅助脚本。
- `docs/`：文档（蓝图 / 路线图 / feature specs）。
- `.claude/`：AI 协作规则（本宪法 / skills 配置）。

`core-rs/` 内部按能力层组织 crate：`core`（共享域类型 / 错误 / 身份边界）、`storage`（快照 / KeyVault 加解密）、`pipeline`（Pingora 管线）、`router`（路由引擎）、`transform`（转换引擎）、`provider`（adapter + 能力族）、`listener`（server / listener 装配）、`snapshot`（RuntimeSnapshot + ArcSwap）、`pingogate-core`（主二进制：bootstrap / 信号 / 装配）。依赖方向自底向上无环，`core` 不依赖任何内部 crate。

`ctrl-go/` 内部按 Go 约定组织：`cmd/pingogate-ctrl/`（主二进制）、`internal/identity/`（L0）、`internal/keymgmt/`（L1，调 Rust KeyVault 加解密）、`internal/vkey/`（L2）、`internal/tenant/`（L4）、`internal/usage/`（L5）、`internal/billing/`（L6）、`internal/admin/`（管理面 + 控制台 BFF）、`internal/storage/`（sqlx + 迁移）、`internal/proto/`（codegen）。

跨语言调用只经 `proto/` 定义的 gRPC 接口；Rust 内核与 Go 非内核不共享内存态，经快照推送通信。根目录作为 AI 协作壳；新功能不属于现有层级时先扩展 / 拆分定义层级。

## V. 代码指标【横切】
- Rust：文件 ≤ 300 行；函数 ≤ 50 行；参数 ≤ 4；圈复杂度 ≤ 10；嵌套 ≤ 3。
- Go：文件 ≤ 300 行；函数 ≤ 50 行；参数 ≤ 4；圈复杂度 ≤ 10；嵌套 ≤ 3。
- 在实现与 CI 强制（Rust clippy + Go staticcheck/revive）。

## VI. 请求管线化【L3+热路径，Rust 内核】
- 请求路径拆成稳定管线，各阶段映射到 Pingora `ProxyHttp` 的 filter 回调；Provider adapter 不拥有网关生命周期。
- 管线：入口 -> 协议识别 -> 鉴权授权 -> 上下文解析 -> 路由 -> 策略 -> 请求转换 -> 上游派发 -> 响应转换 -> 上下文提交 -> 可观测性 -> 用量计费。
- adapter 负责 Provider 命名空间、能力声明、Provider 特有转换，不负责把整个系统绑死。
- 鉴权在 `request_filter` 早期短路；上游认证在 `upstream_request_filter` 单点注入，adapter 不接触明文 Key 与 body / 连接。
- 路由引擎、转换引擎在 Rust 内核；路由 / 转换**规则**由 Go 非内核配置并经快照推给 Rust 执行（Rust 是执行引擎，规则配置在 Go）。
- **上下文桥（ContextMaterializer）归 Go**：带 `previous_response_id` / `previous_interaction_id` 的有状态请求，由 Go 查 ConversationTimeline + materialize 成完整 messages/contents/input items 后推给 Rust 透传。Rust 内核**无状态**，不持有会话历史。请求分流：无上下文请求（无 previous_id）直入 Rust，Go 不介数据路径；带上下文请求经 Go materialize 再推 Rust。
- **Rust「请求转换」**指无状态 body 改写（如 `include_usage` 注入）与协议桥（Responses->Messages 语义转换）；**materialize（状态衍生的请求构造）归 Go**，不在 Rust。
- **声明式 TransformPlan + Rust 编译计划**：转换规则用声明式描述（marketplace schema 规则），Rust 内核把声明式规则**编译成 Rust 执行计划**（运行时高效执行，非解释执行）。放弃 Rhai 作为默认扩展路线；复杂第三方逻辑未来优先考虑签名 WASM（安全边界清晰）。声明式规则由 Go+React marketplace 配置，经快照推 Rust 编译执行。
- L0-L2 控制面无此管线，走 Go net/http + chi 中间件链 + authorize 边界。

## VII. SOLID【横切】
- Rust 内核：adapter / authenticator / router / transform 等聚焦 trait；构造期注入；组合优先于继承。
- Go 非内核：store / service / handler 分层聚焦 interface；构造期注入；组合优先于继承。

## VIII. 低耦合高内聚【横切】
- Rust 内核：`core` 共享类型 / 错误 crate 打破循环依赖；crate 间仅经公共 trait / 类型通信。
- Go 非内核：`internal/` 按能力层分包，包间经 interface 通信。
- Rust 内核与 Go 非内核之间**只经 `proto/` gRPC 接口通信**，不共享内部类型 / 内存态。

## IX. 错误处理【横切 + L3+ 补充】
- Rust 内核：完整分类 `AppError`（Domain / Application / Infrastructure / Validation）。HTTP 状态码仅在边界映射；不在内部层泄漏 HTTP 语义。
- Go 非内核：标准 Go error + errors.Wrap；HTTP 状态码仅在 handler 边界映射。
- **L0-L2 控制面（Go）**：标准 REST 错误体（含错误码、消息、可选详情），经 authorize 边界返回。
- **L3+ 数据面（Rust）**：网关自身错误在管线边界镜像目标 Provider 的原生错误体；协议识别前失败回退 PingoGate 原生错误体；上游自身错误原样透传不改写。

## X. 状态分离【横切】
三类状态严格分离：
- **Capability Repository**（系统能做什么）：provider 能力声明、模型元数据、转换 / 策略模板、schema 转换规则（marketplace）、未来签名能力包。**不保存**用户密钥、租户策略、用量、计费、会话状态。
- **Runtime Config / 快照**（实例如何运行，L3 起）：listener、domain、certificate、enabled providers、upstream channels、model routing、transform rules、policy、metrics、reload。由 Go 非内核管理 DB 源，经 gRPC 推给 Rust 内核构建不可变 RuntimeSnapshot（ArcSwap 原子切换，内存）。来源 YAML / TOML / JSON / env / K8s ConfigMap / DB 发布版本。
- **Control Plane State**（谁在使用、用了多少）：users、tenants、projects、API keys、virtual keys、provider keys（密文）、quota、budgets、usage、billing、audit、**ConversationTimeline（上下文会话历史）**。**全部由 Go 非内核管理写入**，存 SQLite / Postgres（sqlx 可切换）。**Rust 内核零 DB 访问**：用量由 Rust 采集到内存后推 Go 落库；上下文 timeline 由 Go 存储与 materialize。单机模式 Rust 从文件加载快照，无 DB。

重载不触控制面状态；不可变 `RuntimeSnapshot` + `ArcSwap` 原子切换（Rust 内核，L3 起，内存）。**Rust 内核零 DB**（不读不写）；所有 DB 访问在 Go 非内核。L0-L2 控制面 CRUD 直连 DB + 失效相关缓存 + 推新快照给 Rust 内核。

## XI. Provider 抽象（能力族）【L3+，Rust 内核】
- adapter 声明支持的能力族（而非只注册 endpoint handler）；暴露 namespace；不支持的能力族显式拒绝（返回明确「不支持」错误，不静默 / 畸形透传）。
- adapter 拥有 Provider 特定转换（错误体形状、认证方式描述符）。
- 保留 Provider 命名空间边界，不同 Provider 能力与转换不相互越界。
- **10 个能力族**（蓝图 9）：`generation.stateless`（Chat Completions/Messages/generateContent）、`generation.stateful`（Responses/Interactions/managed agents）、`realtime.live`（Realtime/Live WebSocket）、`embedding`、`batch`、`file.media`、`image.audio.video`、`tools.agents`（function calling/MCP/code execution/computer use）、`safety.moderation`、`platform.admin`。L3 起逐步覆盖，`generation.stateless` 优先；其余后置（见路线图 §3D）。
- L0-L2 无 Provider 概念；provider key 在 L1 仅作加密保管的 opaque 凭据（KeyVault 在 Rust 内核，Go 非内核经 gRPC 调加解密），不解析其能力。

## XII. 热重载【L3+，Rust 内核】
- `ArcSwap` snapshot；Go 非内核构建候选快照 -> gRPC 推给 Rust 内核 -> Rust 校验 -> 构建 -> 原子切换 -> 记录版本 / 回滚目标；失败不改活跃 snapshot。
- 进行中请求继续使用启动时绑定的旧 snapshot，零中断。
- 触发源：SIGHUP / Go 管理面 API / 文件监视 / 仓库 revision / UI publish。
- 不原地修改 live route table；不加载 native `.so` / `.dylib` / `.dll` 插件作为主路径。
- L0-L2 控制面变更经 Go CRUD 直达 DB + 推新快照给 Rust 内核，不走 Rust 热重载（L0-L2 无数据面快照）。

## XIII. TDD【横切】
- 契约 / 集成 / 单元测试先写并观察失败。
- L0-L2（Go）覆盖：身份边界、key 加解密（经 gRPC 调 Rust KeyVault）、1Password 可见性、authorize 判定、CRUD 往返。
- L3+（Rust）覆盖：管线阶段、透传往返、快照热重载、鉴权边界、流式中断、转换正确性。
- 跨语言契约：gRPC 接口契约测试（Go / Rust 双侧）。

## XIV. 文档与注释【横切】
- 公共 API 英文文档注释（Rust doc-comments / Go doc comments）；项目文档中文。

## XV. 原子提交【横切】
- Conventional Commits；按工作单元拆分。

## XVI. CodeGraph【横切】
- 存在 `.codegraph/`；编辑前用 `codegraph_explore` 评估影响。

## XVII. AI 技能【横切】
- 涉及库 / API 用 context7 查文档；auth / key / TLS 变更合并前走 security-review；功能完成走 code-review / simplify / verify。

## XVIII. Subagent 监控【横切】
- 如用 subagent 则监控其存活。

## XIX. 可观测性【L0+ 管理（Go）+ L3+/L5 热路径与计量（Rust）】
- **L0+ 管理面（Go）**：slog 结构化日志，管理操作审计（who/what/when），密钥脱敏。
- **L3+ 热路径（Rust）**：tracing 带贯穿管线的 trace / 请求 ID；Prometheus 指标按 provider + 能力族（必需）打标签，MAY 按 principal id（非明文密钥）归因。
- **L5 用量计量（Rust 提取现成 usage + Go 估算/落库/查询）**：必需观测信号--请求量、状态码、延迟、上游延迟、retries、fallback、token 计数（input / output / reasoning / cache）。
  - **Rust 内核**：只提取 provider 响应中**现成**的 usage 字段（OpenAI `stream_options.include_usage` / Anthropic `message_delta` / Gemini `usageMetadata`），增量记录 + 终态对账，推给 Go。**Rust 不带 tokenizer、不做估算**（各 provider tokenizer 算法不同，维护繁琐，交 Go 生态）。
  - **Go 非内核**：响应无 usage 时由 Go 估算（Rust 推 body 给 Go，复用上下文 materialize 的 body 或专门推）；**失败 / 中断的部分计量归 Go 估算**（Rust 推已转发部分）；用量聚合 / 落库 / 查询 / 计费 / 配额规则。Go 零 DB 限制不适用（Go 管 DB）。
  - **单机模式**：Rust 只提取现成 usage（记 log / 指标，不落 DB）；无 usage 的请求不估算（尽力而为，因无 Go）。
  - Rust 零 DB；跨进程传 body 只在估算需要时（且复用上下文 materialize 已有 body）。
- 密钥 / token 经统一脱敏层后才输出（横切，所有层，双语言）。

## XX. 安全 / 保密 / 隐私【横切】
- 100% Safe Rust（内核）；Go 非内核遵循 Go 安全实践。
- **BYOK 可见性（1Password 模型）**：Provider Key（组织 key 或 BYOK key）明文可见性绑定 `created_by`，与 RBAC 正交。只有 `created_by` 能查看明文，其余任何人（任何角色、任何 admin）只见末位 + 元数据。BYOK key 的 `created_by` = owner user，故管理员不可见用户明文 key。明文只在 Rust 内核 `KeyVault::decrypt()` 返回的受保护 `SecretString` 中存活，MUST NOT 进日志 / 指标 / 任何对外输出。Go 非内核只接触密文，永不持有明文 provider key。
- 密钥经引用解析或加密存储；上游 Provider Key 不由 adapter 直接接触（L3+，Rust 内核）。
- Admin / 所有特权操作经 `Principal` / `AuthContext` / `authorize(action, resource)` 边界；不存在「全局 admin token 等值判断」硬编码鉴权；为 OAuth / OIDC / SSO 预留身份映射边界。
- 上游 TLS 默认校验（L3+，Rust 内核）。
- **内容持久化策略（按上下文桥转化类型，非「默认不持久化」）**：
  - 不开上下文桥：无状态透传，不持久化 prompt / response，仅记录元数据 + 指标。
  - 开上下文桥 + 同态透传（如 Responses -> Responses，不跨协议）：provider 服务端存状态，PingoGate 只存 response_id / interaction_id 映射，不存完整内容。
  - 开上下文桥 + 跨协议转化（如 Responses -> Chat Completions，有状态桥到无状态）：**必须存完整 ConversationTimeline**（否则无法 materialize），加密短期可过期，受 ContextPolicy（TTL / 加密 / 脱敏 / 租户隔离）控制。
  - 完整内容持久化（非桥接必需）需租户 / 项目策略显式开启。

## XXI. 性能【L3+热路径，Rust 内核】
- 延迟预算：无状态透传网关在上游延迟之外的额外开销 < 5 ms p50 / < 20 ms p95（Pingora 级）。
- 零拷贝流式（SSE 不缓冲整流）；异步路径无阻塞 I/O；上游过载用显式背压而非无界队列。
- 热路径变更加性能测试；每功能定义资源预算；写锁阻塞热路径读的方案被否。
- L0-L2 控制面（Go）无热路径性能预算；管理面按人速 QPS 设计，但仍须异步无阻塞。
- Go -> Rust 快照推送不得阻塞 Rust 热路径；Rust 热路径只读内存快照，零跨进程调用。

## XXII. 项目语言【横切】
- 文档中文；代码 / API / 提交信息英文。
