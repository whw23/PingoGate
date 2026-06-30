<!-- SPECKIT START -->
当前活跃 feature：**001-gateway-foundation（网关地基）**。

技术栈、项目结构、shell 命令等上下文以该 feature 的计划为准，按需阅读：

- 计划：`specs/001-gateway-foundation/plan.md`
- 规格：`specs/001-gateway-foundation/spec.md`
- 研究决策：`specs/001-gateway-foundation/research.md`
- 数据模型：`specs/001-gateway-foundation/data-model.md`
- 契约：`specs/001-gateway-foundation/contracts/`（admin-api、config-schema、provider-passthrough）
- 快速上手：`specs/001-gateway-foundation/quickstart.md`

技术栈（宪法锁定）：Rust + Pingora（热路径）+ tokio + serde + tracing + Prometheus/OpenTelemetry；
源码位于 `app/` Cargo workspace（按架构层级拆 crate：core/config/listener/pipeline/provider/admin/storage + pingogate 二进制）。
<!-- SPECKIT END -->
