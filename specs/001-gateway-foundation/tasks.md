# Tasks: PingoGate 网关地基（首个实现方向）

**Input**: Design documents from `/specs/001-gateway-foundation/`

**Prerequisites**: [plan.md](./plan.md), [spec.md](./spec.md), [research.md](./research.md), [data-model.md](./data-model.md), [contracts/](./contracts/)

**Tests**: 已包含测试任务 —— 宪法 XIII 要求 TDD（测试先写并观察到失败）。

**Organization**: 按 spec 的 4 个用户故事分组，确保每个故事独立实现、独立测试、独立交付。

## Format: `[ID] [P?] [Story] Description`

- **[P]**: 可并行执行（不同文件、不依赖未完成项）
- **[Story]**: 所属用户故事（US1/US2/US3/US4）
- 描述中均含精确文件路径

---

## Phase 1: Setup（项目初始化）

**Purpose**: 创建 `app/` Cargo workspace 与顶层入口，锁定依赖，配置 CI/lint。

- [x] T001 Create `app/Cargo.toml` workspace root with crates: `core`, `config`, `storage`, `provider`, `pipeline`, `listener`, `admin`, `pingogate` per [plan.md](./plan.md)
- [x] T002 [P] Create workspace-level `.cargo/config.toml` and `rust-toolchain.toml` (latest stable, clippy deny warnings)
- [x] T003 [P] Add root `pingogate.yaml` example and `.env.example` in repository root
- [x] T004 [P] Configure `deny.toml` for license/duplicate crate audit
- [x] T005 Add GitHub workflow `.github/workflows/ci.yml` running `cargo check/clippy/test` and console/CI placeholder

---

## Phase 2: Foundational（阻塞前提）

**Purpose**: 核心基础设施与错误/类型体系，**所有用户故事完成前必须先完成本阶段**。

**⚠️ CRITICAL**: 在用户故事开始前必须完成本阶段。

### Tests for Foundation（先写并失败）

- [x] T006 [P] Unit test `AppError` layer mapping in `app/core/src/error.rs` — domain → application → HTTP boundary
- [x] T007 Unit test trace-id generation and propagation helpers in `app/core/src/context.rs`

### Implementation for Foundation

- [x] T008 [P] Implement `app/core/src/lib.rs` — shared domain types: `ProtocolKind`, `CapabilityFamily`, `ProviderKind`, `AppError`, `RequestContext`, `GatewayError`
- [x] T009 [P] Implement `app/core/src/auth.rs` — `Principal`, `AuthContext`, `authorize(action, resource)` boundary
- [x] T010 [P] Implement `app/storage/src/lib.rs` — `SecretResolver` trait + `env:` reference resolver; no plaintext logging
- [x] T011 Implement `app/config/src/lib.rs` — `pingogate.yaml` serde schema, semantic validation, `RuntimeSnapshot` builder, ArcSwap holder
- [x] T012 Implement `app/provider/src/lib.rs` — `ProviderAdapter` trait, `AuthMethod` descriptor, provider-native error shapes; three adapter stubs (openai/anthropic/gemini) declaring `generation.stateless`
- [ ] T013 Implement `app/listener/src/lib.rs` — Pingora `HttpProxy` service factory and admin `ServeHttp` service factory
- [ ] T014 Implement `app/pipeline/src/lib.rs` — `ProxyHttp` implementation wiring: `request_filter` → `upstream_peer` → `upstream_request_filter` → response/error filters; logging with trace-id
- [ ] T015 Implement `app/admin/src/lib.rs` — Admin HTTP handler with `authorize` boundary and endpoint dispatch stub
- [ ] T016 Implement `app/pingogate/src/main.rs` — bootstrap assembly: load config, start listeners, signal handlers, ArcSwap wiring

**Checkpoint**: Foundation crate-level `cargo test` passes; config validates example `pingogate.yaml`; snapshot ArcSwap stores/loads.

---

## Phase 3: User Story 1 — 应用开发者以最小改动经网关调用上游模型（Priority: P1）🎯 MVP

**Goal**: 客户端仅改 base URL/token/模型名即可让 OpenAI/Anthropic/Gemini 三协议经网关透传，含 SSE 流式。

**Independent Test**: [quickstart.md](./quickstart.md) 步骤 4；三个官方 SDK 各发起非流式与流式请求，响应与直连一致，且客户端只使用网关 Key。

### Tests for User Story 1（先写并失败）

- [ ] T017 [P] [US1] Contract test: OpenAI Chat Completions passthrough in `app/pingogate/tests/contract_openai_passthrough.rs`
- [ ] T018 [P] [US1] Contract test: Anthropic Messages passthrough in `app/pingogate/tests/contract_anthropic_passthrough.rs`
- [ ] T019 [P] [US1] Contract test: Gemini generateContent passthrough in `app/pingogate/tests/contract_gemini_passthrough.rs`
- [ ] T020 [P] [US1] Integration test: SSE streaming for OpenAI-compatible and Anthropic in `app/pingogate/tests/integration_sse.rs`
- [ ] T021 [P] [US1] Contract test: invalid gateway key returns mirrored OpenAI-shaped error in `app/pingogate/tests/contract_auth_error.rs`
- [ ] T022 [P] [US1] Contract test: unknown model alias returns mirrored "no route" error in `app/pingogate/tests/contract_route_error.rs`
- [ ] T023 [P] [US1] Contract test: unsupported capability family returns explicit error in `app/pingogate/tests/contract_unsupported_capability.rs`

### Implementation for User Story 1

- [ ] T024 [P] [US1] Implement protocol detection in `app/pipeline/src/protocol.rs` (OpenAI-compatible / Anthropic / Gemini)
- [ ] T025 [P] [US1] Implement gateway key authentication in `app/pipeline/src/auth_filter.rs`
- [ ] T026 [P] [US1] Implement model alias routing in `app/pipeline/src/router.rs`
- [ ] T027 [P] [US1] Implement upstream auth injection in `app/pipeline/src/upstream_auth.rs` (Bearer / x-api-key / query key)
- [ ] T028 [P] [US1] Implement provider error shape renderers in `app/provider/src/openai.rs`, `app/provider/src/anthropic.rs`, `app/provider/src/gemini.rs`
- [ ] T029 [US1] Implement mirrored gateway error response in `app/pipeline/src/error_response.rs`
- [ ] T030 [US1] Implement SSE byte-stream passthrough in `app/pipeline/src/streaming.rs`
- [ ] T031 [US1] Integrate `pipeline` filters into `ProxyHttp` implementation in `app/pipeline/src/proxy.rs`

**Checkpoint**: User Story 1 独立可运行并可通过其全部契约/集成测试；quickstart 步骤 4 成功。

---

## Phase 4: User Story 2 — 运维人员以声明式配置部署并热重载网关（Priority: P2）

**Goal**: 单一 `pingogate.yaml` 驱动运行时，支持 SIGHUP/Admin API/文件监听触发重载；失败不影响活跃快照；进行中请求绑定旧快照。

**Independent Test**: [quickstart.md](./quickstart.md) 步骤 6；修改配置并触发重载，新路由生效；非法配置重载被拒，旧配置继续服务，在途请求完成。

### Tests for User Story 2（先写并失败）

- [ ] T032 [P] [US2] Integration test: `SIGHUP` reload adds a new route in `app/pingogate/tests/integration_reload_signal.rs`
- [ ] T033 [P] [US2] Integration test: invalid candidate config rejected, active snapshot unchanged in `app/pingogate/tests/integration_reload_rejected.rs`
- [ ] T034 [P] [US2] Integration test: in-flight requests bind old snapshot during reload in `app/pingogate/tests/integration_reload_in_flight.rs`
- [ ] T035 [P] [US2] Contract test: `pingogate.yaml` semantic validation errors in `app/config/tests/validation_test.rs`

### Implementation for User Story 2

- [ ] T036 [P] [US2] Implement `RuntimeSnapshot` immutable builder and ArcSwap atomic switch in `app/config/src/snapshot.rs`
- [ ] T037 [P] [US2] Implement schema + semantic validation in `app/config/src/validate.rs` (route provider existence, key refs resolvable, unique names)
- [ ] T038 [P] [US2] Implement reload orchestrator in `app/admin/src/reload.rs` (load → validate → build → swap → record status)
- [ ] T039 [US2] Implement SIGHUP handler in `app/pingogate/src/signal.rs`
- [ ] T040 [US2] Implement optional file-watch reload trigger in `app/pingogate/src/watch.rs`
- [ ] T041 [US2] Implement `ReloadStatus` in-memory store in `app/admin/src/status.rs`

**Checkpoint**: User Story 2 独立可测试；重载成功/失败/在途请求三种场景均通过。

---

## Phase 5: User Story 3 — 管理员通过机器可读 Admin API 管理与探测网关（Priority: P3）

**Goal**: Admin API 提供 health/readiness/config-validate/reload/reload-status，每个请求经 `authorize` 边界鉴权，bootstrap 静态凭据亦不例外。

**Independent Test**: [quickstart.md](./quickstart.md) 步骤 5；有效/无效凭据访问所有端点；非法配置校验不影响运行时。

### Tests for User Story 3（先写并失败）

- [ ] T042 [P] [US3] Contract test: healthz/readyz return machine-readable states in `app/admin/tests/contract_health.rs`
- [ ] T043 [P] [US3] Contract test: config-validate accepts/rejects config without mutating runtime in `app/admin/tests/contract_validate.rs`
- [ ] T044 [P] [US3] Contract test: reload endpoint triggers reload and returns version/rollback target in `app/admin/tests/contract_reload.rs`
- [ ] T045 [P] [US3] Contract test: reload status endpoint reflects last result in `app/admin/tests/contract_reload_status.rs`
- [ ] T046 [P] [US3] Contract test: missing/invalid admin credentials rejected for every endpoint in `app/admin/tests/contract_admin_auth.rs`

### Implementation for User Story 3

- [ ] T047 [P] [US3] Implement bootstrap admin credentials provider in `app/admin/src/bootstrap.rs` (maps to `Principal`/`AuthContext`)
- [ ] T048 [US3] Implement admin request router and handler in `app/admin/src/handler.rs`
- [ ] T049 [US3] Wire admin `ServeHttp` service into `app/listener/src/admin.rs`
- [ ] T050 [US3] Implement `/config/validate` endpoint using `config::validate` without touching active snapshot
- [ ] T051 [US3] Implement `/reload` endpoint delegating to reload orchestrator
- [ ] T052 [US3] Implement `/reload/status` endpoint reading `ReloadStatus`

**Checkpoint**: User Story 3 独立可测试；所有端点鉴权边界覆盖，无全局 token 等值分支。

---

## Phase 6: User Story 4 — 运维人员通过可观测信号排查流量（Priority: P4）

**Goal**: 结构化日志带 trace-id；Prometheus 指标按 provider/capability family 打标签；敏感值脱敏。

**Independent Test**: [quickstart.md](./quickstart.md) 步骤 7；驱动跨三 Provider 请求，抓取指标端点验证标签与计数，检查日志无密钥明文。

### Tests for User Story 4（先写并失败）

- [ ] T053 [P] [US4] Unit test: metrics labels contain provider + capability family in `app/pipeline/src/metrics.rs`
- [ ] T054 [P] [US4] Integration test: Prometheus endpoint returns expected counters/histograms in `app/pingogate/tests/integration_metrics.rs`
- [ ] T055 [P] [US4] Unit test: structured logs include trace-id and no plaintext keys in `app/pipeline/src/logging.rs`

### Implementation for User Story 4

- [ ] T056 [P] [US4] Implement trace-id generation and `tracing` span injection in `app/core/src/trace.rs`
- [ ] T057 [P] [US4] Implement Prometheus metrics recorder in `app/pipeline/src/metrics.rs` (request count, status, latency, upstream latency, token count)
- [ ] T058 [P] [US4] Implement token counting hooks in `app/provider/src/usage.rs` (input/output/reasoning/cache when available)
- [ ] T059 [P] [US4] Implement sensitive-value redaction layer in `app/core/src/redact.rs`
- [ ] T060 [US4] Wire metrics endpoint into admin listener in `app/admin/src/metrics_endpoint.rs`

**Checkpoint**: User Story 4 独立可测试；日志/指标抽样审计 0 密钥泄漏。

---

## Phase 7: Polish & Cross-Cutting Concerns

**Purpose**: 跨故事优化、基准、安全复核、文档与 quickstart 验证。

- [ ] T061 [P] Run `cargo clippy -- -D warnings` and fix all issues across `app/`
- [ ] T062 [P] Run `cargo fmt --check` and fix formatting
- [ ] T063 [P] Implement end-to-end quickstart validation in `app/pingogate/tests/e2e_quickstart.rs`
- [ ] T064 Run throughput/latency benchmark against stub upstreams and record p50/p95 (target: <5 ms p50 / <20 ms p95)
- [ ] T065 [P] Security-review pass: verify no plaintext keys in logs/metrics/config responses, `authorize` boundary enforced, upstream TLS default-on
- [ ] T066 [P] Update `README.md` with build/run/quickstart instructions
- [ ] T067 Verify `.github/workflows/ci.yml` passes `cargo check/clippy/test`
- [ ] T068 Run `speckit-verify` (or `/verify`) against quickstart scenarios

---

## Dependencies & Execution Order

### Phase Dependencies

- **Setup (Phase 1)**: 无依赖，可立即开始。
- **Foundational (Phase 2)**: 依赖 Setup；阻塞所有用户故事。
- **User Stories (Phase 3–6)**: 依赖 Foundational；理论上可并行（如多开发者），但建议按 P1→P4 顺序集成。
- **Polish (Phase 7)**: 依赖所有用户故事完成。

### User Story Dependencies

- **US1 (P1)**: 仅依赖 Foundation。MVP 范围：完成 Phase 1+2+3 即可交付。
- **US2 (P2)**: 依赖 Foundation + US1 已建立的基本请求路径（可独立开发，但集成测试需 US1 路由/认证可用）。
- **US3 (P3)**: 依赖 Foundation + US2 的重载编排器。
- **US4 (P4)**: 依赖 Foundation + US1 的请求管线。

### Within Each User Story

1. 先写契约/集成测试并观察到失败。
2. 实现实体/模块。
3. 装配到管线/Admin/入口。
4. 运行该故事全部测试至通过。
5. 原子提交。

### Parallel Opportunities

- Phase 1 所有 [P] 任务可并行。
- Phase 2 中 T008–T010 与 T013 可并行；T011/T012/T014/T015/T016 在依赖类型稳定后可并行。
- US1 内 T024–T028 可并行；T029/T030/T031 在它们完成后进行。
- US2/US3/US4 各自的测试任务可并行。
- 多开发者时：US1、US2、US3、US4 可并行推进。

---

## Parallel Example: User Story 1

```bash
# Launch all contract tests for US1 together (they will fail first):
T017 contract_openai_passthrough.rs
T018 contract_anthropic_passthrough.rs
T019 contract_gemini_passthrough.rs
T021 contract_auth_error.rs
T022 contract_route_error.rs
T023 contract_unsupported_capability.rs

# Launch all pipeline modules for US1 together:
T024 protocol.rs
T025 auth_filter.rs
T026 router.rs
T027 upstream_auth.rs
T028 provider/{openai,anthropic,gemini}.rs
```

---

## Implementation Strategy

### MVP First（仅 US1）

1. 完成 Phase 1 Setup。
2. 完成 Phase 2 Foundational（关键阻塞）。
3. 完成 Phase 3 User Story 1。
4. **STOP and VALIDATE**: 独立运行 US1 全部契约/集成测试与 quickstart 步骤 4。
5. 此时 PingoGate 已是一个可用的 LLM 网关 MVP。

### Incremental Delivery

1. Setup + Foundational → Foundation ready。
2. US1 → 测试通过 → 可演示透传 MVP。
3. US2 → 测试通过 → 可运维重载。
4. US3 → 测试通过 → 可编排接管。
5. US4 → 测试通过 → 可观测排障。
6. Polish → 基准、安全、CI、文档。

### Parallel Team Strategy

多开发者时：

- 开发者 A：Foundation + US1
- 开发者 B：US2（依赖 Foundation，可与 US1 并行开发，集成测试需 US1 完成后跑）
- 开发者 C：US3（依赖 US2 reload 编排器）
- 开发者 D：US4 metrics/logging（依赖 Foundation + US1）

---

## Notes

- 每个任务必须对应具体文件路径，避免模糊。
- 测试任务先于实现任务；宪法 XIII 要求测试先写并观察到失败。
- 提交信息使用英文 Conventional Commits，与任务 ID 对应（如 `feat(pipeline): add protocol detection for US1 (T024)`）。
- 任务完成时由 `write-context.py --task TNNN ... --append` 记录，**不要**手改 tasks.md 复选框。
- 在任一站点可停止并独立验证已完成的故事。
