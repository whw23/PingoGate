# PingoGate 项目宪法（Project Constitution）

> **来源与保真说明**：本宪法从 `001-gateway-foundation` feature 的 `plan.md` / `research.md` / `spec.md` 对宪法的引用中提取重建，对应宪法 **v1.6.0**。内容为原则转述，**非逐字原文**——逐字原文未进入仓库，仅以引用形式散落在 001 specs 中。如发现原则与实际宪法源文件有出入，以宪法源文件为准并回写本文件。
>
> 本文件作为项目级 rules 放置在 `.claude/rules/`，对 PingoGate 的所有 AI 协作与代码实现具有**上位约束力**。新 feature 的 spec / plan / 实现 MUST「以宪法为准」。

---

## I. 复杂度层级判定
每个 feature 在 plan 阶段判定复杂度层级（简单 / 中等 / 复杂），并按层级强制对应下述原则的闸门要求。判定依据：新增模块数量、跨模块耦合、条件业务逻辑、是否涉及协议桥 / 上下文虚拟化 / 多租户控制面等。中等及以上强制全部原则；简单可降级部分原则的闸门要求。Complexity Tracking 记录所有违规与豁免理由。

## II. DRY / KISS / YAGNI
- 强制。范围严格限定在 spec 的 FR；不投机抽象；延后能力不预先建框架。
- 不为实现未排期能力预留框架，只预留「位置」（trait 边界、目录层、配置字段）。

## III. 技术栈锁定
- 核心后端 100% Rust（Safe Rust，`unsafe` 禁止除非显式评审）。
- 热路径 MUST 使用 Pingora（`pingora` / `pingora-proxy` / `pingora-core`）。自建 hyper/axum 代理用于热路径被否。
- 异步运行时 tokio；序列化 serde（+ serde_json / serde_yaml）；日志 tracing + tracing-subscriber；指标 Prometheus / metrics + OpenTelemetry。
- 依赖最小化：新增非锁定 crate 须在实现 PR 附理由。Admin / 非热路径优先复用 Pingora 自带能力（如 `ServeHttp`），不引入额外 HTTP 框架。
- 前端仅 TypeScript（嵌入式控制台），所有权限 / 校验 / 策略 / 敏感操作留在 Rust。

## IV. 目录结构
- 顶层目录：`app/` `console/` `deploy/` `scripts/`（宪法 v1.6.0 确定，以宪法为准，覆盖蓝图早期 `product/` 草图）。
- `app/` 为 Cargo workspace，按架构层级拆 crate，每个 crate 对应一个层级且不跨层。
- 依赖方向自底向上无环；跨层调用只经各 crate 公共 trait / 类型。
- 根目录作为 AI 协作壳；新功能不属于现有层级时应先扩展 / 拆分定义层级。

## V. 代码指标
- 文件 ≤ 300 行；函数 ≤ 50 行；参数 ≤ 4；圈复杂度 ≤ 10；嵌套 ≤ 3。
- 在实现与 CI 强制。

## VI. 请求管线化
- 请求路径拆成稳定管线，各阶段映射到 Pingora `ProxyHttp` 的 filter 回调；Provider adapter 不拥有网关生命周期。
- 管线：入口 -> 协议识别 -> 鉴权授权 -> 上下文解析 -> 路由 -> 策略 -> 请求转换 -> 上游派发 -> 响应转换 -> 上下文提交 -> 可观测性 -> 用量计费。
- adapter 负责 Provider 命名空间、能力声明、Provider 特有转换，不负责把整个系统绑死。
- 鉴权在 `request_filter` 早期短路；上游认证在 `upstream_request_filter` 单点注入，adapter 不接触明文 Key 与 body / 连接。

## VII. SOLID
- adapter / authenticator / router 等聚焦 trait；构造期注入；组合优先于继承。

## VIII. 低耦合高内聚
- 引入 `core` 共享类型 / 错误 crate 打破循环依赖；crate 间仅经公共 trait / 类型通信。

## IX. 错误处理
- 完整分类：`AppError`（Domain / Application / Infrastructure / Validation）。
- HTTP 状态码仅在边界映射；不在内部层泄漏 HTTP 语义。
- 网关自身错误在管线边界镜像目标 Provider 的原生错误体；协议识别前失败回退 PingoGate 原生错误体；上游自身错误原样透传不改写。

## X. 状态分离
三类状态严格分离：
- **Capability Repository**（系统能做什么）：provider 能力声明、模型元数据、转换 / 策略模板、未来签名能力包。**不保存**用户密钥、租户策略、用量、计费、会话状态。
- **Runtime Config**（实例如何运行）：listener、domain、certificate、enabled providers、upstream channels、model routing、policy、metrics、reload。来源 YAML / TOML / JSON / env / K8s ConfigMap / DB 发布版本。
- **Control Plane State**（谁在使用、用了多少）：users、tenants、projects、API keys、provider keys、quota、budgets、usage、billing、audit、conversation state。来源 SQLite / Postgres / MySQL / Redis / 对象存储。

重载不触控制面状态；不可变 `RuntimeSnapshot` + `ArcSwap` 原子切换。

## XI. Provider 抽象（能力族）
- adapter 声明支持的能力族（而非只注册 endpoint handler）；暴露 namespace；不支持的能力族显式拒绝（返回明确「不支持」错误，不静默 / 畸形透传）。
- adapter 拥有 Provider 特定转换（错误体形状、认证方式描述符）。
- 保留 Provider 命名空间边界，不同 Provider 能力与转换不相互越界。

## XII. 热重载
- `ArcSwap` snapshot；候选配置 校验 -> 构建 -> 原子切换 -> 记录版本 / 回滚目标；失败不改活跃 snapshot。
- 进行中请求继续使用启动时绑定的旧 snapshot，零中断。
- 触发源：SIGHUP / Admin API / 文件监视 / 仓库 revision / UI publish。
- 不原地修改 live route table；不加载 native `.so` / `.dylib` / `.dll` 插件作为主路径。

## XIII. TDD
- 契约 / 集成 / 单元测试先写并观察失败；覆盖管线、透传往返、重载、鉴权边界。

## XIV. 文档与注释
- 公共 API 英文文档注释；项目文档中文。

## XV. 原子提交
- Conventional Commits；按工作单元拆分。

## XVI. CodeGraph
- 存在 `.codegraph/`；编辑前用 `codegraph_explore` 评估影响。

## XVII. AI 技能
- 涉及库 / API 用 context7 查文档；auth / key / TLS 变更合并前走 security-review；功能完成走 code-review / simplify / verify。

## XVIII. Subagent 监控
- 如用 subagent 则监控其存活。

## XIX. 可观测性
- `tracing` 结构化日志，带贯穿管线的 trace / 请求 ID。
- Prometheus 指标，按 provider + 能力族（必需）打标签，MAY 按 principal id（非明文密钥）归因。
- 必需观测信号：请求量、状态码、延迟、上游延迟、retries、fallback、token 计数（input / output / reasoning / cache）。
- 密钥 / token 经统一脱敏层后才输出。

## XX. 安全 / 保密 / 隐私
- 100% Safe Rust。
- 密钥经引用解析，明文 MUST NOT 进日志 / 指标 / 任何对外输出；上游 Provider Key 不由 adapter 直接接触。
- Admin / 所有特权操作经 `Principal` / `AuthContext` / `authorize(action, resource)` 边界；不存在「全局 admin token 等值判断」硬编码鉴权；为 OAuth / OIDC / SSO 预留身份映射边界。
- 上游 TLS 默认校验。
- 默认不持久化完整 prompt / response 内容，仅记录元数据与指标；内容持久化需租户 / 项目策略显式开启；协议桥接需历史内容时保存加密短期可过期状态。

## XXI. 性能
- 延迟预算：无状态透传网关在上游延迟之外的额外开销 < 5 ms p50 / < 20 ms p95。
- 零拷贝流式（SSE 不缓冲整流）；异步路径无阻塞 I/O；上游过载用显式背压而非无界队列。
- 热路径变更加性能测试；每功能定义资源预算；`RwLock<Config>` 等写锁阻塞热路径读的方案被否。

## XXII. 项目语言
- 文档中文；代码 / API / 提交信息英文。
