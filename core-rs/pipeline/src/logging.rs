//! Structured request-completion logging (FR-031/FR-034; constitution XIX).
//!
//! The completion event carries the trace id and the gateway facets - and only
//! those: no credential ever becomes a log field (FR-034). [`CompletionLog`] is a
//! pure value built from the request context, so the exact field set is
//! unit-testable independently of a tracing subscriber; [`CompletionLog::emit`]
//! logs it at `info`. Emitting inside the request span (see
//! [`pingogate_core_types::request_span`]) correlates it by `trace_id`.

use pingogate_core_types::RequestContext;
use tracing::info;

/// The field names emitted by the completion event. Asserted in tests to never
/// include a credential-bearing field, locking in FR-034 against regressions.
pub const FIELDS: &[&str] = &[
    "trace_id",
    "principal",
    "protocol",
    "provider",
    "capability_family",
    "status",
    "streaming",
    "error",
];

/// A request-completion record, built only from non-secret request facets.
pub struct CompletionLog<'a> {
    trace_id: &'a str,
    principal: &'a str,
    protocol: &'a str,
    provider: &'a str,
    capability_family: &'a str,
    status: u16,
    streaming: bool,
    error: &'a str,
}

impl<'a> CompletionLog<'a> {
    /// Build from the request context and the completion outcome. Absent facets
    /// render as `-` so every field is always present and parseable.
    pub fn new(ctx: &'a RequestContext, status: u16, streaming: bool, error: &'a str) -> Self {
        Self {
            trace_id: ctx.trace_id.as_str(),
            principal: ctx.principal.as_deref().unwrap_or("-"),
            protocol: ctx.protocol.map(|p| p.as_str()).unwrap_or("-"),
            provider: ctx.provider.as_deref().unwrap_or("-"),
            capability_family: ctx.capability_family.map(|f| f.as_str()).unwrap_or("-"),
            status,
            streaming,
            error,
        }
    }

    /// Emit the structured completion event at `info`.
    pub fn emit(&self) {
        info!(
            trace_id = self.trace_id,
            principal = self.principal,
            protocol = self.protocol,
            provider = self.provider,
            capability_family = self.capability_family,
            status = self.status,
            streaming = self.streaming,
            error = self.error,
            "request completed"
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pingogate_core_types::RequestContext;

    #[test]
    fn carries_trace_id_and_no_credential_field() {
        let mut ctx = RequestContext::from_incoming(Some("trace-7"));
        ctx.principal = Some("team-alpha".to_string());
        let log = CompletionLog::new(&ctx, 200, false, "none");
        assert_eq!(log.trace_id, "trace-7");
        assert_eq!(log.principal, "team-alpha");
        assert!(FIELDS.contains(&"trace_id"));
        for forbidden in ["key", "secret", "token", "authorization", "x-api-key"] {
            assert!(
                !FIELDS.contains(&forbidden),
                "field set must never log {forbidden}"
            );
        }
    }

    #[test]
    fn missing_facets_render_as_placeholder() {
        let ctx = RequestContext::from_incoming(None);
        let log = CompletionLog::new(&ctx, 502, false, "upstream unavailable");
        assert_eq!(log.provider, "-");
        assert_eq!(log.capability_family, "-");
        assert_eq!(log.principal, "-");
        assert!(log.trace_id.starts_with("pg-"));
    }
}
