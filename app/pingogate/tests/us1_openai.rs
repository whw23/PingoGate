//! US1 contract test (T017): OpenAI-compatible passthrough.
//!
//! A request to `/v1/chat/completions` is identified, authenticated by gateway
//! key, routed by the body `model`, and forwarded with the gateway key stripped
//! and the upstream bearer credential injected — body byte-for-byte unchanged.
#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use common::client::Request;
use common::{Harness, GATEWAY_KEY, OPENAI_UPSTREAM_KEY};

#[test]
fn openai_chat_completions_passthrough_strips_and_injects() {
    let gw = Harness::start();
    let body = br#"{"model":"gpt-4o","messages":[{"role":"user","content":"hi"}]}"#;

    let resp = gw.send(
        &Request::post("/v1/chat/completions", body)
            .header("authorization", &format!("Bearer {GATEWAY_KEY}")),
    );

    assert_eq!(resp.status, 200, "body: {}", resp.body_str());
    assert_eq!(resp.header("x-observed-method"), Some("POST"));
    assert_eq!(
        resp.header("x-observed-path"),
        Some("/v1/chat/completions"),
        "inbound path must be forwarded verbatim"
    );
    // Gateway key removed; upstream bearer injected (FR-010/FR-011).
    assert_eq!(
        resp.header("x-observed-authorization"),
        Some(format!("Bearer {OPENAI_UPSTREAM_KEY}").as_str())
    );
    // Body forwarded unchanged (research R1: no body transform this phase).
    assert_eq!(resp.body, body);
}

#[test]
fn openai_request_without_gateway_key_is_rejected() {
    let gw = Harness::start();
    let body = br#"{"model":"gpt-4o","messages":[]}"#;

    let resp = gw.send(&Request::post("/v1/chat/completions", body));

    assert_eq!(resp.status, 401);
    let json: serde_json::Value = serde_json::from_slice(&resp.body).unwrap();
    assert_eq!(json["error"]["type"], "invalid_request_error");
    assert_eq!(json["error"]["code"], "invalid_api_key");
}
