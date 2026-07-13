//! PingoGate Rust kernel binary - composition root (constitution VIII).
//!
//! Bootstrap order: init tracing -> resolve config path -> build the initial
//! [`RuntimeSnapshot`] via [`FileSnapshotSource`] (T4 abstraction, standalone
//! mode) -> read `PINGO_ADMIN_TOKEN` (R1: missing = authed endpoints reject) ->
//! assemble the public + admin Pingora services -> run. The Pingora server owns
//! its own runtime, so `main` stays synchronous.
//!
//! Platform mode position is reserved: the same binary will accept a gRPC
//! snapshot source in S2; S1 only runs standalone.

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

mod signal;
mod watch;

/// Default config file name when `--config` is not given.
const DEFAULT_CONFIG_PATH: &str = "pingogate-core.yaml";
/// Env var holding the standalone-mode admin token (R1).
const ADMIN_TOKEN_ENV: &str = "PINGO_ADMIN_TOKEN";
/// Opt-in file-watch interval, in seconds.
const WATCH_INTERVAL_ENV: &str = "PINGO_WATCH_INTERVAL_SECS";

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

/// Read the admin token from the environment (R1). Returns `None` when unset or
/// empty - authed admin endpoints then reject every request.
fn read_admin_token() -> Option<SecretString> {
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
