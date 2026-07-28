//! Shared harness for the S3 Task 32 end-to-end closure tests
//! (`e2e_m0m1_closure.rs`). Spawns both binaries (Rust platform mode + Go
//! control plane) on isolated ephemeral ports, drives the Go HTTP API, sends
//! requests to the Rust public data-plane listener, and cleans up on drop.
//!
//! Constitution V (file <= 300 lines): the harness lives here so the test
//! file itself stays under the limit. The harness is test-only (no production
//! code depends on it).
#![allow(clippy::unwrap_used, clippy::expect_used, dead_code)]

use std::net::{SocketAddr, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use super::client::{self, Request};
use super::{free_port, GATEWAY_KEY};

/// Shared internal token for the mTLS gRPC channel (spec §12A). Both binaries
/// must agree on this value; the test injects it via env var.
pub const INTERNAL_TOKEN: &str = "e2e-m0m1-internal-token";
/// Bootstrap admin token (spec §12B). The first start of the Go binary
/// creates the initial admin from this value; the test uses it to authenticate
/// the first admin HTTP request.
pub const BOOTSTRAP_ADMIN_TOKEN: &str = "e2e-m0m1-bootstrap-admin-token";
/// 32-byte AES-GCM master key for the Rust KeyVault (constitution XX). The
/// test uses a fixed value for reproducibility; production rotates this.
pub const MKEK: &str = "e2e-m0m1-mkek-32bytes-aes-gcm!!"; // 32 bytes
/// Plaintext BYOK provider key the test records on behalf of user-A. The
/// `LEAKMARKER` substring is asserted to never appear in any Go response,
/// matching the S3 no-leak contract (constitution XX).
pub const USER_A_PROVIDER_KEY_PLAINTEXT: &str = "sk-byok-user-a-LEAKMARKER-1234567890";
/// Sentinel: never referenced at runtime; documents the gateway-key contract
/// for readers. Platform mode uses VirtualKeyAuth, not StaticKeyAuth.
#[allow(dead_code)]
pub const _GATEWAY_KEY_DOC: &str = GATEWAY_KEY;

/// Resolved paths and ports for the closure test. All fields are computed
/// once at setup; subprocesses read them via env vars.
pub struct ClosureEnv {
    rust_grpc_port: u16,
    rust_public_port: u16,
    go_http_port: u16,
    go_usage_grpc_port: u16,
    certs_dir: PathBuf,
    db_path: PathBuf,
    ctrl_bin: PathBuf,
}

impl ClosureEnv {
    /// Locate binaries + certs, allocate ephemeral ports, prepare a fresh DB
    /// path. Panics with a clear message if any prerequisite is missing.
    pub fn setup() -> Self {
        let workspace_root = workspace_root();
        let certs_dir = workspace_root
            .join("ctrl-go")
            .join("internal")
            .join("grpcmtls")
            .join("certs");
        if !certs_dir.join("core.pem").exists() {
            panic!(
                "mTLS certs not found at {}. Run `go test ./internal/grpcmtls/ \
                 -run TestEnsureCerts` or start pingogate-ctrl once to generate them.",
                certs_dir.display()
            );
        }

        let ctrl_bin = ctrl_binary_path(&workspace_root);
        if !ctrl_bin.exists() {
            panic!(
                "Go binary not found at {}. Build it with `cd ctrl-go && go build \
                 -o pingogate-ctrl ./cmd/pingogate-ctrl` (or set PINGO_CTRL_BIN).",
                ctrl_bin.display()
            );
        }

        let rust_grpc_port = free_port();
        let rust_public_port = free_port();
        let go_http_port = free_port();
        let go_usage_grpc_port = free_port();

        let db_path = std::env::temp_dir().join(format!(
            "pingogate-e2e-m0m1-{}-{}.db",
            std::process::id(),
            go_http_port
        ));
        // Remove any stale DB so the bootstrap admin is always created fresh.
        let _ = std::fs::remove_file(&db_path);

        Self {
            rust_grpc_port,
            rust_public_port,
            go_http_port,
            go_usage_grpc_port,
            certs_dir,
            db_path,
            ctrl_bin,
        }
    }

    /// Go control-plane HTTP listen address (127.0.0.1:<port>).
    pub fn http_addr(&self) -> String {
        format!("127.0.0.1:{}", self.go_http_port)
    }

    /// Rust public data-plane port.
    pub fn public_port(&self) -> u16 {
        self.rust_public_port
    }

    /// Path to the Go-controlled SQLite DB (for direct usage queries).
    pub fn db_path(&self) -> &Path {
        &self.db_path
    }
}

/// Spawn the Rust kernel in platform mode (gRPC server + Pingora data plane).
/// Returns the child handle; killed on drop.
pub fn spawn_rust_platform(env: &ClosureEnv) -> RustChild {
    let bin = std::env::var("CARGO_BIN_EXE_pingogate-core")
        .expect("CARGO_BIN_EXE_pingogate-core must be set by Cargo for integration tests");
    let grpc_addr = format!("127.0.0.1:{}", env.rust_grpc_port);
    let public_addr = format!("127.0.0.1:{}", env.rust_public_port);

    let mut cmd = Command::new(&bin);
    cmd.arg("--grpc-addr")
        .arg(&grpc_addr)
        .env("PINGO_MODE", "platform")
        .env("PINGO_INTERNAL_TOKEN", INTERNAL_TOKEN)
        .env("PINGO_MKEK", MKEK)
        .env("PINGO_GRPC_CERT", env.certs_dir.join("core.pem").to_str().unwrap())
        .env("PINGO_GRPC_KEY", env.certs_dir.join("core.key").to_str().unwrap())
        .env("PINGO_GRPC_CA", env.certs_dir.join("ca.pem").to_str().unwrap())
        .env(
            "PINGO_USAGE_GRPC_ADDR",
            format!("127.0.0.1:{}", env.go_usage_grpc_port),
        )
        .env("PINGO_PUBLIC_ADDR", &public_addr)
        .env("PINGO_ADMIN_ADDR", format!("127.0.0.1:{}", free_port()))
        .env("RUST_LOG", "warn")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped());
    let mut child = cmd.spawn().expect("spawn pingogate-core platform");

    // Wait for both the gRPC and public data-plane listeners to come up.
    let deadline = Instant::now() + Duration::from_secs(15);
    while Instant::now() < deadline {
        if TcpStream::connect(("127.0.0.1", env.rust_grpc_port)).is_ok()
            && TcpStream::connect(("127.0.0.1", env.rust_public_port)).is_ok()
        {
            return RustChild { child };
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    let _ = child.kill();
    let _ = child.wait();
    panic!(
        "Rust platform-mode listeners did not come up (grpc:{}, public:{})",
        env.rust_grpc_port, env.rust_public_port
    );
}

/// Rust child handle; killed on drop.
pub struct RustChild {
    child: Child,
}

impl Drop for RustChild {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// Spawn the Go control-plane binary. It opens the DB, bootstraps the admin,
/// ensures mTLS certs, dials the Rust gRPC server, pushes the initial
/// snapshot, and serves HTTP + the UsageService gRPC server. Killed on drop.
pub fn spawn_go_ctrl(env: &ClosureEnv) -> GoChild {
    let mut cmd = Command::new(&env.ctrl_bin);
    cmd.env("PINGO_INTERNAL_TOKEN", INTERNAL_TOKEN)
        .env("PINGO_BOOTSTRAP_ADMIN_TOKEN", BOOTSTRAP_ADMIN_TOKEN)
        .env("PINGO_DB_DSN", env.db_path.to_str().unwrap())
        .env("PINGO_HTTP_ADDR", env.http_addr())
        .env("PINGO_GRPC_ADDR", format!("127.0.0.1:{}", env.rust_grpc_port))
        .env(
            "PINGO_USAGE_GRPC_ADDR",
            format!("127.0.0.1:{}", env.go_usage_grpc_port),
        )
        .env("PINGO_GRPC_CERTS_DIR", env.certs_dir.to_str().unwrap())
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped());
    let child = cmd.spawn().expect("spawn pingogate-ctrl");
    GoChild {
        child,
        db_path: env.db_path.clone(),
    }
}

/// Go child handle; killed on drop. Also removes the temp DB.
pub struct GoChild {
    child: Child,
    db_path: PathBuf,
}

impl Drop for GoChild {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        let _ = std::fs::remove_file(&self.db_path);
    }
}

/// Poll the HTTP port until a TCP connection succeeds or the deadline elapses.
pub fn wait_for_http_ready(addr: &str, timeout: Duration) {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        if let Ok(addr_parsed) = addr.parse::<SocketAddr>() {
            if TcpStream::connect(addr_parsed).is_ok() {
                return;
            }
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    panic!("Go HTTP listener did not come up on {addr}");
}

/// Send a GET with a Bearer token; return the parsed response.
pub fn http_get_json(env: &ClosureEnv, path: &str, bearer_token: &str) -> client::Resp {
    client::send(
        env.go_http_port,
        &Request::get(path)
            .header("authorization", &format!("Bearer {bearer_token}")),
    )
}

/// Send a POST with a Bearer token and a JSON body; return the parsed response.
pub fn http_post_json(
    env: &ClosureEnv,
    path: &str,
    bearer_token: &str,
    body: &str,
) -> client::Resp {
    client::send(
        env.go_http_port,
        &Request::post(path, body.as_bytes())
            .header("authorization", &format!("Bearer {bearer_token}")),
    )
}

/// Send a POST with a Bearer token and a JSON body to the **Rust public
/// data-plane listener** (not the Go HTTP API). Returns the parsed response.
pub fn http_post_to_rust(env: &ClosureEnv, path: &str, bearer_token: &str, body: &str) -> client::Resp {
    client::send(
        env.public_port(),
        &Request::post(path, body.as_bytes())
            .header("authorization", &format!("Bearer {bearer_token}")),
    )
}

/// Query the usage table row count for `owner_user_id` directly from the
/// SQLite DB. Uses the `sqlite3` CLI so no extra Rust crate is needed. Returns
/// 0 if sqlite3 is unavailable or the table is empty.
pub fn query_usage_count(db_path: &Path, owner_user_id: &str) -> i64 {
    let sql = format!("SELECT COUNT(*) FROM usage WHERE owner_user_id = '{}';", owner_user_id);
    match Command::new("sqlite3").arg(db_path).arg(&sql).output() {
        Ok(o) => String::from_utf8_lossy(&o.stdout).trim().parse().unwrap_or(0),
        Err(_) => 0,
    }
}

/// Workspace root (repo root): two levels up from `core-rs/pingogate-core/`.
fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .canonicalize()
        .expect("workspace root")
}

/// Resolve the Go binary path: prefer `PINGO_CTRL_BIN` env var; fall back to
/// `ctrl-go/pingogate-ctrl` (or `.exe` on Windows). The operator is expected
/// to build the Go binary first (see module docstring).
fn ctrl_binary_path(workspace_root: &Path) -> PathBuf {
    if let Ok(p) = std::env::var("PINGO_CTRL_BIN") {
        return PathBuf::from(p);
    }
    let exe = if cfg!(windows) {
        "pingogate-ctrl.exe"
    } else {
        "pingogate-ctrl"
    };
    workspace_root.join("ctrl-go").join(exe)
}
