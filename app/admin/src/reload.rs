//! Reload orchestrator: load → parse → validate → build → swap → record.
//!
//! This is the single path that installs a new [`RuntimeSnapshot`], shared by
//! every trigger (SIGHUP, file-watch, Admin API). It is failure-atomic: a parse,
//! validation, or secret-resolution error leaves the active snapshot in place
//! and only updates the [`ReloadStatusStore`] with the reason (FR-019/020/021).
//! In-flight requests are never disturbed — they hold their own `Arc` snapshot.

use std::path::PathBuf;
use std::sync::Arc;

use pingo_config::{ConfigError, GatewayConfig, RuntimeSnapshot, SnapshotHolder};
use pingo_storage::SecretResolver;

use crate::status::ReloadStatusStore;

/// Owns the inputs needed to rebuild and atomically swap the runtime snapshot.
pub struct Reloader {
    holder: Arc<SnapshotHolder>,
    status: Arc<ReloadStatusStore>,
    config_path: PathBuf,
    resolver: Arc<dyn SecretResolver>,
}

impl Reloader {
    pub fn new(
        holder: Arc<SnapshotHolder>,
        status: Arc<ReloadStatusStore>,
        config_path: PathBuf,
        resolver: Arc<dyn SecretResolver>,
    ) -> Self {
        Self {
            holder,
            status,
            config_path,
            resolver,
        }
    }

    /// The status store, so callers (Admin API) can read the latest outcome.
    pub fn status(&self) -> &Arc<ReloadStatusStore> {
        &self.status
    }

    /// Build a candidate snapshot from disk and atomically swap it in. Records
    /// the outcome either way; the active snapshot is untouched on any error.
    /// Returns the new active version on success.
    pub fn reload(&self) -> Result<u64, ConfigError> {
        let next_version = self.holder.load().version().saturating_add(1);
        match self.build_candidate(next_version) {
            Ok(snapshot) => {
                self.holder.store(snapshot);
                self.status.record_success(next_version);
                Ok(next_version)
            }
            Err(err) => {
                self.status.record_failure(next_version, err.to_string());
                Err(err)
            }
        }
    }

    /// Read, parse, validate, and build — without touching the active snapshot.
    fn build_candidate(&self, version: u64) -> Result<Arc<RuntimeSnapshot>, ConfigError> {
        let yaml = std::fs::read_to_string(&self.config_path).map_err(|e| {
            ConfigError::Parse(format!(
                "cannot read config '{}': {e}",
                self.config_path.display()
            ))
        })?;
        let config = GatewayConfig::from_yaml(&yaml)?;
        RuntimeSnapshot::build(&config, self.resolver.as_ref(), version)
    }

    /// Trigger a reload from a named source, logging the outcome. Never panics
    /// and never propagates — suitable for signal/watch handler threads.
    pub fn trigger(&self, source: &str) {
        match self.reload() {
            Ok(version) => {
                tracing::info!(source, version, "configuration reloaded");
            }
            Err(err) => {
                // The reason is recorded in the status store; the active config
                // keeps serving. Error values never carry secret material (XX).
                tracing::warn!(source, error = %err, "reload rejected; active config unchanged");
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pingo_storage::EnvSecretResolver;
    use std::io::Write;

    /// A full config whose `routes:` block is the caller-supplied lines.
    fn config_with_routes(routes: &str) -> String {
        format!(
            "listeners:\n\
             \x20 public: {{ address: \"0.0.0.0:8080\" }}\n\
             \x20 admin: {{ address: \"127.0.0.1:9090\" }}\n\
             gateway_keys:\n\
             \x20 - {{ name: \"team\", secret_ref: \"env:PINGO_RELOAD_GW\" }}\n\
             providers:\n\
             \x20 - name: \"p\"\n\
             \x20\x20\x20 kind: \"openai-compatible\"\n\
             \x20\x20\x20 base_url: \"https://x\"\n\
             \x20\x20\x20 auth: {{ method: \"bearer\", key_ref: \"env:PINGO_RELOAD_KEY\" }}\n\
             \x20\x20\x20 capability_families: [\"generation.stateless\"]\n\
             routes:\n{routes}"
        )
    }

    fn write_tmp(name: &str, yaml: &str) -> PathBuf {
        let path =
            std::env::temp_dir().join(format!("pingo-reload-{}-{name}.yaml", std::process::id()));
        let mut f = std::fs::File::create(&path).unwrap();
        f.write_all(yaml.as_bytes()).unwrap();
        path
    }

    fn reloader_for(path: PathBuf) -> Reloader {
        std::env::set_var("PINGO_RELOAD_GW", "gw");
        std::env::set_var("PINGO_RELOAD_KEY", "sk");
        let yaml =
            config_with_routes("  - { alias: \"a\", provider: \"p\", upstream_model: \"m\" }\n");
        let cfg = GatewayConfig::from_yaml(&yaml).unwrap();
        let snap = RuntimeSnapshot::build(&cfg, &EnvSecretResolver, 1).unwrap();
        let holder = Arc::new(SnapshotHolder::new(snap));
        let status = Arc::new(ReloadStatusStore::new(1));
        Reloader::new(holder, status, path, Arc::new(EnvSecretResolver))
    }

    #[test]
    fn successful_reload_advances_version_and_applies_routes() {
        let yaml = config_with_routes(
            "  - { alias: \"a\", provider: \"p\", upstream_model: \"m\" }\n  - { alias: \"b\", provider: \"p\", upstream_model: \"m2\" }\n",
        );
        let path = write_tmp("ok", &yaml);
        let reloader = reloader_for(path.clone());
        let version = reloader.reload().unwrap();
        assert_eq!(version, 2);
        assert!(reloader.holder.load().route("b").is_some());
        assert_eq!(reloader.status.current().active_version, 2);
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn rejected_reload_keeps_active_snapshot() {
        // Candidate references an unknown provider → semantic validation fails.
        let yaml = config_with_routes(
            "  - { alias: \"a\", provider: \"missing\", upstream_model: \"m\" }\n",
        );
        let path = write_tmp("bad", &yaml);
        let reloader = reloader_for(path.clone());
        let err = reloader.reload().unwrap_err();
        assert!(matches!(err, ConfigError::Invalid(_)));
        // Active snapshot is still version 1 and still has the original route.
        assert_eq!(reloader.holder.load().version(), 1);
        assert!(reloader.holder.load().route("a").is_some());
        let status = reloader.status.current();
        assert_eq!(status.active_version, 1);
        assert_eq!(status.last_attempt_version, Some(2));
        let _ = std::fs::remove_file(path);
    }
}
