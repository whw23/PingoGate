//! Shared harness for the Admin API contract tests.
//!
//! Builds a real [`AdminHandler`] over a temp-file config and an
//! [`EnvSecretResolver`], plus admin/non-admin [`AuthContext`]s, so each
//! contract test drives the public handler surface directly — asserting the
//! status codes and JSON shapes the admin-api contract specifies, including the
//! authorization boundary, without standing up a live Pingora connection.
#![allow(clippy::unwrap_used, clippy::expect_used, dead_code)]

use std::io::Write;
use std::path::PathBuf;
use std::sync::Arc;

use pingo_admin::{AdminHandler, ReloadStatusStore, Reloader};
use pingo_config::{GatewayConfig, RuntimeSnapshot, SnapshotHolder};
use pingo_core::{AuthContext, Principal};
use pingo_pipeline::Metrics;
use pingo_storage::EnvSecretResolver;
use serde_json::Value;

/// A valid base config whose `routes:` block is the caller-supplied lines. The
/// secrets are env references resolved through the process environment.
pub fn config_with_routes(routes: &str) -> String {
    format!(
        "listeners:\n\
         \x20 public: {{ address: \"127.0.0.1:8080\" }}\n\
         \x20 admin: {{ address: \"127.0.0.1:9090\" }}\n\
         gateway_keys:\n\
         \x20 - {{ name: \"team\", secret_ref: \"env:PINGO_ADMIN_IT_GW\" }}\n\
         providers:\n\
         \x20 - name: \"p\"\n\
         \x20\x20\x20 kind: \"openai-compatible\"\n\
         \x20\x20\x20 base_url: \"https://x\"\n\
         \x20\x20\x20 auth: {{ method: \"bearer\", key_ref: \"env:PINGO_ADMIN_IT_KEY\" }}\n\
         \x20\x20\x20 capability_families: [\"generation.stateless\"]\n\
         routes:\n{routes}"
    )
}

/// The default single-route config used by most contract tests.
pub fn base_config() -> String {
    config_with_routes("  - { alias: \"a\", provider: \"p\", upstream_model: \"m\" }\n")
}

/// A built [`AdminHandler`] plus the on-disk config path it reloads from.
pub struct Fixture {
    pub handler: AdminHandler,
    pub config_path: PathBuf,
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.config_path);
    }
}

/// Build a handler over a temp config file containing `yaml`.
pub fn fixture(name: &str, yaml: &str) -> Fixture {
    std::env::set_var("PINGO_ADMIN_IT_GW", "gw-secret");
    std::env::set_var("PINGO_ADMIN_IT_KEY", "sk-upstream");

    let config_path =
        std::env::temp_dir().join(format!("pingo-admin-it-{}-{name}.yaml", std::process::id()));
    let mut f = std::fs::File::create(&config_path).unwrap();
    f.write_all(yaml.as_bytes()).unwrap();

    let config = GatewayConfig::from_yaml(yaml).unwrap();
    let snapshot = RuntimeSnapshot::build(&config, &EnvSecretResolver, 1).unwrap();
    let holder = Arc::new(SnapshotHolder::new(snapshot));
    let status = Arc::new(ReloadStatusStore::new(1));
    let reloader = Arc::new(Reloader::new(
        holder.clone(),
        status,
        config_path.clone(),
        Arc::new(EnvSecretResolver),
    ));
    Fixture {
        handler: AdminHandler::new(holder, reloader, Arc::new(Metrics::new())),
        config_path,
    }
}

/// An authenticated admin caller (as `BootstrapAuth` would produce).
pub fn admin() -> AuthContext {
    AuthContext::new(Principal::admin("bootstrap"))
}

/// A data-plane gateway-key caller — must never pass an admin boundary.
pub fn gateway_key() -> AuthContext {
    AuthContext::new(Principal::gateway_key("team"))
}

/// Parse a handler response body as JSON.
pub fn body_json(resp: &http::Response<Vec<u8>>) -> Value {
    serde_json::from_slice(resp.body()).expect("admin response body is JSON")
}
