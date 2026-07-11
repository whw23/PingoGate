# Quickstart: PingoGate 网关地基

**Feature**: 001-gateway-foundation | 面向首次启动并验证网关的开发者/运维。本文件给出最小可运行路径与验收映射（对应 spec 用户故事与 SC）。

> 注：本阶段尚无后端代码（仍在 plan→tasks→implement 前）。本 quickstart 描述**实现完成后**的预期使用方式，同时作为集成测试的人类可读脚本。

## 前置

- Rust 最新 stable（`rustup`）、`cargo`。
- 三个 Provider 之一的有效上游 Key（用于真实验证；契约测试用桩上游）。

## 1. 构建

```bash
# 在仓库根
cargo build --release --manifest-path app/Cargo.toml
# 产物：单个 pingogate 二进制
```

## 2. 准备配置与密钥

```bash
export OPENAI_API_KEY="sk-..."
export PINGO_KEY_ALPHA="pg-local-dev-key"   # 客户端将用它作为网关 Key
```

最小 `pingogate.yaml`（详见 [contracts/config-schema.md](./contracts/config-schema.md)）：

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

## 3. 启动

```bash
./pingogate --config ./pingogate.yaml
```

## 4. 验证数据面（US1 / SC-001 / SC-002）

用官方 OpenAI SDK 仅改 base URL、token、模型名：

```bash
curl http://localhost:8080/v1/chat/completions \
  -H "Authorization: Bearer pg-local-dev-key" \
  -H "Content-Type: application/json" \
  -d '{"model":"gpt-4o","messages":[{"role":"user","content":"hi"}]}'
```
- 期望：与直连上游一致的成功响应；响应/日志不含 `OPENAI_API_KEY` 明文（SC-008）。
- 流式：加 `"stream":true`，SSE 增量持续返回直至结束（FR-005）。

错误路径快速核对：
- 错误网关 Key → 401，错误体为 OpenAI 形状（FR-013/FR-035）。
- `"model":"unknown"` → 「无路由」镜像错误（FR-016）。
- 请求 Responses/Realtime 等 → 显式「不支持」（FR-007/SC-010）。

## 5. 验证管理面（US3 / SC-006 / SC-007）

```bash
curl http://localhost:9090/healthz  -H "Authorization: Bearer <admin-token>"
curl http://localhost:9090/readyz   -H "Authorization: Bearer <admin-token>"   # 含 active_version
curl -X POST http://localhost:9090/config/validate -H "Authorization: Bearer <admin-token>" \
     --data-binary @pingogate.yaml
```
- 无凭据访问任一端点 → 401/403（SC-006）。

## 6. 验证热重载（US2 / SC-004 / SC-005）

```bash
# 修改 pingogate.yaml（如新增一条 route），然后：
kill -HUP $(pgrep pingogate)         # 或 POST /reload
curl http://localhost:9090/reload/status -H "Authorization: Bearer <admin-token>"
```
- 合法变更：原子生效，在途请求零中断（SC-004）。
- 提交非法配置触发重载：被拒，旧配置 100% 继续服务（SC-005）；`reload/status` 显示旧版本仍生效。

## 7. 验证可观测性（US4 / SC-008）

```bash
curl http://localhost:9090/metrics   # 或独立 metrics 端口
```
- 期望：按 `provider` 与 `capability_family` 打标签的请求量/状态码/延迟/上游延迟/token 计数；无任何 Key/token 明文。

## 验收映射

| 步骤 | 用户故事 | 关键 SC/FR |
| ---- | ---- | ---- |
| 4 | US1 | SC-001, SC-002, FR-002~008, FR-035 |
| 5 | US3 | SC-006, SC-007, FR-023~027 |
| 6 | US2 | SC-004, SC-005, FR-017~022 |
| 7 | US4 | SC-008, FR-031~034 |
