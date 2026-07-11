# PingoGate

基于 [Pingora](https://github.com/cloudflare/pingora) 的高性能 LLM 网关，目标是**多用户、BYOK 优先**的 AI 流量治理平台。

## 当前状态

项目正在以「用户优先 / BYOK 为基座」重写。用户自带 Provider Key 与模型，完全私有，管理员不可见明文 Key；路由策略后置。

- **宪法**：[`.claude/rules/constitution.md`](.claude/rules/constitution.md)（项目宪法 v1.6.0，I~XXII 原则）
- **蓝图**：[`docs/blueprint/`](docs/blueprint/)（完整产品愿景，BYOK 详见第 16A 节）
- 旧实现（路由优先的单管理员代理范式）已归档至 `redesign` 分支，仅作历史参考。

## 技术栈

Rust + Pingora（热路径）+ tokio + serde + tracing + Prometheus/OpenTelemetry。源码位于 `app/` Cargo workspace。

---

> 本仓库处于重写初期，代码与文档随后续 feature 逐步建立。
