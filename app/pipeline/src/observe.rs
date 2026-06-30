//! Per-request observability wiring (FR-031/FR-032; constitution XIX/XX).
//!
//! Keeps metric recording, the bounded token-usage capture, and the
//! trace-spanned completion log out of `proxy.rs` so the `ProxyHttp` stage
//! wiring stays focused. Nothing here persists request/response content: the
//! response capture is transient and capped, dropped once usage is parsed
//! (constitution XX).

use bytes::Bytes;

use pingo_core::{record_facets, request_span};
use pingo_provider::parse_usage;

use crate::ctx::GatewayCtx;
use crate::logging::CompletionLog;
use crate::metrics::{Metrics, RequestRecord};

/// Upper bound on bytes captured from a response solely to read token usage.
const MAX_USAGE_CAPTURE: usize = 256 * 1024;

/// Append a response chunk to the bounded usage-capture buffer. Once the cap is
/// exceeded the buffer is dropped and capture stops (token usage is then unknown
/// for this request rather than risking unbounded memory).
pub(crate) fn capture_chunk(ctx: &mut GatewayCtx, chunk: &Bytes) {
    if ctx.acc_truncated || ctx.streaming {
        return;
    }
    if ctx.response_acc.len() + chunk.len() > MAX_USAGE_CAPTURE {
        ctx.acc_truncated = true;
        ctx.response_acc = Vec::new();
        return;
    }
    ctx.response_acc.extend_from_slice(chunk);
}

/// Parse token usage from the captured response, then release the buffer.
/// Streaming or over-cap responses yield no usage.
pub(crate) fn finish_capture(ctx: &mut GatewayCtx) {
    if !ctx.streaming && !ctx.acc_truncated {
        if let Some(protocol) = ctx.protocol {
            ctx.tokens = parse_usage(protocol, &ctx.response_acc).filter(|u| !u.is_empty());
        }
    }
    ctx.response_acc = Vec::new();
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
