# S3 1Password 可见性 + 虚拟 key + 闭环 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 实现 1Password 可见性(Go+Rust 双防线)、虚拟 key 签发与平台鉴权闭环、usage 提取与落库,达成 M0+M1 完整闭环:用户用虚拟 key 经 Rust 透传调上游,用自己的 BYOK key(KeyVault 解密注入),admin 不可见明文,usage 提取落库。

**Architecture:** S3 在 S2 基础上,加 1Password 双防线(Rust owner 映射拦截,调研结论)、虚拟 key(Go 签发 + Rust VirtualKeyAuth 热路径校验)、usage(Rust inline 事件驱动解析 + Go 估算落库,调研结论)、OpenAI include_usage 注入(transform 最小实现)。S3 结束时 M0+M1 完整闭环可证伪。

**Tech Stack:** S1/S2 全部 + tiktoken-go(Go 估算,无 usage 时);Rust transform crate(include_usage 注入)。

## Global Constraints

- (继承 S1/S2)
- 1Password 双防线(spec §12A 调研结论):Go created_by 校验(第一道)+ Rust owner 映射拦截(第二道,粗粒度,非业务语义)+ 审计(第三道)。Rust 持快照 `key_id -> owner_user_id` 映射。
- usage 提取 Rust inline 事件驱动解析(spec 调研结论):`upstream_response_body_filter` 内 LineBuffer + 标记扫描 + 只解析 usage 行,O(1) 内存;不用旁路 observer(observer 留未来 body materialize)。
- KeyVault 热路径每请求解密(S1 调研结论,~0.2µs),不缓存,SecretString 存 CTX 请求结束清零。
- 虚拟 key 哈希存(bcrypt 或 SHA-256),明文返用户一次。
- 并发配额计数 Rust 内存(热路径零 DB),配额配置 Go 推快照。
- OpenAI Chat Completions 流式注入 `stream_options.include_usage:true`(transform 最小实现,R5)。

## File Structure

**Rust 侧**:
- Modify `core-rs/keyvault/src/grpc_service.rs`:加 owner 映射校验(双防线第二道)。
- Modify `core-rs/snapshot/src/snapshot.rs`:RuntimeSnapshot 加 `virtual_keys`(哈希 -> {owner, scope, provider_key_ref})+ `key_owners`(key_id -> owner_user_id 映射)。
- Create `core-rs/pipeline/src/virtual_key_auth.rs`:VirtualKeyAuth(实现 KeyAuth trait)。
- Modify `core-rs/pipeline/src/upstream_auth.rs`:平台模式调 KeyVault 解密 provider key(每请求 ~0.2µs)注入上游。
- Create `core-rs/pipeline/src/usage_extractor.rs`:inline 事件驱动 usage 提取(LineBuffer + 标记扫描)。
- Modify `core-rs/transform/src/lib.rs`:Transform trait 最小实现(OpenAI include_usage 注入)。
- Modify `core-rs/pingogate-core/src/grpc.rs`:UsageService gRPC 服务端骨架(Rust 推 Go,实际 Go 实现 service);或 Rust 作 client 推 Go UsageService。

**Go 侧**:
- Create `ctrl-go/internal/vkey/{virtual_key,handler}.go`:VirtualKey 签发/撤销/列表 + 哈希存储。
- Create `ctrl-go/internal/usage/{server,estimator,store}.go`:UsageService gRPC 服务端(收 Rust 事件)+ tiktoken-go 估算 + 落库。
- Modify `ctrl-go/internal/keymgmt/handler.go`:1Password 可见性(created_by 校验 + 末位脱敏 + 解密查明文)。
- Modify `ctrl-go/internal/snapshot/builder.go`:快照加 virtual_keys + key_owners 映射。

**DB 迁移**:
- Create `ctrl-go/internal/storage/migrations/003_virtual_keys.up.sql`
- Create `ctrl-go/internal/storage/migrations/004_usage.up.sql`

**proto**:
- Modify `proto/pingogate.proto`:VirtualKeyEntry 填充;UsageEvent 结构化。

## 任务分解

S3 分 11 个任务。依赖:T23(S2 入口检查)-> T24(proto 扩展)-> T25(Rust snapshot 加 virtual_keys + key_owners)-> T26(Rust VirtualKeyAuth + 平台热路径解密)-> T27(Rust usage inline 提取)-> T28(Rust include_usage transform)-> T29(Go vkey 签发)-> T30(Go 1Password 可见性 + Rust owner 双防线)-> T31(Go usage 估算落库)-> T32(端到端闭环测试)-> T33(S3 契约 + M0+M1 证伪)。

---

### Task 23: S2 入口检查(交叉认证)

**Interfaces:**
- Consumes: S2 全部产出
- Produces: 确认 S2 产出可用

- [ ] **Step 1: 跑 S2 Go 契约测试**

Run: `cd ctrl-go && go test ./internal/keymgmt/ -run TestS2Contract`
Expected: PASS(用户 CRUD + key 加密存 + 列表返末位)。

- [ ] **Step 2: 跑 S2 Rust 契约测试**

Run: `cd core-rs && cargo test -p pingogate-core --test s2_contract`
Expected: PASS(KeyVault AES-GCM + GrpcSnapshotSource + 平台热路径解密)。

- [ ] **Step 3: 失败则停止 S3,回 S2 修复**

无 commit。

---

### Task 24: proto 扩展(VirtualKeyEntry + UsageEvent 结构化)

**Files:**
- Modify: `proto/pingogate.proto`

- [ ] **Step 1: 填充 VirtualKeyEntry + UsageEvent**

```protobuf
// proto/pingogate.proto(替换 S2 占位)
message VirtualKeyEntry {
  string id = 1;
  string token_hash = 2;        // bcrypt 或 SHA-256 hash
  string owner_user_id = 3;
  string provider_key_id = 4;   // 引用 encrypted_keys 的 key_id
  repeated string allowed_models = 5;
  repeated string allowed_providers = 6;
  int64 expires_at = 7;         // unix ts,0 = 不过期
  int32 max_concurrency = 8;    // 并发配额,0 = 不限
  bool enabled = 9;
}

message UsageEvent {
  uint64 version = 1;
  string virtual_key_id = 2;
  string owner_user_id = 3;
  string provider = 4;
  string model = 5;
  int64 input_tokens = 6;
  int64 output_tokens = 7;
  int64 reasoning_tokens = 8;
  int64 cache_read_tokens = 9;
  int64 cache_write_tokens = 10;
  bool success = 11;
  int64 latency_ms = 12;
  bool needs_estimate = 13;     // true = 无 usage,Go 估算(Rust 推 body)
  bytes body_ref = 14;          // needs_estimate=true 时的 body 引用/内容
}
```

- [ ] **Step 2: 重新生成 codegen**

Run: `protoc ...(同 S1/S2)` + `cd core-rs && cargo build -p pingogate-core`
Expected: 编译通过。

- [ ] **Step 3: Commit**

```bash
git add proto/ ctrl-go/internal/proto/ core-rs/
git commit -m "feat(proto): fill VirtualKeyEntry + structured UsageEvent"
```

---

### Task 25: Rust snapshot 加 virtual_keys + key_owners 映射

**Files:**
- Modify: `core-rs/snapshot/src/snapshot.rs`

**Interfaces:**
- Consumes: T24 proto VirtualKeyEntry
- Produces: RuntimeSnapshot 含 `virtual_keys: Vec<VirtualKeyEntry>` + `key_owners: HashMap<String, String>`(key_id -> owner_user_id,双防线用)

- [ ] **Step 1: 扩展 RuntimeSnapshot**

```rust
// core-rs/snapshot/src/snapshot.rs
pub struct RuntimeSnapshot {
    pub version: u64,
    pub providers: Vec<ResolvedProvider>,
    pub routes: Vec<Route>,
    pub gateway_keys: Vec<GatewayKey>,         // 单机模式
    pub virtual_keys: Vec<VirtualKeyEntry>,     // 平台模式(S3)
    pub key_owners: std::collections::HashMap<String, String>,  // key_id -> owner_user_id(S3 双防线)
    pub encrypted_keys: std::collections::HashMap<String, Vec<u8>>,  // key_id -> 密文
    pub upstream: UpstreamConfig,
}

pub struct VirtualKeyEntry {
    pub id: String,
    pub token_hash: String,
    pub owner_user_id: String,
    pub provider_key_id: String,
    pub allowed_models: Vec<String>,
    pub allowed_providers: Vec<String>,
    pub expires_at: i64,
    pub max_concurrency: i32,
    pub enabled: bool,
}
```

- [ ] **Step 2: GrpcSnapshotSource 反序列化时填充 virtual_keys + key_owners**

T16 的 `apply_snapshot` 扩展:从 proto Snapshot 解析 virtual_keys + encrypted_keys,构建 key_owners 映射(encrypted_keys 的 owner_user_id)。

- [ ] **Step 3: 测试 snapshot 含 virtual_keys + key_owners**

```rust
#[test]
fn snapshot_has_key_owners_map() {
    // 构建含 encrypted_keys 的快照 -> key_owners 映射正确
}
```

- [ ] **Step 4: Commit**

```bash
git add core-rs/snapshot/src/snapshot.rs core-rs/storage/src/snapshot_source.rs
git commit -m "feat(snapshot): add virtual_keys + key_owners map for dual-defense"
```

---

### Task 26: Rust VirtualKeyAuth + 平台热路径解密

**Files:**
- Create: `core-rs/pipeline/src/virtual_key_auth.rs`
- Modify: `core-rs/pipeline/src/upstream_auth.rs`(平台模式调 KeyVault 解密)

**Interfaces:**
- Consumes: T25 snapshot(virtual_keys + encrypted_keys);T15 KeyVault;S1 KeyAuth trait
- Produces: `VirtualKeyAuth`(实现 KeyAuth)+ 平台热路径每请求 KeyVault 解密 provider key

- [ ] **Step 1: 写 VirtualKeyAuth(哈希比对 + scope 校验 + 并发配额)**

```rust
// core-rs/pipeline/src/virtual_key_auth.rs
use std::collections::HashMap;
use std::sync::atomic::{AtomicI64, Ordering};
use pingogate_core::Principal;
use pingogate_snapshot::RuntimeSnapshot;
use crate::KeyAuth;

pub struct VirtualKeyAuth {
    // 并发计数(内存,热路径零 DB)
    concurrency: HashMap<String, AtomicI64>,  // vkey_id -> in-flight count
}

impl KeyAuth for VirtualKeyAuth {
    fn authenticate(&self, credential: &str, snapshot: &RuntimeSnapshot) -> Option<Principal> {
        // 1. SHA-256(credential) -> 查 snapshot.virtual_keys.token_hash
        let hash = sha256(credential);
        let vk = snapshot.virtual_keys.iter()
            .find(|k| k.enabled && k.token_hash == hash && !expired(k))?;
        // 2. scope 校验(allowed_models/providers 在路由阶段做,此处只验 enabled + 过期)
        // 3. 并发配额(原子计数)
        if vk.max_concurrency > 0 {
            let count = self.concurrency.entry(vk.id.clone()).or_default();
            if count.load(Ordering::Relaxed) >= vk.max_concurrency as i64 {
                return None;  // 超并发配额
            }
            count.fetch_add(1, Ordering::Relaxed);
        }
        Some(Principal::virtual_key(&vk.id, &vk.owner_user_id))
    }
}

// 流结束 decrement 并发计数(在 response_body_filter end_of_stream 或 logging)
```

- [ ] **Step 2: 改 upstream_auth 平台模式调 KeyVault 解密**

```rust
// core-rs/pipeline/src/upstream_auth.rs(平台模式分支)
pub fn build_platform(
    method: AuthMethod,
    encrypted_key: &[u8],
    keyvault: &dyn KeyVault,
    anthropic_version: Option<&str>,
) -> Result<UpstreamAuth, KeyError> {
    // 每请求解密(~0.2µs,S1 调研结论可忽略)
    let plaintext = keyvault.decrypt(encrypted_key)?;  // SecretString
    // 按 method 注入
    match method {
        AuthMethod::Bearer => Ok(UpstreamAuth::BearerHeader(format!("Bearer {}", plaintext.expose()))),
        // ...
    }
    // plaintext 在此函数结束 drop,SecretString 清零
}
```

- [ ] **Step 3: 改 proxy.rs 平台模式用 VirtualKeyAuth + KeyVault 解密**

```rust
// core-rs/pipeline/src/proxy.rs
// request_filter:平台模式用 VirtualKeyAuth::authenticate(credential, &snapshot)
// upstream_request_filter:平台模式调 build_platform(method, encrypted_key, keyvault)
```

- [ ] **Step 4: 测试 VirtualKeyAuth + 平台热路径解密**

```rust
#[test]
fn virtual_key_auth_validates_hash_and_scope() {
    // 签发 vkey -> 哈希存 -> authenticate(明文) -> Some(Principal)
    // 错明文 -> None;过期 -> None;超并发 -> None
}

#[test]
fn platform_hot_path_decrypts_per_request() {
    // mock KeyVault -> 每请求解密一次 -> SecretString 清零
}
```

- [ ] **Step 5: Commit**

```bash
git add core-rs/pipeline/src/{virtual_key_auth.rs,upstream_auth.rs,proxy.rs}
git commit -m "feat(pipeline): VirtualKeyAuth + platform hot-path per-request KeyVault decrypt"
```

---

### Task 27: Rust usage inline 事件驱动提取

**Files:**
- Create: `core-rs/pipeline/src/usage_extractor.rs`
- Modify: `core-rs/pipeline/src/proxy.rs`(upstream_response_body_filter 调 usage 提取)

**Interfaces:**
- Consumes: T24 proto UsageEvent;Pingora `upstream_response_body_filter`
- Produces: inline 事件驱动 usage 提取(LineBuffer + 标记扫描 + 只解析 usage 行,O(1) 内存,调研结论),推 Go

- [ ] **Step 1: 写 LineBuffer(跨 chunk 重组 data: 行)**

```rust
// core-rs/pipeline/src/usage_extractor.rs
pub struct LineBuffer {
    buf: Vec<u8>,  // 残余半行,几 KB 上限
}

impl LineBuffer {
    pub fn push_chunk(&mut self, chunk: &[u8]) -> Vec<Vec<u8>> {
        // 按 \n 分行,返回完整行,保留残余
    }
}
```

- [ ] **Step 2: 写 usage 标记扫描 + 按 provider 归约**

```rust
pub struct UsageExtractor {
    line_buf: LineBuffer,
    protocol: ProtocolKind,
    input_tokens: i64,
    output_tokens: i64,
    // reasoning/cache 按接口
}

impl UsageExtractor {
    pub fn on_body_chunk(&mut self, body: &[u8], end_of_stream: bool) {
        let lines = self.line_buf.push_chunk(body);
        for line in lines {
            // 廉价字节扫描找 usage 标记(调研结论:不全量 JSON 解析)
            if !line_has_usage_marker(&line, self.protocol) { continue; }
            // 命中:只解析这一行
            self.extract_usage_from_line(&line);
        }
        if end_of_stream {
            // 终态对账:推 UsageEvent 给 Go
        }
    }
}

fn line_has_usage_marker(line: &[u8], protocol: ProtocolKind) -> bool {
    match protocol {
        ProtocolKind::OpenAiCompatible => memmem(line, b"\"usage\""),
        ProtocolKind::OpenAiResponses => memmem(line, b"response.completed") || memmem(line, b"\"usage\""),
        ProtocolKind::Anthropic => memmem(line, b"message_start") || memmem(line, b"message_delta"),
        ProtocolKind::Gemini | ProtocolKind::GeminiInteractions => memmem(line, b"usageMetadata"),
    }
}
```

- [ ] **Step 3: 接入 proxy.rs upstream_response_body_filter**

```rust
// core-rs/pipeline/src/proxy.rs
fn upstream_response_body_filter(
    &self, _session: &mut Session, body: &mut Option<Bytes>, end_of_stream: bool, ctx: &mut CTX,
) -> Result<Option<Duration>> {
    if let Some(b) = body {
        ctx.usage_extractor.on_body_chunk(&b, end_of_stream);
    }
    if end_of_stream {
        let event = ctx.usage_extractor.finalize(ctx.principal.clone());
        ctx.usage_sender.try_send(event).ok();  // 推 Go(异步 channel)
    }
    Ok(None)  // 透传不改 body
}
```

- [ ] **Step 4: 测试 usage 提取(各接口位置)**

```rust
#[test]
fn extracts_openai_chat_usage_from_last_chunk() {
    // 模拟 OpenAI Chat 流:多 chunk + 末块 usage -> 提取正确
}

#[test]
fn extracts_anthropic_usage_from_two_events() {
    // message_start(input) + message_delta(output) -> 合并
}

#[test]
fn extracts_gemini_usage_metadata() {
    // 末块 usageMetadata -> 提取
}

#[test]
fn o1_memory_no_full_buffer() {
    // 长流(1000 chunk)-> LineBuffer 不超几 KB
}
```

- [ ] **Step 5: Commit**

```bash
git add core-rs/pipeline/src/usage_extractor.rs core-rs/pipeline/src/proxy.rs
git commit -m "feat(pipeline): inline event-driven usage extraction (O(1) memory, no observer)"
```

---

### Task 28: Rust transform(OpenAI include_usage 注入)

**Files:**
- Modify: `core-rs/transform/src/lib.rs`

**Interfaces:**
- Consumes: S1 Transform trait
- Produces: Transform 最小实现--OpenAI Chat Completions 流式注入 `stream_options.include_usage:true`

- [ ] **Step 1: 写 include_usage 注入测试(失败)**

```rust
// core-rs/transform/src/lib.rs
#[test]
fn injects_include_usage_for_openai_chat_stream() {
    let body = br#"{"model":"gpt-4o","messages":[],"stream":true}"#;
    let transformed = inject_include_usage(body, ProtocolKind::OpenAiCompatible, true);
    let v: serde_json::Value = serde_json::from_slice(&transformed).unwrap();
    assert_eq!(v["stream_options"]["include_usage"], true);
}

#[test]
fn no_inject_for_non_stream() {
    let body = br#"{"model":"gpt-4o","messages":[]}"#;
    let transformed = inject_include_usage(body, ProtocolKind::OpenAiCompatible, false);
    assert_eq!(transformed, body);  // 不改
}
```

- [ ] **Step 2: 实现 inject_include_usage**

```rust
pub fn inject_include_usage(body: &[u8], protocol: ProtocolKind, streaming: bool) -> Vec<u8> {
    if !streaming || protocol != ProtocolKind::OpenAiCompatible { return body.to_vec(); }
    let mut v: serde_json::Value = serde_json::from_slice(body).ok()?;
    v["stream_options"]["include_usage"] = serde_json::Value::Bool(true);
    serde_json::to_vec(&v).ok().unwrap_or_else(|| body.to_vec())
}
```

- [ ] **Step 3: 接入 proxy.rs request_body_filter(OpenAI Chat 流式请求)**

```rust
// proxy.rs request_body_filter
if ctx.protocol == ProtocolKind::OpenAiCompatible && ctx.streaming {
    if let Some(body) = body {
        *body = Bytes::from(inject_include_usage(body, ctx.protocol, true));
    }
}
```

- [ ] **Step 4: 跑测试**

Run: `cd core-rs && cargo test -p pingogate-transform`
Expected: PASS。

- [ ] **Step 5: Commit**

```bash
git add core-rs/transform/src/lib.rs core-rs/pipeline/src/proxy.rs
git commit -m "feat(transform): inject stream_options.include_usage for OpenAI Chat streaming"
```

---

### Task 29: Go vkey(VirtualKey 签发/撤销/哈希存储)

**Files:**
- Create: `ctrl-go/internal/vkey/{virtual_key,handler}.go`
- Create: `ctrl-go/internal/storage/migrations/003_virtual_keys.up.sql`
- Modify: `ctrl-go/internal/snapshot/builder.go`(快照加 virtual_keys)

- [ ] **Step 1: 写 virtual_keys 迁移**

```sql
-- ctrl-go/internal/storage/migrations/003_virtual_keys.up.sql
CREATE TABLE virtual_keys (
    id TEXT PRIMARY KEY,
    token_hash TEXT NOT NULL,
    owner_user_id TEXT NOT NULL REFERENCES users(id),
    provider_key_id TEXT REFERENCES user_provider_keys(id),
    allowed_models TEXT,  -- JSON array
    allowed_providers TEXT,
    expires_at INTEGER NOT NULL DEFAULT 0,
    max_concurrency INTEGER NOT NULL DEFAULT 0,
    enabled INTEGER NOT NULL DEFAULT 1,
    created_at TEXT NOT NULL
);
```

- [ ] **Step 2: 写 VirtualKey 实体 + 签发**

```go
// ctrl-go/internal/vkey/virtual_key.go
type VirtualKey struct {
    ID              string   `db:"id" json:"id"`
    TokenHash       string   `db:"token_hash" json:"-"`
    OwnerUserID     string   `db:"owner_user_id" json:"owner_user_id"`
    ProviderKeyID   string   `db:"provider_key_id" json:"provider_key_id"`
    AllowedModels   []string `db:"allowed_models" json:"allowed_models"`
    AllowedProviders []string `db:"allowed_providers" json:"allowed_providers"`
    ExpiresAt       int64    `db:"expires_at" json:"expires_at"`
    MaxConcurrency  int32    `db:"max_concurrency" json:"max_concurrency"`
    Enabled         bool     `db:"enabled" json:"enabled"`
}

func (s *Store) Issue(ctx context.Context, ownerID, providerKeyID string, scope Scope) (string, *VirtualKey, error) {
    // 生成明文 token(返一次)+ SHA-256 哈希存
    // INSERT
    // 触发快照推送
    return plaintextToken, vk, nil
}
func (s *Store) Revoke(ctx context.Context, id string) error { ... }
func (s *Store) List(ctx context.Context, ownerID string) ([]VirtualKey, error) { ... }
```

- [ ] **Step 3: 写 handler(签发/撤销/列表)**

```go
// POST /api/virtual-keys -> 签发,返明文一次
// DELETE /api/virtual-keys/:id -> 撤销
// GET /api/virtual-keys -> 列表(不返明文)
```

- [ ] **Step 4: 快照构建器加 virtual_keys**

```go
// ctrl-go/internal/snapshot/builder.go(扩展)
func (b *Builder) Build(...) {
    // ... 读 virtual_keys 表 -> pb.VirtualKeyEntry
    // 触发推送(含 virtual_keys + key_owners)
}
```

- [ ] **Step 5: 测试**

```go
func TestIssueAndRevokeVirtualKey(t *testing.T) {
    // 签发 -> 返明文 -> DB 存哈希 -> 推送快照含 vkey
    // 撤销 -> enabled=0 -> 推送
}
```

- [ ] **Step 6: Commit**

```bash
git add ctrl-go/internal/vkey/ ctrl-go/internal/storage/migrations/003_virtual_keys.up.sql ctrl-go/internal/snapshot/builder.go
git commit -m "feat(vkey): VirtualKey issue/revoke/list + hash storage + snapshot push"
```

---

### Task 30: Go 1Password 可见性 + Rust owner 双防线

**Files:**
- Modify: `ctrl-go/internal/keymgmt/handler.go`(created_by 校验 + 末位 + 解密查明文)
- Modify: `core-rs/keyvault/src/grpc_service.rs`(owner 映射拦截,双防线第二道)

**Interfaces:**
- Consumes: T25 key_owners 映射;T26 KeyVault
- Produces: 1Password 完整双防线(Go created_by + Rust owner 拦截 + 审计)

- [ ] **Step 1: Rust KeyVault Decrypt 加 owner 拦截(双防线第二道)**

```rust
// core-rs/keyvault/src/grpc_service.rs(扩展 Decrypt)
async fn decrypt(&self, req: Request<DecryptRequest>) -> Result<Response<DecryptResponse>, Status> {
    let req = req.into_inner();
    if req.intent == "view_plaintext" {
        // 双防线第二道:校验 requester == owner(粗粒度,非业务语义,调研结论)
        let owner = self.key_owners.get(&req.key_id)
            .ok_or_else(|| Status::permission_denied("key_id not found"))?;
        if &req.requester_user_id != owner {
            tracing::warn!(key_id = %req.key_id, requester = %req.requester_user_id, "decrypt denied: not owner");
            return Err(Status::permission_denied("not key owner"));
        }
    }
    // hot_path_inject 不做 owner 校验(已在管线 virtual key scope 校验)
    let pt = self.kv.decrypt(&req.ciphertext)?;
    tracing::info!(key_id = %req.key_id, requester = %req.requester_user_id, intent = %req.intent, "keyvault decrypt");
    Ok(Response::new(DecryptResponse { plaintext: pt, error: String::new() }))
}
```

- [ ] **Step 2: Go keymgmt 查看明文 handler(created_by 校验,第一道)**

```go
// ctrl-go/internal/keymgmt/handler.go
// GET /api/provider-keys/:id/reveal
func (h *Handler) Reveal(w http.ResponseWriter, r *http.Request) {
    user := identity.UserFromContext(r.Context())
    keyID := chi.URLParam(r, "id")
    pk, err := h.store.GetByID(ctx, keyID)
    // 第一道:Go 校验 created_by == requester
    if pk.CreatedBy != user.ID {
        http.Error(w, "forbidden: not key creator", 403); return
    }
    // 调 Rust KeyVault.Decrypt(intent=view_plaintext)
    plaintext, err := h.kv.Decrypt(ctx, pk.EncryptedKey, user.ID, keyID, "view_plaintext")
    // 返明文一次
    json.NewEncoder(w).Encode(map[string]string{"plaintext": string(plaintext)})
}

// GET /api/provider-keys/:id(非 created_by) -> 只返末位
func (h *Handler) Get(w http.ResponseWriter, r *http.Request) {
    user := identity.UserFromContext(r.Context())
    pk, _ := h.store.GetByID(ctx, keyID)
    if pk.CreatedBy != user.ID {
        // 末位 + 元数据
        json.NewEncoder(w).Encode(map[string]any{"key_last4": last4(pk), ...})
        return
    }
    // created_by:可调 Reveal,或直接返明文(经 Rust 第二道)
}
```

- [ ] **Step 3: 写双防线测试**

```rust
// core-rs/keyvault/src/grpc_service.rs 测试
#[test]
fn decrypt_denied_for_non_owner_view_plaintext() {
    // owner = user-A;requester = user-B;intent=view_plaintext
    // -> Rust 拒绝(permission_denied)
}

#[test]
fn decrypt_allowed_for_owner_view_plaintext() {
    // requester == owner -> 解密成功
}

#[test]
fn hot_path_inject_skips_owner_check() {
    // intent=hot_path_inject -> 不校验 owner(管线已鉴权)
}
```

```go
// ctrl-go/internal/keymgmt/handler_test.go
func TestRevealDeniedForNonCreator(t *testing.T) {
    // user-B 调 user-A 创建的 key 的 Reveal -> 403(Go 第一道)
}
```

- [ ] **Step 4: 跑测试**

Run: `cd core-rs && cargo test -p pingogate-keyvault` + `cd ctrl-go && go test ./internal/keymgmt/`
Expected: PASS。

- [ ] **Step 5: Commit**

```bash
git add core-rs/keyvault/src/grpc_service.rs ctrl-go/internal/keymgmt/handler.go
git commit -m "feat(keymgmt,keyvault): 1Password dual-defense (Go created_by + Rust owner intercept)"
```

---

### Task 31: Go usage 估算落库(UsageService + tiktoken-go)

**Files:**
- Create: `ctrl-go/internal/usage/{server,estimator,store}.go`
- Create: `ctrl-go/internal/storage/migrations/004_usage.up.sql`
- Modify: `core-rs/pingogate-core/src/grpc.rs`(Rust 作 UsageService client 推 Go)

- [ ] **Step 1: 写 usage 表迁移**

```sql
-- ctrl-go/internal/storage/migrations/004_usage.up.sql
CREATE TABLE usage (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    virtual_key_id TEXT,
    owner_user_id TEXT NOT NULL,
    provider TEXT NOT NULL,
    model TEXT NOT NULL,
    input_tokens INTEGER NOT NULL DEFAULT 0,
    output_tokens INTEGER NOT NULL DEFAULT 0,
    reasoning_tokens INTEGER NOT NULL DEFAULT 0,
    cache_read_tokens INTEGER NOT NULL DEFAULT 0,
    cache_write_tokens INTEGER NOT NULL DEFAULT 0,
    success INTEGER NOT NULL,
    latency_ms INTEGER,
    estimated INTEGER NOT NULL DEFAULT 0,  -- 1 = Go 估算(无 usage)
    created_at TEXT NOT NULL
);
CREATE INDEX idx_usage_owner ON usage(owner_user_id);
CREATE INDEX idx_usage_created ON usage(created_at);
```

- [ ] **Step 2: 写 UsageService gRPC 服务端(收 Rust 事件)**

```go
// ctrl-go/internal/usage/server.go
type Server struct {
    store    *Store
    estimator *Estimator
    pb.UnimplementedUsageServiceServer
}

func (s *Server) ReportUsage(stream pb.UsageService_ReportUsageServer) error {
    for {
        event, err := stream.Recv()
        if err == io.EOF { return stream.SendAndClose(&pb.Ack{Ok: true}) }
        if err != nil { return err }
        // 若 needs_estimate -> 估算;否则直接落库
        if event.NeedsEstimate {
            input, output := s.estimator.Estimate(event.BodyRef, event.Provider, event.Model)
            event.InputTokens, event.OutputTokens = input, output
        }
        s.store.Insert(ctx, event)
    }
}
```

- [ ] **Step 3: 写 tiktoken-go 估算器**

```go
// ctrl-go/internal/usage/estimator.go
import "github.com/pkoukk/tkcorego-enc"  // tiktoken-go

type Estimator struct{ /* tokenizer 实例缓存 */ }

func (e *Estimator) Estimate(body []byte, provider, model string) (input, output int64) {
    // 按 provider/model 选 tokenizer(OpenAI cl100k/o200k 等)
    // 估算 input(请求)+ output(响应)
}
```

- [ ] **Step 4: Rust 推 usage 给 Go(UsageService client)**

```rust
// core-rs/pingogate-core/src/grpc.rs
// Rust 启动时连 Go UsageService,usage_extractor 推事件
let usage_client = UsageServiceClient::connect(go_addr).await?;
// proxy.rs usage_extractor.finalize() -> usage_client.report_usage(stream).await
```

- [ ] **Step 5: 测试(有 usage 直落 / 无 usage 估算 / 失败部分计量)**

```go
func TestUsageWithProviderUsage(t *testing.T) {
    // Rust 推有 input/output 的事件 -> 直接落库
}
func TestUsageEstimateWhenNoUsage(t *testing.T) {
    // needs_estimate=true -> Go tiktoken 估算 -> 落库
}
func TestUsagePartialOnFailure(t *testing.T) {
    // success=false + 部分 output -> 落库
}
```

- [ ] **Step 6: Commit**

```bash
git add ctrl-go/internal/usage/ ctrl-go/internal/storage/migrations/004_usage.up.sql core-rs/pingogate-core/src/grpc.rs
git commit -m "feat(usage): UsageService server + tiktoken estimate + persist"
```

---

### Task 32: 端到端闭环测试(M0+M1 完整证伪)

**Files:**
- Test: `core-rs/pingogate-core/tests/e2e_m0m1_closure.rs`(新增)

**Interfaces:**
- Consumes: T23-T31 全部
- Produces: M0+M1 完整闭环证伪

- [ ] **Step 1: 写端到端闭环测试**

```rust
// core-rs/pingogate-core/tests/e2e_m0m1_closure.rs
#[tokio::test]
async fn full_closure_user_calls_with_byok_key_admin_blind() {
    // 1. 启 Go + Rust 平台模式
    // 2. bootstrap admin -> 创建用户 user-A
    // 3. user-A 录入 provider key(经 Rust 加密存密文)
    // 4. user-A 签发虚拟 key(关联 BYOK provider key)
    // 5. user-A 用虚拟 key 经 Rust 透传调上游(mock)
    //    -> Rust VirtualKeyAuth 校验 -> KeyVault 解密 BYOK key -> 注入上游 -> 透传
    //    -> usage 提取推 Go -> Go 落库
    // 6. admin 调 GET /api/provider-keys/:id -> 只见末位(非 created_by)
    // 7. user-A 调 Reveal -> 明文(Rust owner 校验通过)
    // 8. admin 调 Reveal -> 403(Rust owner 校验拒绝)
    // 9. 验证 usage 表有记录
}

#[tokio::test]
async fn six_interfaces_homomorphic_passthrough() {
    // 六接口(含 Responses/Interactions 同态透传)经网关调通,响应与直连一致
}

#[tokio::test]
async fn usage_extracted_and_persisted() {
    // OpenAI Chat 流式(include_usage 注入)-> usage 提取 -> Go 落库
}
```

- [ ] **Step 2: 跑端到端测试**

Run:
```bash
# 启 Go + Rust 平台模式
cd core-rs && cargo test -p pingogate-core --test e2e_m0m1_closure -- --ignored
```
Expected: PASS(M0+M1 完整闭环)。

- [ ] **Step 3: Commit**

```bash
git add core-rs/pingogate-core/tests/e2e_m0m1_closure.rs
git commit -m "test(e2e): M0+M1 full closure (BYOK + virtual key + admin-blind + usage)"
```

---

### Task 33: S3 交叉认证 + M0+M1 证伪

**Files:**
- Test: `core-rs/pingogate-core/tests/s3_contract.rs`
- Test: `ctrl-go/internal/usage/s3_contract_test.go`

- [ ] **Step 1: 写 S3 契约测试**

```rust
// core-rs/pingogate-core/tests/s3_contract.rs
#[test]
fn virtual_key_auth_validates() { ... }
#[test]
fn usage_extractor_o1_memory() { ... }
#[test]
fn keyvault_owner_intercept_denies_non_owner() { ... }
#[test]
fn include_usage_injected_for_openai_stream() { ... }
```

```go
// ctrl-go/internal/usage/s3_contract_test.go
func TestS3Contract(t *testing.T) {
    // 1Password 双防线 / vkey 签发 / usage 落库
}
```

- [ ] **Step 2: 跑全部测试(Rust + Go)**

Run:
```bash
cd core-rs && cargo test --workspace
cd ctrl-go && go test ./...
```
Expected: 全通过。

- [ ] **Step 3: M0+M1 完整证伪检查(对照 spec §12 成功标准)**

- [ ] SC-1 六接口透传(T32)
- [ ] SC-2 BYOK key 加密存,只有 created_by 看明文,admin 不可见(T30/T32)
- [ ] SC-3 虚拟 key 经 Rust 调上游,用 BYOK key 解密注入(T26/T32)
- [ ] SC-4 虚拟 key 无效/撤销/过期/scope 超限拒绝(T26)
- [ ] SC-9 Rust 零 DB,不带 tokenizer(T15/T27)
- [ ] SC-10 usage 提取落库(T31)
- [ ] SC-11 单机+平台双模式(T21/T32)
- [ ] SC-12 SnapshotSource/KeyAuth 双实现(T16/T26)

- [ ] **Step 4: Commit**

```bash
git add core-rs/pingogate-core/tests/s3_contract.rs ctrl-go/internal/usage/s3_contract_test.go
git commit -m "test(s3): cross-validation contract + M0+M1 full closure verification"
```

---

## Self-Review

**1. Spec coverage**(对照 spec §11 S3):
- Go 1Password 可见性(created_by + 末位 + 解密):T30 ✅
- Go vkey(签发/撤销/哈希):T29 ✅
- Rust VirtualKeyAuth(哈希 + scope + 并发配额):T26 ✅
- Rust usage 提取(各接口位置):T27 ✅(inline 事件驱动,调研结论)
- Go usage 估算落库(tiktoken-go):T31 ✅
- OpenAI include_usage 注入(transform):T28 ✅
- spec §12A 双防线(Go created_by + Rust owner 拦截):T30 ✅(调研结论)
- spec §12B bootstrap:S2 已有
- spec §12C 快照同步:S2 已有,S3 加 virtual_keys/key_owners
- spec R5 include_usage:S3 实现 ✅
- spec R6 KeyVault 缓存:选 A 每请求解密(S1 调研结论)✅

**2. Placeholder scan**:无 TBD。tiktoken-go 具体库 T31 Step 3 示例,实现期核对最新包名。

**3. Type consistency**:`VirtualKeyAuth` 实现 `KeyAuth` trait(S1 定义,一致);`UsageEvent`(T24 proto)T27/T31 一致;`key_owners` 映射(T25)T30 Rust 校验一致。

**S3 产出契约**(M0+M1 完整):
- M0+M1 完整闭环:用户用虚拟 key 经 Rust 透传调上游,BYOK key 解密注入,admin 不可见明文,usage 落库
- 三 plan 交叉认证:S1(T12)+ S2(T22)+ S3(T33)契约测试,层层入口检查
