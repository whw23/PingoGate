//! PingoGate gateway binary — composition root (constitution VIII).
//!
//! Bootstrap order: init tracing → load + parse config → resolve secrets and
//! build the immutable [`RuntimeSnapshot`] (a missing secret fails startup, not
//! a live request) → assemble the public + admin Pingora services → run. The
//! Pingora server owns its own runtime, so `main` stays synchronous.

use std::sync::Arc;

use pingo_config::{GatewayConfig, RuntimeSnapshot, SnapshotHolder};
use pingo_core::SecretString;
use pingo_listener::{build_admin_service, build_public_service};
use pingo_storage::{EnvSecretResolver, SecretResolver};
use pingora::server::Server;
use tracing_subscriber::EnvFilter;

/// Secret reference for the bootstrap admin token (resolved at startup, XX).
const ADMIN_TOKEN_REF: &str = "env:PINGO_ADMIN_TOKEN";
const DEFAULT_CONFIG_PATH: &str = "pingogate.yaml";

fn main() {
    init_tracing();
    if let Err(e) = run() {
        // Startup failures are fatal and surfaced on stderr; no secret material
        // is ever included in these error values (constitution XX).
        eprintln!("pingogate failed to start: {e}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), Box<dyn std::error::Error>> {
    let config_path = config_path_from_args();
    let yaml = std::fs::read_to_string(&config_path)
        .map_err(|e| format!("cannot read config '{config_path}': {e}"))?;
    let config = GatewayConfig::from_yaml(&yaml)?;

    let resolver = EnvSecretResolver;
    let snapshot = RuntimeSnapshot::build(&config, &resolver, 1)?;
    let holder = Arc::new(SnapshotHolder::new(snapshot));
    let admin_token: SecretString = resolver.resolve(ADMIN_TOKEN_REF)?;

    let mut server = Server::new(None)?;
    server.bootstrap();

    let public_addr = config.listeners.public.address.clone();
    let admin_addr = config.listeners.admin.address.clone();

    let public = build_public_service(&server.configuration, holder.clone(), &public_addr);
    let admin = build_admin_service(admin_token, holder, &admin_addr);
    server.add_service(public);
    server.add_service(admin);

    tracing::info!(public = %public_addr, admin = %admin_addr, "pingogate starting");
    server.run_forever()
}

/// Resolve the config path from `--config <path>` / `--config=<path>`, defaulting
/// to `pingogate.yaml` in the working directory.
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

/// Initialize structured logging, honoring `RUST_LOG` and defaulting to `info`.
fn init_tracing() {
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    tracing_subscriber::fmt().with_env_filter(filter).init();
}
