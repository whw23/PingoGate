//! Admin `/metrics` endpoint (FR-032; constitution XIX).
//!
//! Renders the shared [`Metrics`] registry as Prometheus text exposition. Like
//! every admin endpoint it is reached only after the authorization boundary
//! (see [`crate::handler`]); the body is plain text and carries no secret —
//! labels are routing facets and the values are counts (FR-034).

use std::sync::Arc;

use http::Response;
use pingo_pipeline::Metrics;

/// The exposition content type Prometheus scrapers expect.
const PROMETHEUS_CONTENT_TYPE: &str = "text/plain; version=0.0.4; charset=utf-8";

/// Render the metrics registry into a `200` text/plain exposition response.
pub fn render_metrics(metrics: &Arc<Metrics>) -> Response<Vec<u8>> {
    let body = metrics.render().into_bytes();
    Response::builder()
        .status(200)
        .header("content-type", PROMETHEUS_CONTENT_TYPE)
        .header("content-length", body.len().to_string())
        .body(body)
        .unwrap_or_else(|_| Response::new(Vec::new()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use pingo_pipeline::RequestRecord;
    use std::time::Duration;

    #[test]
    fn renders_text_exposition_with_recorded_series() {
        let metrics = Arc::new(Metrics::new());
        metrics.record(RequestRecord {
            provider: "openai-up".to_string(),
            capability_family: "generation.stateless".to_string(),
            status: 200,
            duration: Duration::from_millis(5),
            upstream_duration: None,
            tokens: None,
        });
        let resp = render_metrics(&metrics);
        assert_eq!(resp.status(), 200);
        assert_eq!(
            resp.headers().get("content-type").unwrap(),
            PROMETHEUS_CONTENT_TYPE
        );
        let body = String::from_utf8(resp.body().clone()).unwrap();
        assert!(body.contains("pingogate_requests_total"));
        assert!(body.contains("provider=\"openai-up\""));
    }
}
