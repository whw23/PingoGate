//! `pingogate-listener` - server and listener assembly.
//!
//! Builds the Pingora services: the public `HttpProxy` service that runs the
//! request pipeline, and the admin `ServeHttp` service. The server lifecycle
//! and address sourcing are owned by the binary (constitution VIII); these are
//! pure factories so they stay trivially testable.

use std::sync::Arc;

use pingogate_pipeline::{GatewayProxy, KeyAuth, Metrics};
use pingogate_snapshot::SnapshotHolder;
use pingora::proxy::{http_proxy_service, HttpProxy};
use pingora::server::configuration::ServerConf;
use pingora::services::listening::Service;

mod admin;

pub use admin::{build_admin_service, AdminServiceConfig, Reloader, ReloadStatusStore};

/// Configuration for the public data-plane service, bundled to keep the factory
/// within the parameter limit (constitution V). Mirrors [`AdminServiceConfig`].
/// `conf` is borrowed from the Pingora `Server` that owns the configuration.
pub struct PublicServiceConfig<'a> {
    pub conf: &'a Arc<ServerConf>,
    pub holder: Arc<SnapshotHolder>,
    pub metrics: Arc<Metrics>,
    pub auth: Arc<dyn KeyAuth>,
    pub address: &'a str,
}

/// Build the public data-plane service: a Pingora `HttpProxy` running the
/// gateway pipeline, bound to `address`. The shared `metrics` registry is the
/// same instance the Admin API renders at `/metrics`, so data-plane records are
/// visible to the control plane.
pub fn build_public_service(
    config: PublicServiceConfig<'_>,
) -> Service<HttpProxy<GatewayProxy, ()>> {
    let mut service = http_proxy_service(
        config.conf,
        GatewayProxy::new(config.holder, config.metrics, config.auth),
    );
    service.add_tcp(config.address);
    service
}
