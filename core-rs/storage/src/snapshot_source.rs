//! Snapshot source abstraction: standalone (file) vs platform (gRPC).
//!
//! S1 implements [`FileSnapshotSource`] for standalone mode: read YAML ->
//! [`GatewayConfig`] -> semantic validate -> resolve `env:` secrets via
//! [`EnvSecretResolver`] -> build an immutable [`RuntimeSnapshot`]. S2 adds a
//! `GrpcSnapshotSource` for platform mode (Go control plane pushes ciphertext).

use std::path::{Path, PathBuf};
use std::sync::Arc;

use pingogate_snapshot::{ConfigError, GatewayConfig, RuntimeSnapshot, SnapshotHolder};

use crate::secret::EnvSecretResolver;

/// Errors raised while building a `RuntimeSnapshot` from a config source.
#[derive(Debug, thiserror::Error)]
pub enum SnapshotError {
    #[error("config parse: {0}")]
    Parse(String),
    #[error("config validate: {0}")]
    Validate(String),
    #[error("secret resolve: {0}")]
    Secret(String),
}

impl From<ConfigError> for SnapshotError {
    fn from(err: ConfigError) -> Self {
        match err {
            ConfigError::Parse(msg) => SnapshotError::Parse(msg),
            ConfigError::Invalid(_) => SnapshotError::Validate(err.to_string()),
            ConfigError::Secret { source, .. } => SnapshotError::Secret(source.to_string()),
        }
    }
}

/// Builds a `RuntimeSnapshot` from some config source.
///
/// `Snapshot` is the snapshot type the impl produces; for [`FileSnapshotSource`]
/// this is [`RuntimeSnapshot`].
pub trait SnapshotSource: Send + Sync {
    /// The snapshot type produced by this source.
    type Snapshot: Send + Sync;

    /// Build a snapshot tagged with `version`. Implementations must be
    /// idempotent w.r.t. the underlying config: the same version built from the
    /// same config yields an equivalent snapshot.
    fn build_snapshot(&self, version: u64) -> Result<Arc<Self::Snapshot>, SnapshotError>;
}

/// File-backed snapshot source for standalone mode. Reads a YAML config,
/// resolves `env:` secret references, and builds a [`RuntimeSnapshot`].
#[derive(Debug)]
pub struct FileSnapshotSource {
    path: PathBuf,
}

impl FileSnapshotSource {
    /// Create a source that reads its config from `path` (YAML, standalone
    /// mode config contract).
    pub fn new(path: PathBuf) -> Self {
        Self { path }
    }

    /// Path of the YAML config this source reads.
    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl SnapshotSource for FileSnapshotSource {
    type Snapshot = RuntimeSnapshot;

    fn build_snapshot(&self, version: u64) -> Result<Arc<RuntimeSnapshot>, SnapshotError> {
        let yaml = std::fs::read_to_string(&self.path).map_err(|e| SnapshotError::Parse(e.to_string()))?;
        let config = GatewayConfig::from_yaml(&yaml)?;
        // `RuntimeSnapshot::build` runs `validate_semantics` + resolves secrets
        // eagerly so a missing key fails the build, not a live request (FR-022).
        let snapshot = RuntimeSnapshot::build(&config, &EnvSecretResolver, version)?;
        Ok(snapshot)
    }
}

/// Platform-mode snapshot source backed by a [`SnapshotHolder`].
///
/// The Go control plane pushes proto `Snapshot` messages over gRPC; the binary
/// crate (`pingogate-core::grpc`) converts each proto message into a
/// [`RuntimeSnapshot`] (providers carry AES-GCM **ciphertext** in
/// [`ResolvedProvider::encrypted_key`], not plaintext) and calls [`apply`]
/// to atomically swap it into the holder (constitution XII: `ArcSwap`).
///
/// This struct is deliberately proto-agnostic: it only holds the holder and
/// exposes [`apply`] to store a pre-built snapshot. The proto -> RuntimeSnapshot
/// conversion lives in the binary crate (which owns the tonic codegen), keeping
/// `pingogate-storage` free of protobuf dependencies (constitution VIII).
///
/// [`apply`]: GrpcSnapshotSource::apply
pub struct GrpcSnapshotSource {
    holder: Arc<SnapshotHolder>,
}

impl GrpcSnapshotSource {
    /// Wrap a [`SnapshotHolder`] (typically constructed via
    /// [`SnapshotHolder::empty`] in platform mode) so the gRPC layer can swap
    /// snapshots into it.
    pub fn new(holder: Arc<SnapshotHolder>) -> Self {
        Self { holder }
    }

    /// Read-only access to the underlying holder (e.g. for the health service
    /// to report the active version).
    pub fn holder(&self) -> &Arc<SnapshotHolder> {
        &self.holder
    }

    /// Atomically install `snapshot` as the active runtime snapshot
    /// (constitution XII: in-flight requests keep using the old `Arc`). The
    /// caller is responsible for proto -> RuntimeSnapshot conversion and
    /// semantic validation before calling this.
    pub fn apply(&self, snapshot: Arc<RuntimeSnapshot>) {
        self.holder.store(snapshot);
    }
}

impl SnapshotSource for GrpcSnapshotSource {
    type Snapshot = RuntimeSnapshot;

    /// Platform mode: the snapshot is built off-band by the gRPC layer and
    /// stored in the holder. `build_snapshot` simply returns the current
    /// active snapshot; `version` is accepted for trait conformance but the
    /// caller should read `holder.load().version` for the authoritative value.
    fn build_snapshot(&self, _version: u64) -> Result<Arc<RuntimeSnapshot>, SnapshotError> {
        Ok(self.holder.load_full())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn file_source_builds_snapshot_from_yaml() {
        let yaml = "listeners:\n  public: { address: \"0.0.0.0:8080\" }\n  admin: { address: \"127.0.0.1:9090\" }\ngateway_keys:\n  - { name: \"a\", secret_ref: \"env:PINGO_KEY_ALPHA\" }\nproviders:\n  - { name: \"openai-main\", kind: \"openai-compatible\", base_url: \"https://api.openai.com\", auth: { method: \"bearer\", key_ref: \"env:OPENAI_API_KEY\" }, capability_families: [\"generation.stateless\"] }\nroutes:\n  - { alias: \"gpt-4o\", provider: \"openai-main\", upstream_model: \"gpt-4o\" }\nupstream: { timeout_ms: 60000 }\n";
        let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../target/test_config.yaml");
        std::fs::write(&path, yaml).unwrap();
        std::env::set_var("PINGO_KEY_ALPHA", "test-key");
        std::env::set_var("OPENAI_API_KEY", "sk-test");
        let source = FileSnapshotSource::new(path);
        let snap = source.build_snapshot(1).unwrap();
        assert_eq!(snap.version, 1);
        assert_eq!(snap.providers.len(), 1);
    }
}
