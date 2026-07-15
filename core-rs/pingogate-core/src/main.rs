//! PingoGate Rust kernel binary - composition root (constitution VIII).
//!
//! Bootstrap order: init tracing -> resolve config path -> build the initial
//! [`RuntimeSnapshot`] via [`FileSnapshotSource`] (T4 abstraction, standalone
//! mode) -> read `PINGO_ADMIN_TOKEN` (R1: missing = authed endpoints reject) ->
//! assemble the public + admin Pingora services -> run. The Pingora server owns
//! its own runtime, so `main` stays synchronous.
//!
//! Platform mode (S2+): `PINGO_MODE=platform` starts the gRPC server (mTLS +
//! internal token, spec §12A) that accepts snapshot pushes from the Go control
//! plane. S1 only ships a stub server to prove connectivity (T11); real
//! snapshot application lands in S2.

use std::net::SocketAddr;
use std::path::PathBuf;
use std::str::FromStr;
use std::sync::Arc;
use std::time::Duration;

use pingogate_core_types::SecretString;
use pingogate_listener::{
    build_admin_service, build_public_service, AdminServiceConfig, PublicServiceConfig,
};
use pingogate_pipeline::{Metrics, StaticKeyAuth};
use pingogate_snapshot::{GatewayConfig, SnapshotHolder};
use pingogate_storage::{FileSnapshotSource, SnapshotSource};
use pingora::server::Server;
use tracing_subscriber::EnvFilter;

mod grpc;
mod grpc_auth;
mod signal;
mod watch;

/// Default config file name when `--config` is not given.
const DEFAULT_CONFIG_PATH: &str = "pingogate-core.yaml";
/// Env var holding the standalone-mode admin token (R1).
const ADMIN_TOKEN_ENV: &str = "PINGO_ADMIN_TOKEN";
/// Opt-in file-watch interval, in seconds.
const WATCH_INTERVAL_ENV: &str = "PINGO_WATCH_INTERVAL_SECS";
/// Env var selecting standalone vs platform bootstrap (`standalone` | `platform`).
const MODE_ENV: &str = "PINGO_MODE";
/// Env var holding the shared gRPC internal token (spec §12A). Required in
/// platform mode; fatal if missing.
const INTERNAL_TOKEN_ENV: &str = "PINGO_INTERNAL_TOKEN";
/// Env var: path to the server (Rust) TLS certificate PEM (platform mode).
const GRPC_CERT_ENV: &str = "PINGO_GRPC_CERT";
/// Env var: path to the server (Rust) TLS private key PEM (platform mode).
const GRPC_KEY_ENV: &str = "PINGO_GRPC_KEY";
/// Env var: path to the CA certificate PEM that signed the Go client cert.
const GRPC_CA_ENV: &str = "PINGO_GRPC_CA";
/// Env var holding the 32-byte AES-GCM master key (constitution XX KeyVault).
/// Required in platform mode; missing or not exactly 32 bytes is fatal. The
/// key encrypts provider keys at rest (Go DB) and decrypts them once per
/// hot-path request inside the Rust kernel.
const MKEK_ENV: &str = "PINGO_MKEK";
/// Default gRPC listen address when `--grpc-addr` is not given (platform mode).
const DEFAULT_GRPC_ADDR: &str = "127.0.0.1:9091";

fn main() {
    init_tracing();
    if let Err(e) = run() {
        // Startup failures are fatal; no secret material is included in error
        // values (constitution XX).
        eprintln!("pingogate-core failed to start: {e}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), Box<dyn std::error::Error>> {
    let mode = std::env::var(MODE_ENV).unwrap_or_else(|_| "standalone".to_string());
    match mode.as_str() {
        "platform" => run_platform(),
        _ => run_standalone(),
    }
}

/// Standalone mode: Pingora data plane + admin API, config from local YAML.
fn run_standalone() -> Result<(), Box<dyn std::error::Error>> {
    let config_path = config_path_from_args();

    // Build the initial snapshot via FileSnapshotSource (T4 abstraction).
    let source = Arc::new(FileSnapshotSource::new(PathBuf::from(&config_path)));
    let snapshot = source.build_snapshot(1)?;
    let holder = Arc::new(SnapshotHolder::new(snapshot));

    // R1: read the admin token. Missing = authed endpoints reject (no
    // unauthenticated degradation). /healthz and /metrics are unaffected.
    let admin_token = read_admin_token();
    if admin_token.is_none() {
        tracing::warn!(
            env = ADMIN_TOKEN_ENV,
            "admin token not set; authed admin endpoints will reject (standalone mode R1)"
        );
    }

    // Reload orchestrator: one path for SIGHUP, file-watch, and Admin API.
    let status = Arc::new(pingogate_listener::ReloadStatusStore::new(1));
    let reloader = Arc::new(pingogate_listener::Reloader::new(
        holder.clone(),
        status,
        source.clone(),
    ));
    signal::spawn_sighup_handler(reloader.clone());
    if let Some(interval) = watch_interval_from_env() {
        watch::spawn_file_watcher(reloader.clone(), PathBuf::from(&config_path), interval);
    }

    // Parse the YAML once for listener addresses (not in RuntimeSnapshot).
    let yaml = std::fs::read_to_string(&config_path)
        .map_err(|e| format!("cannot read config '{config_path}': {e}"))?;
    let config = GatewayConfig::from_yaml(&yaml)?;
    let public_addr = config.listeners.public.address.clone();
    let admin_addr = config.listeners.admin.address.clone();

    let mut server = Server::new(None)?;
    server.bootstrap();

    // One metrics registry, shared between the data plane and the Admin API.
    let metrics = Arc::new(Metrics::new());
    let auth: Arc<dyn pingogate_pipeline::KeyAuth> = Arc::new(StaticKeyAuth);

    let public = build_public_service(PublicServiceConfig {
        conf: &server.configuration,
        holder: holder.clone(),
        metrics: metrics.clone(),
        auth,
        address: &public_addr,
    });
    let admin = build_admin_service(AdminServiceConfig {
        admin_token,
        holder,
        reloader,
        metrics,
        address: admin_addr.clone(),
    });
    server.add_service(public);
    server.add_service(admin);

    tracing::info!(public = %public_addr, admin = %admin_addr, "pingogate-core starting");
    server.run_forever()
}

/// Platform mode: start the gRPC server (mTLS + internal token) that accepts
/// snapshot pushes + KeyVault Encrypt/Decrypt calls from the Go control plane.
/// S2 ships the real AES-GCM KeyVault (loaded from `PINGO_MKEK`) and the real
/// `SnapshotService` (proto `Snapshot` -> `RuntimeSnapshot` -> `ArcSwap`). The
/// `HealthService` reports `ready: false` until the first snapshot is applied
/// (spec §12B). S3+ will also run the Pingora data plane alongside the gRPC
/// server; S2 runs the gRPC server only.
fn run_platform() -> Result<(), Box<dyn std::error::Error>> {
    // rustls 0.23 requires a process-wide CryptoProvider. Install the `ring`
    // provider before tonic's TLS stack touches rustls. Safe to call once at
    // startup; a second install returns Err which we ignore (idempotent intent).
    let _ = rustls::crypto::ring::default_provider().install_default();

    let internal_token = std::env::var(INTERNAL_TOKEN_ENV).map_err(|_| {
        format!("{INTERNAL_TOKEN_ENV} is required in platform mode (spec §12B)")
    })?;
    if internal_token.is_empty() {
        return Err(format!("{INTERNAL_TOKEN_ENV} must not be empty").into());
    }

    let server_cert = PathBuf::from(
        std::env::var(GRPC_CERT_ENV)
            .map_err(|_| format!("{GRPC_CERT_ENV} is required in platform mode"))?,
    );
    let server_key = PathBuf::from(
        std::env::var(GRPC_KEY_ENV)
            .map_err(|_| format!("{GRPC_KEY_ENV} is required in platform mode"))?,
    );
    let client_ca = PathBuf::from(
        std::env::var(GRPC_CA_ENV)
            .map_err(|_| format!("{GRPC_CA_ENV} is required in platform mode"))?,
    );

    // Load the 32-byte master key (PINGO_MKEK) for the AES-GCM KeyVault.
    // Missing or wrong length is fatal: without a valid MKEK the kernel cannot
    // decrypt provider keys for the hot path (constitution XX). No secret
    // material is included in the error value.
    let mkek = load_master_key()?;

    // Construct the AES-GCM KeyVault + gRPC service. The same keyvault instance
    // is shared (via Arc) between the gRPC control-plane service and, from S3,
    // the hot-path KeyVault trait wrapper.
    let keyvault = Arc::new(pingogate_keyvault::AesGcmKeyVault::from_master_key(&mkek));
    let keyvault_service = pingogate_keyvault::KeyVaultGrpcService::new(keyvault);

    // Platform-mode snapshot source: an empty holder (version 0 sentinel) that
    // the Go control plane fills via `PushSnapshot`. The data plane (S3+) will
    // read from this holder; the `HealthService` reports not-ready until the
    // first snapshot lands (spec §12B).
    let holder = Arc::new(pingogate_snapshot::SnapshotHolder::empty());
    let snapshot_source = Arc::new(pingogate_storage::GrpcSnapshotSource::new(holder));

    let addr_str = grpc_addr_from_args();
    let addr = SocketAddr::from_str(&addr_str)
        .map_err(|e| format!("invalid gRPC address '{addr_str}': {e}"))?;

    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;
    runtime.block_on(grpc::serve_grpc(
        addr,
        server_cert,
        server_key,
        client_ca,
        internal_token,
        keyvault_service,
        snapshot_source,
    ))?;
    Ok(())
}

/// Resolve the config path from `--config <path>` / `--config=<path>`, defaulting
/// to `pingogate-core.yaml` in the working directory.
fn config_path_from_args() -> String {
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        if arg == "--config" {
            if let Some(path) = args.next() {
                return path;
            }
        } else if let Some(path) = arg.strip_prefix("--config=") {
            return path.to_string();
        }
    }
    DEFAULT_CONFIG_PATH.to_string()
}

/// Resolve the gRPC listen address from `--grpc-addr <addr>` /
/// `--grpc-addr=<addr>`, defaulting to `127.0.0.1:9091` (spec §12A: loopback
/// only). Used in platform mode.
fn grpc_addr_from_args() -> String {
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        if arg == "--grpc-addr" {
            if let Some(addr) = args.next() {
                return addr;
            }
        } else if let Some(addr) = arg.strip_prefix("--grpc-addr=") {
            return addr.to_string();
        }
    }
    DEFAULT_GRPC_ADDR.to_string()
}

/// Read the admin token from the environment (R1). Returns `None` when unset or
/// empty - authed admin endpoints then reject every request.
fn read_admin_token() -> Option<SecretString> {
    match std::env::var(ADMIN_TOKEN_ENV) {
        Ok(value) if !value.is_empty() => Some(SecretString::new(value)),
        _ => None,
    }
}

/// Load the 32-byte AES-GCM master key (`PINGO_MKEK`) from the environment
/// (platform mode). The env var's UTF-8 bytes are taken as the raw 32-byte key;
/// missing, empty, or not exactly 32 bytes is fatal. The error message carries
/// no secret material (constitution XX).
fn load_master_key() -> Result<[u8; 32], Box<dyn std::error::Error>> {
    let raw = std::env::var(MKEK_ENV).map_err(|_| {
        format!("{MKEK_ENV} is required in platform mode (32-byte AES-GCM master key)")
    })?;
    let bytes = raw.into_bytes();
    if bytes.len() != 32 {
        return Err(format!(
            "{MKEK_ENV} must be exactly 32 bytes, got {} bytes",
            bytes.len()
        )
        .into());
    }
    let mut mkek = [0u8; 32];
    mkek.copy_from_slice(&bytes);
    Ok(mkek)
}

/// Initialize structured logging, honoring `RUST_LOG` and defaulting to `info`.
fn init_tracing() {
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    tracing_subscriber::fmt().with_env_filter(filter).init();
}

/// Parse the opt-in file-watch interval from `PINGO_WATCH_INTERVAL_SECS`. A
/// missing, zero, or unparseable value disables the watcher; SIGHUP and the
/// Admin API remain available as reload triggers.
fn watch_interval_from_env() -> Option<Duration> {
    std::env::var(WATCH_INTERVAL_ENV)
        .ok()?
        .parse::<u64>()
        .ok()
        .filter(|secs| *secs > 0)
        .map(Duration::from_secs)
}
