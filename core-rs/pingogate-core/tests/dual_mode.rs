//! S2 dual-mode integration tests (T21).
//!
//! - standalone: PINGO_MODE=standalone -> FileSnapshotSource + StaticKeyAuth
//!   (end-to-end via T10 harness: gateway-key round-trip through mock upstream).
//! - platform: PINGO_MODE=platform -> GrpcSnapshotSource + gRPC server. `#[ignore]`
//!   (needs mTLS certs from pingogate-ctrl R10); full push in Go client_test.go.
//! - mode_switch_via_env: distinct failure errors prove branch dispatch (no network).
#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use std::net::TcpStream;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use common::client::Request;
use common::{Harness, GATEWAY_KEY};

/// Workspace root (repo root): two levels up from `core-rs/pingogate-core/`.
/// Used to locate the Go-generated mTLS cert directory for the `#[ignore]`
/// platform test.
fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .canonicalize()
        .expect("workspace root")
}

/// Path to the binary under test. `CARGO_BIN_EXE_pingogate-core` is set by
/// Cargo for integration tests.
fn bin_path() -> String {
    std::env::var("CARGO_BIN_EXE_pingogate-core")
        .expect("CARGO_BIN_EXE_pingogate-core must be set by Cargo for integration tests")
}

/// Spawn `pingogate-core` with the given `PINGO_MODE` and env overrides,
/// capturing stderr. Returns `(failed, stderr)` where `failed` is true when the
/// process exited non-zero (the expected outcome for missing-env startup
/// failures).
fn spawn_mode(mode: &str, extra_env: &[(&str, &str)]) -> (bool, String) {
    let mut cmd = Command::new(bin_path());
    cmd.env("PINGO_MODE", mode)
        .env("RUST_LOG", "error")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped());
    for (k, v) in extra_env {
        cmd.env(k, v);
    }
    let output = cmd.output().expect("spawn pingogate-core");
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();
    (!output.status.success(), stderr)
}

/// Poll `127.0.0.1:port` until a TCP connection succeeds or the deadline
/// elapses; returns whether the port came up.
fn port_binds_within(port: u16, timeout: Duration) -> bool {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        if TcpStream::connect(("127.0.0.1", port)).is_ok() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    false
}

// ---------------------------------------------------------------------------
// 1. standalone mode: FileSnapshotSource + StaticKeyAuth end-to-end
// ---------------------------------------------------------------------------

/// Standalone mode builds the snapshot from a YAML file (FileSnapshotSource)
/// and authenticates the gateway key via StaticKeyAuth. A successful
/// authenticated round-trip through the mock upstream proves both paths fired:
/// the snapshot was built (provider routes resolved) and the gateway key was
/// accepted (StaticKeyAuth matched it against the snapshot's `gateway_keys`).
#[test]
fn standalone_mode_uses_file_snapshot_source() {
    let gw = Harness::start();
    let body = br#"{"model":"gpt-4o","messages":[{"role":"user","content":"hi"}]}"#;

    let resp = gw.send(
        &Request::post("/v1/chat/completions", body)
            .header("authorization", &format!("Bearer {GATEWAY_KEY}")),
    );

    assert_eq!(resp.status, 200, "body: {}", resp.body_str());
    // StaticKeyAuth accepted GATEWAY_KEY; pipeline routed via FileSnapshotSource; mock upstream echoes injected upstream key.
    assert!(resp.header("x-observed-authorization").is_some());
}

// ---------------------------------------------------------------------------
// 2. platform mode: GrpcSnapshotSource (#[ignore] - needs mTLS certs)
// ---------------------------------------------------------------------------

/// Platform mode constructs `GrpcSnapshotSource` (wrapping an empty
/// `SnapshotHolder`) and starts the gRPC server with mTLS + internal token.
/// This test verifies the gRPC listener comes up on the expected port, proving
/// the platform branch was taken and `GrpcSnapshotSource` was constructed.
///
/// **Marked `#[ignore]`** because it requires the mTLS cert set (CA + core
/// cert/key + ctrl cert/key) that `pingogate-ctrl` generates on first start
/// (R10). The certs live at `ctrl-go/internal/grpcmtls/certs/` and are
/// gitignored. To run this test:
///
/// 1. Start `pingogate-ctrl` once to generate certs, or run
///    `go test ./internal/grpcmtls/ -run TestEnsureCerts` to generate them.
/// 2. Run: `cargo test -p pingogate-core --test dual_mode -- --ignored
///    platform_mode_uses_grpc_snapshot_source`
///
/// Full snapshot-push verification is covered by the Go-side contract test
/// `ctrl-go/internal/snapshot/client_test.go` (requires a running Rust gRPC
/// server); this Rust test only verifies the platform startup path.
#[test]
#[ignore = "requires mTLS certs generated by pingogate-ctrl (R10)"]
fn platform_mode_uses_grpc_snapshot_source() {
    let certs_dir = workspace_root()
        .join("ctrl-go")
        .join("internal")
        .join("grpcmtls")
        .join("certs");
    let core_cert = certs_dir.join("core.pem");
    let core_key = certs_dir.join("core.key");
    let ca_pem = certs_dir.join("ca.pem");
    if !core_cert.exists() || !core_key.exists() || !ca_pem.exists() {
        panic!(
            "mTLS certs not found at {}. Start pingogate-ctrl once to generate them (R10), \
             then re-run with --ignored.",
            certs_dir.display()
        );
    }

    let grpc_port = common::free_port();
    let grpc_addr = format!("127.0.0.1:{grpc_port}");

    let mkek = "0123456789abcdef0123456789abcdef"; // 32-byte AES-GCM MKEK
    let internal_token = "test-internal-token-dual-mode";

    let mut cmd = Command::new(bin_path());
    cmd.arg("--grpc-addr")
        .arg(&grpc_addr)
        .env("PINGO_MODE", "platform")
        .env("PINGO_INTERNAL_TOKEN", internal_token)
        .env("PINGO_MKEK", mkek)
        .env("PINGO_GRPC_CERT", core_cert.to_str().unwrap())
        .env("PINGO_GRPC_KEY", core_key.to_str().unwrap())
        .env("PINGO_GRPC_CA", ca_pem.to_str().unwrap())
        .env("RUST_LOG", "error")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped());
    let mut child = cmd.spawn().expect("spawn pingogate-core platform");

    // gRPC server binds => platform branch ran (GrpcSnapshotSource constructed + tonic bound).
    let binds = port_binds_within(grpc_port, Duration::from_secs(10));

    let _ = child.kill();
    let _ = child.wait();

    assert!(
        binds,
        "platform mode gRPC listener did not come up on {grpc_addr} (GrpcSnapshotSource path)"
    );
}

// ---------------------------------------------------------------------------
// 3. mode_switch_via_env: PINGO_MODE dispatch logic (hermetic)
// ---------------------------------------------------------------------------

/// `PINGO_MODE` selects the startup branch (constitution: dual-mode). This
/// test verifies the dispatch without needing the network:
///
/// - `PINGO_MODE=standalone` (or unset/garbage) hits `run_standalone`, which
///   without a config file fails with a file-read error.
/// - `PINGO_MODE=platform` hits `run_platform`, which without
///   `PINGO_INTERNAL_TOKEN` fails with the platform-specific required-token
///   error.
///
/// Distinct error strings prove different branches ran. No certs or network
/// are needed.
#[test]
fn mode_switch_via_env() {
    // standalone branch: temp dir with no pingogate-core.yaml -> config path resolution fails -> non-zero exit.
    let temp_dir = std::env::temp_dir().join(format!(
        "pingogate-mode-switch-{}-{}",
        std::process::id(),
        common::free_port()
    ));
    std::fs::create_dir_all(&temp_dir).unwrap();

    let (failed_standalone, stderr_standalone) = {
        let mut cmd = Command::new(bin_path());
        cmd.env("PINGO_MODE", "standalone")
            .env("RUST_LOG", "error")
            .current_dir(&temp_dir)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::piped());
        let out = cmd.output().expect("spawn standalone");
        (
            !out.status.success(),
            String::from_utf8_lossy(&out.stderr).to_string(),
        )
    };
    assert!(
        failed_standalone,
        "standalone without config should fail; stderr: {stderr_standalone}"
    );
    // `run_standalone` -> `FileSnapshotSource::build_snapshot` ->
    // `read_to_string` failure surfaces as a Parse error.
    assert!(
        stderr_standalone.contains("pingogate-core failed to start")
            && (stderr_standalone.contains("config parse")
                || stderr_standalone.contains("No such file")
                || stderr_standalone.contains("cannot read config")),
        "standalone branch should fail with a config/parse error; got: {stderr_standalone}"
    );

    // --- platform branch: missing PINGO_INTERNAL_TOKEN -> platform-specific
    // error ---
    //
    // `run_platform` checks `PINGO_INTERNAL_TOKEN` first; missing -> fatal.
    // The error string is distinct from the standalone config error, proving
    // a different branch ran.
    let (failed_platform, stderr_platform) = spawn_mode("platform", &[]);
    assert!(
        failed_platform,
        "platform without PINGO_INTERNAL_TOKEN should fail; stderr: {stderr_platform}"
    );
    assert!(
        stderr_platform.contains("PINGO_INTERNAL_TOKEN is required in platform mode"),
        "platform branch should fail with the PINGO_INTERNAL_TOKEN required error; got: {stderr_platform}"
    );

    // --- unknown mode falls back to standalone ---
    //
    // `run()` matches "platform" explicitly and falls through to standalone
    // for any other value (including unset). Verify a garbage mode hits the
    // standalone branch (same config-parse error as explicit standalone).
    let (failed_unknown, stderr_unknown) = {
        let mut cmd = Command::new(bin_path());
        cmd.env("PINGO_MODE", "definitely-not-a-real-mode")
            .env("RUST_LOG", "error")
            .current_dir(&temp_dir)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::piped());
        let out = cmd.output().expect("spawn unknown mode");
        (
            !out.status.success(),
            String::from_utf8_lossy(&out.stderr).to_string(),
        )
    };
    assert!(
        failed_unknown,
        "unknown mode should fall back to standalone and fail without config; stderr: {stderr_unknown}"
    );
    // Same standalone config-parse error proves the standalone branch ran.
    assert!(
        stderr_unknown.contains("pingogate-core failed to start")
            && (stderr_unknown.contains("config parse")
                || stderr_unknown.contains("No such file")
                || stderr_unknown.contains("cannot read config")),
        "unknown mode should fall back to standalone (same config error); got: {stderr_unknown}"
    );

    // --- PINGO_MODE unset also falls back to standalone ---
    let (failed_unset, stderr_unset) = {
        let mut cmd = Command::new(bin_path());
        cmd.env_remove("PINGO_MODE")
            .env("RUST_LOG", "error")
            .current_dir(&temp_dir)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::piped());
        let out = cmd.output().expect("spawn unset mode");
        (
            !out.status.success(),
            String::from_utf8_lossy(&out.stderr).to_string(),
        )
    };
    assert!(
        failed_unset,
        "unset PINGO_MODE should fall back to standalone and fail; stderr: {stderr_unset}"
    );
    assert!(
        stderr_unset.contains("pingogate-core failed to start")
            && (stderr_unset.contains("config parse")
                || stderr_unset.contains("No such file")
                || stderr_unset.contains("cannot read config")),
        "unset mode should fall back to standalone (same config error as unknown mode); got: {stderr_unset}"
    );

    // Cleanup temp dir (best-effort).
    let _ = std::fs::remove_dir_all(&temp_dir);
}
