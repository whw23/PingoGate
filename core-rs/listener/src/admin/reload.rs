//! Reload orchestrator: build -> swap -> record.
//!
//! The single path that installs a new [`RuntimeSnapshot`], shared by every
//! trigger (SIGHUP, file-watch, Admin API). Failure-atomic: a build error
//! leaves the active snapshot in place and only updates the
//! [`ReloadStatusStore`] with the reason (FR-019/020/021). In-flight requests
//! are never disturbed - they hold their own `Arc` snapshot.
//!
//! Uses the [`SnapshotSource`] abstraction (T4) so standalone file-source and
//! future platform gRPC-source share the same reload path.

use std::sync::Arc;

use pingogate_snapshot::{RuntimeSnapshot, SnapshotHolder};
use pingogate_storage::{SnapshotError, SnapshotSource};

use crate::admin::status::ReloadStatusStore;

/// Owns the inputs needed to rebuild and atomically swap the runtime snapshot.
pub struct Reloader {
    holder: Arc<SnapshotHolder>,
    status: Arc<ReloadStatusStore>,
    source: Arc<dyn SnapshotSource<Snapshot = RuntimeSnapshot>>,
}

impl Reloader {
    pub fn new(
        holder: Arc<SnapshotHolder>,
        status: Arc<ReloadStatusStore>,
        source: Arc<dyn SnapshotSource<Snapshot = RuntimeSnapshot>>,
    ) -> Self {
        Self {
            holder,
            status,
            source,
        }
    }

    /// The status store, so callers (Admin API) can read the latest outcome.
    pub fn status(&self) -> &Arc<ReloadStatusStore> {
        &self.status
    }

    /// Build a candidate snapshot via the source and atomically swap it in.
    /// Records the outcome either way; the active snapshot is untouched on any
    /// error. Returns the new active version on success.
    pub fn reload(&self) -> Result<u64, SnapshotError> {
        let next_version = self.holder.load().version.saturating_add(1);
        match self.source.build_snapshot(next_version) {
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

    /// Trigger a reload from a named source, logging the outcome. Never panics
    /// and never propagates - suitable for signal/watch handler threads.
    pub fn trigger(&self, source: &str) {
        match self.reload() {
            Ok(version) => {
                tracing::info!(source, version, "configuration reloaded");
            }
            Err(err) => {
                tracing::warn!(source, error = %err, "reload rejected; active config unchanged");
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pingogate_snapshot::GatewayConfig;
    use pingogate_storage::FileSnapshotSource;
    use std::io::Write;
    use std::path::PathBuf;

    fn config_with_routes(models: &str) -> String {
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
             \x20\x20\x20 models:\n{models}"
        )
    }

    fn write_tmp(name: &str, yaml: &str) -> PathBuf {
        let path = std::env::temp_dir()
            .join(format!("pingo-reload-{}-{name}.yaml", std::process::id()));
        let mut f = std::fs::File::create(&path).unwrap();
        f.write_all(yaml.as_bytes()).unwrap();
        path
    }

    fn reloader_for(path: PathBuf) -> Reloader {
        std::env::set_var("PINGO_RELOAD_GW", "gw");
        std::env::set_var("PINGO_RELOAD_KEY", "sk");
        let yaml =
            config_with_routes("      - { alias: \"a\", upstream_model: \"m\" }\n");
        let cfg = GatewayConfig::from_yaml(&yaml).unwrap();
        let snap = RuntimeSnapshot::build(&cfg, &pingogate_storage::EnvSecretResolver, 1).unwrap();
        let holder = Arc::new(SnapshotHolder::new(snap));
        let status = Arc::new(ReloadStatusStore::new(1));
        let source = Arc::new(FileSnapshotSource::new(path));
        Reloader::new(holder, status, source)
    }

    #[test]
    fn successful_reload_advances_version_and_applies_routes() {
        let yaml = config_with_routes(
            "      - { alias: \"a\", upstream_model: \"m\" }\n      - { alias: \"b\", upstream_model: \"m2\" }\n",
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
        // Duplicate model alias fails validation (keeps active snapshot).
        let yaml = config_with_routes(
            "      - { alias: \"a\", upstream_model: \"m\" }\n      - { alias: \"a\", upstream_model: \"m2\" }\n",
        );
        let path = write_tmp("bad", &yaml);
        let reloader = reloader_for(path.clone());
        let err = reloader.reload().unwrap_err();
        assert!(matches!(err, SnapshotError::Validate(_)));
        assert_eq!(reloader.holder.load().version, 1);
        assert!(reloader.holder.load().route("a").is_some());
        let status = reloader.status.current();
        assert_eq!(status.active_version, 1);
        assert_eq!(status.last_attempt_version, Some(2));
        let _ = std::fs::remove_file(path);
    }
}
