//! Per-request pipeline state and gateway-key authentication.
//!
//! [`GatewayCtx`] is pinned to the snapshot it loaded at request start, so a
//! concurrent reload never disturbs an in-flight request (FR-021). It also
//! carries the observability fields (timers, an inline O(1)-memory usage
//! extractor, parsed token counts) consumed in [`crate::observe`]. Extracted
//! from `proxy.rs` to keep that module focused on the `ProxyHttp` stage
//! wiring (constitution: file ≤300 lines).
//!
//! Authentication is delegated to a [`KeyAuth`](crate::key_auth::KeyAuth) impl
//! injected at construction, so the pipeline does not inline gateway-key
//! lookup (constitution VII/ VIII: trait-based, low coupling). The
//! authenticated [`Principal`] is stored on the context so the pipeline can
//! release any per-request state (e.g. VirtualKeyAuth's concurrency counter)
//! at request end via [`KeyAuth::release`](crate::key_auth::KeyAuth::release).

use std::sync::Arc;
use std::time::Instant;

use pingogate_core_types::{AppError, Principal, ProtocolKind, RequestContext, TraceId};
use pingogate_provider::TokenUsage;
use pingogate_snapshot::RuntimeSnapshot;

use crate::auth_filter;
use crate::key_auth::KeyAuth;
use crate::upstream_peer::UpstreamTarget;
use crate::usage_extractor::UsageExtractor;

/// Per-request state shared across pipeline phases.
pub struct GatewayCtx {
    /// Snapshot pinned for this request's whole lifetime (reload-safe).
    pub snapshot: Arc<RuntimeSnapshot>,
    /// Authenticator injected by the proxy (StaticKeyAuth in S1).
    pub(crate) auth: Arc<dyn KeyAuth>,
    pub request: RequestContext,
    /// Name of the authenticated gateway key, once authenticated.
    pub gateway_key_name: Option<String>,
    /// The authenticated principal, if auth succeeded. Stored so the pipeline
    /// can call [`KeyAuth::release`] at request end (VirtualKeyAuth's
    /// concurrency counter; StaticKeyAuth is a no-op).
    pub(crate) authenticated_principal: Option<Principal>,
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
    /// Inline streaming usage extractor (O(1) memory). Replaces the previous
    /// bounded response-capture buffer. Initialized once the inbound protocol
    /// is identified in `request_filter`; remains `None` for unrouted/error
    /// requests where no upstream body will be observed.
    pub(crate) usage_extractor: Option<UsageExtractor>,
    /// Token counts parsed from the upstream response, when available.
    pub(crate) tokens: Option<TokenUsage>,
}

impl GatewayCtx {
    /// Build a fresh context bound to `snapshot` and `auth`, with a generated
    /// trace id (an inbound trace header replaces it in `request_filter`).
    pub(crate) fn new(snapshot: Arc<RuntimeSnapshot>, auth: Arc<dyn KeyAuth>) -> Self {
        Self {
            snapshot,
            auth,
            request: RequestContext::new(TraceId::generate()),
            gateway_key_name: None,
            authenticated_principal: None,
            protocol: None,
            route_provider: None,
            upstream: None,
            streaming: false,
            started: Instant::now(),
            upstream_started: None,
            usage_extractor: None,
            tokens: None,
        }
    }

    /// Authenticate the presented gateway key via the injected [`KeyAuth`] impl
    /// (FR-013). Returns `Some(AuthFailed)` when no key is presented or it does
    /// not match a configured gateway key; `None` only once a key is positively
    /// verified.
    pub(crate) fn authenticate(
        &mut self,
        authz: Option<&str>,
        x_api_key: Option<&str>,
    ) -> Option<AppError> {
        let Some(presented) = auth_filter::presented_key(authz, x_api_key) else {
            return Some(AppError::AuthFailed);
        };
        match self.auth.authenticate(&presented, &self.snapshot) {
            Some(principal) => {
                self.gateway_key_name = Some(principal.id.clone());
                self.request.principal = Some(principal.id.clone());
                self.authenticated_principal = Some(principal);
                None
            }
            None => Some(AppError::AuthFailed),
        }
    }

    /// Release any per-request auth state (e.g. VirtualKeyAuth's concurrency
    /// counter). Called exactly once at request end (success or error) by the
    /// pipeline's `logging` hook. Safe to call when auth never happened (the
    /// principal is `None` and this is a no-op).
    pub(crate) fn release_auth(&mut self) {
        if let Some(principal) = self.authenticated_principal.take() {
            self.auth.release(&principal);
        }
    }
}
