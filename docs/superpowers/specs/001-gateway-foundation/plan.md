# Implementation Plan: PingoGate 网关地基（首个实现方向）

**Branch**: `001-gateway-foundation` | **Date**: 2026-06-29 | **Spec**: [spec.md](./spec.md)

**Input**: Feature specification from `/specs/001-gateway-foundation/spec.md`

## Summary

把 PingoGate 蓝图的「首个实现方向 / 网关地基」落为可运行的单二进制 LLM 网关：以 Pingora 的 `ProxyHttp` 生命周期承载固定请求管线（协议识别 → 网关 Key 鉴权 → 路由 → 上游认证注入 → 透传/SSE → 镜像错误 → 可观测），支持 OpenAI-compatible Chat Completions、Anthropic Messages、Gemini `generateContent` 三类协议的原生透传与流式（SSE）转发；运行时由 `pingogate.yaml` 驱动，通过不可变 `RuntimeSnapshot` + `ArcSwap` 原子热重载；提供经 `Principal` / `AuthContext` / `authorize` 边界鉴权的机器可读 Admin API（health / readiness / config validation / reload / reload status）；默认 file + memory 运行时，无外部数据库。

技术取向（详见 [research.md](./research.md)）：管线各阶段映射到 Pingora `ProxyHttp` 的 filter 回调；网关自身错误在管线边界镜像目标 Provider 的原生错误体；上游认证由核心在 `upstream_request_filter` 注入，adapter 不接触明文 Provider Key；Admin API 用 Pingora `ServeHttp` 服务承载于独立 admin listener，不引入额外 HTTP 框架。

## Technical Context

**Language/Version**: Rust（最新 stable，`edition = "2021"`），核心后端 100% Safe Rust（`unsafe` 禁止，除非显式评审）。

**Primary Dependencies**: Pingora（`pingora` / `pingora-proxy` / `pingora-core`，热路径与 Admin `ServeHttp`）、tokio（异步运行时、signal、超时）、serde + serde_json + serde_yaml（配置/数据序列化）、arc-swap（`RuntimeSnapshot` 原子热切换）、tracing + tracing-subscriber（结构化日志）、prometheus / metrics + opentelemetry（指标）、notify（`pingogate.yaml` 文件监听，可选触发重载）。新增非锁定 crate（arc-swap、serde_yaml、notify、prometheus client、opentelemetry）须在实现 PR 中按宪法 III 附理由。

**Storage**: file + memory（宪法 X 的 Runtime Config 来自 `pingogate.yaml`；Control Plane State——具名网关 Key、reload 版本/回滚目标——驻内存）。本阶段无 SQLite/Postgres。Provider Key 经密钥引用（环境变量）解析，不由本系统持久化。

**Testing**: cargo test —— 单元测试（每个公共函数）、集成测试（管线阶段、provider 透传往返、配置重载）、契约测试（Admin API、`pingogate.yaml` schema、三协议透传与镜像错误）。遵循宪法 XIII TDD：测试先写并观察失败。

**Target Platform**: Linux server（单个 `pingogate` 二进制；x86_64 / aarch64）。

**Project Type**: Rust Cargo workspace（后端网关服务，单一可执行）；本阶段不含 `console/` 前端（嵌入式控制台延后，见 spec Out of Scope）。

**Performance Goals**: 无状态透传场景，网关在上游延迟之外的额外开销 **< 5 ms p50 / < 20 ms p95**（SC-003，源自宪法 XXI 延迟预算）。吞吐初始目标（plan 阶段补足 spec 未定项，待基准验证、非契约性 SC）：4 核节点上 ≥ 2000 req/s 非流式透传、≥ 1000 并发 SSE 流，网关 CPU 不成为瓶颈；最终吞吐主要受上游约束。

**Constraints**: 热路径零拷贝流式转发（SSE body filter 不缓冲整流）；异步上下文无阻塞 I/O；上游过载用显式背压而非无界队列；密钥明文 MUST NOT 进日志/指标；上游 TLS 默认校验。

**Scale/Scope**: 3 个 Provider 家族（openai-compatible / anthropic / gemini）、1 个能力族（`generation.stateless`）、一组具名网关 Key、5 个 Admin API 端点、单一 `pingogate.yaml`。

## Constitution Check

*GATE: Phase 0 前必须通过；Phase 1 设计后复检。*

**复杂度层级判定（宪法 I）**：**中等**。理由：新增多个 provider adapter、路由策略、鉴权模式与跨模块的热重载，含条件业务逻辑，但不涉及协议桥、上下文虚拟化或多租户控制面（那些已划入 Out of Scope）。下列原则按「中等」列强制项核对。

| 原则 | 闸门要求（中等） | 本计划如何满足 | 判定 |
| ---- | ---- | ---- | ---- |
| II DRY/KISS/YAGNI | 强制 | 范围严格限定在 spec 的 FR；crate 仅按架构层级拆分，无投机抽象；延后能力不预先建框架 | ✅ |
| III 技术栈 | 强制 | Rust + Pingora（热路径）+ tokio + serde + tracing + Prometheus/OTel + cargo test，全部锁定栈；新增 crate（arc-swap 等）在 PR 附理由；本阶段无前端故 React/shadcn N/A | ✅ |
| IV 目录结构 | 强制 | `app/` Cargo workspace 按层级拆 crate（见 Project Structure）；无 console；根目录仅配置/AI 壳 | ✅ |
| V 代码指标 | 强制 | 文件 ≤300 行、函数 ≤50 行、参数 ≤4、圈复杂度 ≤10、嵌套 ≤3 —— 在 tasks/实现与 CI 强制 | ✅（实现期强制） |
| VI 请求管线 | 强制 | 管线阶段一一映射 Pingora `ProxyHttp` filter；adapter 不拥有网关生命周期 | ✅ |
| VII SOLID | 强制 | adapter / authenticator / router 等聚焦 trait；构造期注入；组合优先 | ✅ |
| VIII 低耦合高内聚 | 强制 | 引入 `core` 共享类型/错误 crate 打破循环；crate 间仅经公共 trait/类型通信 | ✅ |
| IX 错误处理 | 完整分类强制 | `AppError`（Domain/Application/Infrastructure/Validation）；HTTP 码仅在边界；镜像错误体在管线边界生成 | ✅ |
| X 状态分离 | 强制 | Capability Repo（adapter 能力声明）/ Runtime Config（snapshot）/ Control Plane State（具名 Key、reload 状态）三分；重载不触控制面状态 | ✅ |
| XI Provider 抽象 | 能力族强制 | adapter 声明 `generation.stateless`、暴露 namespace；不支持能力族显式拒绝 | ✅ |
| XII 热重载 | 强制 | ArcSwap snapshot；候选配置 校验→构建→原子切换→记录版本/回滚；失败不改活跃 snapshot | ✅ |
| XIII TDD | 强制 | 契约/集成/单元测试先写；覆盖管线、透传往返、重载 | ✅（tasks 落实） |
| XIV 文档与注释 | 强制 | 公共 API 英文文档注释；本计划文档中文 | ✅ |
| XV 原子提交 | 强制 | Conventional Commits，按工作单元拆分 | ✅ |
| XVI CodeGraph | 强制 | 存在 `.codegraph/`；编辑前用 `codegraph_explore` 评估影响 | ✅ |
| XVII AI 技能 | 强制 | 已用 context7 查 Pingora；auth/key/TLS 变更合并前走 security-review；功能完成走 code-review/simplify/verify | ✅ |
| XVIII Subagent 监控 | —— | 如用 subagent 则监控存活 | ✅ |
| XIX 可观测性 | 强制 | tracing 结构化日志带 trace id；Prometheus 指标按 provider + 能力族打标签；密钥脱敏 | ✅ |
| XX 安全/保密/隐私 | 强制 | Safe Rust；密钥不入日志、经引用解析；Admin 经 authorize 边界；上游 TLS 校验；默认不持久化内容 | ✅ |
| XXI 性能 | 延迟预算推荐 | SC-003 延迟预算；零拷贝流式；热路径变更加性能测试 | ✅ |
| XXII 项目语言 | 强制 | 文档中文、代码/API/提交英文 | ✅ |

**初判结论**：无违规，Complexity Tracking 留空。`core` 与 `config` 两个 crate 超出宪法 IV 显式列举的 `app/` 子目录，但属宪法 IV「新功能不属于现有层级时应先扩展/拆分定义」的合规扩展（`core` 服务全 crate 并打破循环依赖、`config` 承载 RuntimeSnapshot 与热重载），非违规。

**设计后复检（Phase 1 后）**：data-model、contracts、quickstart 未引入新违规——错误处理沿 `AppError` 分层且 HTTP 码仅在边界（IX）、三类状态分离（X）、adapter 声明能力族并显式拒绝（XI）、ArcSwap 快照热重载（XII）、契约测试先行（XIII）、密钥全程脱敏（XX）。Constitution Check 仍 **PASS**。

## Project Structure

### Documentation (this feature)

```text
specs/001-gateway-foundation/
├── plan.md              # 本文件（/speckit-plan 输出）
├── research.md          # Phase 0 输出
├── data-model.md        # Phase 1 输出
├── quickstart.md        # Phase 1 输出
├── contracts/           # Phase 1 输出
│   ├── admin-api.md           # Admin API 端点契约
│   ├── config-schema.md       # pingogate.yaml 配置契约
│   └── provider-passthrough.md # 三协议透传与镜像错误契约
└── tasks.md             # /speckit-tasks 输出（本命令不创建）
```

### Source Code (repository root)

```text
app/                       # 后端应用：Rust Cargo workspace
├── Cargo.toml             # workspace 根
├── pingogate/             # 主二进制与组合根：bootstrap、信号、装配各 crate
│   ├── src/
│   └── tests/             # 端到端/集成测试（三协议透传、重载、Admin API）
├── core/                  # 共享域类型与错误：AppError、RequestContext、
│   ├── src/               #   Principal/AuthContext、CapabilityFamily、ProviderKind
│   └── ...                #   （不依赖其他内部 crate，打破循环）
├── config/                # Runtime Config 层：pingogate.yaml schema、校验、
│   ├── src/               #   RuntimeSnapshot 构建、ArcSwap 持有、密钥引用解析
│   └── ...
├── listener/              # 网关基础设施层：公共入口 + admin 入口、TLS、
│   ├── src/               #   Pingora server/service 装配
│   └── ...
├── pipeline/              # 请求管线层：ProxyHttp 实现 —— 协议识别、网关 Key
│   ├── src/               #   鉴权、路由、上游认证注入编排、透传/SSE、错误映射、观测
│   └── ...
├── provider/              # Provider 命名空间层：adapter trait + 三 adapter
│   ├── src/               #   （namespace、能力族声明、认证注入描述、错误体形状）
│   └── ...
├── admin/                 # 控制面层：Admin API（ServeHttp）+ Principal/AuthContext/
│   ├── src/               #   authorize 边界 + reload 编排 + reload 状态
│   └── ...
└── storage/               # 存储抽象层：trait + file/memory 后端（配置来源、
    ├── src/               #   reload 版本/回滚记录）；env 密钥引用解析器
    └── ...
```

**Structure Decision**: 采用 `app/` 单一 Cargo workspace（宪法 IV），按架构层级拆 crate，每个 crate 对应一个层级且不跨层。依赖方向自底向上无环：`core` ← {`storage`,`config`,`provider`} ← `pipeline` ← {`listener`,`admin`} ← `pingogate`（`admin`/`listener` 依赖 `config`，`config` 依赖 `storage`+`core`）。本阶段不创建 `context/`（上下文桥延后）与 `console/`（控制台延后）。跨层调用只经各 crate 公共 trait/类型（宪法 VIII）。

## Complexity Tracking

> 无宪法违规，无需记录。（`core`/`config` 两 crate 为宪法 IV 允许的层级扩展，非违规项。）
