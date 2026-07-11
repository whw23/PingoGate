# M0 内核单机网关 设计（Design）

> **Feature**：M0 内核单机网关（Kernel Standalone Gateway）
> **日期**：2026-07-12
> **状态**：Draft（待 review）
> **路线图定位**：`docs/01-platform-roadmap.md` 里程碑 M0，能力层 L3（单机模式）
> **上位约束**：`.claude/rules/constitution.md`（双语言内核架构总纲 + I~XXII）

## 1. 目标与范围

### 1.1 目标

交付 PingoGate Rust 内核（`pingogate-core` 二进制）的**单机模式**：一个独立可用的轻量 LLM 网关。客户端只改 base URL / token / 模型名，经网关 Key 鉴权后，请求透传至上游 Provider（OpenAI / Anthropic / Gemini），含 SSE 流式。

M0 是内核的**最小可用形态**；平台模式（gRPC 推送 + 虚拟 key + KeyVault）是后续 M1-M2 的增强。M0 代码结构必须为平台模式预留位置（trait / 枚举变体），但不实现平台侧。

### 1.2 包含（In Scope）

- `core-rs/` Cargo workspace 搭建，按宪法 IV 重组 crate（`core` / `storage` / `pipeline` / `router` / `transform` / `provider` / `listener` / `snapshot` / `pingogate-core`）。
- Pingora 数据面管线：协议识别 -> 网关 Key 鉴权 -> 模型路由 -> 上游认证注入 -> 透传（含 SSE 零拷贝）-> 错误镜像 -> 可观测。
- 三协议原生透传：OpenAI Chat Completions / Anthropic Messages / Gemini generateContent，含 SSE。
- 单机模式配置源：`pingogate-core.yaml` 文件（监听器 / Provider / 路由 / 静态网关 Key）。
- 静态网关 Key 鉴权：一组具名 Key（`env:` 引用），客户端持网关 Key，上游 Provider Key 由网关注入、永不下发客户端。
- Provider Key 经密钥引用（`env:VAR`）解析，**不加密**（单机单用户无隔离需求），KeyVault 不启用。
- `RuntimeSnapshot` + `ArcSwap` 不可变快照热重载（SIGHUP / 文件监视 / Admin API 三触发同一编排器）。
- `SnapshotSource` trait：file 实现（单机模式），gRPC 实现预留位置不实现（平台模式）。
- 最小管理端点：`/healthz` / `/readyz` / `/metrics` / `/reload` / `/reload/status` / `/config/validate`，经 `Principal` / `authorize` 边界鉴权（bootstrap 静态 admin 凭据，但走边界）。
- 可观测性：tracing 结构化日志 + trace id；Prometheus 指标（按 provider + 能力族打标签）；密钥脱敏。

### 1.3 不包含（Out of Scope，后置）

- **KeyVault 加密**（L1，平台模式）：M0 provider key 用 `env:` 明文引用，不加密。
- **虚拟 Key**（L2，平台模式）：M0 用静态网关 Key，不支持 scoped / 撤销 / Go 推送。
- **Go 非内核**（L0-L2 控制面）：M0 无 Go、无 DB、无 OAuth / RBAC / 多用户。
- **用量计量**（L5）：M0 不计 token、不落用量表（用量计量引擎在 L5 引入，M0 热路径不解析 SSE 取 usage）。
- **转换引擎实际规则**（L3+）：M0 只透传不改 body；`transform` crate 留 trait 与空实现，不为 M0 做任何请求 / 响应改写（含 OpenAI `include_usage` 注入）。
- **多协议桥 / 上下文虚拟化**（蓝图 10/11）：远期。
- **嵌入式控制台**（L6）：M0 仅机器可读 Admin API。

### 1.4 数据面复用 001

M0 数据面**移植 001-gateway-foundation 已验证实现**（redesign 分支 `app/`），而非重写。001 数据面范式正确（仅控制面范式错误），代码宪法合规。移植做三处适配（见 §6）。

## 2. 架构

### 2.1 整体形态（单机模式）

```text
客户端（OpenAI/Anthropic/Gemini SDK）
  │ 仅改 base URL / token / 模型名
  ▼
pingogate-core 二进制（Rust 内核，单机模式）
  ├─ public listener（Pingora HttpProxy）
  │   └─ 数据面管线（pipeline crate）
  │       协议识别 -> 网关Key鉴权 -> 模型路由 -> 上游认证注入
  │                -> 透传/SSE -> 错误镜像 -> 可观测
  │       │
  │       └─ 读 RuntimeSnapshot（ArcSwap，lock-free）
  │              ├─ providers（含解析后的 provider key，SecretString）
  │              ├─ routes（模型别名 -> provider/upstream_model）
  │              └─ gateway_keys（静态，name -> secret）
  │
  ├─ admin listener（Pingora ServeHttp）
  │   └─ healthz / readyz / metrics / reload / reload/status / config/validate
  │       （经 Principal / authorize 边界）
  │
  └─ SnapshotSource::File
      └─ 读 pingogate-core.yaml -> 校验 -> 解析密钥 -> 构建快照
          触发：启动 / SIGHUP / 文件监视 / Admin reload
```

### 2.2 双模式骨架

M0 实现单机模式，但代码结构为平台模式预留：

- `SnapshotSource` trait：`build_snapshot() -> Arc<RuntimeSnapshot>`。M0 实现 `FileSnapshotSource`（从 YAML）；平台模式将实现 `GrpcSnapshotSource`（从 Go gRPC 推送，后续 feature）。
- `KeyAuth` 抽象：网关 Key 校验。M0 实现 `StaticKeyAuth`（配置静态 key）；平台模式将实现 `VirtualKeyAuth`（虚拟 key，后续 feature）。
- `Principal` / `authorize`：M0 的 Principal 只有 `GatewayKey` / `Admin` / `System` 三种（沿用 001）；平台模式将扩展 `User` kind 与 scope（L0/L4）。

M0 不建平台模式的空壳实现，只留 trait 位置（宪法 II YAGNI）。

## 3. 组件与 crate 划分

按宪法 IV，`core-rs/` 拆 crate，依赖自底向上无环：

| crate | 职责 | M0 是否建 | 001 对应 |
|---|---|---|---|
| `core` | 共享域类型：`AppError`、`Principal`/`AuthContext`/`authorize`、`SecretString`、`ProtocolKind`/`ProviderKind`/`CapabilityFamily`/`AuthMethod`、`TraceId` | ✅ | `core/`（移植） |
| `storage` | `SecretResolver` trait + `EnvSecretResolver`；`SnapshotSource` trait + `FileSnapshotSource` | ✅ | `storage/secret.rs`（移植）+ 新增 SnapshotSource |
| `snapshot` | `RuntimeSnapshot` + `SnapshotHolder`（ArcSwap）；语义校验；密钥预解析 | ✅ | `config/snapshot.rs` + `config/validate.rs`（移植重组） |
| `provider` | `ProviderAdapter` trait + 三 adapter（OpenAI/Anthropic/Gemini）；能力族声明；错误体形状 | ✅ | `provider/`（移植） |
| `pipeline` | `GatewayProxy`（ProxyHttp 实现）；协议识别；上游认证注入；SSE 流式检测；模型路由调用；可观测 | ✅ | `pipeline/`（移植） |
| `router` | 模型别名提取（body / gemini path）-> 路由解析 | ✅ | `pipeline/router.rs`（拆出独立 crate） |
| `transform` | `Transform` trait（请求/响应转换）；M0 空实现（透传不改 body） | ✅（仅 trait + 空实现） | 001 无（新增，留位置） |
| `listener` | Pingora public / admin service 装配工厂 | ✅ | `listener/`（移植） |
| `pingogate-core` | 主二进制：bootstrap（tracing -> config -> snapshot -> services -> run）；信号 / 文件监视 / reload 编排 | ✅ | `pingogate/`（移植，改名） |

**依赖方向**：`core` ← `storage` ← `snapshot` ← {`provider`/`router`/`transform`} ← `pipeline` ← `listener` ← `pingogate-core`。

**与 001 的 crate 差异**：
- 001 的 `config` crate 拆为 `snapshot`（RuntimeSnapshot + 校验）+ 配置 schema 暂留 `snapshot`（M0 单机模式 YAML schema）。
- 001 的 `admin` crate（Admin API handler）并入 `listener`（admin service）或 `pingogate-core`（reload 编排），M0 不单列 `admin` crate（简化）。
- 新增 `router` 独立 crate（001 在 pipeline 内），为平台模式路由策略扩展。
- 新增 `transform` crate（仅 trait + 空实现），为 L3+ 转换引擎留位置。

## 4. 数据流

### 4.1 请求处理（数据面热路径）

```text
1. Pingora 接收请求，new_ctx() 加载当前 RuntimeSnapshot（ArcSwap::load）
2. request_filter():
   a. 协议识别（protocol::detect）：按 method+path 识别 OpenAI/Anthropic/Gemini
      - 识别失败 -> PingoGate 原生错误（FR-036）
      - 不支持能力族 -> 镜像该 Provider 错误（FR-007）
   b. 网关 Key 鉴权（StaticKeyAuth）：从 Authorization/x-api-key 提取，查 snapshot.gateway_keys
      - 无效/缺失 -> 镜像目标 Provider 鉴权错误，不触达上游（FR-013）
   c. 模型路由（router::extract_model + snapshot.route）：从 body(model) 或 path(gemini) 提取别名 -> 解析 provider+upstream_model
      - 无匹配路由 -> 镜像目标 Provider 错误（FR-016）
   d. 记 CTX：protocol / route / streaming 标记
3. upstream_peer(): 构造 HttpPeer（含上游 TLS，默认校验）
4. upstream_request_filter():
   a. 剥离网关 Key 头（authorization / x-api-key）
   b. 注入上游认证（upstream_auth::UpstreamAuth::build）：
      - OpenAI: Authorization: Bearer <key>
      - Anthropic: x-api-key: <key> + anthropic-version
      - Gemini: URL query ?key=<key>
   c. 不改 body（M0 transform 空实现，透传）
5. Pingora 透传：响应头透传；body 逐 Bytes 块零拷贝转发（SSE 不缓冲整流）
6. 错误路径：上游超时/不可用 -> 镜像目标 Provider 错误（类 504）；上游自身错误原样透传
7. 可观测：logging 记 trace id/protocol/route/status/latency；metrics 计数
```

### 4.2 热重载

```text
触发：SIGHUP | 文件监视 | Admin POST /reload
  ↓
Reloader（串行化，mutex）：
  1. SnapshotSource::File 读 pingogate-core.yaml
  2. serde schema 校验
  3. 语义校验（路由引用的 provider 存在、密钥引用可解析）
  4. 解析密钥引用（env:VAR），缺失则失败
  5. 构建不可变 RuntimeSnapshot（version+1）
  6. SnapshotHolder::store（ArcSwap 原子切换）
  7. 记录 reload 版本 / 状态 / 回滚目标
  ↓
失败：不改活跃 snapshot，返回校验错误（FR-020）
进行中请求：继续用启动时绑定的旧 snapshot（FR-021，零中断）
```

## 5. 配置 schema（pingogate-core.yaml）

M0 单机模式配置（移植 001 schema，适配命名）：

```yaml
listeners:
  public: { address: "0.0.0.0:8080" }
  admin:  { address: "127.0.0.1:9090" }
gateway_keys:
  - { name: "team-alpha", secret_ref: "env:PINGO_KEY_ALPHA", enabled: true }
providers:
  - name: "openai-main"
    kind: "openai-compatible"
    base_url: "https://api.openai.com"
    auth: { method: "bearer", key_ref: "env:OPENAI_API_KEY" }
    capability_families: ["generation.stateless"]
  - name: "anthropic-main"
    kind: "anthropic"
    base_url: "https://api.anthropic.com"
    anthropic_version: "2023-06-01"
    auth: { method: "api_key_header", key_ref: "env:ANTHROPIC_API_KEY" }
    capability_families: ["generation.stateless"]
  - name: "gemini-main"
    kind: "gemini"
    base_url: "https://generativelanguage.googleapis.com"
    auth: { method: "query_key", key_ref: "env:GEMINI_API_KEY" }
    capability_families: ["generation.stateless"]
routes:
  - { alias: "gpt-4o", provider: "openai-main", upstream_model: "gpt-4o" }
  - { alias: "claude", provider: "anthropic-main", upstream_model: "claude-3-5-sonnet-latest" }
  - { alias: "gemini-pro", provider: "gemini-main", upstream_model: "gemini-1.5-pro" }
upstream: { timeout_ms: 60000 }
```

- `deny_unknown_fields`：未知字段拒绝（typo 失败而非静默）。
- 密钥一律 `env:VAR` 引用，永不内联；启动时缺失即失败（不在请求期暴露）。
- M0 不含 `virtual_keys` / `users` / `billing` 等平台字段（平台模式 gRPC 推送，不经此文件）。

## 6. 从 001 移植的适配

### 6.1 依赖结构重构

- 001 crate 名 `pingo_core`/`pingo_config`/... -> M0 crate 名 `pingogate_core`/`pingogate_snapshot`/...（统一前缀）。
- 001 的 `config` crate 拆为 `snapshot`（RuntimeSnapshot + 校验）。
- 001 的 `admin` crate 并入 `listener` + `pingogate-core`。
- 新增 `router`（从 pipeline 拆出）、`transform`（新增空 trait）。

### 6.2 SnapshotSource 双模式

001 的 snapshot 只从文件构建。M0 抽象 `SnapshotSource` trait：

```rust
pub trait SnapshotSource: Send + Sync {
    fn build_snapshot(&self, version: u64) -> Result<Arc<RuntimeSnapshot>, SnapshotError>;
}
```

- `FileSnapshotSource`：读 YAML + 校验 + 解析密钥（M0 实现）。
- `GrpcSnapshotSource`：从 Go gRPC 推送构建（平台模式，M0 仅留 trait 位置，不实现）。

`SnapshotHolder` 持有 `Arc<dyn SnapshotSource>`，reload 时调 `build_snapshot`。

### 6.3 Key 鉴权抽象

001 的 `gateway_keys` 校验内联在 pipeline。M0 抽象 `KeyAuth` trait：

```rust
pub trait KeyAuth: Send + Sync {
    fn authenticate(&self, credential: &str) -> Option<Principal>;
}
```

- `StaticKeyAuth`：查 snapshot.gateway_keys（M0 实现）。
- `VirtualKeyAuth`：虚拟 key 校验（平台模式，M0 留位置）。

## 7. 错误处理（宪法 IX）

- `AppError`（Domain/Application/Infrastructure/Validation），HTTP 码仅在边界映射。
- **网关自身错误**（鉴权失败/无路由/能力不支持/上游超时）镜像目标 Provider 原生错误体（OpenAI `{"error":{...}}` / Anthropic `{"type":"error",...}` / Gemini `{"error":{...}}`）。
- 协议识别前失败 -> PingoGate 原生错误体。
- 上游自身错误原样透传不改写。
- 密钥明文 MUST NOT 进错误信息 / 日志 / 指标。

## 8. 安全（宪法 XX）

- 100% Safe Rust。
- 网关 Key / Provider Key 经 `env:` 引用解析，明文不进日志 / 指标 / 任何对外输出。
- 上游 Provider Key 不由 adapter 直接接触（pipeline `upstream_request_filter` 单点注入）。
- Admin / 所有特权操作经 `Principal` / `authorize(action, resource)` 边界；无全局 admin token 等值判断。
- 上游 TLS 默认校验。
- M0 不持久化 prompt/response 内容（仅元数据 + 指标）。
- **M0 不启用 KeyVault**（单机单用户，provider key 用 `env:` 明文引用，无多用户隔离需求）。KeyVault 在 L1 平台模式引入。

## 9. 可观测性（宪法 XIX）

- tracing 结构化日志，每请求 trace id 贯穿管线。
- Prometheus 指标：按 provider + 能力族（`generation.stateless`）打标签；请求量 / 状态码 / 延迟 / 上游延迟。M0 不计 token（L5 才引入）。
- 密钥 / token 经统一脱敏层后才输出。

## 10. 性能（宪法 XXI）

- 延迟预算：无状态透传网关在上游延迟之外额外开销 < 5 ms p50 / < 20 ms p95（Pingora 级，沿用 001 基准 p50≈1.5ms/p95≈1.7ms）。
- 零拷贝流式（SSE 不缓冲整流）；异步路径无阻塞 I/O；上游过载用显式背压。
- 热路径零 DB（M0 无 DB）；只读内存 RuntimeSnapshot。

## 11. 测试（宪法 XIII TDD）

契约 / 集成 / 单元测试先行。

- **单元**：协议识别（三协议 + 不支持能力 + 未识别）；上游认证注入（三方式）；模型别名提取；snapshot 构建与校验；KeyAuth；authorize 边界；密钥脱敏。
- **集成**：管线阶段往返；三协议透传（非流式 + SSE）；热重载（成功切换 / 失败不改活跃 / 在途请求零中断）；密钥引用缺失启动失败。
- **契约**：Admin API（六端点 + 鉴权边界）；`pingogate-core.yaml` schema；三协议透传 + 镜像错误。
- **基准**：无状态透传延迟基准（默认 `#[ignore]`，显式运行），回归阻止合入。

## 12. 成功标准（Success Criteria）

- **SC-1**：客户端仅改 base URL / token / 模型名，三协议（含 SSE）经网关调通上游，响应与直连一致。
- **SC-2**：网关 Key 无效 / 缺失在鉴权阶段被拒，不触达上游。
- **SC-3**：上游 Provider Key 不下发客户端、不进日志 / 指标。
- **SC-4**：热重载成功切换；非法配置被拒且活跃 snapshot 100% 继续服务；在途请求零中断。
- **SC-5**：100% Admin API 请求经 authorize 边界；缺失 / 无效凭据被拒。
- **SC-6**：无状态透传 p50 < 5 ms / p95 < 20 ms。
- **SC-7**：不支持的能力族返回显式「不支持」错误，不畸形透传。
- **SC-8**：单二进制启动服务全部能力，无外部 DB / 协调服务。
- **SC-9**：`SnapshotSource` / `KeyAuth` trait 存在，`FileSnapshotSource` / `StaticKeyAuth` 实现，平台模式实现位置预留（编译通过，不实现）。

## 13. 风险与未决

- **R1 未决**：单机模式 admin 鉴权用 `env:PINGO_ADMIN_TOKEN` 单 token，还是一组具名 admin key？倾向单 token（单机场景够用），plan 阶段定。
- **R2 未决**：`router` 是否 M0 独立 crate，还是暂留 pipeline 内？倾向独立（为平台模式路由策略扩展），但增加一个 crate 的开销。plan 阶段定。
- **R3 风险**：001 移植时 Pingora 版本可能需升级（001 用 pingora-proxy 0.8.0），升级 API 变更风险。实现期用 context7 核对当前版本（宪法 XVII）。
- **R4 风险**：crate 重组后 001 的测试需重新组织，可能暴露隐藏耦合。移植时逐 crate 验证。
