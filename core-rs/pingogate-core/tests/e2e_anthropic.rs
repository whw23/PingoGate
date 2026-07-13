//! S1 e2e test: Anthropic Messages passthrough.
//!
//! `/v1/messages` is routed by the body `model`; the gateway strips the client
//! `x-api-key` (the gateway key) and injects the upstream `x-api-key` plus the
//! configured `anthropic-version`, forwarding the body unchanged.
//!
//! Ported from 001 (`app/pingogate/tests/us1_anthropic.rs`).
#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use common::client::Request;
use common::{Harness, ANTHROPIC_UPSTREAM_KEY, GATEWAY_KEY};

#[test]
fn anthropic_messages_passthrough_injects_key_and_version() {
    let gw = Harness::start();
    let body = br#"{"model":"claude-3-5-sonnet","max_tokens":16,"messages":[{"role":"user","content":"hi"}]}"#;

    // Anthropic clients present the key via x-api-key; here it is the gateway key.
    let resp = gw.send(&Request::post("/v1/messages", body).header("x-api-key", GATEWAY_KEY));

    assert_eq!(resp.status, 200, "body: {}", resp.body_str());
    assert_eq!(resp.header("x-observed-path"), Some("/v1/messages"));
    // Upstream x-api-key injected, gateway key removed (FR-010/FR-012).
    assert_eq!(
        resp.header("x-observed-x-api-key"),
        Some(ANTHROPIC_UPSTREAM_KEY)
    );
    // Configured anthropic-version forwarded.
    assert_eq!(
        resp.header("x-observed-anthropic-version"),
        Some("2023-06-01")
    );
    assert_eq!(resp.body, body);
}
