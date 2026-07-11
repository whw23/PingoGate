# Contract: `pingogate.yaml` 配置

**Feature**: 001-gateway-foundation | 单一声明式配置来源（FR-017）。人写 YAML，机器索引/锁文件用 JSON（宪法 III）。键名英文（宪法 XXII）。

## 顶层结构

```yaml
# pingogate.yaml
listeners:
  public:
    address: "0.0.0.0:8080"
    tls:                      # 可选
      cert: "/path/cert.pem"
      key_ref: "env:TLS_KEY"
  admin:
    address: "127.0.0.1:9090"

gateway_keys:                 # 一组具名网关 Key（FR-009）
  - name: "team-alpha"
    secret_ref: "env:PINGO_KEY_ALPHA"
    enabled: true

providers:                    # 上游声明（FR-011/FR-012）
  - name: "openai-main"
    kind: "openai-compatible"
    base_url: "https://api.openai.com"
    auth:
      method: "bearer"
      key_ref: "env:OPENAI_API_KEY"
    capability_families: ["generation.stateless"]

  - name: "anthropic-main"
    kind: "anthropic"
    base_url: "https://api.anthropic.com"
    anthropic_version: "2023-06-01"
    auth:
      method: "api_key_header"   # x-api-key + anthropic-version
      key_ref: "env:ANTHROPIC_API_KEY"
    capability_families: ["generation.stateless"]

  - name: "gemini-main"
    kind: "gemini"
    base_url: "https://generativelanguage.googleapis.com"
    auth:
      method: "query_key"        # ?key=
      key_ref: "env:GEMINI_API_KEY"
    capability_families: ["generation.stateless"]

routes:                       # 模型别名 → Provider/上游模型（FR-014）
  - alias: "gpt-4o"
    provider: "openai-main"
    upstream_model: "gpt-4o"
  - alias: "claude"
    provider: "anthropic-main"
    upstream_model: "claude-3-5-sonnet-latest"
  - alias: "gemini-pro"
    provider: "gemini-main"
    upstream_model: "gemini-1.5-pro"

observability:                # 可选
  metrics:
    label_by_principal: false   # 默认仅 provider + capability family（FR-033）
upstream:
  timeout_ms: 60000             # 可配置上游超时，含合理默认（FR-038）
```

## 校验规则（schema + 语义）

| 规则 | 类型 | 失败行为 |
| ---- | ---- | ---- |
| 未知字段 / 类型不符 | schema | 重载/校验拒绝，活跃配置不变（FR-020） |
| `routes[].provider` 必须存在于 `providers[].name` | 语义 | 拒绝并报具体 path（FR-016） |
| 所有 `*_ref` 密钥引用可解析（`env:VAR` 存在） | 语义 | 拒绝（Edge Case：缺失密钥不可启动后崩溃，FR-022） |
| `kind=anthropic` 必须有 `anthropic_version` | 语义 | 拒绝 |
| `gateway_keys[].name` / `providers[].name` / `routes[].alias` 各自唯一 | 语义 | 拒绝 |
| `auth.method` 与 `kind` 兼容 | 语义 | 拒绝 |

## 密钥引用约定

- 本阶段仅 `env:VAR_NAME`。解析在 `RuntimeSnapshot` 构建时完成，解析值受保护驻内存。
- 明文密钥 MUST NOT 出现在配置回显、日志、指标、Admin API 响应中（FR-022/FR-034、宪法 XX）。

## 契约测试（先写并失败）

1. 最小合法配置 → 构建出有效 `RuntimeSnapshot`。
2. 路由引用不存在 provider → 语义校验失败，错误指向 `routes[i].provider`。
3. 缺失 `env:` 密钥 → 校验失败（非启动后崩溃）。
4. `anthropic` 缺 `anthropic_version` → 校验失败。
5. 重复 alias/name → 校验失败。
6. 任意输出路径（validate 回显、日志）断言不含密钥明文。
