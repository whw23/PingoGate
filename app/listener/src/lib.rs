//! `pingo-listener` — gateway infrastructure layer.
//!
//! Assembles the Pingora services: the public `HttpProxy` service that runs the
//! request pipeline, and the admin `ServeHttp` service. The server lifecycle
//! and address sourcing are owned by the binary (constitution VIII); these are
//! pure factories so they stay trivially testable.

use std::sync::Arc;

use pingo_admin::AdminApp;
use pingo_config::SnapshotHolder;
use pingo_core::SecretString;
use pingo_pipeline::GatewayProxy;
use pingora::proxy::{http_proxy_service, HttpProxy};
use pingora::server::configuration::ServerConf;
use pingora::services::listening::Service;

/// Build the public data-plane service: a Pingora `HttpProxy` running the
/// gateway pipeline, bound to `address`.
pub fn build_public_service(
    conf: &Arc<ServerConf>,
    holder: Arc<SnapshotHolder>,
    address: &str,
) -> Service<HttpProxy<GatewayProxy, ()>> {
    let mut service = http_proxy_service(conf, GatewayProxy::new(holder));
    service.add_tcp(address);
    service
}

/// Build the control-plane service: the Admin API `ServeHttp` app bound to
/// `address` (typically a loopback interface, kept off the public listener).
pub fn build_admin_service(
    admin_token: SecretString,
    holder: Arc<SnapshotHolder>,
    address: &str,
) -> Service<AdminApp> {
    let mut service = Service::new(
        "pingogate-admin".to_string(),
        AdminApp::new(admin_token, holder),
    );
    service.add_tcp(address);
    service
}
