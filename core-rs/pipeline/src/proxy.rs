//! The gateway's `ProxyHttp` implementation (constitution VI; research R1).
//!
//! Fixed pipeline: protocol -> auth -> routing -> upstream-auth -> SSE
//! passthrough -> mirrored error -> observability. Pinned to `new_ctx` snapshot
//! (FR-021). Dual mode (XII): `keyvault: None` (standalone) vs `Some` (platform).

use std::sync::Arc;
use std::time::{Duration, Instant};
use async_trait::async_trait;
use bytes::Bytes;
use pingogate_core_types::{AppError, CapabilityFamily, ProtocolKind, TraceId, TRACE_HEADER};
use pingogate_snapshot::SnapshotHolder;
use pingogate_storage::KeyVault;
use pingora::http::{RequestHeader, ResponseHeader};
use pingora::proxy::{FailToProxy, ProxyHttp, Session};
use pingora::upstreams::peer::HttpPeer;
use pingora::{Error, ErrorType, Result};

use crate::ctx::GatewayCtx;
use crate::key_auth::KeyAuth;
use crate::metrics::Metrics;
use crate::observe;
use crate::protocol::{self, Detected, Detection};
use crate::streaming;
use crate::upstream_auth::GATEWAY_KEY_HEADERS;
use crate::upstream_peer::{resolve_route, UpstreamTarget};
use crate::usage_extractor::UsageExtractor;
use crate::wire;

/// The gateway proxy. Mode (XII): `keyvault: None` = standalone;
/// `Some` = platform (per-request AES-GCM decrypt).
pub struct GatewayProxy {
    holder: Arc<SnapshotHolder>,
    metrics: Arc<Metrics>,
    auth: Arc<dyn KeyAuth>,
    keyvault: Option<Arc<dyn KeyVault>>,
}

impl GatewayProxy {
    /// Standalone-mode construct: env-resolved plaintext keys, no KeyVault.
    pub fn new(
        holder: Arc<SnapshotHolder>,
        metrics: Arc<Metrics>,
        auth: Arc<dyn KeyAuth>,
    ) -> Self {
        Self { holder, metrics, auth, keyvault: None }
    }

    /// Platform-mode construct: per-request KeyVault decrypt (constitution XX).
    pub fn new_platform(
        holder: Arc<SnapshotHolder>,
        metrics: Arc<Metrics>,
        auth: Arc<dyn KeyAuth>,
        keyvault: Arc<dyn KeyVault>,
    ) -> Self {
        Self { holder, metrics, auth, keyvault: Some(keyvault) }
    }
}

#[async_trait]
impl ProxyHttp for GatewayProxy {
    type CTX = GatewayCtx;

    fn new_ctx(&self) -> Self::CTX {
        GatewayCtx::new(self.holder.load_full(), self.auth.clone())
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
        let Some(provider_name) = ctx.route_provider.clone() else { return Ok(()); };
        let Some(provider) = ctx.snapshot.provider(&provider_name) else { return Ok(()); };
        let base_path = ctx.upstream.as_ref().map(|t| t.base_path.clone());
        let host = ctx.upstream.as_ref().map(|t| t.sni.clone());

        for header in GATEWAY_KEY_HEADERS {
            upstream_request.remove_header(header);
        }
        let original = upstream_request
            .uri.path_and_query()
            .map(|pq| pq.as_str().to_string())
            .unwrap_or_else(|| "/".to_string());
        let mut path = format!("{}{}", base_path.unwrap_or_default(), original);

        let auth = crate::upstream_auth::build_upstream_auth(&self.keyvault, provider)?;
        wire::apply_upstream_auth(upstream_request, auth, &mut path)?;
        if let Some(host) = host {
            upstream_request.insert_header("host", host.as_str())?;
        }
        // Update Content-Length to match the injected body (T28): Pingora sends
        // headers before body, so the original Content-Length would mismatch
        // the stream_options-injected body and truncate it at the upstream.
        if let Some(ref injected) = ctx.injected_request_body {
            upstream_request
                .insert_header("content-length", injected.len().to_string())?;
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

    /// Swap in the pre-computed `include_usage`-injected body (T28).
    async fn request_body_filter(
        &self,
        _session: &mut Session,
        body: &mut Option<Bytes>,
        _end_of_stream: bool,
        ctx: &mut Self::CTX,
    ) -> Result<()> {
        if let Some(injected) = ctx.injected_request_body.take() {
            *body = Some(injected);
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

    /// Tap the response stream to extract token usage inline (constitution
    /// XX/XXI; O(1) memory via [`UsageExtractor`]).
    fn response_body_filter(
        &self,
        _session: &mut Session,
        body: &mut Option<Bytes>,
        end_of_stream: bool,
        ctx: &mut Self::CTX,
    ) -> Result<Option<Duration>> {
        observe::tap_body_chunk(ctx, body, end_of_stream);
        Ok(None)
    }

    async fn logging(&self, session: &mut Session, e: Option<&Error>, ctx: &mut Self::CTX) {
        // Release per-request auth state (VirtualKeyAuth concurrency counter).
        // `logging` fires on both success and error; idempotent via take().
        ctx.release_auth();

        let status = session
            .as_downstream()
            .response_written()
            .map(|resp| resp.status.as_u16())
            .unwrap_or(0);
        let error = e.map(|e| e.to_string());
        observe::complete(&self.metrics, ctx, status, error.as_deref().unwrap_or("none"));
    }
}

/// Identify, authenticate, and route the request. Returns `Some((protocol,
/// err))` to short-circuit with a mirrored error, or `None` to proceed.
async fn prepare(
    session: &mut Session,
    ctx: &mut GatewayCtx,
) -> Result<Option<(Option<ProtocolKind>, AppError)>> {
    let req = session.req_header();
    let method = req.method.as_str().to_string();
    let path = req.uri.path_and_query().map(|pq| pq.as_str().to_string()).unwrap_or_default();
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

/// Record the resolved route, streaming flag, and usage extractor on the
/// request context. For OpenAI Chat streaming, pre-computes the injected body
/// (T28: `stream_options.include_usage`) so `upstream_request_filter` can set
/// `Content-Length` before headers ship (Pingora sends headers before body).
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
    ctx.request.capability_family = Some(CapabilityFamily::GenerationStateless);
    ctx.route_provider = Some(provider);
    ctx.upstream = Some(target);
    ctx.usage_extractor = Some(UsageExtractor::new(detected.protocol));
    if ctx.streaming && detected.protocol == ProtocolKind::OpenAiCompatible {
        if let Some(b) = body.as_deref() {
            let injected = pingogate_transform::inject_include_usage(
                b,
                ProtocolKind::OpenAiCompatible,
                true,
            );
            if injected != b {
                ctx.injected_request_body = Some(Bytes::from(injected));
            }
        }
    }
}
