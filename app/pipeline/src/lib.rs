//! `pingo-pipeline` — request pipeline layer.
//!
//! Hosts the Pingora `ProxyHttp` implementation and the fixed pipeline stages:
//! protocol identification → gateway-key auth → routing → upstream-auth
//! injection → passthrough/SSE → mirrored error → observability (constitution VI).

pub mod auth_filter;
mod ctx;
pub mod error_response;
pub mod logging;
pub mod metrics;
mod observe;
pub mod protocol;
pub mod proxy;
pub mod router;
pub mod streaming;
pub mod upstream_auth;
pub mod upstream_peer;
pub mod wire;

pub use ctx::GatewayCtx;
pub use metrics::{Metrics, RequestRecord};
pub use protocol::{detect, Detected, Detection};
pub use proxy::GatewayProxy;
pub use upstream_auth::{UpstreamAuth, GATEWAY_KEY_HEADERS};
pub use upstream_peer::{resolve_route, UpstreamTarget};
