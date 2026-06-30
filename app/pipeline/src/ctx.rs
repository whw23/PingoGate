//! Per-request pipeline state and gateway-key authentication.
//!
//! [`GatewayCtx`] is pinned to the snapshot it loaded at request start, so a
//! concurrent reload never disturbs an in-flight request (FR-021). It also
//! carries the observability fields (timers, a bounded transient response
//! capture for token parsing, parsed token counts) consumed in
//! [`crate::observe`]. Extracted from `proxy.rs` to keep that module focused on
//! the `ProxyHttp` stage wiring (constitution: file ≤300 lines).

use std::sync::Arc;
use std::time::Instant;

use pingo_config::RuntimeSnapshot;
use pingo_core::{AppError, ProtocolKind, RequestContext, TraceId};
use pingo_provider::TokenUsage;

use crate::auth_filter;
use crate::upstream_peer::UpstreamTarget;

/// Per-request state shared across pipeline phases.
pub struct GatewayCtx {
    /// Snapshot pinned for this request's whole lifetime (reload-safe).
    pub snapshot: Arc<RuntimeSnapshot>,
    pub request: RequestContext,
    /// Name of the authenticated gateway key, once authenticated.
    pub gateway_key_name: Option<String>,
    /// Identified inbound protocol (drives the mirrored error shape).
    pub(crate) protocol: Option<ProtocolKind>,
    /// Resolved upstream provider name.
    pub(crate) route_provider: Option<String>,
    /// Resolved upstream connection target.
    pub(crate) upstream: Option<UpstreamTarget>,
    /// Whether this exchange is a stream (observability marker).
    pub(crate) streaming: bool,
    /// Monotonic request start, for end-to-end latency.
    pub(crate) started: Instant,
    /// Upstream send time, for upstream latency.
    pub(crate) upstream_started: Option<Instant>,
    /// Bounded, transient response capture used only to parse token usage —
    /// never persisted and dropped at request end (constitution XX).
    pub(crate) response_acc: Vec<u8>,
    /// Set once the capture exceeds its cap; token parsing is then skipped.
    pub(crate) acc_truncated: bool,
    /// Token counts parsed from the upstream response, when available.
    pub(crate) tokens: Option<TokenUsage>,
}

impl GatewayCtx {
    /// Build a fresh context bound to `snapshot`, with a generated trace id (an
    /// inbound trace header replaces it in `request_filter`).
    pub(crate) fn new(snapshot: Arc<RuntimeSnapshot>) -> Self {
        Self {
            snapshot,
            request: RequestContext::new(TraceId::generate()),
            gateway_key_name: None,
            protocol: None,
            route_provider: None,
            upstream: None,
            streaming: false,
            started: Instant::now(),
            upstream_started: None,
            response_acc: Vec::new(),
            acc_truncated: false,
            tokens: None,
        }
    }

    /// Authenticate the presented gateway key against the snapshot (FR-013).
    /// Returns `Some(AuthFailed)` when no key is presented or it does not match a
    /// configured gateway key; `None` only once a key is positively verified.
    pub(crate) fn authenticate(
        &mut self,
        authz: Option<&str>,
        x_api_key: Option<&str>,
    ) -> Option<AppError> {
        let Some(presented) = auth_filter::presented_key(authz, x_api_key) else {
            return Some(AppError::AuthFailed);
        };
        let Some(entry) = self.snapshot.authenticate_gateway_key(&presented) else {
            return Some(AppError::AuthFailed);
        };
        self.gateway_key_name = Some(entry.name.clone());
        self.request.principal = Some(entry.name.clone());
        None
    }
}
