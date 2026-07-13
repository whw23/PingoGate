//! Per-request tracing span construction (constitution XIX; FR-031).
//!
//! [`TraceId`](crate::TraceId) generates or propagates the id; this module turns
//! a [`RequestContext`] into a `tracing` span carrying that id and the gateway
//! facets, so every event emitted while the request is in scope is correlated by
//! `trace_id`. Facets unknown at span creation (provider, capability family, the
//! authenticated principal) are declared empty and filled in via
//! [`record_facets`] once routing and auth resolve them.

use tracing::field::Empty;
use tracing::{info_span, Span};

use crate::context::RequestContext;

/// Build the per-request span keyed by trace id. Only non-secret facets are ever
/// recorded - never a credential (constitution XX).
pub fn request_span(ctx: &RequestContext) -> Span {
    info_span!(
        "request",
        trace_id = %ctx.trace_id.as_str(),
        protocol = Empty,
        provider = Empty,
        capability_family = Empty,
        principal = Empty,
    )
}

/// Record the facets resolved during the request onto its span.
pub fn record_facets(span: &Span, ctx: &RequestContext) {
    if let Some(protocol) = ctx.protocol {
        span.record("protocol", protocol.as_str());
    }
    if let Some(provider) = ctx.provider.as_deref() {
        span.record("provider", provider);
    }
    if let Some(family) = ctx.capability_family {
        span.record("capability_family", family.as_str());
    }
    if let Some(principal) = ctx.principal.as_deref() {
        span.record("principal", principal);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::ProtocolKind;

    #[test]
    fn span_is_enabled_and_records_facets_under_subscriber() {
        let subscriber = tracing_subscriber::fmt()
            .with_max_level(tracing::Level::INFO)
            .finish();
        tracing::subscriber::with_default(subscriber, || {
            let ctx = RequestContext::from_incoming(Some("t-42"))
                .with_protocol(ProtocolKind::Anthropic)
                .with_provider("anthropic-up");
            let span = request_span(&ctx);
            assert!(
                !span.is_disabled(),
                "request span should be enabled under an INFO subscriber"
            );
            record_facets(&span, &ctx);
            span.in_scope(|| tracing::info!("inside the request span"));
        });
    }

    #[test]
    fn record_facets_on_an_empty_context_is_a_noop() {
        let ctx = RequestContext::from_incoming(None);
        let span = request_span(&ctx);
        record_facets(&span, &ctx); // no protocol/provider/etc. - must not panic
    }
}
