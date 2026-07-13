//! Snapshot source abstraction: standalone (file) vs platform (gRPC).
//!
//! S1 introduces the [`SnapshotSource`] trait and a minimal [`FileSnapshotSource`]
//! stub; the full YAML-parse + validate + secret-resolve + snapshot-build
//! implementation lands in T5, which defines `RuntimeSnapshot` in the
//! `pingogate-snapshot` crate. S2 adds `GrpcSnapshotSource` for platform mode.
//!
//! T4/T5 dependency note: the brief's target signature is
//! `fn build_snapshot(&self, version: u64) -> Result<Arc<RuntimeSnapshot>, SnapshotError>`.
//! `RuntimeSnapshot` does not exist yet (T5 owns it), so the trait is defined
//! with an associated [`SnapshotSource::Snapshot`] type that compiles without a
//! hard dependency on `pingogate-snapshot`. When T5 lands `RuntimeSnapshot`, it
//! either pins `type Snapshot = RuntimeSnapshot` on the `FileSnapshotSource`
//! impl or refactors the trait to the concrete return form - and fills in the
//! `SnapshotSource` impl + the `file_source_builds_snapshot_from_yaml` test.

use std::path::{Path, PathBuf};
use std::sync::Arc;

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

/// Builds a `RuntimeSnapshot` from some config source.
///
/// `Snapshot` is the snapshot type the impl produces; for `FileSnapshotSource`
/// in T5 this becomes `pingogate_snapshot::RuntimeSnapshot`.
pub trait SnapshotSource: Send + Sync {
    /// The snapshot type produced by this source.
    type Snapshot: Send + Sync;

    /// Build a snapshot tagged with `version`. Implementations must be
    /// idempotent w.r.t. the underlying config: the same version built from the
    /// same config yields an equivalent snapshot.
    fn build_snapshot(&self, version: u64) -> Result<Arc<Self::Snapshot>, SnapshotError>;
}

/// File-backed snapshot source for standalone mode. Reads a YAML config,
/// resolves `env:` secret references, and builds a `RuntimeSnapshot`.
///
/// **Status (T4):** Only the struct + constructor ship here. The
/// [`SnapshotSource`] impl (YAML parse + validate + secret resolve + snapshot
/// build) and the `file_source_builds_snapshot_from_yaml` test are deferred to
/// T5, which introduces `RuntimeSnapshot` in `pingogate-snapshot`.
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
