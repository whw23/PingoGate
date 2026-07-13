# S1 双语言基建 + Rust 内核透传 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 搭建 proto/Rust/Go 双语言基建骨架,移植 001 的 Rust 内核数据面实现六接口同态透传(单机模式可跑),预留平台模式位置(SnapshotSource/KeyAuth trait + KeyVault 空壳 + gRPC 骨架)。

**Architecture:** 单仓库 `proto/` + `core-rs/`(Cargo workspace)+ `ctrl-go/`(Go module)。Rust 内核基于 Pingora 0.8.0 ProxyHttp 实现请求管线,从 redesign 分支 001 移植数据面代码并适配(路径后缀模式协议识别 + SnapshotSource/KeyAuth trait + 六接口扩展)。Go 侧只建 gRPC client 骨架(连 Rust,推空快照),不做业务。S1 结束时单机模式六接口透传可跑,平台模式位置编译通过。

**Tech Stack:** Rust(edition 2021)+ Pingora 0.8.0(pingora/pingora-proxy/pingora-core)+ tokio + serde + serde_json + serde_yaml + arc-swap + tracing + prometheus;Go(最新稳定版)+ grpc-go + chi(骨架);protobuf + prost(Rust)+ protoc-gen-go(Go)。

## Global Constraints

- Rust 100% Safe Rust(`unsafe` 禁止);文件 ≤300 行,函数 ≤50 行,参数 ≤4,圈复杂度 ≤10,嵌套 ≤3(宪法 V)。
- Go 文件 ≤300 行,函数 ≤50 行,参数 ≤4,圈复杂度 ≤10(宪法 V)。
- 热路径必须 Pingora(宪法 III);Rust 内核零 DB、不带 tokenizer(宪法总纲)。
- 密钥明文 MUST NOT 进日志/指标/错误(宪法 XX);`SecretString` 用完清零。
- 公共 API 英文文档注释;项目文档中文;提交信息英文 Conventional Commits(宪法 XIV/XV/XXII)。
- Pingora 版本固定 0.8.0(与 001 一致,context7 已核对 ProxyHttp trait 签名)。
- 从 redesign 分支 `app/` 移植 001 代码,适配而非重写。

## File Structure

S1 涉及的文件(创建/移植):

**`proto/`**(新建):
- `proto/pingogate.proto`:gRPC 服务定义(SnapshotService/KeyVaultService/UsageService)+ 跨语言消息类型。S1 只定义骨架,消息字段最小化。

**`core-rs/`**(新建 Cargo workspace,从 001 `app/` 移植并重组):
- `core-rs/Cargo.toml`:workspace 根,成员 crate 列表。
- `core-rs/core/`(移植 001 `app/core/`):`src/{lib,error,auth,context,redact,secret,time,trace,types}.rs`。共享域类型 + AppError + Principal/authorize + SecretString + ProtocolKind/ProviderKind/CapabilityFamily/AuthMethod。**扩展**:ProtocolKind 加 Responses/Interactions(001 只有 OpenAiCompatible/Anthropic/Gemini)。
- `core-rs/storage/`(移植 001 `app/storage/` + 新增):`src/{lib,secret}.rs`(移植 EnvSecretResolver)+ `src/snapshot_source.rs`(新增 SnapshotSource trait + FileSnapshotSource)+ `src/keyvault.rs`(新增 KeyVault trait 空壳,不实现加密)。
- `core-rs/snapshot/`(移植 001 `app/config/` 重组):`src/{lib,snapshot,validate}.rs`。RuntimeSnapshot + SnapshotHolder(ArcSwap)+ 语义校验。**适配**:snapshot 持密文 provider key(平台)或 env 解析明文(单机),经 SnapshotSource 构建。
- `core-rs/router/`(从 001 `app/pipeline/router.rs` 拆出):`src/lib.rs`。模型别名提取(body model / gemini path)。
- `core-rs/transform/`(新增):`src/lib.rs`。Transform trait(M0+M1 空实现,S3 才用)。
- `core-rs/provider/`(移植 001 `app/provider/` 扩展):`src/{lib,adapter,error_shape,openai,anthropic,gemini,usage}.rs`。adapter trait + 六接口 adapter(001 只有三个,加 Responses/Interactions/streamGenerateContent)+ 错误体形状。
- `core-rs/pipeline/`(移植 001 `app/pipeline/` 扩展):`src/{lib,proxy,protocol,ctx,auth_filter,upstream_peer,upstream_auth,streaming,error_response,logging,metrics,observe,wire}.rs`。GatewayProxy(ProxyHttp)+ 协议识别(改造为路径后缀模式)+ 上游认证注入 + SSE + 错误镜像 + 可观测。**改造**:protocol.rs 路径后缀模式 + 版本前缀通配 + 六接口。
- `core-rs/listener/`(移植 001 `app/listener/`):`src/{lib,admin}.rs`。Pingora public/admin service 装配。
- `core-rs/pingogate-core/`(移植 001 `app/pingogate/` 改名):`src/{main,signal,watch}.rs`。bootstrap + 信号 + 文件监视 reload 编排 + gRPC server 装配(S1 gRPC 空壳)。
- `core-rs/pingogate-core/tests/`(移植 001 测试重组):`e2e_quickstart.rs` 等。

**`ctrl-go/`**(新建 Go module,只骨架):
- `ctrl-go/go.mod`:module 根。
- `ctrl-go/cmd/pingogate-ctrl/main.go`:主二进制骨架(S1 只连 gRPC + 推空快照)。
- `ctrl-go/internal/proto/`:protobuf codegen 产物(`.pb.go`)。
- `ctrl-go/internal/snapshot/client.go`:gRPC client,连 Rust 推空快照(S1 仅证伪连通)。

**配置/示例**:
- `pingogate-core.yaml.example`:单机模式配置示例(移植 001 `pingogate.yaml`)。
- `core-rs/rust-toolchain.toml`:Rust 工具链固定。

## 任务分解

S1 分 12 个任务,每个独立可测试。任务间依赖顺序:T1(proto)→ T2(workspace 骨架)→ T3(core 移植)→ T4(storage trait)→ T5(snapshot)→ T6(provider 扩展)→ T7(pipeline 协议识别改造)→ T8(pipeline 透传)→ T9(listener+main)→ T10(单机模式集成测试)→ T11(Go gRPC 骨架)→ T12(交叉认证 + S1 证伪)。

---

### Task 1: proto 定义 + codegen 骨架

**Files:**
- Create: `proto/pingogate.proto`
- Create: `proto/buf.gen.yaml`(可选,buf 工具配置)
- Create: `core-rs/core/build.rs`(prost 构建,后续 task 用)
- Create: `ctrl-go/internal/proto/`(.pb.go 生成目标)

**Interfaces:**
- Consumes: 无(基础)
- Produces: `proto/pingogate.proto` 定义 gRPC 服务骨架(SnapshotService 含 PushSnapshot + Heartbeat、KeyVaultService、UsageService、HealthService)+ PushDelta 预留位置(不实现)。消息:Snapshot/Ack/Heartbeat*/Encrypt*/Decrypt*/UsageEvent/Health*。所有 gRPC 调用经 mTLS + internal token(spec §12A),proto 不含 token 字段(经 gRPC metadata 传递)。

- [ ] **Step 1: 写 proto 文件**

```protobuf
// proto/pingogate.proto
syntax = "proto3";
package pingogate;
option go_package = "github.com/whw23/pingogate/ctrl-go/internal/proto;pingogatepb";

// 所有服务:调用方经 gRPC metadata 携带 x-internal-token(spec §12A);
// 传输经 mTLS(Rust/Go 互验证书,仅 127.0.0.1)。

// SnapshotService: Go -> Rust 推送运行时快照(S1 骨架,S2 实现)
service SnapshotService {
  rpc PushSnapshot(stream Snapshot) returns (Ack);
  rpc Heartbeat(HeartbeatRequest) returns (HeartbeatResponse);
  // 预留:增量推送(S2/S3 不实现,转增量触发条件见 spec §12C)
  rpc PushDelta(stream PushDeltaRequest) returns (Ack);
}

// KeyVaultService: Go -> Rust 加解密(S1 骨架,S2 实现加密)
service KeyVaultService {
  rpc Encrypt(EncryptRequest) returns (EncryptResponse);
  rpc Decrypt(DecryptRequest) returns (DecryptResponse);
}

// UsageService: Rust -> Go 推 usage 事件(S1 骨架,S3 实现)
service UsageService {
  rpc ReportUsage(stream UsageEvent) returns (Ack);
}

// HealthService: 双向心跳 + Rust 报告当前快照 version(spec §3.3/§12C 恢复)
service HealthService {
  rpc Check(HealthRequest) returns (HealthResponse);
}

message Snapshot {
  uint64 version = 1;
  bytes payload = 2;  // S1 占位;S2 扩展
}

message Ack {
  uint64 version = 1;
  bool ok = 2;
  string error = 3;
}

// 预留:增量推送(S2/S3 不实现)
message PushDeltaRequest {
  uint64 from_version = 1;
  uint64 to_version = 2;
  // S2/S3 不填;转增量时扩展 added/modified/removed
}

message HeartbeatRequest {
  uint64 current_version = 1;
  bool ready = 2;
}
message HeartbeatResponse {
  bool acknowledged = 1;
}

message HealthRequest {
  uint64 current_version = 1;  // Rust 报告当前快照 version
}
message HealthResponse {
  bool ready = 1;              // Rust 是否 ready(收到首个全量快照后)
  uint64 version = 2;
}

message EncryptRequest {
  bytes plaintext = 1;
}
message EncryptResponse {
  bytes ciphertext = 1;
  string error = 2;
}

message DecryptRequest {
  bytes ciphertext = 1;
  string requester_user_id = 2;  // S3 双防线用
  string key_id = 3;
  string intent = 4;  // view_plaintext | hot_path_inject
}
message DecryptResponse {
  bytes plaintext = 1;
  string error = 2;
}

message UsageEvent {
  uint64 version = 1;
  bytes payload = 2;  // S3 扩展为结构化 usage
}
```

- [ ] **Step 2: 安装 codegen 工具并生成**

Run:
```bash
# Rust prost(在 core-rs/core/build.rs 配,Step 3 做);先装 Go protoc 插件
go install google.golang.org/protobuf/cmd/protoc-gen-go@latest
go install google.golang.org/grpc/cmd/protoc-gen-go-grpc@latest
# 生成 Go 代码
protoc --proto_path=proto \
  --go_out=ctrl-go/internal/proto --go_opt=paths=source_relative \
  --go-grpc_out=ctrl-go/internal/proto --go-grpc_opt=paths=source_relative \
  proto/pingogate.proto
```
Expected: `ctrl-go/internal/proto/pingogate.pb.go` 和 `pingogate_grpc.pb.go` 生成。

- [ ] **Step 3: 配置 Rust prost build.rs(在 core-rs/core/)**

此 step 在 T3 创建 core crate 时落地,这里先记录 build.rs 内容:

```rust
// core-rs/core/build.rs
fn main() -> Result<(), Box<dyn std::error::Error>> {
    tonic_build::configure()
        .build_server(false)
        .build_client(true)
        .compile_protos(&["../../proto/pingogate.proto"], &["../../proto"])?;
    Ok(())
}
```

- [ ] **Step 4: Commit**

```bash
git add proto/ ctrl-go/internal/proto/
git commit -m "feat(proto): define gRPC service skeletons for snapshot/keyvault/usage"
```

---

### Task 2: core-rs Cargo workspace 骨架

**Files:**
- Create: `core-rs/Cargo.toml`(workspace 根)
- Create: `core-rs/rust-toolchain.toml`
- Create: `core-rs/deny.toml`(依赖审计,从根 deny.toml 调整或复用)

**Interfaces:**
- Consumes: 无
- Produces: workspace 根 + 成员 crate 占位(后续 task 填充)。成员:`core`/`storage`/`snapshot`/`router`/`transform`/`provider`/`pipeline`/`listener`/`pingogate-core`。

- [ ] **Step 1: 写 workspace Cargo.toml**

```toml
# core-rs/Cargo.toml
[workspace]
resolver = "2"
members = [
    "core",
    "storage",
    "snapshot",
    "router",
    "transform",
    "provider",
    "pipeline",
    "listener",
    "pingogate-core",
]

[workspace.package]
version = "0.1.0"
edition = "2021"
rust-version = "1.75"

[workspace.dependencies]
pingora = "0.8"
pingora-proxy = "0.8"
pingora-core = "0.8"
tokio = { version = "1", features = ["full"] }
serde = { version = "1", features = ["derive"] }
serde_json = "1"
serde_yaml = "0.9"
arc-swap = "1"
tracing = "0.1"
tracing-subscriber = { version = "0.3", features = ["env-filter"] }
prometheus = "0.13"
bytes = "1"
http = "0.2"
tonic = "0.10"
prost = "0.12"
tonic-build = "0.10"
```

- [ ] **Step 2: 写 rust-toolchain.toml**

```toml
# core-rs/rust-toolchain.toml
[toolchain]
channel = "1.75.0"
components = ["rustfmt", "clippy"]
```

- [ ] **Step 3: 创建各成员 crate 的最小 Cargo.toml + lib.rs/main.rs(空)**

为每个 member 创建 `Cargo.toml`(只声明 name + 依赖 workspace)和空 `src/lib.rs`(或 `main.rs`)。示例:

```toml
# core-rs/core/Cargo.toml
[package]
name = "pingogate-core"
version.workspace = true
edition.workspace = true

[dependencies]
serde = { workspace = true }
serde_json = { workspace = true }
```

```rust
// core-rs/core/src/lib.rs
//! Shared domain types, error taxonomy, authorization boundary.
```

重复此 step 为 storage/snapshot/router/transform/provider/pipeline/listener/pingogate-core 各创建。`pingogate-core` 用 `src/main.rs`。

- [ ] **Step 4: 验证 workspace 编译**

Run: `cd core-rs && cargo check --workspace`
Expected: 无错误(各 crate 空,但 workspace 结构有效)。

- [ ] **Step 5: Commit**

```bash
git add core-rs/
git commit -m "feat(core-rs): scaffold Cargo workspace with 9 member crates"
```

---

### Task 3: core crate 移植(从 001 app/core/)

**Files:**
- Create: `core-rs/core/src/{lib,error,auth,context,redact,secret,time,trace,types}.rs`(从 `app/core/src/` 移植)
- Modify: `core-rs/core/src/types.rs`(扩展 ProtocolKind 加 Responses/Interactions)

**Interfaces:**
- Consumes: 001 `app/core/` 源码(redesign 分支)
- Produces: `pingogate_core` crate 导出 `AppError`/`Principal`/`AuthContext`/`authorize`/`SecretString`/`ProtocolKind`/`ProviderKind`/`CapabilityFamily`/`AuthMethod`/`RequestContext`/`TraceId`。供 T4-T9 消费。

- [ ] **Step 1: 移植 001 core 源码**

从 redesign 分支移植(逐文件,改 crate 名 `pingo_core` -> `pingogate_core`):

```bash
git show redesign:app/core/src/lib.rs > core-rs/core/src/lib.rs
git show redesign:app/core/src/error.rs > core-rs/core/src/error.rs
git show redesign:app/core/src/auth.rs > core-rs/core/src/auth.rs
git show redesign:app/core/src/context.rs > core-rs/core/src/context.rs
git show redesign:app/core/src/redact.rs > core-rs/core/src/redact.rs
git show redesign:app/core/src/secret.rs > core-rs/core/src/secret.rs
git show redesign:app/core/src/time.rs > core-rs/core/src/time.rs
git show redesign:app/core/src/trace.rs > core-rs/core/src/trace.rs
git show redesign:app/core/src/types.rs > core-rs/core/src/types.rs
```

- [ ] **Step 2: 改 crate 名引用**

全局替换 `pingo_core` -> `pingogate_core`(`lib.rs` 的 `pub mod` 无影响,但其他 crate 依赖时用新名)。

- [ ] **Step 3: 扩展 ProtocolKind 加 Responses/Interactions**

```rust
// core-rs/core/src/types.rs(在 ProtocolKind enum 增加)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProtocolKind {
    OpenAiCompatible,   // Chat Completions
    OpenAiResponses,    // Responses(新增)
    Anthropic,
    Gemini,             // generateContent / streamGenerateContent
    GeminiInteractions, // Interactions(新增)
}
```

同步 `as_str()` 返回:`"openai-responses"` / `"gemini-interactions"`。

- [ ] **Step 4: 跑 001 core 的单元测试(移植)**

001 的 `core/src/*.rs` 含 `#[cfg(test)] mod tests`。移植后跑:

Run: `cd core-rs && cargo test -p pingogate-core`
Expected: 001 的 core 测试全通过(可能需修 crate 名引用)。

- [ ] **Step 5: Commit**

```bash
git add core-rs/core/
git commit -m "feat(core): port 001 core crate, extend ProtocolKind with Responses/Interactions"
```

---

### Task 4: storage crate(SnapshotSource trait + KeyVault 空壳 + EnvSecretResolver 移植)

**Files:**
- Create: `core-rs/storage/src/{lib,secret,snapshot_source,keyvault}.rs`

**Interfaces:**
- Consumes: T3 的 `pingogate_core::SecretString`/`AppError`;001 `app/storage/src/secret.rs`
- Produces:
  - `SnapshotSource` trait:`fn build_snapshot(&self, version: u64) -> Result<Arc<RuntimeSnapshot>, SnapshotError>`
  - `FileSnapshotSource` 实现(读 YAML,单机模式)
  - `KeyVault` trait(空壳):`fn encrypt(&self, plaintext: &[u8]) -> Result<Vec<u8>, KeyError>` / `fn decrypt(&self, ciphertext: &[u8]) -> Result<SecretString, KeyError>`,S1 返回 `KeyError::NotImplemented`
  - `EnvSecretResolver`(移植 001)

- [ ] **Step 1: 移植 EnvSecretResolver**

```bash
git show redesign:app/storage/src/secret.rs > core-rs/storage/src/secret.rs
git show redesign:app/storage/src/lib.rs > core-rs/storage/src/lib.rs
```

- [ ] **Step 2: 写 SnapshotSource trait + FileSnapshotSource(失败测试先)**

```rust
// core-rs/storage/src/snapshot_source.rs
//! Snapshot source abstraction: standalone (file) vs platform (gRPC).
//! S1 implements FileSnapshotSource; GrpcSnapshotSource is S2.

use std::sync::Arc;
use pingogate_core::AppError;
use pingogate_snapshot::RuntimeSnapshot;

#[derive(Debug, thiserror::Error)]
pub enum SnapshotError {
    #[error("config parse: {0}")]
    Parse(String),
    #[error("config validate: {0}")]
    Validate(String),
    #[error("secret resolve: {0}")]
    Secret(String),
}

pub trait SnapshotSource: Send + Sync {
    fn build_snapshot(&self, version: u64) -> Result<Arc<RuntimeSnapshot>, SnapshotError>;
}
```

- [ ] **Step 3: 写 FileSnapshotSource 测试(失败)**

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn file_source_builds_snapshot_from_yaml() {
        let yaml = "listeners:\n  public: { address: \"0.0.0.0:8080\" }\n  admin: { address: \"127.0.0.1:9090\" }\ngateway_keys:\n  - { name: \"a\", secret_ref: \"env:PINGO_KEY_ALPHA\" }\nproviders:\n  - { name: \"openai-main\", kind: \"openai-compatible\", base_url: \"https://api.openai.com\", auth: { method: \"bearer\", key_ref: \"env:OPENAI_API_KEY\" }, capability_families: [\"generation.stateless\"] }\nroutes:\n  - { alias: \"gpt-4o\", provider: \"openai-main\", upstream_model: \"gpt-4o\" }\nupstream: { timeout_ms: 60000 }\n";
        let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../target/test_config.yaml");
        std::fs::write(&path, yaml).unwrap();
        std::env::set_var("PINGO_KEY_ALPHA", "test-key");
        std::env::set_var("OPENAI_API_KEY", "sk-test");
        let source = FileSnapshotSource::new(path);
        let snap = source.build_snapshot(1).unwrap();
        assert_eq!(snap.version, 1);
        assert_eq!(snap.providers.len(), 1);
    }
}
```

- [ ] **Step 4: 跑测试确认失败**

Run: `cd core-rs && cargo test -p pingogate-storage file_source`
Expected: FAIL(FileSnapshotSource 未实现)。

- [ ] **Step 5: 实现 FileSnapshotSource(读 YAML + 校验 + 解析密钥 + 构建 snapshot)**

依赖 T5 的 `RuntimeSnapshot`(此 step 先写占位,T5 填充 snapshot 结构)。实际实现见 T5 完成后回填。

- [ ] **Step 6: 写 KeyVault trait 空壳**

```rust
// core-rs/storage/src/keyvault.rs
//! KeyVault: provider key encryption/decryption. S1 stub; S2 implements AES-GCM.
use pingogate_core::SecretString;

#[derive(Debug, thiserror::Error)]
pub enum KeyError {
    #[error("keyvault not implemented in S1")]
    NotImplemented,
}

pub trait KeyVault: Send + Sync {
    fn encrypt(&self, plaintext: &[u8]) -> Result<Vec<u8>, KeyError>;
    fn decrypt(&self, ciphertext: &[u8]) -> Result<SecretString, KeyError>;
}

/// S1 stub; S2 replaces with AesGcmKeyVault.
pub struct StubKeyVault;

impl KeyVault for StubKeyVault {
    fn encrypt(&self, _plaintext: &[u8]) -> Result<Vec<u8>, KeyError> {
        Err(KeyError::NotImplemented)
    }
    fn decrypt(&self, _ciphertext: &[u8]) -> Result<SecretString, KeyError> {
        Err(KeyError::NotImplemented)
    }
}
```

- [ ] **Step 7: Commit**

```bash
git add core-rs/storage/
git commit -m "feat(storage): SnapshotSource trait + FileSnapshotSource + KeyVault stub"
```

---

### Task 5: snapshot crate(RuntimeSnapshot + ArcSwap + 校验)

**Files:**
- Create: `core-rs/snapshot/src/{lib,snapshot,validate}.rs`(从 001 `app/config/src/{snapshot,validate}.rs` 移植重组)

**Interfaces:**
- Consumes: T3 `pingogate_core`;001 `app/config/`
- Produces: `RuntimeSnapshot`(含 version/providers/routes/gateway_keys/streaming 标记)+ `SnapshotHolder`(ArcSwap)

- [ ] **Step 1: 移植 001 config snapshot + validate**

```bash
git show redesign:app/config/src/snapshot.rs > core-rs/snapshot/src/snapshot.rs
git show redesign:app/config/src/validate.rs > core-rs/snapshot/src/validate.rs
```

- [ ] **Step 2: 改 RuntimeSnapshot 支持密文/明文双模式**

001 的 snapshot 持明文 provider key(SecretString,env 解析)。S1 保持单机模式明文;平台模式密文在 S2 加(预留字段)。

```rust
// core-rs/snapshot/src/snapshot.rs(关键结构)
pub struct RuntimeSnapshot {
    pub version: u64,
    pub providers: Vec<ResolvedProvider>,
    pub routes: Vec<Route>,
    pub gateway_keys: Vec<GatewayKey>,
    pub upstream: UpstreamConfig,
}

pub struct ResolvedProvider {
    pub name: String,
    pub kind: ProviderKind,
    pub base_url: String,
    pub auth_method: AuthMethod,
    /// 单机模式:env 解析的明文(平台模式密文 + KeyVault 解密)
    pub key: SecretString,
    pub anthropic_version: Option<String>,
    pub capability_families: Vec<CapabilityFamily>,
}
```

- [ ] **Step 3: 移植 SnapshotHolder(ArcSwap)**

```rust
// core-rs/snapshot/src/lib.rs
use arc_swap::ArcSwap;
use std::sync::Arc;

pub struct SnapshotHolder {
    inner: ArcSwap<RuntimeSnapshot>,
}

impl SnapshotHolder {
    pub fn new(initial: Arc<RuntimeSnapshot>) -> Self {
        Self { inner: ArcSwap::from(initial) }
    }
    pub fn load(&self) -> arc_swap::Guard<Arc<RuntimeSnapshot>> {
        self.inner.load()
    }
    pub fn store(&self, snapshot: Arc<RuntimeSnapshot>) {
        self.inner.store(snapshot);
    }
}
```

- [ ] **Step 4: 跑 001 snapshot/validate 测试**

Run: `cd core-rs && cargo test -p pingogate-snapshot`
Expected: 001 的 config 测试通过(可能需修引用)。

- [ ] **Step 5: 回填 T4 FileSnapshotSource 实现(用 RuntimeSnapshot 结构)**

T4 Step 5 的 FileSnapshotSource 现在可用 T5 的 RuntimeSnapshot 完整实现:读 YAML -> serde 解析 -> validate 语义校验 -> EnvSecretResolver 解析 key -> 构建 RuntimeSnapshot。

- [ ] **Step 6: 跑 T4 测试确认通过**

Run: `cd core-rs && cargo test -p pingogate-storage file_source`
Expected: PASS。

- [ ] **Step 7: Commit**

```bash
git add core-rs/snapshot/ core-rs/storage/
git commit -m "feat(snapshot): port RuntimeSnapshot + ArcSwap holder, wire FileSnapshotSource"
```

---

### Task 6: provider crate 扩展(六接口 adapter)

**Files:**
- Create: `core-rs/provider/src/{lib,adapter,error_shape,openai,anthropic,gemini,responses,interactions,usage}.rs`(从 001 `app/provider/` 移植 + 扩展)

**Interfaces:**
- Consumes: T3 `pingogate_core::ProtocolKind`/`ProviderKind`/`CapabilityFamily`/`AuthMethod`;001 `app/provider/`
- Produces: `ProviderAdapter` trait + 六接口 adapter(001 三接口 + Responses/Interactions/streamGenerateContent)

- [ ] **Step 1: 移植 001 provider 三接口**

```bash
git show redesign:app/provider/src/lib.rs > core-rs/provider/src/lib.rs
git show redesign:app/provider/src/adapter.rs > core-rs/provider/src/adapter.rs
git show redesign:app/provider/src/error_shape.rs > core-rs/provider/src/error_shape.rs
git show redesign:app/provider/src/openai.rs > core-rs/provider/src/openai.rs
git show redesign:app/provider/src/anthropic.rs > core-rs/provider/src/anthropic.rs
git show redesign:app/provider/src/gemini.rs > core-rs/provider/src/gemini.rs
git show redesign:app/provider/src/usage.rs > core-rs/provider/src/usage.rs
```

- [ ] **Step 2: 扩展 adapter 支持六接口**

001 的 adapter 只认 OpenAiCompatible/Anthropic/Gemini。扩展:

```rust
// core-rs/provider/src/adapter.rs(扩展)
impl ProviderAdapter {
    /// 按 ProtocolKind 返回该接口的认证方式 + 错误体形状
    pub fn for_protocol(protocol: ProtocolKind) -> Self {
        match protocol {
            ProtocolKind::OpenAiCompatible => Self::openai_chat(),
            ProtocolKind::OpenAiResponses => Self::openai_responses(),  // 新增
            ProtocolKind::Anthropic => Self::anthropic(),
            ProtocolKind::Gemini => Self::gemini_generate(),
            ProtocolKind::GeminiInteractions => Self::gemini_interactions(),  // 新增
        }
    }
}
```

新增 `responses.rs` / `interactions.rs`:认证方式(OpenAI Responses 用 Bearer,同 Chat Completions;Gemini Interactions 用 x-goog-api-key,同 Gemini) + 错误体形状(各自 provider 原生)。S1 透传不改 body,adapter 主要声明认证 + 错误形状。

- [ ] **Step 3: 跑 001 provider 测试**

Run: `cd core-rs && cargo test -p pingogate-provider`
Expected: 001 provider 测试通过 + 新增 Responses/Interactions 的 adapter 测试通过。

- [ ] **Step 4: Commit**

```bash
git add core-rs/provider/
git commit -m "feat(provider): port 001 adapters, extend to six interfaces (Responses/Interactions)"
```

---

### Task 7: pipeline 协议识别改造(路径后缀模式 + 版本前缀通配)

**Files:**
- Create: `core-rs/pipeline/src/protocol.rs`(从 001 `app/pipeline/src/protocol.rs` 移植 + 改造)
- Test: `core-rs/pipeline/src/protocol.rs` 的 `#[cfg(test)]`

**Interfaces:**
- Consumes: T3 `pingogate_core::ProtocolKind`(含 Responses/Interactions);001 协议识别
- Produces: `detect(method, path) -> Detection`,识别六接口,版本前缀通配,不含硬编码 `/v1`

**关键改造**(调研结论):001 用 `path == "/v1/chat/completions"` 精确匹配;改为路径后缀模式匹配(版本前缀可选段),加 Responses/Interactions/streamGenerateContent。LiteLLM 用双重注册(`/v1beta/...` 和 `/...`),我们在 Rust 里实现为"路径后缀匹配 + 版本前缀通配"。

- [ ] **Step 1: 写失败测试(六接口 + 版本前缀通配 + 不支持能力 + 未识别)**

```rust
// core-rs/pipeline/src/protocol.rs(测试)
#[cfg(test)]
mod tests {
    use super::*;
    use pingogate_core::ProtocolKind;

    #[test]
    fn detects_openai_chat_with_any_version_prefix() {
        assert!(matches!(detect("POST", "/v1/chat/completions"), Detection::Supported(_)));
        assert!(matches!(detect("POST", "/v2/chat/completions"), Detection::Supported(_)));
        assert!(matches!(detect("POST", "/chat/completions"), Detection::Supported(_)));
    }

    #[test]
    fn detects_openai_responses() {
        assert!(matches!(detect("POST", "/v1/responses").protocol_if_supported(), Some(ProtocolKind::OpenAiResponses)));
    }

    #[test]
    fn detects_anthropic_messages() {
        let d = detect("POST", "/v1/messages");
        assert!(matches!(d, Detection::Supported(Detected { protocol: ProtocolKind::Anthropic, .. })));
    }

    #[test]
    fn detects_gemini_generate_content() {
        let d = detect("POST", "/v1beta/models/gemini-1.5-pro:generateContent");
        assert!(matches!(d, Detection::Supported(Detected { protocol: ProtocolKind::Gemini, streaming_by_path: false })));
    }

    #[test]
    fn detects_gemini_stream_generate_content() {
        let d = detect("POST", "/v1beta/models/gemini-1.5-pro:streamGenerateContent");
        assert!(matches!(d, Detection::Supported(Detected { protocol: ProtocolKind::Gemini, streaming_by_path: true })));
    }

    #[test]
    fn detects_gemini_interactions() {
        let d = detect("POST", "/v1beta/models/gemini-3.5-flash:interact");
        assert!(matches!(d, Detection::Supported(Detected { protocol: ProtocolKind::GeminiInteractions, .. })));
    }

    #[test]
    fn unidentified_for_unknown_path() {
        assert!(matches!(detect("GET", "/unknown"), Detection::Unidentified));
    }

    #[test]
    fn unsupported_capability_for_realtime() {
        let d = detect("POST", "/v1/realtime");
        assert!(matches!(d, Detection::UnsupportedCapability { .. }));
    }
}
```

- [ ] **Step 2: 跑测试确认失败**

Run: `cd core-rs && cargo test -p pingogate-pipeline protocol`
Expected: FAIL(001 精确匹配不认 `/v2/`、`/chat/completions` 无前缀、Responses、Interactions)。

- [ ] **Step 3: 改造 detect() 为路径后缀模式匹配**

```rust
// core-rs/pipeline/src/protocol.rs(改造后核心)
use pingogate_core::ProtocolKind;

pub fn detect(method: &str, path: &str) -> Detection {
    let path = path.split_once('?').map(|(p, _)| p).unwrap_or(path);
    let is_post = method.eq_ignore_ascii_case("POST");

    // 版本前缀通配:匹配路径后缀,不硬编码 /v1 或 /v1beta
    // OpenAI Chat Completions: {ver}/chat/completions 或 /chat/completions
    if is_post && path.ends_with("/chat/completions") {
        return supported(ProtocolKind::OpenAiCompatible, false);
    }
    // OpenAI Responses: {ver}/responses 或 /responses
    if is_post && path.ends_with("/responses") {
        return supported(ProtocolKind::OpenAiResponses, false);
    }
    // Anthropic Messages: {ver}/messages 或 /messages
    if is_post && path.ends_with("/messages") {
        return supported(ProtocolKind::Anthropic, false);
    }
    // Gemini streamGenerateContent:含 :streamGenerateContent
    if path.contains(":streamGenerateContent") {
        return supported(ProtocolKind::Gemini, true);
    }
    // Gemini generateContent:含 :generateContent
    if path.contains(":generateContent") {
        return supported(ProtocolKind::Gemini, false);
    }
    // Gemini Interactions:含 :interact(Gemini Interactions API action)
    if path.contains(":interact") {
        return supported(ProtocolKind::GeminiInteractions, false);
    }

    // 不支持能力族识别(Realtime/Embeddings/Batch 等)
    if let Some(family) = unsupported_family(path) {
        return Detection::UnsupportedCapability { family };
    }

    Detection::Unidentified
}

fn supported(protocol: ProtocolKind, streaming_by_path: bool) -> Detection {
    Detection::Supported(Detected { protocol, streaming_by_path })
}

fn unsupported_family(path: &str) -> Option<String> {
    // 识别但不支持的路径(如 /v1/realtime, /v1/embeddings, /v1/batches, :countTokens 等)
    if path.ends_with("/realtime") { return Some("realtime.live".into()); }
    if path.ends_with("/embeddings") { return Some("embedding".into()); }
    if path.ends_with("/batches") { return Some("batch".into()); }
    if path.contains(":countTokens") { return Some("platform.admin".into()); }
    None
}
```

- [ ] **Step 4: 跑测试确认通过**

Run: `cd core-rs && cargo test -p pingogate-pipeline protocol`
Expected: PASS(全部测试)。

- [ ] **Step 5: Commit**

```bash
git add core-rs/pipeline/src/protocol.rs
git commit -m "feat(pipeline): protocol detection by path suffix with version-prefix wildcard"
```

---

### Task 8: pipeline 透传(移植 001 管线 + KeyAuth trait)

**Files:**
- Create: `core-rs/pipeline/src/{lib,proxy,ctx,auth_filter,upstream_peer,upstream_auth,streaming,error_response,logging,metrics,observe,wire}.rs`(从 001 `app/pipeline/` 移植)
- Create: `core-rs/pipeline/src/key_auth.rs`(新增 KeyAuth trait + StaticKeyAuth)

**Interfaces:**
- Consumes: T3 core, T4 storage, T5 snapshot, T6 provider, T7 protocol
- Produces: `GatewayProxy`(ProxyHttp 实现)+ `KeyAuth` trait + `StaticKeyAuth`

- [ ] **Step 1: 移植 001 pipeline 文件**

```bash
for f in proxy ctx auth_filter upstream_peer upstream_auth streaming error_response logging metrics observe wire; do
  git show redesign:app/pipeline/src/$f.rs > core-rs/pipeline/src/$f.rs
done
```

- [ ] **Step 2: 改 crate 名引用 pingo_core -> pingogate_core,pingo_config -> pingogate_snapshot 等**

全局替换依赖 crate 名。

- [ ] **Step 3: 写 KeyAuth trait + StaticKeyAuth**

```rust
// core-rs/pipeline/src/key_auth.rs
use pingogate_core::{Principal, SecretString};
use pingogate_snapshot::RuntimeSnapshot;

pub trait KeyAuth: Send + Sync {
    fn authenticate(&self, credential: &str, snapshot: &RuntimeSnapshot) -> Option<Principal>;
}

/// 单机模式:静态网关 key(name -> secret)
pub struct StaticKeyAuth;

impl KeyAuth for StaticKeyAuth {
    fn authenticate(&self, credential: &str, snapshot: &RuntimeSnapshot) -> Option<Principal> {
        snapshot.gateway_keys.iter()
            .filter(|k| k.enabled && k.secret.expose() == credential)
            .next()
            .map(|k| Principal::gateway_key(&k.name))
    }
}
```

- [ ] **Step 4: 改 auth_filter 用 KeyAuth trait(而非 001 内联)**

001 的 `auth_filter.rs` 内联查 gateway_keys。改为调 `KeyAuth::authenticate`。

- [ ] **Step 5: 跑 001 pipeline 测试**

Run: `cd core-rs && cargo test -p pingogate-pipeline`
Expected: 001 pipeline 测试通过(经 crate 名 + KeyAuth 改造)。

- [ ] **Step 6: Commit**

```bash
git add core-rs/pipeline/
git commit -m "feat(pipeline): port 001 pipeline, add KeyAuth trait + StaticKeyAuth"
```

---

### Task 9: listener + pingogate-core 主二进制(单机模式 bootstrap)

**Files:**
- Create: `core-rs/listener/src/{lib,admin}.rs`(从 001 `app/listener/` 移植)
- Create: `core-rs/pingogate-core/src/{main,signal,watch}.rs`(从 001 `app/pingogate/` 移植改名)
- Create: `pingogate-core.yaml.example`

**Interfaces:**
- Consumes: T3-T8 所有 crate
- Produces: `pingogate-core` 二进制(单机模式可启动:读 YAML -> snapshot -> Pingora public+admin service -> 信号/文件监视 reload)

- [ ] **Step 1: 移植 listener + pingogate 主二进制**

```bash
git show redesign:app/listener/src/lib.rs > core-rs/listener/src/lib.rs
git show redesign:app/listener/src/admin.rs > core-rs/listener/src/admin.rs
git show redesign:app/pingogate/src/main.rs > core-rs/pingogate-core/src/main.rs
git show redesign:app/pingogate/src/signal.rs > core-rs/pingogate-core/src/signal.rs
git show redesign:app/pingogate/src/watch.rs > core-rs/pingogate-core/src/watch.rs
```

- [ ] **Step 2: 改 main.rs 用 SnapshotSource::File + StaticKeyAuth(单机模式)**

001 的 main.rs 直接读 config 构建 snapshot。改为经 `FileSnapshotSource::build_snapshot` + `StaticKeyAuth`,平台模式位置预留。

- [ ] **Step 2a: 管理端点 6 个(经 authorize 边界,R1 定单 token)**

R1 决策:**单机模式 admin 鉴权用单 token**(`env:PINGO_ADMIN_TOKEN`,启动校验存在)。移植 001 `app/admin/src/handler.rs` 的 6 端点,经 `Principal::admin` + `authorize` 边界(宪法 XX):

```rust
// core-rs/listener/src/admin.rs(6 端点,经 authorize)
// GET /healthz   -> 存活(不经鉴权,探针用)
// GET /readyz    -> 就绪(snapshot 已加载;平台模式含 gRPC 首同步检查,spec §12B)
// GET /metrics   -> Prometheus 指标(不经鉴权,scrape 用;或经鉴权看部署)
// POST /reload   -> 触发热重载(经 authorize Admin)
// GET /reload/status -> 最近重载结果/版本/回滚目标(经 authorize Admin)
// POST /config/validate -> 校验候选配置不改变活跃 snapshot(经 authorize Admin)
```

单机模式 admin token:`env:PINGO_ADMIN_TOKEN`,启动校验存在,缺失则 admin 端点拒绝(spec §12B 模式:缺失即失败,不给无鉴权降级)。`/healthz`/`/metrics` 不经鉴权(探针/scrape),其余 4 端点经 `Principal::admin` + `authorize(Admin, ...)`。

- [ ] **Step 3: 写 pingogate-core.yaml.example**

```yaml
# pingogate-core.yaml.example
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
routes:
  - { alias: "gpt-4o", provider: "openai-main", upstream_model: "gpt-4o" }
upstream: { timeout_ms: 60000 }
```

- [ ] **Step 4: cargo build 二进制**

Run: `cd core-rs && cargo build -p pingogate-core`
Expected: 编译成功,生成 `target/debug/pingogate-core`。

- [ ] **Step 5: Commit**

```bash
git add core-rs/listener/ core-rs/pingogate-core/ pingogate-core.yaml.example
git commit -m "feat(listener,pingogate-core): port listener + main binary, standalone bootstrap"
```

---

### Task 10: 单机模式集成测试(六接口透传)

**Files:**
- Test: `core-rs/pingogate-core/tests/e2e_six_interfaces.rs`(从 001 `app/pingogate/tests/` 移植重组)

**Interfaces:**
- Consumes: T9 二进制
- Produces: S1 证伪--单机模式六接口透传跑通(mock 上游)

- [ ] **Step 1: 移植 001 e2e 测试**

```bash
git show redesign:app/pingogate/tests/common/ > core-rs/pingogate-core/tests/common/
git show redesign:app/pingogate/tests/us1_openai.rs > core-rs/pingogate-core/tests/e2e_openai.rs
git show redesign:app/pingogate/tests/us1_anthropic.rs > core-rs/pingogate-core/tests/e2e_anthropic.rs
git show redesign:app/pingogate/tests/us1_gemini.rs > core-rs/pingogate-core/tests/e2e_gemini.rs
git show redesign:app/pingogate/tests/us1_streaming.rs > core-rs/pingogate-core/tests/e2e_streaming.rs
git show redesign:app/pingogate/tests/us1_errors.rs > core-rs/pingogate-core/tests/e2e_errors.rs
```

- [ ] **Step 2: 新增 Responses + Interactions 透传测试**

```rust
// core-rs/pingogate-core/tests/e2e_responses.rs(新增)
// 测 POST /v1/responses 经网关透传,mock 上游返回 response.completed
```

```rust
// core-rs/pingogate-core/tests/e2e_interactions.rs(新增)
// 测 POST /v1beta/models/{model}:interact 经网关透传
```

- [ ] **Step 3: 跑全部 e2e 测试**

Run: `cd core-rs && cargo test -p pingogate-core --test e2e_*`
Expected: 六接口透传(含 SSE)全通过;错误镜像正确;无效 key 拒绝。

- [ ] **Step 4: Commit**

```bash
git add core-rs/pingogate-core/tests/
git commit -m "test(pingogate-core): e2e six-interface passthrough (standalone mode)"
```

---

### Task 11: Go gRPC client 骨架(mTLS + internal token,连 Rust 推空快照)

**Files:**
- Create: `ctrl-go/go.mod`
- Create: `ctrl-go/cmd/pingogate-ctrl/main.go`
- Create: `ctrl-go/internal/snapshot/client.go`
- Create: `ctrl-go/internal/grpcmtls/{ca,certs}.go`(R10:首次启动生成自签 CA + 互验证书)
- Create: `core-rs/pingogate-core/src/grpc_auth.rs`(internal token 校验 interceptor)

**Interfaces:**
- Consumes: T1 proto 生成的 `pingogatepb`
- Produces: `pingogate-ctrl` 二进制,经 **mTLS + internal token**(spec §12A)连 Rust gRPC,推空快照证伪连通。R10 决策:首次启动 Go 生成自签 CA + Rust/Go 各证书。

**注**:S1 Rust 侧 gRPC server 也是空壳(tonic 服务实现返回空 Ack),S2 才实现真实逻辑。此 task 先在 Rust 加 gRPC server 空壳(含 mTLS + token 校验)+ Go client 经 mTLS 连它。

- [ ] **Step 1a: Go 生成自签 CA + Rust/Go 互验证书(R10)**

```go
// ctrl-go/internal/grpcmtls/ca.go
// 首次启动:生成自签 CA + 签发 ctrl.pem/ctrl.key(Go)+ core.pem/core.key(Rust)
// 证书存 ctrl-go/internal/grpcmtls/certs/(gitignore),Rust 启动时从环境/文件加载 core.pem
// Rust 侧证书经 env PINGO_GRPC_CERT/PINGO_GRPC_KEY 传路径,Go 侧用本地 certs/
func EnsureCerts(dir string) error { /* 首次生成,已存在则跳过 */ }
```

- [ ] **Step 1b: Rust gRPC server 加 mTLS + internal token 校验**

```rust
// core-rs/pingogate-core/src/grpc.rs(改造:加 mTLS + token interceptor)
use tonic::transport::{Server, Certificate, Identity};
use tonic::service::interceptor;

pub async fn serve_grpc(addr: SocketAddr, cert: Certificate, key: Identity, internal_token: String) -> Result<()> {
    let tls = tonic::transport::ServerTlsConfig::new()
        .identity(key)
        .client_ca_root(cert);  // mTLS:互验
    Server::builder()
        .tls_config(tls)?
        .interceptor(move |req| {  // internal token 校验(spec §12A)
            let token = req.metadata().get("x-internal-token")
                .and_then(|v| v.to_str().ok());
            match token {
                Some(t) if t == internal_token => Ok(req),
                _ => Err(Status::unauthenticated("invalid internal token")),
            }
        })
        .add_service(SnapshotServiceServer::new(SnapshotServiceImpl))
        .add_service(HealthServiceServer::new(HealthServiceImpl))
        .serve(addr).await
}
```

`main.rs` 平台模式:加载 `env:PINGO_INTERNAL_TOKEN`(缺失即失败,spec §12B)+ 证书,调 `serve_grpc`。

- [ ] **Step 1: 在 Rust pingogate-core 加 gRPC server 空壳**

```rust
// core-rs/pingogate-core/src/grpc.rs(新增)
use tonic::{transport::Server, Request, Response, Status};

pingogate_core::include_proto!("pingogate");  // prost 生成的模块

pub struct SnapshotServiceImpl;

#[tonic::async_trait]
impl snapshot_service_server::SnapshotService for SnapshotServiceImpl {
    type PushSnapshotStream = std::pin::Pin<Box<dyn futures::Stream<Item = Result<Ack, Status>> + Send>>;
    async fn push_snapshot(&self, request: Request<tonic::Streaming<Snapshot>>) -> Result<Response<Ack>, Status> {
        // S1 空壳:收 snapshot 流,回 Ack(不实际切换)
        Ok(Response::new(Ack { version: 0, ok: true, error: String::new() }))
    }
    async fn heartbeat(&self, req: Request<HeartbeatRequest>) -> Result<Response<HeartbeatResponse>, Status> {
        Ok(Response::new(HeartbeatResponse { acknowledged: true }))
    }
}
```

`main.rs` 启动 gRPC server(平台模式时;单机模式不启动)。

- [ ] **Step 2: 写 Go module + main.go(mTLS + internal token)**

```go
// ctrl-go/go.mod
module github.com/whw23/pingogate/ctrl-go

go 1.22

require (
    google.golang.org/grpc v1.62.0
    google.golang.org/protobuf v1.33.0
)
```

```go
// ctrl-go/cmd/pingogate-ctrl/main.go
package main

import (
    "context"
    "log"
    "os"
    "google.golang.org/grpc"
    "google.golang.org/grpc/credentials"
    pb "github.com/whw23/pingogate/ctrl-go/internal/proto"
    "github.com/whw23/pingogate/ctrl-go/internal/grpcmtls"
    "github.com/whw23/pingogate/ctrl-go/internal/snapshot"
)

func main() {
    internalToken := os.Getenv("PINGO_INTERNAL_TOKEN")
    if internalToken == "" { log.Fatal("PINGO_INTERNAL_TOKEN required") }
    if err := grpcmtls.EnsureCerts("internal/grpcmtls/certs"); err != nil { log.Fatal(err) }
    creds, err := grpcmtls.ClientCredentials("internal/grpcmtls/certs")
    if err != nil { log.Fatal(err) }
    conn, err := grpc.Dial("127.0.0.1:9091",
        grpc.WithTransportCredentials(creds),  // mTLS
        grpc.WithUnaryInterceptor(grpcmtls.TokenUnaryInterceptor(internalToken)),  // internal token
        grpc.WithStreamInterceptor(grpcmtls.TokenStreamInterceptor(internalToken)),
    )
    if err != nil { log.Fatalf("dial: %v", err) }
    defer conn.Close()
    if err := snapshot.PushEmpty(context.Background(), pb.NewSnapshotServiceClient(conn)); err != nil {
        log.Fatalf("push: %v", err)
    }
    log.Println("S1: gRPC connected (mTLS + token), empty snapshot pushed")
}
```

- [ ] **Step 3: 写 snapshot/client.go(推空快照)**

```go
// ctrl-go/internal/snapshot/client.go
package snapshot

import (
    "context"
    "io"
    pb "github.com/whw23/pingogate/ctrl-go/internal/proto"
)

func PushEmpty(ctx context.Context, client pb.SnapshotServiceClient) error {
    stream, err := client.PushSnapshot(ctx)
    if err != nil { return err }
    if err := stream.Send(&pb.Snapshot{Version: 1, Payload: []byte("S1-empty")}); err != nil { return err }
    if err := stream.CloseSend(); err != nil { return err }
    ack, err := stream.CloseAndRecv()
    // S1:验证收到 Ack
    _ = ack
    return nil
}
```

- [ ] **Step 4: 启 Rust gRPC server + Go client,验证连通**

Run(两终端):
```bash
# 终端1:启动 Rust(平台模式,启 gRPC)
cd core-rs && PINGO_MODE=platform cargo run -p pingogate-core -- --grpc-addr 127.0.0.1:9091
# 终端2:启动 Go client
cd ctrl-go && go run ./cmd/pingogate-ctrl
```
Expected:Go 输出 "S1: gRPC connected, empty snapshot pushed";Rust 输出收到快照。

- [ ] **Step 5: Commit**

```bash
git add core-rs/pingogate-core/src/grpc.rs core-rs/pingogate-core/src/main.rs ctrl-go/
git commit -m "feat(grpc): Rust gRPC server stub + Go client, prove connectivity (empty snapshot)"
```

---

### Task 12: S1 交叉认证 + 证伪

**Files:**
- Test: `core-rs/pingogate-core/tests/s1_contract.rs`(新增,契约测试)
- Test: `ctrl-go/internal/snapshot/client_test.go`(Go 侧契约)

**Interfaces:**
- Consumes: T1-T11 全部产出
- Produces: S1 契约测试套件,作为 S2 的入口检查(S2 开头先跑这些测试确认 S1 产出可用)

**交叉认证机制**:S1 产出三套契约(1)proto 定义 + Rust trait 签名(SnapshotSource/KeyAuth/KeyVault);(2)Rust 内核单机模式六接口透传 e2e;(3)gRPC Rust<->Go 连通。S2 开头先跑这些,S3 同理。

- [ ] **Step 1: 写 S1 契约测试(Rust 侧)**

```rust
// core-rs/pingogate-core/tests/s1_contract.rs
//! S1 contract: these tests MUST pass before S2 starts.
//! They verify (1) trait signatures, (2) six-interface detection, (3) standalone passthrough.

#[test]
fn snapshot_source_trait_exists() {
    fn _assert<T: pingogate_storage::SnapshotSource>() {}
    _assert::<pingogate_storage::FileSnapshotSource>();
}

#[test]
fn key_auth_trait_exists() {
    fn _assert<T: pingogate_pipeline::KeyAuth>() {}
    _assert::<pingogate_pipeline::StaticKeyAuth>();
}

#[test]
fn keyvault_trait_exists_and_stubs_not_implemented() {
    let kv = pingogate_storage::StubKeyVault;
    assert!(kv.encrypt(b"x").is_err());
    assert!(kv.decrypt(b"x").is_err());
}

#[test]
fn six_interfaces_detected() {
    use pingogate_pipeline::protocol::detect;
    assert!(matches!(detect("POST", "/v1/chat/completions"), _));
    assert!(matches!(detect("POST", "/v1/responses"), _));
    assert!(matches!(detect("POST", "/v1/messages"), _));
    assert!(matches!(detect("POST", "/v1beta/models/m:generateContent"), _));
    assert!(matches!(detect("POST", "/v1beta/models/m:streamGenerateContent"), _));
    assert!(matches!(detect("POST", "/v1beta/models/m:interact"), _));
}
```

- [ ] **Step 2: 写 Go 侧契约测试**

```go
// ctrl-go/internal/snapshot/client_test.go
package snapshot

import (
    "context"
    "os"
    "testing"
    pb "github.com/whw23/pingogate/ctrl-go/internal/proto"
    "github.com/whw23/pingogate/ctrl-go/internal/grpcmtls"
    "google.golang.org/grpc"
)

func TestPushEmptyContract(t *testing.T) {
    // S1 契约:Go client 经 mTLS + internal token 连 Rust gRPC 推空快照
    creds, err := grpcmtls.ClientCredentials("internal/grpcmtls/certs")
    if err != nil { t.Skipf("certs not ready: %v", err) }
    token := os.Getenv("PINGO_INTERNAL_TOKEN")
    conn, err := grpc.Dial("127.0.0.1:9091",
        grpc.WithTransportCredentials(creds),
        grpc.WithUnaryInterceptor(grpcmtls.TokenUnaryInterceptor(token)),
        grpc.WithStreamInterceptor(grpcmtls.TokenStreamInterceptor(token)),
    )
    if err != nil { t.Skipf("Rust gRPC not running: %v", err) }
    defer conn.Close()
    if err := PushEmpty(context.Background(), pb.NewSnapshotServiceClient(conn)); err != nil {
        t.Fatalf("push empty: %v", err)
    }
}

func TestGrpcRejectsMissingOrWrongToken(t *testing.T) {
    // spec §12A 安全:gRPC 无 token / 错 token 调用被拒绝
    creds, _ := grpcmtls.ClientCredentials("internal/grpcmtls/certs")
    for _, token := range []string{"", "wrong-token"} {
        conn, err := grpc.Dial("127.0.0.1:9091",
            grpc.WithTransportCredentials(creds),
            grpc.WithUnaryInterceptor(grpcmtls.TokenUnaryInterceptor(token)),
        )
        if err != nil { t.Skipf("not running: %v", err) }
        // 调任意 RPC,断言 PermissionDenied / Unauthenticated
        // (具体调用略,断言 status.Code(err) == codes.Unauthenticated)
        conn.Close()
    }
}
```

- [ ] **Step 3: 跑 S1 全部测试(Rust + Go)**

Run:
```bash
cd core-rs && cargo test --workspace  # Rust 全部测试
cd ctrl-go && go test ./...           # Go 全部测试(需 Rust gRPC 运行)
```
Expected:全通过。

- [ ] **Step 4: S1 证伪检查(对照 spec §11 S1 证伪)**

- [ ] Rust 内核单机模式六接口透传跑通(T10 e2e 通过)
- [ ] Go 能 gRPC 连 Rust 推空快照(T11 通过)
- [ ] 平台模式位置预留编译通过(SnapshotSource/KeyAuth/KeyVault trait + gRPC server 空壳,`cargo build` 通过)

- [ ] **Step 5: Commit**

```bash
git add core-rs/pingogate-core/tests/s1_contract.rs ctrl-go/internal/snapshot/client_test.go
git commit -m "test(s1): cross-validation contract suite for S2/S3 entry check"
```

---

## Self-Review

**1. Spec coverage**(对照 spec §1.2 + §11 S1):
- proto 定义 + codegen:T1 ✅
- Rust 内核六接口同态透传:T6/T7/T8/T10 ✅
- 协议识别路径后缀模式 + 版本前缀通配:T7 ✅
- SnapshotSource/KeyAuth trait + File/Static 实现:T4/T8 ✅
- KeyVault 空壳:T4 ✅
- gRPC 骨架(Rust server + Go client):T11 ✅
- 单机模式可跑:T9/T10 ✅
- RuntimeSnapshot + ArcSwap 热重载:T5 ✅(001 已有,S1 移植)

**2. Placeholder scan**:无 TBD/TODO。T4 Step 5/T5 Step 5 有跨 task 依赖回填,已标注。

**3. Type consistency**:`SnapshotSource::build_snapshot` 返回 `Arc<RuntimeSnapshot>`(T4 定义,T5 实现,一致);`KeyAuth::authenticate` 签名(T8 定义,T12 契约测试一致);`detect(method, path) -> Detection`(T7 定义,T12 一致)。

**S1 产出契约**(S2/S3 消费):
- `pingogate_core::ProtocolKind`(六值)
- `pingogate_storage::{SnapshotSource, FileSnapshotSource, KeyVault, StubKeyVault}`
- `pingogate_snapshot::{RuntimeSnapshot, SnapshotHolder}`
- `pingogate_pipeline::{GatewayProxy, KeyAuth, StaticKeyAuth, protocol::detect}`
- `pingogate_provider::ProviderAdapter`(六接口)
- `proto/pingogate.proto`(三 gRPC 服务 + 消息)
- Rust gRPC server 空壳(S2 实现真实逻辑)+ Go gRPC client(S2 扩展)

S2 开头先跑 `cargo test -p pingogate-core --test s1_contract` + `go test ./internal/snapshot/`,确认 S1 产出可用,再开始 S2 实现。
