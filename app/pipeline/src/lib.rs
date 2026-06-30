//! `pingo-pipeline` — request pipeline layer.
//!
//! Hosts the Pingora `ProxyHttp` implementation and the fixed pipeline stages:
//! protocol identification → gateway-key auth → routing → upstream-auth
//! injection → passthrough/SSE → mirrored error → observability (constitution VI).

pub mod proxy;

pub use proxy::{GatewayCtx, GatewayProxy};
