//! `pingo-listener` — gateway infrastructure layer.
//!
//! Assembles the Pingora services: the public `HttpProxy` service that runs the
//! request pipeline, and the admin `ServeHttp` service. The server lifecycle
//! and address sourcing are owned by the binary (constitution VIII); these are
//! pure factories so they stay trivially testable.

use std::sync::Arc;

use pingo_config::SnapshotHolder;
use pingo_pipeline::{GatewayProxy, Metrics};
use pingora::proxy::{http_proxy_service, HttpProxy};
use pingora::server::configuration::ServerConf;
use pingora::services::listening::Service;

mod admin;

pub use admin::{build_admin_service, AdminServiceConfig};

/// Build the public data-plane service: a Pingora `HttpProxy` running the
/// gateway pipeline, bound to `address`. The shared `metrics` registry is the
/// same instance the Admin API renders at `/metrics`, so data-plane records are
/// visible to the control plane.
pub fn build_public_service(
    conf: &Arc<ServerConf>,
    holder: Arc<SnapshotHolder>,
    metrics: Arc<Metrics>,
    address: &str,
) -> Service<HttpProxy<GatewayProxy, ()>> {
    let mut service = http_proxy_service(conf, GatewayProxy::new(holder, metrics));
    service.add_tcp(address);
    service
}
