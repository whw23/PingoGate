# PingoGate

多用户、BYOK 优先的 LLM 流量治理 SaaS 平台。用户自带 Provider Key 与模型，完全私有，管理员不可见明文 Key。

## 架构

双语言内核架构：

- **Rust 内核**（`core-rs/`，`pingogate-core`）：请求处理核心引擎 + 安全核心。Pingora 数据面管线（协议识别 / 鉴权 / 请求路由 / 请求转换 / 透传 / SSE）+ 路由引擎 + 转换引擎 + KeyVault（密钥加解密）+ 快照引擎。
- **Go 非内核**（`ctrl-go/`，`pingogate-ctrl`）：业务面 + 平台治理。控制面 API（用户 / 组织 / RBAC / 虚拟 key CRUD）+ OAuth / 会话 / 计费 / 用量 / 审计 + 控制台 BFF + DB。
- **通信**：Go 经 gRPC 把快照推给 Rust 内核，热路径零 DB 零解密。

## 当前状态

项目以「用户优先 / BYOK 为基座」重写。旧实现（路由优先单管理员代理）已归档至 `redesign` 分支，仅作历史参考。

- **宪法**：[`.claude/rules/constitution.md`](.claude/rules/constitution.md)（I~XXII 原则 + 双语言内核架构总纲）
- **路线图**：[`docs/platform-roadmap.md`](docs/platform-roadmap.md)（SaaS 平台 L0-L6 能力层 + 里程碑 + 横切架构）

## 技术栈

- Rust 内核：Pingora + tokio + serde + tracing + Prometheus/OTel（Safe Rust）。
- Go 非内核：net/http + chi + sqlx（SQLite/PG 可切）+ slog + prometheus/client_go。
- 跨语言：gRPC + `proto/` codegen。
- 前端：TypeScript/React，build 后嵌入 Go（L6 实现）。

---

> 本仓库处于重写初期，代码与文档随后续 feature 逐步建立。
