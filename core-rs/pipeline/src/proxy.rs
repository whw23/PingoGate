//! The gateway's `ProxyHttp` implementation (constitution VI; research R1).
//!
//! Fixed pipeline: protocol -> auth -> routing -> upstream-auth -> SSE
//! passthrough -> mirrored error -> observability. Pinned to `new_ctx` snapshot
//! (FR-021). Dual mode (XII): `keyvault: None` (standalone) vs `Some` (platform).

use std::sync::Arc;
use std::time::{Duration, Instant};
use async_trait::async_trait;
use bytes::Bytes;
use pingogate_core_types::{AppError, TraceId, TRACE_HEADER};
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
use crate::prepare::prepare;
use crate::streaming;
use crate::upstream_auth::GATEWAY_KEY_HEADERS;
use crate::usage_event::build_usage_event;
use crate::usage_reporter::UsageReporter;
use crate::wire;

/// The gateway proxy. Mode (XII): `keyvault: None` = standalone;
/// `Some` = platform (per-request AES-GCM decrypt).
pub struct GatewayProxy {
    holder: Arc<SnapshotHolder>,
    metrics: Arc<Metrics>,
    auth: Arc<dyn KeyAuth>,
    keyvault: Option<Arc<dyn KeyVault>>,
    usage: Arc<dyn UsageReporter>,
}

impl GatewayProxy {
    /// Standalone-mode construct: env-resolved plaintext keys, no KeyVault,
    /// no usage push (NoopUsageReporter).
    pub fn new(
        holder: Arc<SnapshotHolder>,
        metrics: Arc<Metrics>,
        auth: Arc<dyn KeyAuth>,
    ) -> Self {
        Self {
            holder,
            metrics,
            auth,
            keyvault: None,
            usage: Arc::new(crate::usage_reporter::NoopUsageReporter),
        }
    }

    /// Platform-mode construct: per-request KeyVault decrypt (constitution XX)
    /// and usage push to the Go control plane via the injected reporter.
    pub fn new_platform(
        holder: Arc<SnapshotHolder>,
        metrics: Arc<Metrics>,
        auth: Arc<dyn KeyAuth>,
        keyvault: Arc<dyn KeyVault>,
        usage: Arc<dyn UsageReporter>,
    ) -> Self {
        Self {
            holder,
            metrics,
            auth,
            keyvault: Some(keyvault),
            usage,
        }
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
        let mut peer = HttpPeer::new(target.addr.as_str(), target.tls, target.sni);
        // Apply per-provider timeout (issue 3: timeout follows the provider).
        // Pingora's PeerOptions exposes connect/read/write/idle timeouts.
        let timeout = Duration::from_millis(ctx.upstream_timeout_ms);
        peer.options.connection_timeout = Some(timeout);
        peer.options.read_timeout = Some(timeout);
        peer.options.write_timeout = Some(timeout);
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
        // Path construction (issue 2: configurable upstream path). When the
        // route has an upstream_path rewrite, use it instead of the original
        // inbound path. Both are prepended with the provider's base_path.
        let path_body = ctx
            .upstream_path_rewrite
            .as_deref()
            .unwrap_or(&original);
        let mut path = format!("{}{}", base_path.unwrap_or_default(), path_body);

        // Auth injection (issue 1: per-route auth override). When the route
        // specifies an auth_method override, use it instead of the provider's
        // default. The key is still the provider's key.
        let effective_auth_method = ctx.route_auth_override.unwrap_or(provider.auth_method);
        let effective_anthropic_version = ctx.route_anthropic_version.as_deref();
        let auth = crate::upstream_auth::build_upstream_auth_with_method(
            &self.keyvault,
            provider,
            effective_auth_method,
            effective_anthropic_version,
        )?;
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

        // Push usage to the Go control plane (T31). Fire-and-forget: the
        // reporter logs failures and never blocks the hot path. Built here
        // (before observe::complete finalizes) so the event carries the
        // merged token counts. Skipped when no route resolved (auth
        // failures, unknown protocols) - nothing to bill.
        if let Some(event) = build_usage_event(ctx, status) {
            let reporter = self.usage.clone();
            // Spawn a detached task so the Pingora worker is not blocked on
            // the gRPC push. The reporter enforces its own internal timeout.
            tokio::spawn(async move { reporter.report(event).await });
        }

        observe::complete(&self.metrics, ctx, status, error.as_deref().unwrap_or("none"));
    }
}
