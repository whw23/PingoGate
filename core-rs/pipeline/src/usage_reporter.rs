// Usage reporting trait (constitution XIX; T31).
//
// Abstracts the push of a UsageEvent from the Rust kernel to the Go control
// plane UsageService gRPC server. The pipeline calls UsageReporter::report
// at request end (the logging hook); the concrete implementation (gRPC
// client to Go) lives in the pingogate-core binary so the pipeline crate
// stays free of tonic dependencies (constitution VIII).
//
// The trait is Send + Sync so it can be shared via Arc<dyn UsageReporter>
// across Pingora worker threads. The report method is fire-and-forget from
// the pipeline perspective: failures are logged but do not fail the request
// (usage is best-effort; a Go outage must not block the data plane).

use async_trait::async_trait;

// A usage event ready to push to the Go control plane. Built from the
// per-request context at the logging hook. Field names mirror the proto
// UsageEvent (proto/pingogate.proto).
#[derive(Debug, Clone, Default)]
pub struct UsageEvent {
    pub virtual_key_id: String,
    pub owner_user_id: String,
    pub provider: String,
    pub model: String,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub reasoning_tokens: u64,
    pub cache_read_tokens: u64,
    pub cache_write_tokens: u64,
    pub success: bool,
    pub latency_ms: u64,
    // True when the provider returned no usage block and Go should run the
    // tiktoken estimator on body_ref.
    pub needs_estimate: bool,
    // Request body (and response body if available) for estimation. Sent only
    // when needs_estimate is true. Cleared by the Go server after estimation;
    // never persisted (constitution XX).
    pub body_ref: Vec<u8>,
}

// Fire-and-forget usage push from the Rust kernel to the Go control plane.
// Implementations MUST NOT block the caller beyond a short bounded timeout
// (constitution XXI: Go -> Rust snapshot push must not block the Rust hot
// path; the same applies to Rust -> Go usage push).
#[async_trait]
pub trait UsageReporter: Send + Sync {
    // Push one usage event. Errors are logged by the implementation; the
    // pipeline does not retry or fail the request on a push failure.
    async fn report(&self, event: UsageEvent);
}

// No-op reporter for standalone mode (constitution XII: standalone has no Go
// control plane to push to). report is a no-op; usage is still extracted and
// logged via tracing (constitution XIX: standalone only extracts ready
// usage, logs it, does not persist to DB).
#[derive(Debug, Default, Clone)]
pub struct NoopUsageReporter;

#[async_trait]
impl UsageReporter for NoopUsageReporter {
    async fn report(&self, event: UsageEvent) {
        tracing::debug!(
            input = event.input_tokens,
            output = event.output_tokens,
            provider = %event.provider,
            model = %event.model,
            "usage (standalone mode, not persisted): extracted tokens",
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn noop_reporter_does_not_panic() {
        let reporter = NoopUsageReporter;
        reporter
            .report(UsageEvent {
                input_tokens: 10,
                output_tokens: 5,
                provider: "openai".to_string(),
                model: "gpt-4o".to_string(),
                ..Default::default()
            })
            .await;
    }
}
