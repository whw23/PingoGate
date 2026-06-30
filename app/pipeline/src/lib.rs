//! `pingo-pipeline` — request pipeline layer.
//!
//! Hosts the Pingora `ProxyHttp` implementation and the fixed pipeline stages:
//! protocol identification → gateway-key auth → routing → upstream-auth
//! injection → passthrough/SSE → mirrored error → observability. Pingora is
//! wired in starting at task T014 (this scaffold compiles without it).
