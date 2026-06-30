//! Request context and trace-id propagation (constitution XIX).
//!
//! A [`TraceId`] is reused from an inbound `x-pingo-trace-id` header when
//! present, otherwise generated. [`RequestContext`] carries the trace id plus
//! the gateway-relevant facets used for structured logging and metric labels.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::OnceLock;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::types::{CapabilityFamily, ProtocolKind};

/// Header used to propagate a trace id across the gateway boundary.
pub const TRACE_HEADER: &str = "x-pingo-trace-id";

static COUNTER: AtomicU64 = AtomicU64::new(0);

/// Per-process seed so ids from different runs do not collide on the prefix.
fn process_seed() -> u64 {
    static SEED: OnceLock<u64> = OnceLock::new();
    *SEED.get_or_init(|| {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_nanos() as u64)
            .unwrap_or(0)
    })
}

/// An opaque, process-unique trace identifier.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct TraceId(String);

impl TraceId {
    /// Generate a fresh trace id. Unique within a process via the monotonic
    /// counter; the seed disambiguates across processes.
    pub fn generate() -> Self {
        let seq = COUNTER.fetch_add(1, Ordering::Relaxed);
        Self(format!("pg-{:08x}-{:08x}", process_seed() & 0xffff_ffff, seq))
    }

    /// Reuse a non-empty inbound trace id, otherwise generate a new one.
    pub fn from_header(incoming: Option<&str>) -> Self {
        match incoming {
            Some(v) if !v.trim().is_empty() => Self(v.trim().to_string()),
            _ => Self::generate(),
        }
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Gateway-relevant request facets carried through the pipeline.
#[derive(Debug, Clone)]
pub struct RequestContext {
    pub trace_id: TraceId,
    pub protocol: Option<ProtocolKind>,
    pub provider: Option<String>,
    pub principal: Option<String>,
    pub capability_family: Option<CapabilityFamily>,
}

impl RequestContext {
    pub fn new(trace_id: TraceId) -> Self {
        Self {
            trace_id,
            protocol: None,
            provider: None,
            principal: None,
            capability_family: None,
        }
    }

    /// Build a context from an optional inbound trace header.
    pub fn from_incoming(trace_header: Option<&str>) -> Self {
        Self::new(TraceId::from_header(trace_header))
    }

    pub fn with_protocol(mut self, protocol: ProtocolKind) -> Self {
        self.protocol = Some(protocol);
        self
    }

    pub fn with_provider(mut self, provider: impl Into<String>) -> Self {
        self.provider = Some(provider.into());
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generated_ids_are_unique_and_prefixed() {
        let a = TraceId::generate();
        let b = TraceId::generate();
        assert!(a.as_str().starts_with("pg-"));
        assert_ne!(a, b);
        assert!(!a.as_str().is_empty());
    }

    #[test]
    fn from_header_reuses_non_empty_value() {
        let id = TraceId::from_header(Some("upstream-123"));
        assert_eq!(id.as_str(), "upstream-123");
    }

    #[test]
    fn from_header_generates_when_absent_or_blank() {
        assert!(TraceId::from_header(None).as_str().starts_with("pg-"));
        assert!(TraceId::from_header(Some("   ")).as_str().starts_with("pg-"));
    }

    #[test]
    fn context_propagates_trace_and_builds_facets() {
        let ctx = RequestContext::from_incoming(Some("t-1"))
            .with_protocol(ProtocolKind::Anthropic)
            .with_provider("anthropic-main");
        assert_eq!(ctx.trace_id.as_str(), "t-1");
        assert_eq!(ctx.protocol, Some(ProtocolKind::Anthropic));
        assert_eq!(ctx.provider.as_deref(), Some("anthropic-main"));
    }
}
