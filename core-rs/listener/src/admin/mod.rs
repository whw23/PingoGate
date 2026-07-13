//! Admin API service factory and module re-exports.
//!
//! Bundles the control-plane surface: [`AdminApp`] (Pingora `ServeHttp` glue),
//! [`BootstrapAuth`] (R1 single-token admin auth), [`AdminHandler`] (6
//! endpoints via `authorize` boundary), [`Reloader`] (reload orchestrator),
//! and [`ReloadStatusStore`] (lock-free reload status).

mod app;
mod auth;
mod handler;
mod metrics_endpoint;
mod reload;
mod status;

use std::sync::Arc;

use pingogate_core_types::SecretString;
use pingogate_pipeline::Metrics;
use pingogate_snapshot::SnapshotHolder;

pub use app::build_admin_service;
pub use reload::Reloader;
pub use status::ReloadStatusStore;

/// Configuration for the admin service, bundled to keep the factory within the
/// parameter limit (constitution V). `admin_token` is `None` when
/// `PINGO_ADMIN_TOKEN` is unset at startup - the authed endpoints then reject
/// every request (R1: missing = reject, no unauthenticated degradation).
pub struct AdminServiceConfig {
    pub admin_token: Option<SecretString>,
    pub holder: Arc<SnapshotHolder>,
    pub reloader: Arc<Reloader>,
    pub metrics: Arc<Metrics>,
    pub address: String,
}
