//! US4 integration test (T054): observability end-to-end.
//!
//! Drives one proxied request whose echoed body carries an OpenAI `usage` block,
//! then scrapes the admin `/metrics` endpoint and proves the gateway recorded the
//! request under its `provider` + `capability_family` labels (FR-033), exposed a
//! latency histogram and token counters (FR-032), gated the endpoint behind the
//! admin credential (constitution XX), and leaked no secret material into the
//! exposition (FR-034). The mock upstream echoes the request body verbatim, so a
//! request body containing `usage` reaches `response_body_filter` as the response.
#![allow(clippy::unwrap_used, clippy::expect_used, dead_code)]

mod common;

use common::client::{self, Request};
use common::{Harness, GATEWAY_KEY, OPENAI_UPSTREAM_KEY};

/// The bootstrap admin token the harness starts the gateway with.
const ADMIN_TOKEN: &str = "admin-secret";

/// Scrape `/metrics` on the admin listener with the bootstrap admin credential.
fn scrape_metrics(gw: &Harness) -> client::Resp {
    client::send(
        gw.admin_port(),
        &Request::get("/metrics").header("authorization", &format!("Bearer {ADMIN_TOKEN}")),
    )
}

#[test]
fn proxied_request_is_recorded_and_rendered_at_metrics() {
    let gw = Harness::start();
    // OpenAI-shaped body with a usage block; the mock echoes it back verbatim so
    // the gateway parses token usage from the response stream (passthrough).
    let body = br#"{"model":"gpt-4o","messages":[{"role":"user","content":"hi"}],"usage":{"prompt_tokens":11,"completion_tokens":22}}"#;

    let resp = gw.send(
        &Request::post("/v1/chat/completions", body)
            .header("authorization", &format!("Bearer {GATEWAY_KEY}")),
    );
    assert_eq!(resp.status, 200, "body: {}", resp.body_str());

    // The completion metric is recorded in the logging phase, which runs after
    // the downstream response is flushed — poll until it lands (or time out).
    let mut metrics = String::new();
    let recorded = gw.poll_until(|gw| {
        let scraped = scrape_metrics(gw);
        if scraped.status != 200 {
            return false;
        }
        metrics = scraped.body_str();
        metrics.contains("pingogate_requests_total{provider=\"openai-up\",capability_family=\"generation.stateless\",status=\"200\"}")
    });
    assert!(
        recorded,
        "request was never recorded; last /metrics body:\n{metrics}"
    );

    // Latency histograms for the request and the upstream leg (FR-032).
    assert!(
        metrics.contains("pingogate_request_duration_seconds_count{provider=\"openai-up\",capability_family=\"generation.stateless\"}"),
        "missing request latency histogram:\n{metrics}"
    );
    assert!(
        metrics.contains("pingogate_upstream_duration_seconds_count{provider=\"openai-up\",capability_family=\"generation.stateless\"}"),
        "missing upstream latency histogram:\n{metrics}"
    );

    // Token counters parsed from the echoed OpenAI usage block (FR-032).
    assert!(
        metrics.contains("pingogate_tokens_total{provider=\"openai-up\",capability_family=\"generation.stateless\",direction=\"input\"} 11"),
        "missing input token counter:\n{metrics}"
    );
    assert!(
        metrics.contains("pingogate_tokens_total{provider=\"openai-up\",capability_family=\"generation.stateless\",direction=\"output\"} 22"),
        "missing output token counter:\n{metrics}"
    );

    // No secret material is ever a label or value (FR-034): not the gateway key,
    // the upstream credential, nor the admin token.
    assert!(
        !metrics.contains(GATEWAY_KEY),
        "gateway key leaked into /metrics"
    );
    assert!(
        !metrics.contains(OPENAI_UPSTREAM_KEY),
        "upstream key leaked into /metrics"
    );
    assert!(
        !metrics.contains(ADMIN_TOKEN),
        "admin token leaked into /metrics"
    );
    assert!(
        !metrics.contains("Bearer "),
        "an Authorization value leaked into /metrics"
    );
}

#[test]
fn metrics_endpoint_requires_admin_credential() {
    let gw = Harness::start();

    // No credential: the endpoint must not expose the exposition (SC-006).
    let unauth = client::send(gw.admin_port(), &Request::get("/metrics"));
    assert_eq!(
        unauth.status, 401,
        "unauthenticated /metrics must be rejected"
    );

    // A wrong token is equally rejected, revealing nothing about the endpoint.
    let wrong = client::send(
        gw.admin_port(),
        &Request::get("/metrics").header("authorization", "Bearer not-the-admin-token"),
    );
    assert_eq!(wrong.status, 401, "wrong-token /metrics must be rejected");
}
