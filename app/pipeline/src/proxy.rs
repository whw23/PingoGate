//! The gateway's `ProxyHttp` implementation (constitution VI; research R1).
//!
//! Stage wiring is established here. Protocol identification, model routing,
//! body passthrough, SSE streaming and mirrored error bodies are filled in by
//! User Story 1 (tasks T024–T031). This phase authenticates the gateway key and
//! binds each in-flight request to the snapshot loaded at `new_ctx` time, so a
//! concurrent reload never disturbs requests already in progress (FR-021).

use std::sync::Arc;

use async_trait::async_trait;
use pingo_config::{RuntimeSnapshot, SnapshotHolder};
use pingo_core::{RequestContext, TraceId, TRACE_HEADER};
use pingora::http::RequestHeader;
use pingora::proxy::{ProxyHttp, Session};
use pingora::upstreams::peer::HttpPeer;
use pingora::{Error, ErrorType, Result};
use tracing::info;

/// The gateway proxy. Holds the snapshot holder so each request reads the
/// active configuration through a lock-free `ArcSwap` load.
pub struct GatewayProxy {
    holder: Arc<SnapshotHolder>,
}

impl GatewayProxy {
    pub fn new(holder: Arc<SnapshotHolder>) -> Self {
        Self { holder }
    }
}

/// Per-request state shared across pipeline phases.
pub struct GatewayCtx {
    /// Snapshot pinned for this request's whole lifetime (reload-safe).
    pub snapshot: Arc<RuntimeSnapshot>,
    pub request: RequestContext,
    /// Name of the authenticated gateway key, once authenticated.
    pub gateway_key_name: Option<String>,
}

#[async_trait]
impl ProxyHttp for GatewayProxy {
    type CTX = GatewayCtx;

    fn new_ctx(&self) -> Self::CTX {
        GatewayCtx {
            snapshot: self.holder.load(),
            request: RequestContext::new(TraceId::generate()),
            gateway_key_name: None,
        }
    }

    /// Phase 1: trace id propagation + gateway-key authentication. Routing and
    /// protocol identification are layered on in US1.
    async fn request_filter(&self, session: &mut Session, ctx: &mut Self::CTX) -> Result<bool> {
        let inbound_trace = header_str(session.req_header(), TRACE_HEADER);
        ctx.request.trace_id = TraceId::from_header(inbound_trace.as_deref());

        let Some(presented) = presented_gateway_key(session.req_header()) else {
            session.respond_error(401).await?;
            return Ok(true);
        };
        match ctx.snapshot.authenticate_gateway_key(&presented) {
            Some(entry) => {
                ctx.gateway_key_name = Some(entry.name.clone());
                ctx.request.principal = Some(entry.name.clone());
            }
            None => {
                session.respond_error(401).await?;
                return Ok(true);
            }
        }

        // Model routing and passthrough are implemented in US1 (T024–T031).
        // Until then the data path is explicitly "not implemented" rather than
        // silently misrouting an authenticated request.
        session.respond_error(501).await?;
        Ok(true)
    }

    /// Phase 2 (required): choose the upstream. Unreachable until US1 wires
    /// routing, because `request_filter` currently short-circuits every request.
    async fn upstream_peer(
        &self,
        _session: &mut Session,
        _ctx: &mut Self::CTX,
    ) -> Result<Box<HttpPeer>> {
        Err(Error::explain(
            ErrorType::InternalError,
            "upstream routing not yet implemented (US1)",
        ))
    }

    /// Phase 5: structured per-request log carrying the trace id (constitution XIX).
    async fn logging(&self, _session: &mut Session, e: Option<&Error>, ctx: &mut Self::CTX) {
        info!(
            trace_id = ctx.request.trace_id.as_str(),
            principal = ctx.gateway_key_name.as_deref().unwrap_or("-"),
            error = e.map(|e| e.to_string()).as_deref().unwrap_or("none"),
            "request completed"
        );
    }
}

/// Read a request header value as an owned `String`.
fn header_str(req: &RequestHeader, name: &str) -> Option<String> {
    req.headers
        .get(name)
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string())
}

/// Extract the gateway key the client presented: `Authorization: Bearer <k>`
/// (OpenAI / Gemini style) or `x-api-key: <k>` (Anthropic style). It is matched
/// against the snapshot, then stripped before forwarding upstream in US1 (FR-010).
fn presented_gateway_key(req: &RequestHeader) -> Option<String> {
    if let Some(auth) = header_str(req, "authorization") {
        if let Some(rest) = auth.strip_prefix("Bearer ") {
            return Some(rest.trim().to_string());
        }
    }
    header_str(req, "x-api-key")
}
