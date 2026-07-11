# Feature Specification: PingoGate 网关地基（首个实现方向）

**Feature Branch**: `001-gateway-foundation`

**Created**: 2026-06-29

**Status**: Draft

**Input**: User description: "将现有的 blueprint 完整转化为spec"

> **范围说明（重要）**：`blueprint/` 描述的是 PingoGate 的**完整产品愿景**——从单二进制网关，到团队控制台、企业治理平台、协议桥接与能力仓库。本 spec 遵循「一次 specify 只定义一个有界、可测试的 feature」的约束，把蓝图**完整转化**为：
> - **首个实现方向（网关地基）** → 落为本 spec 的用户故事、功能需求、成功标准；
> - **蓝图中倾向延后的全部能力** → 完整记录在 [Out of Scope](#out-of-scope范围之外) 并映射到候选阶段；
> - **蓝图决策日志中的已确认结论** → 完整记录在 [Assumptions](#assumptions)。
>
> 蓝图内容一项不丢失：要么成为本阶段需求，要么成为显式划定边界的未来工作。后续阶段能力将由各自独立的 spec 承接。

## Clarifications

### Session 2026-06-29

- Q: 网关自身产生错误（Key 无效、无路由、能力不支持、上游超时）时，错误响应采用何种结构？ → A: 镜像目标 Provider 的原生错误体结构；协议识别成功前即失败时回退到 PingoGate 原生错误体。
- Q: 「静态网关 Key 模式」具体是单个共享密钥还是一组具名 Key？ → A: 配置一组具名网关 Key，每个 Key 是可识别主体（principal）；指标/日志可按 Key 标识归因，为未来项目/虚拟 Key 预留迁移边界。
- Q: 首阶段是否强制默认上游超时，超时时客户端收到什么？ → A: 提供可配置的上游超时（含合理默认值）；超时/不可用时按 FR-035 返回镜像目标 Provider 的错误（类 504 语义）并记入指标。

## User Scenarios & Testing *(mandatory)*

PingoGate 网关地基面向三类核心角色：**应用开发者**（把流量接入统一网关）、**运维/AI Infra**（部署、配置、观测网关）、**网关管理员**（通过机器可读接口管理与探测网关）。下列用户故事按重要性排序，每个故事都是可独立交付、独立测试的功能切片。

### User Story 1 - 应用开发者以最小改动经网关调用上游模型（Priority: P1）

应用开发者已经在用官方 SDK（OpenAI / Anthropic / Gemini）。他只把客户端的 base URL、访问 token 和模型名改成 PingoGate 提供的值，其余代码不动，就能让请求经 PingoGate 转发到对应上游 Provider，并拿到与直连上游一致的响应；流式（SSE）场景下，token 持续增量返回。客户端持有的是**网关 Key**，从不接触上游 Provider 的真实密钥。

**Why this priority**: 这是整个网关存在的理由，也是蓝图「首个实现方向」的热路径核心。没有它，平台、观测、管理等一切都无意义。它独立成立即构成一个有用的 MVP——一个能跑、可替换客户端入口的单二进制 LLM 网关。

**Independent Test**: 配置三个上游 Provider 各一个模型，分别用 OpenAI-compatible Chat Completions、Anthropic Messages、Gemini generateContent 的官方 SDK 仅修改 base URL/token/模型名发起请求；验证非流式与流式（SSE）两种模式下，响应内容、错误语义与直连上游一致，且客户端全程只使用网关 Key。

**Acceptance Scenarios**:

1. **Given** 网关已配置 OpenAI-compatible 上游与一个模型别名，**When** 客户端用有效网关 Key 发起非流式 Chat Completions 请求，**Then** 收到与直连上游一致的成功响应，且响应中不泄露上游 Provider Key。
2. **Given** 网关已配置 Anthropic 上游，**When** 客户端发起 `stream: true` 的 Messages 请求，**Then** SSE 事件按上游顺序持续转发，直至流正常结束。
3. **Given** 网关已配置 Gemini 上游，**When** 客户端调用 `generateContent`（含 streaming 变体），**Then** 请求被识别为 Gemini 协议并正确路由、注入上游认证、返回上游响应。
4. **Given** 客户端请求一个未在配置中声明的模型别名，**When** 请求到达网关，**Then** 返回明确的「无可用路由」错误，而非静默失败或错误转发。
5. **Given** 客户端使用无效或缺失的网关 Key，**When** 请求到达公共入口，**Then** 请求在鉴权阶段被拒绝，且不会触达任何上游。
6. **Given** 客户端请求当前阶段不支持的能力（如 Responses / Realtime / Batch），**When** 请求到达网关，**Then** 返回明确的「能力不支持」错误，而非畸形透传。

---

### User Story 2 - 运维人员以声明式配置部署并热重载网关（Priority: P2）

运维/AI Infra 工程师把单个 `pingogate` 二进制部署到环境中，编写一份声明式配置（监听器、Provider、上游密钥引用、模型路由）。需要变更时，他修改配置并触发重载；配置先经校验，通过后原子切换生效，进行中的请求不受影响；若新配置非法，重载被拒绝且当前运行时完全不受影响。

**Why this priority**: 声明式配置 + 安全热重载是把网关做成「可运行、可替换、可运维」基础设施的前提，也是蓝图「运行时骨架」与「配置热重载优先」原则的落地。它可在 P1 之上独立验证，让网关具备生产可运维性。

**Independent Test**: 启动网关后修改 `pingogate.yaml`（例如新增一个模型路由）并触发重载，验证新路由生效；构造一份语法/语义非法的配置触发重载，验证重载被拒绝、错误被清晰报告、且重载前正在处理与已建立的请求继续按旧快照正常完成。

**Acceptance Scenarios**:

1. **Given** 网关以一份有效配置运行，**When** 运维修改配置并触发重载，**Then** 重载流程依次执行「校验候选配置 → 构建不可变运行时快照 → 原子切换 → 记录版本与回滚目标」，新配置生效。
2. **Given** 一份新候选配置存在 schema 或语义错误，**When** 触发重载，**Then** 重载失败，活跃运行时快照保持不变，并返回可定位的校验错误。
3. **Given** 重载正在发生，**When** 此刻有进行中的请求，**Then** 这些请求继续使用其启动时的运行时快照，不被中断或改变路由。
4. **Given** 配置中通过引用方式声明上游密钥，**When** 配置被加载，**Then** 密钥引用被解析用于发往上游，但密钥明文不出现在日志或任何对外输出中。

---

### User Story 3 - 管理员通过机器可读 Admin API 管理与探测网关（Priority: P3）

网关管理员（首阶段使用 bootstrap 静态管理员凭据）调用机器可读的管理接口，用于健康检查、就绪探测、配置校验、触发重载、查询重载状态。每一次管理请求都经过身份与授权边界鉴权，即使在 bootstrap 模式下也不例外，从而为未来接入 OAuth / OIDC / 企业 SSO 预留身份边界，而无需改写接口语义。

**Why this priority**: 管理面最小闭环让网关可被编排系统（健康/就绪探针、CI 校验、自动化重载）接管，是蓝图「管理面最小闭环」与宪法安全规则（Admin API 必须经 `Principal`/`AuthContext`/`authorize` 边界）的落地。它独立于人机界面，先于嵌入式控制台交付。

**Independent Test**: 用有效管理员凭据依次调用 health、readiness、config validation、reload、reload status，验证返回结构化结果；用缺失/无效凭据调用同样接口，验证全部被授权边界拒绝；提交一份非法候选配置给 config validation 接口，验证返回校验失败但不影响运行时。

**Acceptance Scenarios**:

1. **Given** 网关运行中，**When** 管理员用有效凭据调用 health 与 readiness，**Then** 分别返回机器可读的存活与就绪状态。
2. **Given** 一份候选配置，**When** 管理员调用 config validation 接口，**Then** 返回是否通过校验及具体错误，且不改变活跃运行时。
3. **Given** 管理员调用 reload，**When** 候选配置有效，**Then** 触发与 User Story 2 一致的重载流程，并可通过 reload status 查询本次重载的结果、版本与回滚目标。
4. **Given** 任意管理请求缺失或携带无效凭据，**When** 请求到达管理入口，**Then** 在授权边界被拒绝；系统不存在「全局 admin token 等值判断」这类硬编码鉴权。

---

### User Story 4 - 运维人员通过可观测信号排查流量（Priority: P4）

运维/AI Infra 工程师在网关处理流量时，能从结构化日志与机器可读指标中观察系统行为：每条请求带可贯穿管线的请求/追踪 ID；指标按 Provider 和能力族打标签，覆盖请求量、状态码、延迟、上游延迟与 token 计数；日志与指标中均不出现任何密钥明文。

**Why this priority**: 可观测性是网关规模化运维的一等特性（宪法原则 XIX），也是蓝图「基础观测」候选范围。它独立于路由逻辑之外，可在 P1/P2 之上单独验证。

**Independent Test**: 驱动一批跨三个 Provider 的请求（含成功、失败、流式），抓取指标端点验证标签（provider、capability family）与计数正确；检查日志包含贯穿管线的请求 ID，且对 Key/token 做了脱敏。

**Acceptance Scenarios**:

1. **Given** 网关处理了若干请求，**When** 运维抓取指标端点，**Then** 能看到按 Provider 与能力族打标签的请求量、状态码、延迟、上游延迟与 token 计数。
2. **Given** 一条请求流经管线，**When** 运维查看其日志，**Then** 该请求各阶段日志携带同一请求/追踪 ID。
3. **Given** 请求或配置中包含 Provider Key / 网关 Key，**When** 任何日志或指标被输出，**Then** 这些敏感值已被脱敏，不以明文出现。

---

### Edge Cases

- **上游错误与超时**：上游返回 4xx/5xx 或超时，错误语义如何忠实传回客户端（上游自身的错误原样透传；网关因上游**可配置超时**到期或上游不可用而自行产生的错误按 FR-035/FR-038 镜像目标 Provider 错误体，超时用类 504 语义并记入指标）。
- **流式中断**：SSE 流在上游或客户端侧中途断开时，连接清理与（可观测的）断开原因记录。
- **重载与在途请求竞争**：重载发生瞬间有大量在途请求，新旧快照如何隔离，确保在途请求零中断。
- **并发重载**：多个重载请求同时到达时的串行化与最终一致结果。
- **密钥引用无法解析**：配置引用的上游密钥（环境变量/密钥引用）缺失时，校验阶段应明确报错而非启动后运行时崩溃。
- **路由缺失**：模型别名未配置、或对应 Provider 不可用时的明确错误。
- **认证缺失**：缺失/无效网关 Key（公共入口）或管理员凭据（管理入口）的拒绝行为。
- **不支持的能力**：请求落在当前阶段未实现的能力族（Responses/Conversations、Realtime/Live、Batch、Embeddings、Files 等）时返回显式「不支持」错误。
- **上游认证差异**：不同 Provider 认证位置不同（Authorization Bearer / `x-api-key` + 版本头 / URL query `?key=`），注入逻辑需各自正确。

## Requirements *(mandatory)*

> 说明：以下需求中出现的 Provider 协议名（OpenAI-compatible Chat Completions、Anthropic Messages、Gemini generateContent）、流式（SSE）、声明式配置、指标抓取端点等，是 PingoGate 作为「兼容多 Provider 的统一入口」的**对外契约与产品需求**，而非内部实现选择。具体技术栈（Rust/Pingora 等）由宪法锁定并在 plan 阶段展开，不在本 spec 约束。

### Functional Requirements

#### 入口与协议兼容

- **FR-001**: 系统 MUST 在公共入口对每个请求进行协议识别，判定其目标 Provider 协议或兼容表面。
- **FR-002**: 系统 MUST 支持 OpenAI-compatible Chat Completions 的原生透传。
- **FR-003**: 系统 MUST 支持 Anthropic Messages 的原生透传。
- **FR-004**: 系统 MUST 支持 Gemini `generateContent` 的原生透传。
- **FR-005**: 系统 MUST 支持上述三类协议的流式（SSE）响应透传，按上游事件顺序持续转发直至流结束。
- **FR-006**: 公共路径 MUST 保持 SDK 友好（provider-native 或 compatibility-native），MUST NOT 把 `/openai/...` 这类前缀作为主路径。
- **FR-007**: 对当前阶段不支持的能力或能力族，系统 MUST 返回显式的「不支持」错误，MUST NOT 静默处理或畸形透传。
- **FR-008**: 透传路径 MUST 忠实保留上游的成功响应与错误语义（含状态码与错误体），不掩盖上游错误。

#### 鉴权与密钥分离

- **FR-009**: 系统 MUST 提供静态网关 Key 模式：在配置中声明**一组具名网关 Key**，每个 Key 是可识别的主体（principal，含名称/标识但不含明文外泄）；客户端以某个网关 Key 向 PingoGate 鉴权。
- **FR-010**: 系统 MUST 严格分离**网关 Key**（客户端 → PingoGate）与**Provider Key**（PingoGate → 上游），二者不得混用或互相泄露。
- **FR-011**: 系统 MUST 支持在配置中声明上游 Provider Key（通过密钥引用），并在派发前由核心注入上游所需认证；Provider 适配逻辑 MUST NOT 直接接触 Provider Key 明文。
- **FR-012**: 系统 MUST 按各 Provider 的认证方式正确注入凭据（例如 `Authorization: Bearer`、`x-api-key` + 版本头、URL query `?key=`）。
- **FR-013**: 缺失或无效的网关 Key MUST 在鉴权阶段被拒绝，且请求 MUST NOT 触达任何上游。

#### 路由

- **FR-014**: 系统 MUST 依据协议默认与模型配置（模型别名 → Provider/上游）对请求进行路由。
- **FR-015**: 系统 MUST 保留 Provider 命名空间边界，不同 Provider 的能力与转换不得相互越界。
- **FR-016**: 当模型别名无匹配路由或目标上游不可用时，系统 MUST 返回明确错误。

#### 配置与热重载

- **FR-017**: 系统 MUST 以单一声明式配置文件（`pingogate.yaml`）描述监听器、路由、上游/Provider 与证书。
- **FR-018**: 系统 MUST 支持热重载，且可由 `SIGHUP`、Admin API 或文件监听触发。
- **FR-019**: 重载流程 MUST 依次为：加载候选配置 → schema 校验 → 语义校验 → 构建不可变运行时快照 → 原子切换 → 记录版本与回滚目标。
- **FR-020**: 重载失败 MUST NOT 修改活跃运行时快照。
- **FR-021**: 进行中的请求 MUST 继续使用其启动时所绑定的运行时快照。
- **FR-022**: 配置中的上游密钥引用 MUST 被解析用于上游调用，但密钥明文 MUST NOT 出现在日志或任何对外输出中。

#### 管理接口（Admin API）

- **FR-023**: 系统 MUST 暴露机器可读的管理接口，至少覆盖：health、readiness、config validation、reload、reload status。
- **FR-024**: 每一次管理请求 MUST 经过身份与授权边界（`Principal` / `AuthContext` / `authorize(action, resource)` 抽象）鉴权，即使在 bootstrap 静态管理员模式下亦然。
- **FR-025**: 系统 MUST NOT 以「全局 admin token 等值判断」这类硬编码方式实现管理鉴权；授权边界 MUST 为未来接入 OAuth / OIDC / GitHub / Google / 企业 SSO 预留映射到内部用户、角色、项目成员与服务账号的位置（仅边界，不在本阶段实现）。
- **FR-026**: config validation 接口 MUST 能在不改变活跃运行时的前提下，返回候选配置是否通过校验及具体错误。
- **FR-027**: reload status MUST 可查询最近一次重载的结果、生效版本与回滚目标。

#### 运行形态

- **FR-028**: 默认部署形态 MUST 为单个 `pingogate` 二进制；本阶段 MUST 能以 file + memory 运行时启动，不强制依赖外部数据库或其他服务。
- **FR-029**: Provider 适配 MUST 声明其支持的能力族；本阶段目标能力族为无状态生成（`generation.stateless`）。
- **FR-030**: 热路径 MUST 基于不可变运行时快照执行，不得在请求处理过程中被重载阻塞或改变。

#### 可观测性

- **FR-031**: 系统 MUST 输出结构化日志，字段命名一致，且每条请求携带可贯穿管线全程的请求/追踪 ID。
- **FR-032**: 系统 MUST 暴露机器可读、可被常见监控系统抓取的指标端点（Prometheus 兼容），至少覆盖请求量、状态码、延迟、上游延迟与 token 计数（输入/输出，以及上游可提供时的 reasoning / cache token）。
- **FR-033**: 指标 MUST 按能力族与 Provider 打标签；MAY 额外按网关 Key 主体标识（principal id，非明文密钥）归因。
- **FR-034**: 所有敏感值（网关 Key、Provider Key、token）MUST 在日志与指标输出中脱敏，MUST NOT 以明文出现。

#### 错误响应契约

- **FR-035**: 网关自身产生的错误（鉴权失败、无匹配路由、能力不支持、上游超时/不可用等）MUST 镜像该请求所属目标 Provider 的原生错误体结构，使现有 SDK 的错误解析逻辑无需改动即可工作。
- **FR-036**: 当请求在协议识别成功之前即失败（目标 Provider 尚不可知）时，系统 MUST 回退到 PingoGate 原生错误体结构。
- **FR-037**: 透传路径上由上游返回的错误（FR-008）原样转发；本契约（FR-035/FR-036）仅约束**网关自身**产生的错误，二者不冲突。
- **FR-038**: 系统 MUST 为上游请求提供可配置的超时（每路由/每上游，含合理默认值）；上游超时或不可用时，MUST 按 FR-035 返回镜像目标 Provider 的错误（超时采用类 504 语义），并将该失败记入可观测指标（FR-032）。

### Key Entities *(include if feature involves data)*

- **网关 Key（Gateway Key）**：客户端访问 PingoGate 的凭据；本阶段为配置中声明的**一组具名 Key**，每个 Key 是可识别主体（principal，有名称/标识，用于日志与指标归因）；与 Provider Key 严格隔离。
- **Provider Key**：PingoGate 访问上游的凭据；以引用方式声明，加密/受保护存储，明文不外泄。
- **Provider（上游）**：一个上游来源的声明，含类型（openai-compatible / anthropic / gemini）、base URL、认证方式与可用模型。
- **模型路由 / 别名（Model Route / Alias）**：客户端可见模型名到具体 Provider 与上游模型的映射。
- **运行时快照（Runtime Snapshot）**：由配置构建的不可变运行时视图，带版本与回滚目标；通过原子切换生效。
- **能力族（Capability Family）**：按能力而非单一 endpoint 建模的分类（本阶段：`generation.stateless`）；用于路由、指标标签与「不支持」判定。
- **Principal / AuthContext**：管理请求的身份与授权上下文，承载授权动作判定的边界。
- **请求上下文（Request Context）**：贯穿管线的请求级状态，携带请求/追踪 ID、识别出的协议、路由决策与所绑定的运行时快照。
- **声明式配置（`pingogate.yaml`）**：描述监听器、路由、上游/Provider、密钥引用与证书的单一配置来源。

## Success Criteria *(mandatory)*

### Measurable Outcomes

- **SC-001**: 应用开发者无需改动业务代码，仅修改 base URL、token、模型名三项，即可让现有应用经 PingoGate 调通——对 OpenAI-compatible、Anthropic、Gemini 三个 Provider 家族各自成立。
- **SC-002**: 三个 Provider 家族的非流式与流式（SSE）请求均能得到与直连上游一致的响应内容与错误语义。
- **SC-003**: 无状态透传场景下，网关在上游延迟之外引入的额外开销中位数低于 5 ms、p95 低于 20 ms。
- **SC-004**: 配置重载过程中，因重载导致的在途请求失败数为 0；进行中与已建立的请求全部正常完成。
- **SC-005**: 提交一份非法候选配置时，重载/校验被拒绝，先前运行的配置 100% 不中断地继续服务流量。
- **SC-006**: 100% 的管理 API 请求都经过身份与授权判定；缺失/无效凭据的管理请求被全部拒绝。
- **SC-007**: 运维可通过机器可读接口确定网关的健康、就绪状态与当前生效配置版本，无需读取人工界面。
- **SC-008**: 100% 的请求出现在按 Provider 与能力族打标签的指标中；任何日志或指标输出中均不出现 Key/token 明文（抽样审计 0 泄漏）。
- **SC-009**: 网关以单个二进制启动并服务全部首阶段能力，无需任何外部数据库或协调服务。
- **SC-010**: 请求落在未支持能力族时，100% 返回显式且文档化的「不支持」错误，而非畸形透传或静默失败。

## Assumptions

本节记录据蓝图、宪法与蓝图「已确认决策」做出的合理默认与范围决定。

### 范围与阶段

- 本 spec 仅覆盖蓝图的「**首个实现方向 / 网关地基**」——单二进制原生透传网关。该范围是蓝图明确指定的起步方向（见蓝图 `#phase-one`、`#decisions`）。完整产品愿景的其余部分见 [Out of Scope](#out-of-scope范围之外)，由后续独立 spec 承接。
- 蓝图中标注为「候选」的首阶段能力（FR-001 ~ FR-034 覆盖项）在本 spec 中被确认为首阶段范围；标注「倾向延后」的能力一律划入 Out of Scope。

### 已确认决策（来自蓝图 Decision Log）

- **配置格式**：用户主配置与 proxy profile 使用 YAML；marketplace catalog、锁文件与 schema 使用 JSON；TOML 不作为核心 profile 格式。
- **Responses / Interactions**：不进入首阶段；若未来进入，倾向只做同 Provider 原生透传，不模拟状态、不桥接到无状态上游。
- **管理鉴权**：首阶段用 bootstrap 静态管理员凭据，但必须经 `Principal` / `AuthContext` / `authorize(action, resource)` 边界（FR-024、FR-025）。
- **第三方登录**：OAuth / OIDC / GitHub / Google / 企业 SSO 延后到控制台与用户体系阶段，本阶段仅预留身份边界。
- **控制台时机**：嵌入式 TypeScript 控制台不进入首阶段，本阶段仅提供机器可读管理 API 与身份边界。
- **管理模型**：长期控制面同时支持企业管理员与用户/项目负责人，普通用户可通过 BYOK 自带 Provider Key 与模型（完全私有）——均属后续阶段。
- **Proxy 分发产物**：默认分发 YAML/JSON 声明式 profile 包（描述 URL、header、query、body、streaming、capabilities），WASM 仅作未来高级扩展。
- **脚本引擎**：不引入 Rhai 作为默认扩展路线；主路径使用声明式 TransformPlan 与编译后的 Rust 计划，复杂第三方逻辑未来优先考虑签名 WASM。
- **仓库与分发**：根目录作为 AI 协作壳，产品代码进入按 infra 层级组织的顶层目录。**注**：蓝图早期草图使用 `product/backend|frontend|...`，而项目宪法 v1.6.0 已将顶层目录最终确定为 `app/` `console/` `deploy/` `scripts/`；以**宪法为准**。具体目录落点在 plan 阶段确认，不在本 spec 约束。

### 默认取舍

- **上游密钥来源**：首阶段以「配置中声明的上游 Provider Key」为主路径；蓝图提到的「客户端 upstream key pass-through」视为可选/延后能力，默认关闭，不阻塞 MVP。
- **转换路径范围**：本阶段透传路径仅做「协议识别 → 路由 → 认证注入」，body 原样流式转发。蓝图附录《架构讨论记录》中的「L1 配置改写」（由 Core 直接改写 model 别名以外的 endpoint / header / query）与跨 family 插件转换均**不在本阶段**，归入未来 Provider profile 改写与插件能力；仅「模型别名 → Provider/上游」的映射（FR-014）属本阶段。
- **运行时后端**：首阶段使用 file + memory，不引入 SQLite/Postgres 等持久化后端。
- **TLS / 证书**：监听器与上游 TLS 属网关基础设施层；上游 TLS 证书默认校验（受控开发环境可显式禁用）。证书的完整生命周期管理（ACME 等）延后。
- **性能预算**：沿用宪法延迟预算（无状态透传网关开销 < 5 ms p50 / 20 ms p95）作为 SC-003 的可度量目标。
- **隐私默认**：默认不持久化完整 prompt/response 内容，仅记录元数据与指标；内容持久化需后续由租户/项目策略显式开启。

### Out of Scope（范围之外）

以下蓝图能力**明确不在本 spec 范围**，记录于此以保证蓝图被完整转化，并标注其在蓝图路线图中的候选阶段归属（最终阶段归属待后续确认）：

- **控制面与平台基础（蓝图 P2 候选）**：多租户数据库（SQLite/Postgres/MySQL）、用户/项目/API Key 管理、Provider/channel/model 管理、模型别名管理界面、请求日志持久化、用量记录、证书管理、嵌入式控制台、第三方登录。
- **主流网关能力对齐（蓝图 P3–P4 候选）**：virtual keys、团队/项目预算、模型组、权重路由、fallback/retry、成本核算、Provider 健康检查、文本生成协议转换。
- **上下文桥与有状态协议模拟（蓝图长期差异点）**：`ClientStateRef`、`ConversationTimeline`、`ContextMaterializer`、PingoGate 自生成 Responses/Interactions id、跨协议上下文 materialization、加密短期状态与内容留存策略。
- **非文本与异步能力（蓝图 P5 候选）**：Files/Uploads、对象存储抽象、Embeddings、Batch jobs、图片/音频/视频、Provider 文件引用映射、prompt/context cache 控制。
- **Realtime / Live / Tools / Agentic（蓝图 P5+ 候选）**：双向 session 管线、OpenAI Realtime、Gemini Live、ephemeral token、tool call 事件桥接、MCP、computer use、代码执行、search grounding、URL context、managed agent。
- **企业能力与生态（蓝图 P5+ 候选）**：多节点/高可用、Redis 协调、OIDC/SAML/LDAP、mTLS、ACME、完整审计、billing/支付、能力仓库同步、签名能力包、WASM policy/converter 插件、proxy marketplace/catalog/registry、GitHub PR 驱动的 proxy 生态、Docker Compose / Kubernetes / Helm 指南。
- **BYOK 子系统**：Channel Scope 分层（platform/user）、`UserProviderKey`、`UserChannel`、用户「我的模型」控制台——延后到控制面阶段。
- **Provider 扩展插件架构（蓝图附录《架构讨论记录》，P5+ 候选）**：`blueprint/sections/architecture-discussion.html` 沉淀的 M² 双向 adapter 模型、「透传 / L1 配置改写 / 插件转换」三层路径、WASM 优先的预编译插件分发（动态库备选）、插件 I/O 权限（默认禁止网络）、插件 Registry（官方源/自建源、签名、哈希校验、企业白名单），以及该记录中的 6 个待确认问题（插件格式、声明式模板取舍、插件 I/O 权限、Registry 形态、认证范围与 AWS SigV4 / GCP Vertex OAuth 延后、MVP 边界）。其中**「MVP 边界」一问在本 spec 已定调**：网关地基阶段为纯原生透传 + 路由 + 认证注入，**不含**跨 family adapter 或转换插件链路；插件架构整体延后，由后续独立 spec 承接。认证方面，本阶段仅实现蓝图三大家族所需的 Bearer / `x-api-key`+版本头 / URL query key（FR-012），Azure `api-key`、AWS SigV4、GCP Vertex OAuth 等随其 Provider 一并延后。

> 架构约束：尽管上述能力不在本阶段实现，网关地基的架构 MUST 为其预留位置（保留 Provider 命名空间边界、能力族建模、身份/授权边界、不可变快照热路径、按能力族打标签的指标），不得做成「只会聊天补全」的封闭设计。
