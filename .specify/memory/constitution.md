<!--
  Sync Impact Report
  ==================
  Version change: 1.5.0 → 1.6.0
  Bump rationale: MINOR — 将顶层目录最终确定为 app/console/deploy/scripts 结构；
    新增目录与架构层级映射说明，删除已移除参考文件的根目录条目
  Reorganization: N/A
  从 UniverWriter Constitution 吸纳：
    - 分层原则适用（I）
    - DRY/KISS/YAGNI 格言（II）
    - 代码指标与限制（V）
    - SOLID 原则（VII）
    - 低耦合 / 高内聚（VIII）
    - 分层错误处理策略（IX）
    - 测试驱动开发（XIII）
    - 原子提交（XV）
    - CodeGraph 优先（XVI）
    - AI 辅助开发技能概念（XVII）
  PingoGate 特有补充：
    - 技术栈锁定为 Rust/Pingora + React + shadcn/ui（III）
    - 顶层目录按 infra 层级组织：app/console/deploy/scripts/；app/ 内部再按架构分层
      映射到 crate（IV）
    - 请求管线架构（VI）
    - 状态分离（X）
    - 按能力族抽象的 Provider（XI）
    - 配置驱动运行时与热重载（XII）
    - Rust/TS 文档与英文注释规范（XIV）
    - Subagent 存活监控，使用默认超时（XVIII）
    - 可观测性（XIX）
    - 安全、保密与隐私（XX）
    - 性能与延迟（XXI）
    - 项目语言规范：文档中文 / 代码与 API 英文（XXII）
  Templates requiring updates:
    - .specify/templates/plan-template.md        ⚠ pending (Constitution Check 占位符)
    - .specify/templates/spec-template.md        ✅ compatible
    - .specify/templates/tasks-template.md       ✅ compatible
  Follow-up TODOs: none
-->

# PingoGate 宪法

## 基础格言

*先读这两条原则 —— 它们决定其他所有原则如何适用。*

### I. 原则适用层级

不是所有变更都值得完整的网关仪式。本原则定义三个复杂度层级，并规定在每个
层级上哪些原则是**强制**、**推荐**或**可选**的。

#### 复杂度层级

| 层级 | 判定标准 | 示例 |
| ---- | -------- | ---- |
| **简单** | 配置变更、纯工具函数、孤立 bugfix、文档更新。无跨模块协调。 | 新增指标标签、修复解析器边界情况、更新 README |
| **中等** | 新增 provider adapter、路由策略、鉴权模式或跨模块功能。包含条件业务逻辑。 | 新增 OpenAI chat completion 透传、实现限流策略 |
| **复杂** | 协议桥、上下文虚拟化、多租户控制面、realtime 会话管线。 | Responses → Chat Completions 桥、ConversationTimeline 存储、BYOK 子系统 |

#### 原则矩阵

| 原则 | 简单 | 中等 | 复杂 |
| ---- | ---- | ---- | ---- |
| II. DRY/KISS/YAGNI | **强制** | **强制** | **强制** |
| III. 技术栈 | **强制** | **强制** | **强制** |
| IV. 目录结构 | **强制** | **强制** | **强制** |
| V. 代码指标 | **强制** | **强制** | **强制** |
| VI. 请求管线 | **强制** | **强制** | **强制** |
| VII. SOLID | SRP 强制；其余推荐 | **强制** | **强制** |
| VIII. 低耦合高内聚 | **强制** | **强制** | **强制** |
| IX. 错误处理 | 基础错误类型足够 | 完整分类强制 | **强制** |
| X. 状态分离 | 了解即可 | **强制** | **强制** |
| XI. Provider 抽象 | Adapter 接口强制 | 能力族强制 | **强制** |
| XII. 热重载 | 配置校验强制 | **强制** | **强制** |
| XIII. TDD | 单元测试推荐 | **强制** | **强制** |
| XIV. 文档与注释 | 公共 API 文档强制 | **强制** | **强制** |
| XV. 原子提交 | **强制** | **强制** | **强制** |
| XVI. CodeGraph | **强制** | **强制** | **强制** |
| XVII. AI Skills | **强制** | **强制** | **强制** |
| XVIII. 可观测性 | 指标/日志推荐 | **强制** | **强制** |
| XIX. 安全/保密/隐私 | Safe Rust 强制 | **强制** | **强制** |
| XX. 性能 | 避免回归 | 延迟预算推荐 | **强制** |
| XXI. 项目语言 | **强制** | **强制** | **强制** |

#### 如何判定层级

开始一个功能前，开发者 MUST：

1. 阅读 spec 或 user story。
2. 统计：涉及多少层？多少种状态？是否存在协议转换或多步协调？
3. 使用上表标准分配层级。
4. 在任务或 PR 描述中记录层级选择。

功能实现过程中如果浮现隐藏复杂度，MAY 升级层级。一旦已在更高层级开始工作，
MUST NOT 降级以绕过原则。

#### 跨原则冲突解决

当两个原则在具体场景冲突时：

1. 先查上方层级矩阵 —— 当前层级可能豁免其中一方。
2. 若双方在当层级均为强制，以原则 II（KISS/YAGNI）为决胜者：选择更简单路径。
3. 若仍模糊，在 PR 描述中记录冲突及选择方案，供评审。

**rationale**：要求给日志标签变更强制使用完整上下文桥的宪法会被无视。层级化
适用让规则在复杂度需要时严格、在简单场景下轻量化，从而保持可信度。

### II. DRY / KISS / YAGNI 设计格言

三条不可协商的设计格言，约束每一行代码。

#### DRY — Don't Repeat Yourself

- 每一条知识在代码库中 MUST 只有**单一、权威**的表达。
- 当相同逻辑出现在 2 处及以上时，应提取为共享函数、常量、trait 或类型。
- **注意**：看起来相似但代表不同领域概念的代码 NOT 重复。两个恰好都检查
  `is_empty()` 但业务原因不同的校验器 SHOULD 保持独立。DRY 针对的是**知识**，
  不是语法。

#### KISS — Keep It Simple, Stupid

- 选择能正确解决问题的最简单方案。
- 如果一个 20 行函数能工作，就不要把它包进带动态注册表的 trait 层次结构。
  只在复杂度**已出现**而非**被预期**时才添加抽象。
- 每个抽象（trait、泛型、间接层）MUST 通过服务至少 2 个具体消费者或强制某条
  宪法原则来证明其存在价值。
- 可读性胜过聪明。任何人都能看懂的 `for` 循环，优于需要白板才能理解的
  高阶函数链。

#### YAGNI — You Aren't Gonna Need It

- MUST NOT 实现当前 spec 或 user story 不需要的功能、参数、配置项或抽象。
- 为假设的明天而提前写的“未来防护”代码被禁止。未来到来时，需求会更清晰，
  实现也会更好。
- 如果你发现自己在写“这个以后可能有用”——停下并删除它。

#### 张力解决

这些格言会冲突。冲突时：

1. **KISS vs DRY**：如果提取共享代码让结果更难读，保留重复（KISS 胜）。
2. **DRY vs YAGNI**：如果消除重复需要构建没人要求的通用框架，保留重复
   （YAGNI 胜）。
3. **默认**：不确定时，选择代码行数和文件数更少的方案。

**rationale**：这三条格言是对过度工程的解药。SOLID、DDD 和多范式设计提供结构；
DRY/KISS/YAGNI 确保结构服务于问题，而不是成为问题本身。

## 项目基础

*无论复杂度层级如何，都适用于每一行代码的固定约束。*

### III. 技术栈 — 强制

以下技术栈是**锁定的**。偏离需要修改宪法。

| 层级 | 技术 | 约束 |
| ---- | ---- | ---- |
| 语言 | **Rust**（最新 stable） | 核心后端 MUST 使用 Rust；`unsafe` 需要显式理由（原则 XIX） |
| HTTP 框架 | **Pingora** | 请求生命周期、上游代理、TLS 终结、连接管理 |
| 异步运行时 | **tokio** | 异步 I/O、任务调度、超时 |
| 序列化 | **serde** + **serde_json** | JSON 与 YAML 配置/数据序列化 |
| 配置 | **YAML/JSON** | 运行时配置使用 `pingogate.yaml`；能力目录使用 JSON |
| 日志 | **tracing** | 请求全生命周期结构化、上下文化日志 |
| 指标 | **Prometheus** + **OpenTelemetry** | 请求、token、成本、网关指标 |
| 测试 | **cargo test** | 单元、集成、契约测试 |
| 前端框架 | **React** | 仅用于嵌入式管理控制台；禁止在 TypeScript 中写后端逻辑 |
| UI 组件 | **shadcn/ui + Radix UI** | 所有控制台 UI 组件通过 `shadcn` CLI 安装；默认使用 Lucide 图标 |
| 样式 | **Tailwind CSS** | 控制台 utility-first 样式 |
| 前端构建 | **Vite 或 Next.js** | 由具体实现计划选定并在计划的 tech context 中锁定 |
| 包管理 | **cargo**（Rust）、**pnpm**（控制台） | Rust workspace 使用 cargo；控制台依赖使用 pnpm |

附加约束：

- `Cargo.toml` MUST 启用 `edition = "2021"`（或更新），并配置合理的 lint
  （CI 中将 `clippy::all` 与 `warnings` 视为错误）。
- 新增 Rust 依赖 MUST 附带理由并通过审批（在 PR 中说明为何必要、无现有 crate
  可替代）。
- 新增控制台依赖（包括 shadcn/ui 组件）MUST 通过 `shadcn` CLI 安装
  （`npx shadcn@latest add <component> -c console`），或作为手动例外审批。
- Pingora MUST 用于请求热路径；禁止在热路径使用其他 HTTP server 框架。

### IV. 项目目录结构

仓库 MUST 保持 AI 协作壳（仓库根）与产品源代码之间的严格分离。

#### 顶层目录 — 按 infra 层级组织

产品代码 MUST 按基础设施/架构层级放在根目录下的顶层目录中，而不是按团队、日期
或临时功能随意拆分。每个顶层目录对应一个清晰的 infra 关切：

```text
app/                   # 后端应用：Rust workspace，包含网关核心
├── Cargo.toml
├── pingogate/         # 主二进制与组合根
├── listener/          # 网关基础设施层：HTTP/HTTPS/TLS listener
├── pipeline/          # 请求管线层：鉴权、路由、策略、派发
├── provider/          # Provider 命名空间层：adapter 与能力族
├── context/           # 上下文桥层：conversation timeline 与状态虚拟化
├── admin/             # 控制面层：Admin API 与平台治理
└── storage/           # 存储抽象层：trait 与适配器（file、SQLite 等）

console/               # 嵌入式控制台：React + shadcn/ui 静态资源
├── package.json
└── src/

deploy/                # 部署与运维基础设施
├── docker/
├── k8s/
└── ...

scripts/               # 脚本与开发工具
├── dev.sh
├── build.sh
└── ...
```

#### 目录与架构层级映射

- 每个顶层目录对应一个 infra 关切：`app` = 后端网关、`console` = 管理控制台、
  `deploy` = 部署、`scripts` = 脚本工具。
- `app/` 内部按 Cargo workspace 分层，每个 crate 对应一个架构层级或边界；禁止一个
  crate 跨越多个层级。
- 如果新功能不属于现有层级或 infra 关切，应先扩展/拆分定义，而不是在现有目录中
  硬塞逻辑。
- 跨层调用 MUST 通过该层暴露的公共 API（trait/类型/消息契约），禁止直接穿透到
  内部模块。

#### 项目根目录 — 配置与 AI 协作壳

除上述 infra 顶层目录外，项目根目录 SHOULD **只**包含配置、工具链和 AI vibe-coding
相关文件。允许的根级条目：

- **包管理**：`Cargo.toml`（workspace 根，若 `app/` 已含 workspace 则可省略）、
  `Cargo.lock`
- **Rust 工具链**：`rust-toolchain.toml`、`clippy.toml`
- **构建/配置**：`pingogate.yaml`
- **格式化/Lint**：`.rustfmt.toml`、`deny.toml`
- **Git**：`.gitignore`、`.gitattributes`
- **AI / Vibe Coding**：`CLAUDE.md`、`.claude/`、`.specify/`、`.codegraph/`、
  `.cursorrules`
- **环境**：`.env`、`.env.example`
- **CI/CD**：`.github/workflows/`
- **文档**：`README.md`、`blueprint/`（仅限根目录）

#### 根目录禁止项

- MUST NOT 在项目根放置 `.rs` 源文件（workspace `Cargo.toml` 除外）。
- MUST NOT 创建与 infra 层级无关的顶层目录（例如根目录下禁止 `src/`、`crates/`、
  `services/`、`utils/`、`temp/`）。
- MUST NOT 在根目录散落测试文件 —— 测试与源码同位于 `app/**/src/` 或
  `app/**/tests/`（控制台测试位于 `console/` 内）。

**rationale**：顶层目录直接反映 infra 关切和架构分层，任何人看到 `app/`、
`console/`、`deploy/`、`scripts/` 就能理解系统边界。`app/` 内部的 crate 再按网关
分层映射，使新功能该放到哪个位置由层级决定，而不是个人判断。

### V. 代码指标与限制

可执行、可度量的约束，确保每个代码单元都保持小而聚焦、可评审。

- **文件长度**：单个源文件 MUST NOT 超过 **300 行**（不含空行与 import）。超出则
  按职责拆分。
- **函数体**：MUST NOT 超过 **50 行**。提取辅助函数或分解步骤。
- **函数参数**：MUST NOT 超过 **4 个**。需要更多上下文时使用 struct 或 builder。
- **圈复杂度**：单个函数 MUST NOT 超过 **10**。将条件分支提取为命名谓词或策略函数。
- **嵌套深度**：MUST NOT 超过 **3 层**（if/for/while/match arm/try）。使用提前返回、
  guard clause 或提取内部块。
- **模块公共表面**：crate/module SHOULD 暴露尽可能窄的公共 API。没有外部消费者时
  优先使用 `pub(crate)` 而非 `pub`。

**rationale**：小单元更容易测试、评审和理解。硬限制防止渐进式腐化。

## 架构与设计

*如何组织代码。具有层级依赖性 —— 在应用完整 DDD 战术模式或完整错误分类前，
先查阅原则 I。*

### VI. 请求管线架构

每个请求 MUST 流过一个稳定、有序的管线。Provider adapter MUST NOT 拥有整个
网关生命周期。

```text
入口
→ 协议识别
→ 鉴权与授权
→ 上下文解析
→ 路由
→ 策略执行
→ 请求转换
→ 上游派发
→ 响应转换
→ 上下文提交
→ 可观测性
→ 用量与计费
```

#### 管线规则

- 每个阶段 MUST 有单一、明确定义的职责。
- 阶段之间 MUST 通过显式、类型的契约（struct/trait）通信，而非原始 HTTP 工件。
- 阶段 MUST NOT 回退到更早阶段；状态通过共享请求上下文向前流动。
- Provider adapter 负责 Provider namespace、能力声明和 Provider 特定转换 ——
  不负责 listener 管理、路由或策略执行。

**rationale**：固定管线让网关可预测、可测试、可扩展。新能力插入已知位置，
而不是创建临时旁路。

### VII. SOLID 原则

每个模块、struct、trait 和函数 MUST 遵守 SOLID：

- **S — 单一职责**：每个单元只有一个变更理由。listener MUST NOT 解析请求体；
  router MUST NOT 直接调用上游。
- **O — 开闭原则**：通过新增 provider adapter、策略插件或转换步骤扩展行为，
  而不是修改现有管线代码。
- **L — 里氏替换**：子类型 MUST 能在不改变正确性的前提下替换基类型。优先组合
  而非继承，以避免 LSP 违例。
- **I — 接口隔离**：消费者 MUST NOT 依赖不用的方法。将宽泛 trait 拆分为聚焦的
  trait（例如 `Authenticator`、`RateLimiter`、`Router`）。
- **D — 依赖倒置**：高层模块 MUST 依赖抽象（trait/类型），而非具体实现。
  依赖在构造时注入。

**rationale**：SOLID 让代码库保持灵活、可测试，并抵御连锁变更。

### VIII. 低耦合、高内聚

- 模块 MUST 通过窄而明确的接口通信（trait、事件契约、请求上下文）。
- 模块 MUST NOT 导入另一模块的内部实现细节 —— 只使用其公共 API。
- 相关逻辑 MUST 放在一起。如果两个函数总是因同一理由变更，它们应属于同一模块。
- 循环依赖 **禁止**。引入 trait 或中介者来打破循环。
- 当时间耦合不必要，优先使用消息传递或领域事件，而非直接跨模块调用。

**rationale**：低耦合支持独立开发、测试和部署；高内聚让模块自解释。

### IX. 错误处理策略

错误 MUST 被分类、分层，并在正确层级处理。未处理错误、泛化捕获和静默吞没
均被禁止。

#### 错误分类

| 错误类型 | 层级 | 命名约定 | 示例 |
| -------- | ---- | -------- | ---- |
| 领域错误 | Domain | `{Context}{Reason}Error` | `InvalidModelAliasError` |
| 应用错误 | Application | `{UseCase}{Reason}Error` | `UpstreamUnavailableError` |
| 基础设施错误 | Infrastructure | 包装外部错误 | `HttpClientError` |
| 校验错误 | 边界 | 配置/schema 解析错误 | `ConfigValidationError` |
| 未预期错误 | 任意 | 不捕获 —— 让它崩溃 | panic、对外部输入使用 `unwrap()` |

#### 错误分层规则

- **领域层**：MUST 返回用统一语言描述业务规则违反的领域错误。领域错误 MUST NOT
  引用 HTTP 状态码或传输细节。
- **应用层**：MUST 将领域错误翻译为应用层结果。MAY 添加上下文（哪个请求、哪个
  上游）。MUST NOT 让基础设施错误未捕获地泄漏给调用方。
- **HTTP 边界**：MUST 将应用错误映射为 HTTP 响应。这是**唯一**出现 HTTP 状态码的
  地方。
- **基础设施层**：MUST 将外部 crate 错误转换为项目定义的错误类型。原始
  `reqwest::Error` 或 `serde_json::Error` 未经转换 MUST NOT 超出 adapter。

#### 禁止的错误模式

- **静默吞没**：忽略错误的 match arm 或 `if let` 被禁止。每个错误 MUST 被处理、
  带上下文记录，或被传播。
- **泛化 catch-all**：`catch_unwind` 或把 Result 扁平化到丢失错误信息的宽匹配
  被禁止。匹配具体错误变体。
- **对外部输入 panic**：在外部输入上使用 `unwrap()`、`expect()` 被禁止。使用 `?`
  和显式错误变体。
- **领域层出现 HTTP 码**：领域层 MUST NOT 导入或引用 HTTP 状态码或任何传输相关
  内容。

#### 自定义错误基类

所有项目特定错误 SHOULD 使用公共错误 enum 或 trait，以支持统一日志、指标标签和
HTTP 映射：

```rust
pub enum AppError {
    Domain(DomainError),
    Application(ApplicationError),
    Infrastructure(InfrastructureError),
    Validation(ValidationError),
}
```

**rationale**：结构化错误处理防止两个最坏结果：静默失败导致数据损坏，以及泄漏
内部实现给调用方。每一层使用自己的错误语言。

### X. 状态分离

PingoGate MUST 明确区分三类状态。

#### 能力仓库（Capability Repository）

描述系统**能做什么**：Provider 能力声明、模型元数据、转换模板、策略模板。
后端可以是本地目录、Git、HTTP registry 或 OCI artifact registry。MUST NOT 存储
用户密钥、租户策略、用量或会话状态。

#### 运行时配置（Runtime Config）

描述本实例**如何运行**：listener 配置、证书、启用的 provider、上游 channel、
模型路由、策略、指标 sink。后端可以是 YAML/JSON、环境变量、Kubernetes
ConfigMap/Secret 或数据库发布版本。通过 `ArcSwap` 风格的 snapshot 原子重载。

#### 控制面状态（Control Plane State）

描述**谁在使用平台以及用了多少**：用户、租户、项目、API key、provider key、
配额、预算、用量、审计、会话状态。后端可以是 file/memory、SQLite、Postgres、
Redis 或对象存储。

#### 分离规则

- 仓库 MUST NOT 把能力元数据与运行时密钥混为一谈。
- 运行时配置重载 MUST NOT 改变控制面状态。
- 控制面变更 MUST NOT 直接修改活跃运行时 snapshot，除非经过显式的发布/重载步骤。

**rationale**：清晰的分离防止意外数据泄漏、支持审计，并允许数据面与控制面
独立扩展。

### XI. Provider 抽象

Provider adapter MUST 按**能力族（capability family）**建模，而非仅按 endpoint。
Endpoint 一致性重要；能力一致性更重要。

#### 能力族示例

- `generation.stateless`：OpenAI Chat Completions、Anthropic Messages、Gemini
  `generateContent`。
- `generation.stateful`：OpenAI Responses、OpenAI Conversations、Google
  Interactions。
- `realtime.live`：OpenAI Realtime、Gemini Live。
- `embedding`、`batch`、`file.media`、`image.audio.video`、`tools.agents`、
  `safety.moderation`、`platform.admin`。

#### Adapter 规则

- 每个 adapter MUST 声明支持哪些能力族。
- 每个 adapter MUST 暴露 Provider namespace（例如 `openai`、`anthropic`、
  `gemini`）及兼容表面（如适用）。
- Adapter MUST 通过能力 IR 实现转换步骤，而不是通过一次性 endpoint handler。
- 不支持的能力族 MUST 被显式拒绝并返回清晰错误。

**rationale**：按能力族建模支持跨协议桥接、统一计量和可预测路由。

### XII. 配置驱动运行时与热重载

运行时行为 MUST 由声明式配置驱动，采用 Nginx 风格热重载，而不是把原生动态插件
作为主扩展路径。

#### 配置来源

- 主配置：`pingogate.yaml`（YAML）和 JSON 能力目录。
- 覆盖：环境变量、Kubernetes ConfigMap/Secret、数据库发布版本。

#### 热重载规则

- 重载 MUST 可通过 `SIGHUP`、admin API、文件监听或 UI 发布动作触发。
- 重载流程：加载候选配置 → schema 校验 → 语义校验 → 构建不可变
  `RuntimeSnapshot` → dry-run / 健康检查 → 原子切换 → 记录版本与回滚目标。
- 重载失败 MUST NOT 修改活跃运行时 snapshot。
- 进行中的请求 MUST 继续使用其启动时的 snapshot。

#### 扩展路径

- 默认扩展机制：声明式 TransformPlan 和 profile 包。
- WASM 插件是未来高级选项，不是默认。
- 原生 `.so`/`.dylib`/`.dll` 插件 NOT 默认扩展路径。

**rationale**：声明式重载让网关可被基础设施团队观察、审计和操作。它避免了在
热路径引入动态原生代码的安全与兼容性风险。

## 代码质量与实践

*如何编写、测试、记录和提交代码。*

### XIII. 测试驱动开发（TDD）— 不可协商

所有生产代码 MUST 遵循 Red-Green-Refactor 循环。

1. **Red**：编写一个失败的测试来捕获需求。
2. **Green**：编写最小实现让测试通过。
3. **Refactor**：在测试保持绿色的前提下清理重复和结构。

- 测试 MUST 在实现开始前编写并被观察到**失败**。
- 每个公共函数或方法 MUST 至少有一个单元测试。
- 集成测试 MUST 覆盖：跨模块通信、provider adapter 往返、请求管线阶段、
  配置重载。
- 测试名称 MUST 描述被测行为，而非实现（例如 `rejects_empty_document_title`，
  而非 `test_create_document`）。

**rationale**：TDD 产生可验证正确的代码，并作为活的文档。

### XIV. 文档与注释

所有 Rust 与 TypeScript 代码 MUST 一致地文档化，且**注释是强制性的**。所有代码
注释（包括文档注释和行内注释）MUST 使用**英文**。

#### 文档注释要求

- 每个**公共**模块、struct、trait、enum、函数、方法和类型别名 MUST 有文档注释。
- 私有辅助函数如果名称和签名自解释，MAY 省略文档。
- 文档注释 MUST 解释**为什么**（非显而易见的 design choice），而非仅重述名称。

#### Rust 文档注释格式

```rust
/// Brief one-line summary in imperative mood.
///
/// Longer description when needed, explaining purpose and constraints.
///
/// # Arguments
///
/// * `name` - Description of the parameter
///
/// # Returns
///
/// Description of the return value
///
/// # Errors
///
/// Returns `AppError::Validation` when the input is invalid.
///
/// # Examples
///
/// ```
/// let result = my_function("input");
/// ```
```

#### 注释风格细则

- **模块文档**使用 `//!`；**项文档**使用 `///`。
- **行内注释**使用 `//`；避免使用 `/* */` 块注释。
- 摘要行 MUST 使用祈使语气（例如 "Fetch user data"，而非 "Fetches user data" 或
  "This function fetches user data"）。
- 行内注释 MUST 解释**为什么**，never **什么**。代码自身说明什么。
- 每行注释/文档 SHOULD 不超过 100 个字符，避免 awkward 硬折行。
- **TODO 格式**：`// TODO(username): description`，例如
  `// TODO(alice): implement exponential backoff after upstream health check failure`。
- **FIXME 格式**：`// FIXME(username): known issue description`。
- **SAFETY 注释**：每个 `unsafe` 块 MUST 有 `// SAFETY:` 注释，说明为何必要及
  保持不变式。
- 禁止空/桩文档注释（`/// TODO` 或裸 `///`）。
- 禁止在文档中重复类型签名（例如不要写 "takes a string and returns a number"）。

#### TypeScript / React 注释

- 控制台代码同样适用“公共 API 必须注释”规则，且注释 MUST 使用英文。
- React 组件 props interface SHOULD 与组件同文件，或位于同目录的 `types.ts`。
- 复杂 hook、context 或工具函数 MUST 有 JSDoc/TSDoc 风格注释。

**rationale**：一致的文档化让 IDE 悬停提示、自动化文档和新人 onboarding 都成为
可能。强制注释防止“代码即文档”的借口。

### XV. 原子提交

- 每个完成的功能、bugfix 或重构步骤 MUST 作为独立 git 提交，然后才开始下一单位
  工作。
- 提交信息 MUST 遵循
  [Conventional Commits](https://www.conventionalcommits.org/)：
  `<type>(<scope>): <subject>`。
- 一个 commit MUST NOT 混合无关变更。如果任务同时产生功能和重构，应拆分为多个
  提交。
- 每次提交前测试 MUST 通过。

**rationale**：原子提交创造可评审、可 bisect、可回滚的历史。

## 工具与 AI 工作流

*支持开发的工具和 AI 技能。*

### XVI. CodeGraph 优先

当仓库根存在 `.codegraph/` 目录时：

- 在诉诸 grep/find/手动读文件之前，MUST 使用 `codegraph_explore`（MCP 工具或 CLI）。
- 一次 `codegraph_explore` 调用返回逐字源码、调用路径和影响范围分析 —— 用它
  指导编辑。
- 修改代码时，在变更任何公共 API 前查询 CodeGraph 的调用方和依赖方，以评估影响。
- 若 `.codegraph/` 不存在，完全跳过 CodeGraph。

**rationale**：预索引智能消除重复探索、降低 token 成本，并揭示隐藏依赖。

### XVII. AI 辅助开发技能

开发工作流 MUST 利用可用的 AI 编码技能（slash 命令），以确保持续一致的质量和
最新知识。技能不是可选便利 —— 它们是特定工作阶段的强制工具。

#### 按活动强制使用的技能

| 活动 | 技能 | 触发时机 |
| ---- | ---- | -------- |
| 库/框架使用 | `context7` | 在编写任何使用库、框架、SDK 或 API 的代码之前。即使对知名 crate 或 React 包也要查询当前文档。 |
| UI/UX 设计决策 | `frontend-design` | 在构建新控制台组件或重塑现有组件之前。用于审美方向、字体和避免模板化默认界面。 |
| UI 实现 | `ui-ux-pro-max` | 在规划、构建、评审或改进任何控制台 UI/UX 代码时。覆盖样式、调色板、字体配对、UX 指南和 shadcn/ui 使用。 |
| 安全敏感变更 | `security-review` | 在合并认证、授权、密钥处理或上游 TLS 变更之前。 |
| 代码评审 | `code-review` | 在认为功能完成之前；用于正确性、复用和简化。 |
| 简化 | `simplify` | 功能可用后，清理重复和偶然复杂度。 |
| 验证 | `verify` | 实现后，在运行中的应用或测试中确认行为。 |
| 深度研究 | `deep-research` | 在需要多源验证的架构决策之前。 |
| 能力发现 | `find-skills` | 当遇到可能受益于未知专业技能的任务时。 |

#### 使用规则

- **context7 不可协商**：每次代码引用外部 crate、React 包或框架 API 时，MUST
  调用 `context7` 获取当前文档。即使开发者自认熟悉 API —— 版本变化、破坏性变更
  和弃用没有文档检查是看不见的。
- **UI 工作需要 design 和 implementation 两个技能**：任何控制台 UI 任务，先调用
  `frontend-design` 定方向，再调用 `ui-ux-pro-max` 获取实现指导。跳过设计评审会
  产生通用、千篇一律的界面。
- **手动努力前先 find-skills**：面对不熟悉领域或任务类型时，调用 `find-skills`
  检查可安装技能，避免重复造轮子。
- **技能补充而非替代原则**：技能输出 MUST 仍遵守所有其他宪法原则（代码指标、
  TDD、SOLID 等）。一个建议 200 行函数的技能并不覆盖原则 V 的 50 行函数限制。

#### 扩展技能集

当安装一个广泛适用本项目的新技能时：

1. 通过宪法修正案将其加入上表。
2. 明确触发条件（“何时调用”）。
3. 修正案遵循标准治理流程（原则版本、带 rationale 的 PR）。

**rationale**：AI 辅助开发的好坏取决于工具纪律。强制调用技能可防止知识腐化
（过时的 API 使用）、设计漂移（通用 UI）和重复劳动（重复造轮子）。

### XVIII. Subagent 开发监控

当使用 subagent（`Agent`、`Workflow` 或后台 Bash 任务）执行开发任务时，MUST
监控其存活状态，防止静默失败。

#### 监控规则

- 主会话 MUST 在 subagent 返回前保持关注；不得假设 agent 会自行报告失败。
- 若 subagent 异常退出或无响应，主会话 MUST 向用户报告失败状态、已输出内容和
  后续建议，而不是继续假装任务完成。
- 对长时间运行的后台任务，MUST 使用 `TaskOutput` 或等效机制检查状态。
- 对并行 subagent，MUST 等待所有结果并单独处理失败项；禁止因一个 agent 失败而
  丢弃其余结果且不报告。

**rationale**：Subagent 是放大生产力的工具，但静默失败会让工作流产生幻觉式
进度。显式监控是信任的前提。

## 网关专项工程

*构建高信任、高吞吐 LLM 网关时出现的原则。*

### XIX. 可观测性

PingoGate MUST 默认暴露分层、机器可读的可观测性。

#### 必需观测信号

- **请求指标**：请求速率、状态码、延迟、上游延迟、重试次数、fallback 次数。
- **Token 指标**：输入 token、输出 token、推理 token、缓存读/写 token。
- **成本指标**：provider 成本、租户价格、配额消耗。
- **缓存指标**：prompt cache 命中/未命中、上下文桥 materialization 大小。
- **Realtime 指标**：会话时长、事件数、断开原因。
- **网关指标**：TLS 错误、证书过期、listener 健康、重载状态。

#### 日志规则

- 使用结构化日志（`tracing`）并保持字段名一致。
- 每个请求 MUST 携带 trace/请求 ID 贯穿管线。
- 敏感值（API key、token）MUST 从日志中脱敏。

#### 指标 Sink

- stdout 结构化日志
- Prometheus
- OpenTelemetry
- 数据库用量表
- 审计事件流

**rationale**：没有可观测性的网关无法规模化运维。指标和日志是一等特性，不是
事后补充。

### XX. 安全、保密与隐私

PingoGate 处理高价值凭证和用户数据。安全与隐私不可协商。

#### 安全规则

- **默认 Safe Rust**：`unsafe` 代码被禁止，除非绝对必要并经过显式评审。
- **unsafe 必须说明理由**：每个 `unsafe` 块 MUST 有注释说明为何必要及保持不变式。
- **不信任输入不 panic**：对外部数据使用 `unwrap()`、`expect()` 和直接索引被
  禁止。使用 `Result` 和显式错误处理。

#### 保密规则

- Provider key、API key 和证书 MUST 静态加密，且不得以明文形式记录。
- Admin API 端点 MUST 通过 `Principal` / `AuthContext` /
  `authorize(action, resource)` 抽象认证每个请求，即使是 bootstrap 模式。
- 上游连接的 TLS 证书 MUST 被校验，除非在受控开发环境中显式禁用。

#### 隐私规则

- 默认 MUST NOT 持久化完整 prompt/response 内容。
- 仅在租户或项目策略明确允许时才可持久化内容，且 MUST 遵守保留和加密策略。
- 审计日志 MUST 记录访问模式，而不存储敏感内容。

**rationale**：一个泄漏的 Provider key 或存储的 prompt 可能危及整个租户。安全与
隐私必须内建于架构，而非后期补丁。

### XXI. 性能与延迟

PingoGate 是网关；延迟和吞吐是产品的一部分。

#### 性能规则

- **尽可能零拷贝**：在热请求路径避免不必要的缓冲区拷贝。
- **异步上下文中无阻塞 I/O**：所有 I/O MUST 是异步的。CPU 密集型工作 MUST
  放到阻塞线程或专门任务中。
- **背压**：上游过载 MUST 用显式背压处理，而非无界队列。
- **资源预算**：每个功能在设计阶段 MUST 定义预期的 p50/p95 延迟和内存影响。

#### 延迟预算

- 网关开销（不含上游延迟） Stateless 透传应低于 **5 ms p50** 和 **20 ms p95**。
- 协议桥开销应低于 **50 ms p95**。
- 超出预算的功能 MUST 在 PR 中标记并重新评估。

#### 度量

- 热路径变更 MUST 添加性能测试。
- 性能回归 MUST 阻止合入，除非有明确理由。

**rationale**：用户选择 Rust + Pingora 是为了性能。宪法必须保护这一投资。

### XXII. 项目语言规范

项目级文档与沟通 MUST 使用**中文**；代码、API 与提交信息 MUST 使用**英文**。

#### 适用范围

- **强制中文**：宪法、spec、plan、tasks、bug 报告、ADR、设计文档、PR 描述、
  代码评审评论、团队内部讨论。
- **强制英文**：
  - 代码注释（Rust `//`/`///`，TypeScript/JSDoc，React 组件注释）。
  - 所有代码标识符：函数名、变量名、类型名、模块名、crate 名、API endpoint、
    错误码、配置键。
  - 提交信息：遵循 Conventional Commits，完整使用英文。
  - 公共 API 文档、README（面向开发者）、OpenAPI/JSON schema 字段名。
- **用户可见文案**：控制台 UI、API 错误消息、CLI 输出等优先中文，并预留国际化
  扩展点。

#### 例外

- 外部引用、链接、工具命令、第三方术语（如 `pingogate.yaml`、Cargo、Pingora、
  `ArcSwap`）保持原文。
- 提交信息中的 scope 使用英文，与代码标识符一致。

**rationale**：项目文档用中文降低团队沟通成本；代码、API 和提交信息用英文保持与
Rust/TypeScript 生态、开源社区和自动化工具的兼容。用户可见文本优先中文则符合
产品定位。

## 开发工作流

### 实施前检查清单

1. 确认任务可追溯至 spec 或 user story。
2. 按原则 I 判定复杂度层级（简单/中等/复杂）并记录。
3. 运行 `codegraph explore` 了解受影响范围（原则 XVI）。
4. 按原则 XVII 调用强制 AI 技能（例如 crate API 用 `context7`、控制台 UI 用
   `frontend-design` + `ui-ux-pro-max`）。
5. 编写失败测试（TDD Red 阶段，原则 XIII）。
6. 实现最小方案（TDD Green 阶段）。
7. 在测试保持绿色时重构（TDD Refactor 阶段）。
8. 验证所有代码指标（原则 V）通过。
9. 若使用 subagent，确认其已返回结果，并处理失败（原则 XVIII）。
10. 使用 Conventional Commit 提交（原则 XV）。

### 代码评审闸门

- 评审时必须对照本宪法所有原则检查，范围限于功能声明的复杂度层级。
- 评审者 MUST 验证：
  - [ ] 复杂度层级已记录（原则 I）。
  - [ ] 无投机代码或过早抽象（原则 II）。
  - [ ] 新增依赖有理由（原则 III）。
  - [ ] 无应用代码在 `app/`、`console/` 等 infra 顶层目录外（原则 IV）。
  - [ ] 代码指标符合限制（原则 V）。
  - [ ] 请求管线阶段被尊重（原则 VI）。
  - [ ] 无循环 crate 依赖（原则 VIII）。
  - [ ] 错误按层分类处理（原则 IX）。
  - [ ] 状态分离被尊重（原则 X）。
  - [ ] Provider adapter 声明能力族（原则 XI）。
  - [ ] 配置变更遵守热重载规则（原则 XII）。
  - [ ] 测试先写并被观察到失败（原则 XIII）。
  - [ ] 公共 API 已注释（原则 XIV）。
  - [ ] 提交信息符合 Conventional Commits（原则 XV）。
  - [ ] AI 技能在要求处已调用（原则 XVII）。
  - [ ] 使用 subagent 时已监控且无静默失败（原则 XVIII）。
  - [ ] 无未经说明的 `unsafe`（原则 XX）。
  - [ ] 日志或配置中无明文密钥（原则 XX）。
  - [ ] 控制台组件使用 shadcn/ui 或已审批例外（原则 III）。
  - [ ] TypeScript 控制台不包含业务逻辑、鉴权或密钥处理（原则 III）。
  - [ ] 项目文档使用中文，代码/API/提交信息使用英文（原则 XXII）。

### 持续集成

- 每次 push CI MUST 运行：
  - Rust（在 `app/` 目录）：`cargo check`、`cargo clippy`、`cargo test`、格式化检查。
  - 控制台（在 `console/` 目录）：`pnpm install`、类型检查、lint、构建。
  - 部署配置（在 `deploy/` 目录）：语法/模板校验（如适用）。
- CI MUST 拒绝对外部输入使用 `unwrap()` 或 `expect()` 的 PR。
- CI MUST 拒绝测试未通过的 PR。
- CI SHOULD 对热路径变更运行性能基准测试。

## 治理

本宪法是 PingoGate 所有开发决策的最高权威。当实践与本文件冲突时，以本宪法为准。

### 修订程序

1. 在独立 PR 中提出变更并附带 rationale。
2. 修正案 MUST 包含现有代码违反新规则的迁移计划。
3. 按语义化版本更新 `CONSTITUTION_VERSION`：
   - **MAJOR**：原则移除、不兼容重定义或结构重组。
   - **MINOR**：新增原则或实质性扩展。
   - **PATCH**：澄清、错别字或非语义性精炼。
4. 将 `LAST_AMENDED_DATE` 更新为合并日期。

### 合规审查

- 每个 PR MUST 包含针对核心原则的自我评估，范围限于声明的复杂度层级
  （原则 I）。
- 季度审计 SHOULD 验证全代码库合规性。
- 合并后发现的违例 MUST 作为技术债务 issue 跟踪，并在当前 sprint 内解决。

### 指导文件

- 使用 `CLAUDE.md` 和 `.claude/CLAUDE.md` 存放 AI 辅助编码的运行时开发指导。
- 使用 `blueprint/` 存放长期产品与架构方向。
- 本宪法提供权威规则；指导文件提供操作指令，但 MUST NOT 与之矛盾。

**版本**：1.6.0 | **通过日期**：2026-06-29 | **最后修订**：2026-06-29
