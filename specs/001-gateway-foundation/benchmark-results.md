# 基准测试结果：PingoGate 网关地基

**Feature**: 001-gateway-foundation | **任务**: T064 | **日期**: 2026-06-30

## 方法

- 基准用例：[`app/pingogate/tests/benchmark.rs`](../../app/pingogate/tests/benchmark.rs)（标记 `#[ignore]`，不进入常规 CI 门禁）。
- 拓扑：真实 `pingogate` 子进程（release 构建）→ echo 桩上游（loopback），三协议默认路由。
- 负载：OpenAI `/v1/chat/completions` 透传请求，单线程顺序发送。
- 客户端：每请求新建 TCP 连接（`connection: close`），即测量值**包含**每次连接建立开销，构成网关处理开销的上界。
- 预热 200 次，测量 3000 次。

### 复现

```bash
cargo test --release --test benchmark -- --ignored --nocapture
```

## 结果

| 指标 | 实测 | 目标（T064） | 结论 |
| ---- | ---- | ---- | ---- |
| p50 延迟 | **1.54 ms** | < 5 ms | ✅ 通过 |
| p95 延迟 | **1.74 ms** | < 20 ms | ✅ 通过 |
| p90 延迟 | 1.69 ms | — | — |
| p99 延迟 | 1.86 ms | — | — |
| max 延迟 | 6.04 ms | — | — |
| 吞吐 | 644 req/s（单连接顺序） | — | 受顺序往返限制，非吞吐上限 |

## 说明

- p50/p95 均**显著优于**目标；由于测量含每请求 TCP 建连开销，网关自身代理开销低于上表数值。
- 644 req/s 为单线程顺序往返速率（每次等待完整响应后再发下一条），反映单连接延迟而非并发吞吐天花板；并发吞吐基准留作后续容量规划项。
- 桩上游每连接 `connection: close`，故每次请求网关需重建上游连接，已计入实测延迟。
