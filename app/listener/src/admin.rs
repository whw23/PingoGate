//! Control-plane service factory.
//!
//! Builds the Admin API `ServeHttp` service, bound to a (typically loopback)
//! admin listener kept off the public data path. The reload orchestrator is
//! shared with the SIGHUP/file-watch triggers so every reload path — signal,
//! file-watch, and the `/reload` endpoint — funnels through one snapshot swap
//! (FR-019); the metrics registry is shared with the data plane so `/metrics`
//! renders live counters. Pure factory: the server lifecycle stays in the binary
//! (VIII).

use std::sync::Arc;

use pingo_admin::{AdminApp, Reloader};
use pingo_config::SnapshotHolder;
use pingo_core::SecretString;
use pingo_pipeline::Metrics;
use pingora::services::listening::Service;

/// The shared handles and address the Admin API service binds. Bundled into one
/// struct so the factory stays within the parameter limit (constitution).
pub struct AdminServiceConfig {
    pub admin_token: SecretString,
    pub holder: Arc<SnapshotHolder>,
    pub reloader: Arc<Reloader>,
    pub metrics: Arc<Metrics>,
    pub address: String,
}

/// Build the Admin API service from its configuration.
pub fn build_admin_service(config: AdminServiceConfig) -> Service<AdminApp> {
    let mut service = Service::new(
        "pingogate-admin".to_string(),
        AdminApp::new(
            config.admin_token,
            config.holder,
            config.reloader,
            config.metrics,
        ),
    );
    service.add_tcp(&config.address);
    service
}
