# S2 用户身份 + Go 控制面 + KeyVault 加密 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 实现 Go 控制面用户身份(User CRUD + bootstrap admin + authorize)与 BYOK provider key 管理(经 Rust KeyVault AES-GCM 加密存 DB),Go 经 gRPC 把含密文 key 的快照推给 Rust,Rust 平台模式可跑(接收推送 + KeyVault 解密注入上游)。

**Architecture:** S2 在 S1 双语言基建上,填充 Go 业务(identity/keymgmt/snapshot/storage)与 Rust 安全核心(keyvault crate AES-GCM 实现 + SnapshotSource::Grpc + KeyVault gRPC 服务端)。Go DB(sqlx/SQLite)存 users + user_provider_keys(密文)。Go 推快照含密文 key,Rust 热路径每请求 KeyVault 解密(S1 调研结论:~0.2µs 可忽略)。S2 结束时单机+平台双模式可切换,用户能 CRUD + 录入加密 key + Go 推快照 Rust 接收。

**Tech Stack:** Go + chi + sqlx + SQLite + bcrypt(API token 哈希)+ grpc-go;Rust + aes-gcm + tonic(gRPC server)+ prost。S1 产出的 proto/trait/骨架。

## Global Constraints

- (继承 S1 Global Constraints)
- KeyVault AES-GCM,主密钥 `env:PINGO_MKEK`(32 bytes),启动校验存在,缺失即失败。
- API token 用 bcrypt 哈希存 DB,不存明文。
- provider key 明文只在 Rust `KeyVault::decrypt()` 返回的 `SecretString` 存活;Go 只持密文(宪法 XX)。
- 内部 gRPC mTLS + internal token(宪法 XX,spec §12A);S2 实现 mTLS(自签 CA)。
- DB 迁移用 sqlx migrate(spec §13 R9,倾向 sqlx migrate)。
- 热路径零 DB;Go 推快照不阻塞 Rust 热路径(宪法 XXI)。

## File Structure

**Rust 侧(`core-rs/`)**:
- `core-rs/keyvault/`(新增 crate):`src/{lib,crypto,grpc_service}.rs`。AES-GCM 加解密 + 主密钥加载 + gRPC KeyVaultService 服务端实现。
- Modify `core-rs/storage/src/keyvault.rs`:加 `AesGcmKeyVault`(实现 KeyVault trait,调 keyvault crate)。
- Modify `core-rs/storage/src/snapshot_source.rs`:加 `GrpcSnapshotSource`(接收 Go 推送,平台模式)。
- Modify `core-rs/pingogate-core/src/{main,grpc}.rs`:平台模式启动 KeyVault gRPC server + SnapshotService gRPC server(真实实现,替换 S1 空壳)。
- Modify `core-rs/pipeline/src/upstream_auth.rs`:平台模式调 KeyVault 解密 provider key(每请求 ~0.2µs)。

**Go 侧(`ctrl-go/`)**:
- `ctrl-go/internal/storage/`:`{db,migrations}.go`。sqlx 连接 + 迁移(users/user_provider_keys 表)。
- `ctrl-go/internal/identity/`:`{user,handler,middleware}.go`。User 实体 + CRUD + bootstrap admin + authorize。
- `ctrl-go/internal/keymgmt/`:`{provider_key,handler,keyvault_client}.go`。UserProviderKey CRUD + KeyVault gRPC client(加密)。
- `ctrl-go/internal/snapshot/`:`{builder,pusher}.go`(扩展 S1 client.go)。DB 变更 -> 构建快照(密文)-> gRPC 推 Rust。
- `ctrl-go/cmd/pingogate-ctrl/main.go`:装配 DB + identity + keymgmt + snapshot pusher + gRPC server(控制面 API)。
- `ctrl-go/internal/proto/`:`pingogate.pb.go`(扩展,Snapshot 消息字段填充)。

**proto(扩展)**:
- Modify `proto/pingogate.proto`:Snapshot 消息填充真实字段(providers/routes/virtual_keys 占位/encrypted_keys)。

## 任务分解

S2 分 10 个任务。依赖:T13(S1 入口检查)-> T14(proto 扩展)-> T15(Rust keyvault crate)-> T16(Rust GrpcSnapshotSource + 平台 main)-> T17(Go DB + 迁移)-> T18(Go identity)-> T19(Go keymgmt + KeyVault client)-> T20(Go snapshot 推送)-> T21(双模式集成)-> T22(S2 契约 + 证伪)。

**交叉认证**:T13 是 S2 第一个任务,**先跑 S1 契约测试**确认 S1 产出可用。T22 产出 S2 契约(用户/key 加密/快照推送),S3 入口检查。

---

### Task 13: S1 入口检查(交叉认证)

**Files:**
- Test: 复用 S1 `core-rs/pingogate-core/tests/s1_contract.rs` + `ctrl-go/internal/snapshot/client_test.go`

**Interfaces:**
- Consumes: S1 全部产出
- Produces: 确认 S1 产出可用(契约通过),S2 可开始

- [ ] **Step 1: 跑 S1 Rust 契约测试**

Run: `cd core-rs && cargo test -p pingogate-core --test s1_contract`
Expected: PASS(trait 签名 + 六接口识别 + KeyVault 空壳)。

- [ ] **Step 2: 启 Rust gRPC server,跑 S1 Go 契约测试**

```bash
# 终端1
cd core-rs && PINGO_MODE=platform cargo run -p pingogate-core -- --grpc-addr 127.0.0.1:9091
# 终端2
cd ctrl-go && go test ./internal/snapshot/ -run TestPushEmptyContract
```
Expected: PASS(gRPC 连通)。

- [ ] **Step 3: 若任一失败,停止 S2,回 S1 修复**

S1 契约是 S2 前提,失败不可继续。

- [ ] **Step 4: 记录 S1 产出契约(供 S2 实现引用)**

S2 消费的 S1 产出:
- `pingogate_storage::{SnapshotSource, KeyVault, StubKeyVault, FileSnapshotSource, SnapshotError, KeyError}`
- `pingogate_snapshot::{RuntimeSnapshot, SnapshotHolder, ResolvedProvider}`
- `pingogate_pipeline::{KeyAuth, StaticKeyAuth, GatewayProxy}`
- `pingogate_core::{Principal, AuthContext, authorize, SecretString, ProtocolKind}`
- `proto/pingogate.proto`(SnapshotService/KeyVaultService/UsageService + 消息)
- Rust gRPC server 骨架(`pingogate_core/src/grpc.rs` S1 空壳)

无 commit(纯检查任务)。

---

### Task 14: proto 扩展(Snapshot 真实字段)

**Files:**
- Modify: `proto/pingogate.proto`

**Interfaces:**
- Consumes: S1 proto 骨架
- Produces: Snapshot 消息含 providers/routes/encrypted_provider_keys 真实字段;KeyVault 消息已含(S1 定义)。S2 实现填充。

- [ ] **Step 1: 扩展 Snapshot 消息字段**

```protobuf
// proto/pingogate.proto(替换 S1 的 Snapshot 占位)
message Snapshot {
  uint64 version = 1;
  repeated ProviderEntry providers = 2;
  repeated RouteEntry routes = 3;
  repeated VirtualKeyEntry virtual_keys = 4;  // S3 填充,S2 留空
  repeated EncryptedProviderKey encrypted_keys = 5;
}

message ProviderEntry {
  string name = 1;
  string kind = 2;            // openai-compatible / anthropic / gemini
  string base_url = 3;
  string auth_method = 4;     // bearer / api_key_header / query_key
  string encrypted_key_ref = 5;  // 引用 encrypted_keys 中的 id
  string anthropic_version = 6;
  repeated string capability_families = 7;
}

message RouteEntry {
  string alias = 1;
  string provider = 2;
  string upstream_model = 3;
}

message EncryptedProviderKey {
  string id = 1;
  bytes ciphertext = 2;
  string owner_user_id = 3;  // S3 双防线用
  string created_by = 4;
}

message VirtualKeyEntry {
  // S3 填充,S2 留空消息
  string id = 1;
}
```

- [ ] **Step 2: 重新生成 Go + Rust codegen**

Run:
```bash
protoc --proto_path=proto \
  --go_out=ctrl-go/internal/proto --go_opt=paths=source_relative \
  --go-grpc_out=ctrl-go/internal/proto --go-grpc_opt=paths=source_relative \
  proto/pingogate.proto
cd core-rs && cargo build -p pingogate-core  # prost 重新生成
```
Expected: 生成新消息类型,编译通过。

- [ ] **Step 3: Commit**

```bash
git add proto/ ctrl-go/internal/proto/ core-rs/
git commit -m "feat(proto): expand Snapshot with providers/routes/encrypted_keys fields"
```

---

### Task 15: Rust keyvault crate(AES-GCM 实现)

**Files:**
- Create: `core-rs/keyvault/Cargo.toml`
- Create: `core-rs/keyvault/src/{lib,crypto,grpc_service}.rs`
- Modify: `core-rs/storage/src/keyvault.rs`(加 AesGcmKeyVault)
- Modify: `core-rs/Cargo.toml`(workspace 加 keyvault member + aes-gcm 依赖)

**Interfaces:**
- Consumes: `pingogate_core::SecretString`;主密钥 `env:PINGO_MKEK`
- Produces:
  - `pingogate_keyvault::AesGcmKeyVault`(实现 `KeyVault` trait):`encrypt(plaintext) -> ciphertext` / `decrypt(ciphertext) -> SecretString`
  - `pingogate_keyvault::KeyVaultGrpcService`(实现 `KeyVaultService` gRPC 服务端)
  - 主密钥从 `env:PINGO_MKEK`(32 bytes)加载,启动校验

- [ ] **Step 1: 加 keyvault crate 到 workspace**

```toml
# core-rs/Cargo.toml(members 加 keyvault)
members = ["core", "storage", "snapshot", "router", "transform", "provider", "pipeline", "listener", "keyvault", "pingogate-core"]

# [workspace.dependencies] 加
aes-gcm = "0.10"
tonic = "0.10"
prost = "0.12"
```

```toml
# core-rs/keyvault/Cargo.toml
[package]
name = "pingogate-keyvault"
version.workspace = true
edition.workspace = true

[dependencies]
aes-gcm = { workspace = true }
rand = "0.8"
pingogate-core = { path = "../core" }
pingogate-storage = { path = "../storage" }
tonic = { workspace = true }
prost = { workspace = true }
tokio = { workspace = true }
thiserror = "1"
```

- [ ] **Step 2: 写 AES-GCM 加解密测试(失败)**

```rust
// core-rs/keyvault/src/crypto.rs
use aes_gcm::{Aes256Gcm, Key, Nonce};
use aes_gcm::aead::{Aead, KeyInit};
use rand::RngCore;

pub struct AesGcmKeyVault {
    cipher: Aes256Gcm,
}

impl AesGcmKeyVault {
    pub fn from_master_key(mkek: &[u8; 32]) -> Self {
        let key = Key::<Aes256Gcm>::from_slice(mkek);
        Self { cipher: Aes256Gcm::new(key) }
    }

    pub fn encrypt(&self, plaintext: &[u8]) -> Result<Vec<u8>, KeyError> {
        // 随机 nonce(12 字节),前缀到密文:ct = nonce || ciphertext
        // AES-GCM 固定 nonce + 同 key = nonce 重用灾难,必须随机
        let mut nonce_bytes = [0u8; 12];
        rand::thread_rng().fill_bytes(&mut nonce_bytes);
        let nonce = Nonce::from_slice(&nonce_bytes);
        let ct = self.cipher.encrypt(nonce, plaintext).map_err(|_| KeyError::EncryptFailed)?;
        let mut out = Vec::with_capacity(12 + ct.len());
        out.extend_from_slice(&nonce_bytes);
        out.extend_from_slice(&ct);
        Ok(out)
    }

    pub fn decrypt(&self, ciphertext: &[u8]) -> Result<Vec<u8>, KeyError> {
        // 读前 12 字节作 nonce,余下作 ciphertext
        if ciphertext.len() < 12 { return Err(KeyError::DecryptFailed); }
        let (nonce_bytes, ct) = ciphertext.split_at(12);
        let nonce = Nonce::from_slice(nonce_bytes);
        self.cipher.decrypt(nonce, ct).map_err(|_| KeyError::DecryptFailed)
    }
}
```

```rust
// core-rs/keyvault/src/lib.rs(测试)
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encrypt_decrypt_roundtrip() {
        let kv = AesGcmKeyVault::from_master_key(&[42u8; 32]);
        let pt = b"sk-openai-test-key";
        let ct = kv.encrypt(pt).unwrap();
        assert_ne!(&ct[..], pt);
        let decrypted = kv.decrypt(&ct).unwrap();
        assert_eq!(&decrypted[..], pt);
    }

    #[test]
    fn decrypt_wrong_key_fails() {
        let kv1 = AesGcmKeyVault::from_master_key(&[1u8; 32]);
        let kv2 = AesGcmKeyVault::from_master_key(&[2u8; 32]);
        let ct = kv1.encrypt(b"secret").unwrap();
        assert!(kv2.decrypt(&ct).is_err());
    }

    #[test]
    fn nonce_is_random_no_reuse() {
        // 同明文加密两次,密文不同(nonce 随机,防 nonce 重用)
        let kv = AesGcmKeyVault::from_master_key(&[42u8; 32]);
        let ct1 = kv.encrypt(b"same-plaintext").unwrap();
        let ct2 = kv.encrypt(b"same-plaintext").unwrap();
        assert_ne!(ct1, ct2, "nonce must be random - ciphertexts must differ");
        // 两者都能正确解密
        assert_eq!(kv.decrypt(&ct1).unwrap(), b"same-plaintext");
        assert_eq!(kv.decrypt(&ct2).unwrap(), b"same-plaintext");
    }
}
```

- [ ] **Step 3: 跑测试确认通过**

Run: `cd core-rs && cargo test -p pingogate-keyvault`
Expected: PASS(roundtrip + 错 key 失败)。

- [ ] **Step 4: 实现 AesGcmKeyVault 适配 KeyVault trait(在 storage crate)**

```rust
// core-rs/storage/src/keyvault.rs(加 AesGcmKeyVault 包装)
use pingogate_keyvault::AesGcmKeyVault as InnerKv;

pub struct AesGcmKeyVault(pub InnerKv);

impl KeyVault for AesGcmKeyVault {
    fn encrypt(&self, plaintext: &[u8]) -> Result<Vec<u8>, KeyError> {
        self.0.encrypt(plaintext).map_err(|_| KeyError::EncryptFailed)
    }
    fn decrypt(&self, ciphertext: &[u8]) -> Result<SecretString, KeyError> {
        let pt = self.0.decrypt(ciphertext).map_err(|_| KeyError::DecryptFailed)?;
        Ok(SecretString::new(pt))
    }
}
```

- [ ] **Step 5: 写 KeyVault gRPC 服务端(含 owner 校验占位,S3 完整双防线)**

```rust
// core-rs/keyvault/src/grpc_service.rs
use tonic::{Request, Response, Status};

pub struct KeyVaultGrpcService {
    pub kv: Arc<AesGcmKeyVault>,
    // S3 加 owner 映射做双防线;S2 仅 mTLS + internal token
}

#[tonic::async_trait]
impl key_vault_service_server::KeyVaultService for KeyVaultGrpcService {
    async fn encrypt(&self, req: Request<EncryptRequest>) -> Result<Response<EncryptResponse>, Status> {
        let ct = self.kv.encrypt(&req.into_inner().plaintext)
            .map_err(|e| Status::internal(e.to_string()))?;
        Ok(Response::new(EncryptResponse { ciphertext: ct, error: String::new() }))
    }
    async fn decrypt(&self, req: Request<DecryptRequest>) -> Result<Response<DecryptResponse>, Status> {
        let req = req.into_inner();
        // S3:校验 requester_user_id == owner(S2 占位,信任 Go mTLS)
        let pt = self.kv.decrypt(&req.ciphertext)
            .map_err(|e| Status::internal(e.to_string()))?;
        // 审计日志(S2 已加,spec §12A L3)
        tracing::info!(key_id = %req.key_id, requester = %req.requester_user_id, intent = %req.intent, "keyvault decrypt");
        Ok(Response::new(DecryptResponse { plaintext: pt, error: String::new() }))
    }
}
```

- [ ] **Step 6: Commit**

```bash
git add core-rs/keyvault/ core-rs/storage/src/keyvault.rs core-rs/Cargo.toml
git commit -m "feat(keyvault): AES-GCM encrypt/decrypt + gRPC KeyVaultService server"
```

---

### Task 16: Rust GrpcSnapshotSource + 平台模式 main

**Files:**
- Modify: `core-rs/storage/src/snapshot_source.rs`(加 GrpcSnapshotSource)
- Modify: `core-rs/pingogate-core/src/grpc.rs`(SnapshotService 真实实现,替换 S1 空壳)
- Modify: `core-rs/pingogate-core/src/main.rs`(平台模式启动 KeyVault + Snapshot gRPC server)

**Interfaces:**
- Consumes: T14 proto;T15 keyvault;S1 SnapshotHolder
- Produces: Rust 平台模式可启动(接收 Go 快照推送 -> ArcSwap 切换 -> 热路径读);KeyVault gRPC server 可解密

- [ ] **Step 1: 写 GrpcSnapshotSource(接收 Go 推送,构建 RuntimeSnapshot)**

```rust
// core-rs/storage/src/snapshot_source.rs(加 GrpcSnapshotSource)
use tokio::sync::watch;

pub struct GrpcSnapshotSource {
    pub holder: Arc<SnapshotHolder>,
    // 接收 gRPC PushSnapshot 流,构建 RuntimeSnapshot,存 holder
}

impl GrpcSnapshotSource {
    pub async fn apply_snapshot(&self, snap_msg: Snapshot) -> Result<Ack, SnapshotError> {
        // 反序列化 proto Snapshot -> RuntimeSnapshot(含密文 provider key)
        // ArcSwap 切换
        // 返回 Ack{version, ok}
    }
}

impl SnapshotSource for GrpcSnapshotSource {
    fn build_snapshot(&self, version: u64) -> Result<Arc<RuntimeSnapshot>, SnapshotError> {
        // 平台模式:从 holder 读当前快照(已由 gRPC 推送设置)
        Ok(self.holder.load().clone_arc())
    }
}
```

- [ ] **Step 2: SnapshotService gRPC 服务端真实实现(替换 S1 空壳)**

```rust
// core-rs/pingogate-core/src/grpc.rs(替换 S1 空壳)
pub struct SnapshotServiceImpl {
    pub source: Arc<GrpcSnapshotSource>,
}

#[tonic::async_trait]
impl snapshot_service_server::SnapshotService for SnapshotServiceImpl {
    async fn push_snapshot(&self, req: Request<tonic::Streaming<Snapshot>>) -> Result<Response<Ack>, Status> {
        let mut stream = req.into_inner();
        let mut last_version = 0;
        while let Some(snap) = stream.next().await {
            let snap = snap?;
            last_version = snap.version;
            self.source.apply_snapshot(snap).await
                .map_err(|e| Status::internal(e.to_string()))?;
        }
        Ok(Response::new(Ack { version: last_version, ok: true, error: String::new() }))
    }
    // heartbeat 同 S1
}
```

- [ ] **Step 3: main.rs 平台模式装配(KeyVault + Snapshot gRPC server + 数据面)**

```rust
// core-rs/pingogate-core/src/main.rs(平台模式分支)
if mode == Mode::Platform {
    let mkek = env::var("PINGO_MKEK")?.parse()?;  // 32 bytes
    let kv = AesGcmKeyVault(InnerKv::from_master_key(&mkek));
    let grpc_addr = config.grpc.address.parse()?;
    let holder = Arc::new(SnapshotHolder::empty());
    let source = Arc::new(GrpcSnapshotSource { holder: holder.clone() });

    Server::builder()
        .add_service(KeyVaultServiceServer::new(KeyVaultGrpcService { kv: Arc::new(kv) }))
        .add_service(SnapshotServiceServer::new(SnapshotServiceImpl { source: source.clone() }))
        .serve(grpc_addr).await?;
    // 数据面用 holder(等首个快照后才 ready)
}
```

- [ ] **Step 4: 平台模式编译 + readyz 检查**

Run: `cd core-rs && PINGO_MODE=platform cargo build -p pingogate-core`
Expected: 编译通过。readyz 在首个快照前返回 not-ready(spec §12B)。

- [ ] **Step 5: Commit**

```bash
git add core-rs/storage/src/snapshot_source.rs core-rs/pingogate-core/src/{grpc,main}.rs
git commit -m "feat(snapshot,grpc): GrpcSnapshotSource + platform mode main with KeyVault+Snapshot gRPC"
```

---

### Task 17: Go DB + 迁移(users + user_provider_keys 表)

**Files:**
- Create: `ctrl-go/internal/storage/db.go`
- Create: `ctrl-go/internal/storage/migrations/`(sqlx migrate 文件)
- Modify: `ctrl-go/go.mod`(加 sqlx + sqlite + bcrypt)

**Interfaces:**
- Consumes: 无(Go 内部)
- Produces: `storage.DB`(sqlx 连接)+ 迁移(users + user_provider_keys 表)

- [ ] **Step 1: 加 Go 依赖**

```bash
cd ctrl-go
go get github.com/jmoiron/sqlx
go get github.com/mattn/go-sqlite3
go get golang.org/x/crypto/bcrypt
go get github.com/sqlc-dev/sqlx/migrate  # 或 github.com/golang-migrate/migrate
```

- [ ] **Step 2: 写迁移 SQL**

```sql
-- ctrl-go/internal/storage/migrations/001_users.up.sql
CREATE TABLE users (
    id TEXT PRIMARY KEY,
    email TEXT UNIQUE NOT NULL,
    api_token_hash TEXT NOT NULL,  -- bcrypt hash
    is_admin INTEGER NOT NULL DEFAULT 0,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);

-- ctrl-go/internal/storage/migrations/002_user_provider_keys.up.sql
CREATE TABLE user_provider_keys (
    id TEXT PRIMARY KEY,
    owner_user_id TEXT NOT NULL REFERENCES users(id),
    provider_type TEXT NOT NULL,  -- openai / anthropic / gemini / openai-compatible
    encrypted_key BLOB NOT NULL,  -- 密文(Rust KeyVault 加密)
    base_url TEXT,
    created_by TEXT NOT NULL REFERENCES users(id),  -- = owner_user_id(BYOK)
    created_at TEXT NOT NULL,
    last_used_at TEXT,
    enabled INTEGER NOT NULL DEFAULT 1
);
CREATE INDEX idx_provider_keys_owner ON user_provider_keys(owner_user_id);
```

- [ ] **Step 3: 写 db.go(连接 + 迁移)**

```go
// ctrl-go/internal/storage/db.go
package storage

import (
    "database/sql"
    "github.com/jmoiron/sqlx"
    _ "github.com/mattn/go-sqlite3"
)

type DB struct {
    *sqlx.DB
}

func Open(dsn string) (*DB, error) {
    db, err := sqlx.Connect("sqlite3", dsn)
    if err != nil { return nil, err }
    if err := migrate(db); err != nil { return nil, err }
    return &DB{db}, nil
}

func migrate(db *sqlx.DB) error {
    // sqlx migrate 或 golang-migrate 执行 migrations/
    // 略:实现见 migrate.go
    return nil
}
```

- [ ] **Step 4: 写 db 测试**

```go
// ctrl-go/internal/storage/db_test.go
func TestOpenAndMigrate(t *testing.T) {
    db, err := Open(":memory:")
    if err != nil { t.Fatal(err) }
    defer db.Close()
    var name string
    err = db.Get(&name, "SELECT name FROM sqlite_master WHERE type='table' AND name='users'")
    if err != nil { t.Fatalf("users table not created: %v", err) }
}
```

- [ ] **Step 5: 跑测试**

Run: `cd ctrl-go && go test ./internal/storage/`
Expected: PASS。

- [ ] **Step 6: Commit**

```bash
git add ctrl-go/internal/storage/ ctrl-go/go.mod ctrl-go/go.sum
git commit -m "feat(storage): sqlx + SQLite with users/user_provider_keys migrations"
```

---

### Task 18: Go identity(User CRUD + bootstrap admin + authorize)

**Files:**
- Create: `ctrl-go/internal/identity/{user,handler,middleware}.go`

**Interfaces:**
- Consumes: T17 storage;`pingogate_core::Principal`(经 proto 或 Go 本地定义)
- Produces: User CRUD + bootstrap admin(`env:PINGO_BOOTSTRAP_ADMIN_TOKEN`)+ authorize 边界

- [ ] **Step 1: 写 User 实体 + CRUD**

```go
// ctrl-go/internal/identity/user.go
package identity

type User struct {
    ID           string `db:"id" json:"id"`
    Email        string `db:"email" json:"email"`
    APITokenHash string `db:"api_token_hash" json:"-"`
    IsAdmin      bool   `db:"is_admin" json:"is_admin"`
    CreatedAt    string `db:"created_at" json:"created_at"`
    UpdatedAt    string `db:"updated_at" json:"updated_at"`
}

type Store struct{ db *storage.DB }

func (s *Store) Create(ctx context.Context, email, apiToken string) (*User, string, error) {
    // 生成明文 token(返一次)+ bcrypt 哈希存
    // INSERT user
}

func (s *Store) GetByID(ctx context.Context, id string) (*User, error) { ... }
func (s *Store) List(ctx context.Context) ([]User, error) { ... }
func (s *Store) Delete(ctx context.Context, id string) error { ... }
```

- [ ] **Step 2: 写 bootstrap admin(首次启动创建)**

```go
// ctrl-go/internal/identity/bootstrap.go
func BootstrapAdmin(ctx context.Context, store *Store, token string) error {
    count, err := store.CountAdmins(ctx)
    if err != nil { return err }
    if count > 0 { return nil }  // 已有 admin,跳过
    _, _, err = store.Create(ctx, "admin@bootstrap.local", token)
    store.SetAdmin(ctx, ...)  // is_admin = 1
    return err
}
```

main.go 启动调:`BootstrapAdmin(ctx, store, os.Getenv("PINGO_BOOTSTRAP_ADMIN_TOKEN"))`。

- [ ] **Step 2a: 启动校验(spec §12B:环境变量缺失即失败,不给降级路径)**

```go
// ctrl-go/cmd/pingogate-ctrl/main.go(启动校验)
func main() {
    // spec §12B:主密钥 + gRPC 内部 token + bootstrap admin token 缺失即失败
    mkek := os.Getenv("PINGO_MKEK")
    if len(mkek) != 32 { log.Fatal("PINGO_MKEK must be 32 bytes") }
    internalToken := os.Getenv("PINGO_INTERNAL_TOKEN")
    if internalToken == "" { log.Fatal("PINGO_INTERNAL_TOKEN required") }
    bootstrapToken := os.Getenv("PINGO_BOOTSTRAP_ADMIN_TOKEN")
    if bootstrapToken == "" { log.Fatal("PINGO_BOOTSTRAP_ADMIN_TOKEN required (first start)") }
    // ... 装配
}
```

Rust 侧 `main.rs` 同样校验 `PINGO_MKEK`(32 bytes)+ `PINGO_INTERNAL_TOKEN` 缺失即 panic(spec §12B,不给明文存 key 降级)。T16 已加 MKEK,补 internal token 校验。

- [ ] **Step 3: 写 authorize 中间件(Principal + authorize 边界)**

```go
// ctrl-go/internal/identity/middleware.go
func Authorize(action string, resource string) func(http.Handler) http.Handler {
    return func(next http.Handler) http.Handler {
        return http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
            user := UserFromContext(r.Context())
            if user == nil { http.Error(w, "unauthorized", 401); return }
            if !canPerform(user, action, resource) {
                http.Error(w, "forbidden", 403); return
            }
            next.ServeHTTP(w, r)
        })
    }
}

// Principal 经 API token 鉴权后注入 context
func AuthMiddleware(store *Store) func(http.Handler) http.Handler {
    // 提取 Authorization: Bearer <token> -> bcrypt 比对 -> 注入 User
}
```

- [ ] **Step 4: 写 handler(REST CRUD)**

```go
// ctrl-go/internal/identity/handler.go
// POST /api/users, GET /api/users, GET /api/users/:id, DELETE /api/users/:id
// 经 AuthMiddleware + Authorize("manage", "users")
```

- [ ] **Step 5: 写测试**

```go
// ctrl-go/internal/identity/handler_test.go
func TestCreateUserAndAuth(t *testing.T) {
    // 创建用户 -> 用返回的 token 调受保护端点 -> 200
    // 无 token -> 401;错 token -> 401
}
func TestBootstrapAdmin(t *testing.T) {
    // 首次创建 admin -> 第二次跳过
}
```

- [ ] **Step 6: 跑测试**

Run: `cd ctrl-go && go test ./internal/identity/`
Expected: PASS。

- [ ] **Step 7: Commit**

```bash
git add ctrl-go/internal/identity/
git commit -m "feat(identity): User CRUD + bootstrap admin + authorize middleware"
```

---

### Task 19: Go keymgmt(UserProviderKey CRUD + KeyVault gRPC client)

**Files:**
- Create: `ctrl-go/internal/keymgmt/{provider_key,handler,keyvault_client}.go`

**Interfaces:**
- Consumes: T17 storage;T18 identity;T15 Rust KeyVault gRPC(经 client)
- Produces: UserProviderKey CRUD;录入 key -> 调 Rust KeyVault.Encrypt -> 存密文 DB

- [ ] **Step 1: 写 UserProviderKey 实体 + CRUD**

```go
// ctrl-go/internal/keymgmt/provider_key.go
type UserProviderKey struct {
    ID            string `db:"id" json:"id"`
    OwnerUserID   string `db:"owner_user_id" json:"owner_user_id"`
    ProviderType  string `db:"provider_type" json:"provider_type"`
    EncryptedKey  []byte `db:"encrypted_key" json:"-"`  // 密文,不返
    BaseURL       string `db:"base_url" json:"base_url,omitempty"`
    CreatedBy     string `db:"created_by" json:"created_by"`  // = owner(BYOK)
    CreatedAt     string `db:"created_at" json:"created_at"`
    LastUsedAt    *string `db:"last_used_at" json:"last_used_at,omitempty"`
    Enabled       bool   `db:"enabled" json:"enabled"`
}
```

- [ ] **Step 2: 写 KeyVault gRPC client(加密)**

```go
// ctrl-go/internal/keymgmt/keyvault_client.go
type KeyVaultClient struct {
    client pb.KeyVaultServiceClient
}

func (c *KeyVaultClient) Encrypt(ctx context.Context, plaintext []byte) ([]byte, error) {
    resp, err := c.client.Encrypt(ctx, &pb.EncryptRequest{Plaintext: plaintext})
    if err != nil { return nil, err }
    return resp.Ciphertext, nil
}

func (c *KeyVaultClient) Decrypt(ctx context.Context, ciphertext []byte, requesterID, keyID, intent string) ([]byte, error) {
    resp, err := c.client.Decrypt(ctx, &pb.DecryptRequest{
        Ciphertext: ciphertext, RequesterUserId: requesterID, KeyId: keyID, Intent: intent,
    })
    if err != nil { return nil, err }
    return resp.Plaintext, nil
}
```

- [ ] **Step 3: 写录入 handler(明文 -> Rust 加密 -> 存密文)**

```go
// ctrl-go/internal/keymgmt/handler.go
// POST /api/provider-keys
func (h *Handler) Create(w http.ResponseWriter, r *http.Request) {
    user := identity.UserFromContext(r.Context())
    var req struct {
        ProviderType string `json:"provider_type"`
        PlaintextKey string `json:"plaintext_key"`  // 明文,一次
        BaseURL      string `json:"base_url,omitempty"`
    }
    // 1. 调 Rust KeyVault.Encrypt(plaintext)
    ciphertext, err := h.kv.Encrypt(ctx, []byte(req.PlaintextKey))
    // 2. INSERT user_provider_keys(密文 + created_by = user.ID)
    // 3. 触发快照推送(T20)
    // 返回 key 元数据(不含明文)
}
```

- [ ] **Step 4: 写列表/查看 handler(S2 只返末位,S3 加 1Password)**

```go
// GET /api/provider-keys -> 列表(只返末位 + 元数据,S3 加 created_by 查明文)
func (h *Handler) List(w http.ResponseWriter, r *http.Request) {
    user := identity.UserFromContext(r.Context())
    keys, _ := h.store.ListByOwner(ctx, user.ID)
    // 返回末位:key[len-4:],不返密文
}
```

- [ ] **Step 5: 写测试(录入 -> 密文存 -> 列表返末位)**

```go
// ctrl-go/internal/keymgmt/handler_test.go
func TestCreateAndListProviderKey(t *testing.T) {
    // mock KeyVault client(或启 Rust)
    // 录入明文 -> DB 存密文 -> 列表返末位(末4位),不返明文
}
```

- [ ] **Step 6: 跑测试**

Run: `cd ctrl-go && go test ./internal/keymgmt/`
Expected: PASS。

- [ ] **Step 7: Commit**

```bash
git add ctrl-go/internal/keymgmt/
git commit -m "feat(keymgmt): UserProviderKey CRUD + KeyVault gRPC client (encrypt on store)"
```

---

### Task 20: Go snapshot 推送(DB 变更 -> 构建快照 -> gRPC 推 Rust)

**Files:**
- Create: `ctrl-go/internal/snapshot/{builder,pusher}.go`(扩展 S1 client.go)

**Interfaces:**
- Consumes: T17 storage;T19 keymgmt;T14 proto;S1 Rust gRPC server
- Produces: DB 变更 -> 构建含密文 key 的 Snapshot -> gRPC 推 Rust -> Rust ArcSwap 切换

- [ ] **Step 1: 写快照构建器(读 DB -> proto Snapshot)**

```go
// ctrl-go/internal/snapshot/builder.go
type Builder struct{ db *storage.DB }

func (b *Builder) Build(ctx context.Context, version uint64) (*pb.Snapshot, error) {
    // 读 DB:users(略)/user_provider_keys(密文)/routes(配置)/providers(配置)
    // 组装 pb.Snapshot{version, providers, routes, encrypted_keys}
    // 注意:virtual_keys S3 填充,S2 留空
}
```

- [ ] **Step 2: 写推送器(全量 + version + ack,串行 mutex)**

```go
// ctrl-go/internal/snapshot/pusher.go
type Pusher struct {
    client  pb.SnapshotServiceClient
    builder *Builder
    mu      sync.Mutex  // 串行推送(spec §12C)
    version uint64
}

func (p *Pusher) Push(ctx context.Context) error {
    p.mu.Lock()
    defer p.mu.Unlock()
    p.version++
    snap, err := p.builder.Build(ctx, p.version)
    if err != nil { return err }
    stream, err := p.client.PushSnapshot(ctx)
    if err != nil { return err }
    if err := stream.Send(snap); err != nil { return err }
    ack, err := stream.CloseAndRecv()
    if err != nil { return err }
    if !ack.Ok { return fmt.Errorf("rust rejected: %s", ack.Error) }
    return nil  // ack.Version == p.version
}
```

- [ ] **Step 3: DB 变更触发推送(观察者)**

keymgmt Create/Update/Delete 后调 `pusher.Push(ctx)`。S2 简单同步触发;S3 可改异步。

- [ ] **Step 4: 写测试(录入 key -> 推送 -> Rust 接收)**

```go
// ctrl-go/internal/snapshot/pusher_test.go
func TestPushAfterKeyCreate(t *testing.T) {
    // 启 Rust gRPC server(平台模式)
    // 录入 provider key -> 触发推送 -> Rust holder 更新
    // 跑 Rust 侧验证 holder.load().version == pushed
}
```

- [ ] **Step 5: 跑测试(需 Rust 平台模式运行)**

Run:
```bash
# 终端1
cd core-rs && PINGO_MODE=platform PINGO_MKEK=<32bytes> cargo run -p pingogate-core -- --grpc-addr 127.0.0.1:9091
# 终端2
cd ctrl-go && go test ./internal/snapshot/ -run TestPushAfterKeyCreate
```
Expected: PASS。

- [ ] **Step 6: Commit**

```bash
git add ctrl-go/internal/snapshot/
git commit -m "feat(snapshot): build snapshot from DB + push to Rust via gRPC (full + version + ack)"
```

- [ ] **Step 7: 崩溃恢复测试(spec §12C)**

```go
// ctrl-go/internal/snapshot/recovery_test.go
func TestRustCrashRecovery(t *testing.T) {
    // 1. Go 推快照 v1 -> Rust 接收
    // 2. 模拟 Rust 崩溃重启(内存空)
    // 3. Go 检测重连(HealthService.Check 报 ready=false/version=0)
    // 4. Go 推全量 v1 -> Rust ready=true
    // 5. 验证 Rust holder.version == 1
}

func TestGoCrashRustServesWithOldSnapshot(t *testing.T) {
    // 1. Go 推快照 v1 -> Rust 接收
    // 2. Go 崩溃 -> Rust 热路径继续读旧快照(零中断)
    // 3. 请求仍能透传(用旧快照的 provider key)
    // 4. Go 重启 -> 读 DB -> 推当前全量 -> Rust 切换
}
```

Run: `cd ctrl-go && go test ./internal/snapshot/ -run TestRustCrash\|TestGoCrash`
Expected: PASS(崩溃恢复语义正确)。

- [ ] **Step 8: Commit**

```bash
git add ctrl-go/internal/snapshot/recovery_test.go
git commit -m "test(snapshot): crash recovery (Rust restart re-sync, Go crash Rust serves old)"
```

---

### Task 20a: Go admin package(管理面 API + reload 编排)

**Files:**
- Create: `ctrl-go/internal/admin/{handler,reload}.go`

**Interfaces:**
- Consumes: T18 identity(authorize);T20 snapshot(触发 Rust 热重载);S1 Rust 管理端点(经 gRPC 或 HTTP 转发)
- Produces: Go 管理面 API(健康/就绪/reload 编排)+ 触发 Rust 热重载

- [ ] **Step 1: 写 admin handler(健康/就绪/reload 编排)**

```go
// ctrl-go/internal/admin/handler.go
// GET /admin/healthz -> Go 存活
// GET /admin/readyz -> Go 就绪(DB + gRPC 连 Rust)
// POST /admin/reload -> 触发 Rust 热重载(经 gRPC SnapshotService 推当前全量,或调 Rust /reload)
//   reload 编排:Go 读 DB -> 构建快照 -> 推 Rust -> 等 ack -> 返回结果
// 经 identity.Authorize(Admin, ...) 边界
```

- [ ] **Step 2: 写 reload 编排(触发 Rust 热重载)**

```go
// ctrl-go/internal/admin/reload.go
func (h *Handler) Reload(ctx context.Context) error {
    // Go 侧:重建快照(读 DB)-> 推 Rust(spec §4.4 触发源之一)
    return h.pusher.Push(ctx)
}
```

- [ ] **Step 3: 测试 admin 端点 + authorize 边界**

```go
func TestAdminReloadRequiresAdmin(t *testing.T) {
    // 非 admin 调 /admin/reload -> 403
    // admin 调 -> 触发推送 -> 200
}
```

- [ ] **Step 4: Commit**

```bash
git add ctrl-go/internal/admin/
git commit -m "feat(admin): management API (healthz/readyz/reload orchestration) + authorize"
```

---

### Task 21: 双模式集成测试(单机 + 平台切换)

**Files:**
- Test: `core-rs/pingogate-core/tests/dual_mode.rs`(新增)

**Interfaces:**
- Consumes: T16 平台模式;S1 单机模式
- Produces: S2 证伪--单机+平台双模式可切换

- [ ] **Step 1: 写双模式测试**

```rust
// core-rs/pingogate-core/tests/dual_mode.rs
#[test]
fn standalone_mode_uses_file_snapshot_source() {
    // 启 standalone 模式 -> 验证用 FileSnapshotSource + StaticKeyAuth
}

#[test]
fn platform_mode_uses_grpc_snapshot_source() {
    // 启 platform 模式(需 Go 运行)-> 验证用 GrpcSnapshotSource + (S3 VirtualKeyAuth,S2 用 StaticKeyAuth 占位)
}

#[test]
fn mode_switch_via_env() {
    // PINGO_MODE=standalone vs platform 启动不同分支
}
```

- [ ] **Step 2: 跑测试**

Run: `cd core-rs && cargo test -p pingogate-core --test dual_mode`
Expected: PASS(单机模式完整;平台模式需 Go 运行,可能标 #[ignore] 或用 mock)。

- [ ] **Step 3: Commit**

```bash
git add core-rs/pingogate-core/tests/dual_mode.rs
git commit -m "test(pingogate-core): dual-mode (standalone + platform) integration"
```

---

### Task 22: S2 交叉认证 + 证伪

**Files:**
- Test: `ctrl-go/internal/keymgmt/s2_contract_test.go`(Go)
- Test: `core-rs/pingogate-core/tests/s2_contract.rs`(Rust)

**Interfaces:**
- Consumes: T13-T21 全部
- Produces: S2 契约测试套件(S3 入口检查)

- [ ] **Step 1: 写 Go S2 契约测试**

```go
// ctrl-go/internal/keymgmt/s2_contract_test.go
func TestS2Contract(t *testing.T) {
    // 1. 用户 CRUD 可用(identity)
    // 2. 录入 provider key -> 密文存 DB(keymgmt)
    // 3. 列表返末位不返明文
    // 4. DB 无明文 key(抽样:SELECT encrypted_key,断言非明文格式)
}
```

- [ ] **Step 2: 写 Rust S2 契约测试**

```rust
// core-rs/pingogate-core/tests/s2_contract.rs
#[test]
fn keyvault_aesgcm_works() {
    let kv = AesGcmKeyVault(InnerKv::from_master_key(&[42u8; 32]));
    let ct = kv.encrypt(b"test").unwrap();
    let pt = kv.decrypt(&ct).unwrap();
    assert_eq!(pt.expose(), b"test");
}

#[test]
fn grpc_snapshot_source_receives_push() {
    // 启 platform 模式 -> Go 推快照 -> Rust holder 更新
}

#[test]
fn platform_mode_decrypts_on_hot_path() {
    // 平台模式热路径调 KeyVault 解密 provider key 注入上游
}
```

- [ ] **Step 3: 跑 S2 全部测试**

Run:
```bash
cd core-rs && cargo test --workspace
cd ctrl-go && go test ./...
```
Expected: 全通过。

- [ ] **Step 4: S2 证伪检查(对照 spec §11 S2 证伪)**

- [ ] 用户能 CRUD(T18 通过)
- [ ] 录入 provider key 经 Rust 加密存 DB(密文)(T19 通过)
- [ ] Go 推快照 Rust 接收(T20 通过)
- [ ] 单机+平台双模式切换(T21 通过)

- [ ] **Step 5: Commit**

```bash
git add ctrl-go/internal/keymgmt/s2_contract_test.go core-rs/pingogate-core/tests/s2_contract.rs
git commit -m "test(s2): cross-validation contract suite for S3 entry check"
```

---

## Self-Review

**1. Spec coverage**(对照 spec §11 S2):
- Go identity(User CRUD + bootstrap admin + authorize):T18 ✅
- Go keymgmt(UserProviderKey CRUD + KeyVault client):T19 ✅
- Rust keyvault(AES-GCM + 主密钥 + gRPC):T15 ✅
- Go snapshot(DB -> 快照 -> gRPC 推):T20 ✅
- Rust GrpcSnapshotSource + KeyAuth 扩展:T16 ✅(KeyAuth 扩展 S3 VirtualKeyAuth)
- 双模式切换:T21 ✅
- spec §12A 内部 gRPC 安全(mTLS + internal token):T16/S2 需补 mTLS(R10 未决,plan 阶段定自签 CA)
- spec §12B bootstrap:T18 ✅
- spec §12C 快照同步:T20 ✅(全量 + version + ack + mutex 串行)

**2. Placeholder scan**:T15 Step 2 nonce 固定(S3 改随机)已标注。T19 列表返末位 S3 加 1Password,已标注。

**3. Type consistency**:`KeyVault` trait(S1 定义)T15 AesGcmKeyVault 实现(一致);`SnapshotSource` trait(S1)T16 GrpcSnapshotSource 实现(一致);proto Snapshot(T14 定义)T20 构建(一致)。

**S2 产出契约**(S3 消费):
- Rust:`pingogate_keyvault::{AesGcmKeyVault, KeyVaultGrpcService}`;`pingogate_storage::GrpcSnapshotSource`
- Go:`identity.Store`(User CRUD + bootstrap + authorize);`keymgmt.Store`(ProviderKey CRUD + KeyVaultClient);`snapshot.Pusher`(推快照)
- proto:Snapshot 含真实字段(providers/routes/encrypted_keys);virtual_keys 占位(S3 填充)
- DB schema:users + user_provider_keys 表

S3 开头先跑 `go test ./internal/keymgmt/ -run TestS2Contract` + `cargo test -p pingogate-core --test s2_contract`,确认 S2 产出可用。
