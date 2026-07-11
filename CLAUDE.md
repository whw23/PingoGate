<!-- SPECKIT START -->
当前活跃 feature：**待建（用户优先 / BYOK 为基座的重写）**。

项目方向已从 `001-gateway-foundation`（路由优先的单管理员中心化代理）转为「用户优先 / BYOK 为基座」：用户自带 Provider Key 与模型，完全私有，管理员不可见明文 Key；路由策略后置。旧实现已归档至 `redesign` 分支，仅作历史参考。

- **宪法**：`.claude/rules/constitution.md`（项目宪法 v1.6.0，I~XXII 原则，对所有 feature 具上位约束力）
- **蓝图**：`docs/blueprint/`（完整产品愿景与已确认决策，BYOK 详见第 16A 节）

技术栈（宪法 III 锁定）：Rust + Pingora（热路径）+ tokio + serde + tracing + Prometheus/OpenTelemetry；源码位于 `app/` Cargo workspace（按架构层级拆 crate）。前端仅 TypeScript（嵌入式控制台，延后）。
<!-- SPECKIT END -->
