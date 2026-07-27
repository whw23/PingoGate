//! PingoGate Rust kernel binary - composition root (constitution VIII).
//!
//! Bootstrap: init tracing -> resolve config -> build `RuntimeSnapshot` via
//! `FileSnapshotSource` (standalone) -> read `PINGO_ADMIN_TOKEN` (R1) ->
//! assemble Pingora services -> run. Platform mode (`PINGO_MODE=platform`)
//! delegates to [`platform::run_platform`], which starts both the gRPC server
//! (mTLS + internal token, spec §12A; Go snapshot pushes + KeyVault calls) and
//! the Pingora data plane (VirtualKeyAuth + KeyVault decrypt + usage push).

use std::path::PathBuf;
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
mod platform;
mod signal;
mod usage_client;
mod watch;

/// Default config file name when `--config` is not given.
const DEFAULT_CONFIG_PATH: &str = "pingogate-core.yaml";
/// Env var: standalone-mode admin token (R1). Missing = authed endpoints reject.
pub(crate) const ADMIN_TOKEN_ENV: &str = "PINGO_ADMIN_TOKEN";
/// Opt-in file-watch interval, in seconds.
const WATCH_INTERVAL_ENV: &str = "PINGO_WATCH_INTERVAL_SECS";
/// Env var selecting standalone vs platform bootstrap (`standalone` | `platform`).
const MODE_ENV: &str = "PINGO_MODE";

fn main() {
    init_tracing();
    if let Err(e) = run() {
        // Startup failures are fatal; no secret material in error (constitution XX).
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

    let metrics = Arc::new(Metrics::new());
    let auth: Arc<dyn pingogate_pipeline::KeyAuth> = Arc::new(StaticKeyAuth);

    let public = build_public_service(PublicServiceConfig {
        conf: &server.configuration,
        holder: holder.clone(),
        metrics: metrics.clone(),
        auth,
        keyvault: None, // standalone: env-resolved plaintext keys (constitution XII)
        usage: None,    // standalone: no Go control plane to push usage to
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

/// Platform mode: delegate to [`platform::run_platform`], which starts the
/// gRPC server (mTLS + internal token) for Go snapshot pushes + KeyVault
/// calls, and the Pingora data plane (VirtualKeyAuth + KeyVault decrypt +
/// usage push). See [`platform`] for the full architecture.
fn run_platform() -> Result<(), Box<dyn std::error::Error>> {
    platform::run_platform()
}

/// Resolve `--config <path>` / `--config=<path>`, defaulting to
/// `pingogate-core.yaml` in the working directory.
fn config_path_from_args() -> String {
    arg_value("--config").unwrap_or_else(|| DEFAULT_CONFIG_PATH.to_string())
}

/// Shared `--flag <value>` / `--flag=<value>` parser. Returns the first match.
pub(crate) fn arg_value(flag: &str) -> Option<String> {
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        if arg == flag {
            if let Some(v) = args.next() {
                return Some(v);
            }
        } else if let Some(v) = arg.strip_prefix(&format!("{flag}=")) {
            return Some(v.to_string());
        }
    }
    None
}

/// Read the admin token from the environment (R1). Returns `None` when unset or
/// empty - authed admin endpoints then reject every request.
pub(crate) fn read_admin_token() -> Option<SecretString> {
    match std::env::var(ADMIN_TOKEN_ENV) {
        Ok(value) if !value.is_empty() => Some(SecretString::new(value)),
        _ => None,
    }
}

/// Initialize structured logging, honoring `RUST_LOG` and defaulting to `info`.
fn init_tracing() {
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    tracing_subscriber::fmt().with_env_filter(filter).init();
}

/// Parse the opt-in file-watch interval from `PINGO_WATCH_INTERVAL_SECS`.
/// Missing, zero, or unparseable disables the watcher; SIGHUP and Admin API
/// remain available as reload triggers.
fn watch_interval_from_env() -> Option<Duration> {
    std::env::var(WATCH_INTERVAL_ENV)
        .ok()?
        .parse::<u64>()
        .ok()
        .filter(|secs| *secs > 0)
        .map(Duration::from_secs)
}
