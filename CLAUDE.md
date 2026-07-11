<!-- SPECKIT START -->
当前活跃 feature：**待建（用户优先 / BYOK 为基座的重写）**。

项目方向已从 `001-gateway-foundation`（路由优先的单管理员中心化代理）转为「用户优先 / BYOK 为基座」：用户自带 Provider Key 与模型，完全私有，管理员不可见明文 Key；路由策略后置。旧实现已归档至 `redesign` 分支，仅作历史参考。

- **宪法**：`.claude/rules/constitution.md`（项目宪法，I~XXII 原则 + 双语言内核架构总纲，对所有 feature 具上位约束力）
- **路线图**：`docs/platform-roadmap.md`（SaaS 平台 L0-L6 能力层分解 + 里程碑 + 横切架构：鉴权/协议转换/计费/网关基础设施/已确认决策）

**双语言内核架构**（宪法总纲）：内核 Rust（请求处理核心引擎 + 安全核心：Pingora 数据面 / 路由引擎 / 转换引擎 / KeyVault / 快照引擎）+ 非内核 Go（业务面 + 平台治理：控制面 API / OAuth / 计费 / 用量 / 控制台 BFF / DB）。双二进制，Go 经 gRPC 推快照给 Rust 内核，热路径零 DB 零解密。

技术栈（宪法 III 锁定）：

- Rust 内核（`core-rs/`，`pingogate-core`）：Pingora + tokio + serde + tracing + Prometheus/OTel；Safe Rust。
- Go 非内核（`ctrl-go/`，`pingogate-ctrl`）：net/http + chi + sqlx（SQLite/PG 可切）+ slog + prometheus/client_go。
- 跨语言：gRPC + `proto/` codegen。
- 前端（`console/`）：TypeScript/React，build 后嵌入 Go，L6 实现。
<!-- SPECKIT END -->
