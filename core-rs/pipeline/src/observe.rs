//! Per-request observability wiring (FR-031/FR-032; constitution XIX/XX).
//!
//! Keeps metric recording, the inline usage-extractor tap, and the
//! trace-spanned completion log out of `proxy.rs` so the `ProxyHttp` stage
//! wiring stays focused. Token-usage extraction is O(1) memory
//! ([`crate::usage_extractor`]); nothing here persists request/response
//! content (constitution XX).

use bytes::Bytes;

use pingogate_core_types::{record_facets, request_span};

use crate::ctx::GatewayCtx;
use crate::logging::CompletionLog;
use crate::metrics::{Metrics, RequestRecord};

/// Tap a response body chunk for inline usage extraction. Called from
/// `response_body_filter` on every chunk; safe when no extractor is
/// initialized (unrouted/error requests).
pub(crate) fn tap_body_chunk(ctx: &mut GatewayCtx, body: &Option<Bytes>, end_of_stream: bool) {
    let Some(ex) = ctx.usage_extractor.as_mut() else {
        return;
    };
    if ctx.streaming {
        // Streaming (SSE): each usage-bearing event fits on one line.
        match body.as_ref() {
            Some(chunk) => ex.on_body_chunk(chunk, end_of_stream),
            // Pingora can signal stream end with a `None` body; flush the
            // residual line anyway.
            None if end_of_stream => ex.on_body_chunk(&[], true),
            None => {}
        }
    } else {
        // Non-streaming: parse the accumulated body whole. A pretty-printed
        // JSON body has no trailing newline and `usageMetadata` spans lines,
        // so line-splitting would break it apart.
        match body.as_ref() {
            Some(chunk) => ex.on_complete_body(chunk, end_of_stream),
            None if end_of_stream => ex.on_complete_body(&[], true),
            None => {}
        }
    }
    if end_of_stream {
        ctx.tokens = ex.finalize().filter(|u| !u.is_empty());
    }
}

/// Emit the trace-spanned completion log and record metrics for the request.
pub(crate) fn complete(metrics: &Metrics, ctx: &GatewayCtx, status: u16, error: &str) {
    let span = request_span(&ctx.request);
    record_facets(&span, &ctx.request);
    span.in_scope(|| CompletionLog::new(&ctx.request, status, ctx.streaming, error).emit());
    metrics.record(record_for(ctx, status));
}

/// Build the metric record from the request context. Unrouted requests (auth
/// failures, unknown protocols) are still counted, under `unknown` labels.
fn record_for(ctx: &GatewayCtx, status: u16) -> RequestRecord {
    RequestRecord {
        provider: ctx
            .route_provider
            .clone()
            .unwrap_or_else(|| "unknown".to_string()),
        capability_family: ctx
            .request
            .capability_family
            .map(|f| f.as_str().to_string())
            .unwrap_or_else(|| "unknown".to_string()),
        status,
        duration: ctx.started.elapsed(),
        upstream_duration: ctx.upstream_started.map(|t| t.elapsed()),
        tokens: ctx.tokens.clone(),
    }
}
