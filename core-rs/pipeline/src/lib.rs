//! `pingogate-pipeline` - request pipeline layer.
//!
//! Hosts the Pingora `ProxyHttp` implementation and the fixed pipeline stages:
//! protocol identification -> gateway-key auth -> routing -> upstream-auth
//! injection -> passthrough/SSE -> mirrored error -> observability (constitution VI).
//!
//! Authentication is abstracted behind the [`KeyAuth`] trait so the pipeline
//! does not inline gateway-key lookup: S1 ships [`StaticKeyAuth`] (standalone
//! mode), S2 will add a platform-mode impl backed by virtual keys.

pub mod auth_filter;
mod ctx;
pub mod error_response;
pub mod key_auth;
pub mod logging;
pub mod metrics;
mod observe;
mod prepare;
mod usage_event;
pub mod protocol;
pub mod proxy;
pub mod router;
pub mod streaming;
pub mod upstream_auth;
pub mod upstream_peer;
pub mod usage_extractor;
pub mod usage_reporter;
pub mod virtual_key_auth;
pub mod wire;

pub use ctx::GatewayCtx;
pub use key_auth::{KeyAuth, StaticKeyAuth};
pub use metrics::{Metrics, RequestRecord};
pub use protocol::{detect, Detected, Detection};
pub use proxy::GatewayProxy;
pub use upstream_auth::{UpstreamAuth, GATEWAY_KEY_HEADERS};
pub use upstream_peer::{resolve_route, UpstreamTarget};
pub use usage_reporter::{NoopUsageReporter, UsageEvent, UsageReporter};
pub use virtual_key_auth::VirtualKeyAuth;
