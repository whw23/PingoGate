//! The gateway's `ProxyHttp` implementation (constitution VI; research R1).
//!
//! Wires the fixed pipeline: protocol identification → gateway-key auth →
//! model routing → upstream-auth injection → body/SSE passthrough → mirrored
//! error → observability. Each in-flight request is pinned to the snapshot it
//! loaded at `new_ctx` time, so a concurrent reload never disturbs it (FR-021).
//! Request bodies are NOT transformed: the inbound path/body are forwarded
//! verbatim; routing only selects the upstream provider. Pingora-facing
//! adapters live in [`crate::wire`].

use std::sync::Arc;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use bytes::Bytes;
use pingo_config::SnapshotHolder;
use pingo_core::{AppError, CapabilityFamily, ProtocolKind, TraceId, TRACE_HEADER};
use pingora::http::{RequestHeader, ResponseHeader};
use pingora::proxy::{FailToProxy, ProxyHttp, Session};
use pingora::upstreams::peer::HttpPeer;
use pingora::{Error, ErrorType, Result};

use crate::ctx::GatewayCtx;
use crate::metrics::Metrics;
use crate::observe;
use crate::protocol::{self, Detected, Detection};
use crate::streaming;
use crate::upstream_auth::{UpstreamAuth, GATEWAY_KEY_HEADERS};
use crate::upstream_peer::{resolve_route, UpstreamTarget};
use crate::wire;

/// The gateway proxy. Reads the active configuration through a lock-free
/// `ArcSwap` load per request, and records observability signals via `metrics`.
pub struct GatewayProxy {
    holder: Arc<SnapshotHolder>,
    metrics: Arc<Metrics>,
}

impl GatewayProxy {
    pub fn new(holder: Arc<SnapshotHolder>, metrics: Arc<Metrics>) -> Self {
        Self { holder, metrics }
    }
}

#[async_trait]
impl ProxyHttp for GatewayProxy {
    type CTX = GatewayCtx;

    fn new_ctx(&self) -> Self::CTX {
        GatewayCtx::new(self.holder.load())
    }

    async fn request_filter(&self, session: &mut Session, ctx: &mut Self::CTX) -> Result<bool> {
        let inbound_trace = wire::header_str(session.req_header(), TRACE_HEADER);
        ctx.request.trace_id = TraceId::from_header(inbound_trace.as_deref());

        match prepare(session, ctx).await? {
            Some((protocol, err)) => {
                wire::respond_rendered(session, protocol, &err).await?;
                Ok(true)
            }
            None => Ok(false),
        }
    }

    async fn upstream_peer(
        &self,
        _session: &mut Session,
        ctx: &mut Self::CTX,
    ) -> Result<Box<HttpPeer>> {
        let target = ctx.upstream.clone().ok_or_else(|| {
            Error::explain(ErrorType::InternalError, "no upstream resolved for request")
        })?;
        let peer = HttpPeer::new(target.addr.as_str(), target.tls, target.sni);
        Ok(Box::new(peer))
    }

    async fn upstream_request_filter(
        &self,
        _session: &mut Session,
        upstream_request: &mut RequestHeader,
        ctx: &mut Self::CTX,
    ) -> Result<()> {
        ctx.upstream_started = Some(Instant::now());
        let Some(provider_name) = ctx.route_provider.clone() else {
            return Ok(());
        };
        let Some(provider) = ctx.snapshot.provider(&provider_name) else {
            return Ok(());
        };
        let base_path = ctx.upstream.as_ref().map(|t| t.base_path.clone());
        let host = ctx.upstream.as_ref().map(|t| t.sni.clone());

        for header in GATEWAY_KEY_HEADERS {
            upstream_request.remove_header(header);
        }
        let original = upstream_request
            .uri
            .path_and_query()
            .map(|pq| pq.as_str().to_string())
            .unwrap_or_else(|| "/".to_string());
        let mut path = format!("{}{}", base_path.unwrap_or_default(), original);

        let auth = UpstreamAuth::build(
            provider.auth_method,
            provider.key.expose(),
            provider.anthropic_version.as_deref(),
        );
        wire::apply_upstream_auth(upstream_request, auth, &mut path)?;
        if let Some(host) = host {
            upstream_request.insert_header("host", host.as_str())?;
        }
        if let Ok(uri) = path.parse::<http::Uri>() {
            upstream_request.set_uri(uri);
        }
        Ok(())
    }

    async fn upstream_response_filter(
        &self,
        _session: &mut Session,
        upstream_response: &mut ResponseHeader,
        ctx: &mut Self::CTX,
    ) -> Result<()> {
        let content_type = upstream_response
            .headers
            .get("content-type")
            .and_then(|v| v.to_str().ok());
        if streaming::response_is_streaming(content_type) {
            ctx.streaming = true;
        }
        Ok(())
    }

    async fn fail_to_proxy(
        &self,
        session: &mut Session,
        e: &Error,
        ctx: &mut Self::CTX,
    ) -> FailToProxy {
        let provider = ctx
            .route_provider
            .clone()
            .unwrap_or_else(|| "upstream".to_string());
        let err = match e.etype() {
            ErrorType::ConnectTimedout | ErrorType::ReadTimedout | ErrorType::WriteTimedout => {
                AppError::UpstreamTimeout { provider }
            }
            _ => AppError::UpstreamUnavailable { provider },
        };
        let status = err.http_status();
        if session.as_downstream().response_written().is_none() {
            let _ = wire::respond_rendered(session, ctx.protocol, &err).await;
        }
        FailToProxy {
            error_code: status,
            can_reuse_downstream: false,
        }
    }

    /// Tap the response stream to capture token usage (bounded, non-persisting)
    /// without altering the bytes relayed to the client (constitution XX).
    fn response_body_filter(
        &self,
        _session: &mut Session,
        body: &mut Option<Bytes>,
        end_of_stream: bool,
        ctx: &mut Self::CTX,
    ) -> Result<Option<Duration>> {
        if let Some(chunk) = body.as_ref() {
            observe::capture_chunk(ctx, chunk);
        }
        if end_of_stream {
            observe::finish_capture(ctx);
        }
        Ok(None)
    }

    async fn logging(&self, session: &mut Session, e: Option<&Error>, ctx: &mut Self::CTX) {
        let status = session
            .as_downstream()
            .response_written()
            .map(|resp| resp.status.as_u16())
            .unwrap_or(0);
        let error = e.map(|e| e.to_string());
        observe::complete(
            &self.metrics,
            ctx,
            status,
            error.as_deref().unwrap_or("none"),
        );
    }
}

/// Identify, authenticate and route the request, buffering the body when
/// routing needs it. Returns `Some((protocol, err))` to short-circuit with a
/// mirrored error, or `None` to proceed to the upstream.
async fn prepare(
    session: &mut Session,
    ctx: &mut GatewayCtx,
) -> Result<Option<(Option<ProtocolKind>, AppError)>> {
    let req = session.req_header();
    let method = req.method.as_str().to_string();
    let path = req
        .uri
        .path_and_query()
        .map(|pq| pq.as_str().to_string())
        .unwrap_or_default();
    let authz = wire::header_str(req, "authorization");
    let x_api_key = wire::header_str(req, "x-api-key");

    let detection = protocol::detect(&method, &path);
    let protocol = wire::detection_protocol(&detection);
    let detected: Detected = match detection {
        Detection::Unidentified => return Ok(Some((None, AppError::UnknownProtocol))),
        Detection::UnsupportedCapability { family, .. } => {
            if let Some(err) = ctx.authenticate(authz.as_deref(), x_api_key.as_deref()) {
                return Ok(Some((protocol, err)));
            }
            return Ok(Some((protocol, AppError::UnsupportedCapability { family })));
        }
        Detection::Supported(detected) => detected,
    };

    if let Some(err) = ctx.authenticate(authz.as_deref(), x_api_key.as_deref()) {
        return Ok(Some((protocol, err)));
    }

    let (model, body) = match wire::extract_model_and_body(session, detected.protocol, &path).await
    {
        Ok(pair) => pair,
        Err(err) => return Ok(Some((protocol, err))),
    };
    let Some(alias) = model else {
        let err = AppError::Validation {
            message: "request is missing the 'model' field".to_string(),
        };
        return Ok(Some((protocol, err)));
    };

    match resolve_route(&ctx.snapshot, &alias) {
        Ok((provider, target)) => {
            commit_route(ctx, detected, provider, target, body);
            Ok(None)
        }
        Err(err) => Ok(Some((protocol, err))),
    }
}

/// Record the resolved route and streaming flag on the request context.
fn commit_route(
    ctx: &mut GatewayCtx,
    detected: Detected,
    provider: String,
    target: UpstreamTarget,
    body: Option<Vec<u8>>,
) {
    ctx.streaming =
        streaming::request_is_streaming(detected.streaming_by_path, body.as_deref().unwrap_or(b""));
    ctx.protocol = Some(detected.protocol);
    ctx.request.protocol = Some(detected.protocol);
    ctx.request.provider = Some(provider.clone());
    // This phase serves a single capability family; label metrics by it (FR-033).
    ctx.request.capability_family = Some(CapabilityFamily::GenerationStateless);
    ctx.route_provider = Some(provider);
    ctx.upstream = Some(target);
}
