# PingoGate
🚀 An ultra-fast, Rust-powered LLM Gateway &amp; Proxy built on Cloudflare's Pingora. A drop-in high-performance replacement for LiteLLM and New API.

PingoGate 是基于 [Pingora](https://github.com/cloudflare/pingora) 的高性能 LLM 网关：单二进制、Safe Rust、声明式配置热重载。当前阶段（001-gateway-foundation）交付**三协议透传地基** —— OpenAI / Anthropic / Gemini 请求经网关 Key 鉴权后透传至对应上游，并具备配置热重载、鉴权管理面与可观测性。

## 特性

- **三协议透传**：OpenAI（`/v1/chat/completions`）、Anthropic（`/v1/messages`）、Gemini（`:generateContent`），含 SSE 流式；请求体逐字节转发，仅替换路由与凭据。
- **网关 Key 鉴权**：客户端持网关 Key，网关剥离后注入真实上游凭据，上游 Key 永不下发客户端。
- **声明式配置 + 热重载**：不可变 `RuntimeSnapshot` + `ArcSwap` 原子切换；SIGHUP / 文件监视 / `POST /reload` 三触发同一编排器，非法配置被拒且旧配置 100% 继续服务。
- **鉴权管理面**：健康/就绪/配置校验/重载/重载状态/指标端点，统一过 `Principal`/`authorize` 边界。
- **可观测性**：Prometheus `/metrics`（按 `provider`+`capability_family` 打标签的请求量/状态码/延迟/上游延迟/token 计数）、结构化 trace 日志，日志与指标 0 密钥泄漏。
- **上游 TLS 默认开启**并校验证书。

性能：本地基准 p50 ≈ 1.5 ms、p95 ≈ 1.7 ms（目标 <5 ms / <20 ms），详见 [基准结果](specs/001-gateway-foundation/benchmark-results.md)。

## 构建

需要最新 stable Rust（`rustup`）与 `cargo`（构建依赖 `cmake`）。

```bash
# 在仓库根，产物为单个 pingogate 二进制
cargo build --release --manifest-path app/Cargo.toml
```

## 配置与密钥

密钥通过环境引用（`env:VAR`）解析，启动时若缺失即启动失败（不会在请求期暴露）。最小 `pingogate.yaml`：

```yaml
listeners:
  public: { address: "0.0.0.0:8080" }
  admin:  { address: "127.0.0.1:9090" }
gateway_keys:
  - { name: "team-alpha", secret_ref: "env:PINGO_KEY_ALPHA" }
providers:
  - name: "openai-main"
    kind: "openai-compatible"
    base_url: "https://api.openai.com"
    auth: { method: "bearer", key_ref: "env:OPENAI_API_KEY" }
    capability_families: ["generation.stateless"]
routes:
  - { alias: "gpt-4o", provider: "openai-main", upstream_model: "gpt-4o" }
upstream: { timeout_ms: 60000 }
```

```bash
export OPENAI_API_KEY="sk-..."
export PINGO_KEY_ALPHA="pg-local-dev-key"   # 客户端使用的网关 Key
export PINGO_ADMIN_TOKEN="admin-secret"     # 管理面 Bootstrap 令牌
```

完整字段见 [config-schema](specs/001-gateway-foundation/contracts/config-schema.md)。

## 运行

```bash
./pingogate --config ./pingogate.yaml   # 默认读取工作目录 pingogate.yaml
```

数据面请求（仅改 base URL / token / 模型名即可复用官方 SDK）：

```bash
curl http://localhost:8080/v1/chat/completions \
  -H "Authorization: Bearer pg-local-dev-key" \
  -H "Content-Type: application/json" \
  -d '{"model":"gpt-4o","messages":[{"role":"user","content":"hi"}]}'
```

## 管理面

所有 Admin 端点需 `Authorization: Bearer $PINGO_ADMIN_TOKEN`（无凭据 → 401）：

```bash
curl http://localhost:9090/healthz       -H "Authorization: Bearer admin-secret"
curl http://localhost:9090/readyz        -H "Authorization: Bearer admin-secret"   # 含 active_version
curl http://localhost:9090/metrics       -H "Authorization: Bearer admin-secret"
curl -X POST http://localhost:9090/config/validate -H "Authorization: Bearer admin-secret" --data-binary @pingogate.yaml
curl -X POST http://localhost:9090/reload          -H "Authorization: Bearer admin-secret"
curl http://localhost:9090/reload/status -H "Authorization: Bearer admin-secret"
```

热重载三触发：`kill -HUP $(pgrep pingogate)`、`POST /reload`，或设 `PINGO_WATCH_INTERVAL_SECS` 启用文件监视。

## 测试

```bash
cargo test --workspace --manifest-path app/Cargo.toml          # 单元 + 契约 + 集成 + e2e
cargo clippy --all-targets --manifest-path app/Cargo.toml -- -D warnings
cargo fmt --check --manifest-path app/Cargo.toml
# 延迟基准（默认 #[ignore]，需显式运行）
cargo test --release --manifest-path app/Cargo.toml --test benchmark -- --ignored --nocapture
```

人类可读验收脚本见 [quickstart](specs/001-gateway-foundation/quickstart.md)。

## 蓝图

- [中文开发蓝图](blueprint/index.html): 面向展示和讨论的 HTML 蓝图，按当前开发路线组织 PingoGate 的阶段交付、架构边界、能力范围和长期目标。完整 Markdown 原文归档于 [`blueprint/sections/source-template.html`](blueprint/sections/source-template.html)。
