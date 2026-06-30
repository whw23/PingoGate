# 安全复核：PingoGate 网关地基

**Feature**: 001-gateway-foundation | **任务**: T065 | **日期**: 2026-06-30

本复核覆盖 T065 的三条核心安全属性，结论：**通过**（含 1 项非阻塞硬化建议）。

## 1. 日志/指标/配置响应中无明文密钥

| 检查点 | 证据 | 结论 |
| ---- | ---- | ---- |
| `SecretString` 默认不泄露 | [`core/src/secret.rs`](../../app/core/src/secret.rs)：`Debug`→`SecretString([REDACTED])`，`Display`→`[REDACTED]`；唯一明文出口 `expose()`，有单测守护 | ✅ |
| `expose()` 调用点审计 | 全仓仅 3 处非测试调用：上游注入 [`proxy.rs:106`](../../app/pipeline/src/proxy.rs#L106)、快照构建 [`snapshot.rs:109`](../../app/config/src/snapshot.rs#L109)、Admin 令牌比对 [`bootstrap.rs:31`](../../app/admin/src/bootstrap.rs#L31)。均为合法用途，无一进入日志/指标 | ✅ |
| 日志宏无密钥 | 完成日志 [`logging.rs`](../../app/pipeline/src/logging.rs) 字段集 `FIELDS` 仅 trace_id/principal/protocol/provider/capability_family/status/streaming/error；单测断言字段集排除 key/secret/authorization；全仓日志宏 grep 无 `expose/secret/.key` 引用 | ✅ |
| 指标无密钥 | [`metrics.rs`](../../app/pipeline/src/metrics.rs) 标签仅 `provider`/`capability_family`/`status`/`direction`/`le`，值仅计数；集成测试断言 `/metrics` 不含网关 Key/上游 Key/Admin 令牌 | ✅ |
| 配置响应无密钥 | `/config/validate`、`/reload` 永不解析密钥（仅解析+语义校验）；错误体为 `{path,message}`，message 含密钥**引用串**（如 `env:OPENAI_KEY`）而非明文 | ✅ |
| 专项脱敏层 | [`core/src/redact.rs`](../../app/core/src/redact.rs)：敏感头/查询参数/Bearer 值统一回 `***` | ✅ |

## 2. `authorize` 边界强制

| 检查点 | 证据 | 结论 |
| ---- | ---- | ---- |
| 每个 Admin 端点过授权 | [`handler.rs`](../../app/admin/src/handler.rs) 全部端点经 `guard()` → `AuthContext::authorize(action, resource)` 后才执行 | ✅ |
| 认证与授权分离 | `BootstrapAuth` 仅做认证（令牌→Principal）；令牌相等永不单独成为特权门（XX） | ✅ |
| 数据面网关 Key 不可越权管理面 | 契约测试 `gateway_key_principal_is_forbidden_on_every_endpoint` 对全部端点返回 403 | ✅ |
| 未认证不泄露端点存在 | 无凭据 → 401（`AdminApp` 先认证后路由）；e2e 断言匿名 `/healthz`/`/metrics` 401 | ✅ |

## 3. 上游 TLS 默认开启且校验证书

| 检查点 | 证据 | 结论 |
| ---- | ---- | ---- |
| TLS 默认开 | [`upstream_peer.rs:31-32`](../../app/pipeline/src/upstream_peer.rs#L31)：`https` 与**无 scheme** 均 `tls=true`，仅显式 `http` 关闭，其他 scheme 报错 | ✅ |
| 证书校验默认开 | `HttpPeer::new(addr, tls, sni)`（[`proxy.rs:74`](../../app/pipeline/src/proxy.rs#L74)）；全仓无 `verify_cert=false`/`insecure`/`danger_accept_*` 任何降级开关 | ✅ |
| SNI 正确 | `sni` 取自 base_url host，并同步写回上游 `Host` 头 | ✅ |
| 客户端凭据不外泄上游 | 转发前剥离 `GATEWAY_KEY_HEADERS=["authorization","x-api-key"]`，再注入上游凭据 | ✅ |

## 非阻塞硬化建议

1. **Admin 令牌常量时间比对**：[`bootstrap.rs:31`](../../app/admin/src/bootstrap.rs#L31) 当前为直接 `==` 比较，存在理论上的时序侧信道。Bootstrap 路径可接受，建议后续引入 `subtle`/常量时间比较（需在 PR 中论证非锁定依赖，或自实现）。已在源码 NOTE 标记。

## 结论

T065 三项核心属性全部满足，无阻塞性问题。SC-008（日志/指标 0 密钥泄漏）由单元、契约、集成、e2e 多层测试共同守护。
