//! S1 e2e test (new): OpenAI Responses API passthrough.
//!
//! `POST /v1/responses` is identified as `ProtocolKind::OpenAiResponses` (T7
//! path-suffix detection), authenticated by gateway key (Bearer), routed by the
//! body `model`, and forwarded with the gateway key stripped and the upstream
//! bearer credential injected - body byte-for-byte unchanged. The OpenAI
//! Responses adapter shares the same auth method (Bearer) and error envelope as
//! Chat Completions (T8 adapter collapse), so this test verifies the new
//! protocol surface end-to-end through the five-protocol pipeline.
//!
//! The mock upstream echoes the request body verbatim, which lets us assert the
//! body was forwarded unchanged. A real OpenAI Responses endpoint would return
//! a `response.completed` event; the echo is a stronger assertion for
//! passthrough verification (constitution IX: upstream responses are passed
//! through untouched, FR-008).
#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use common::client::Request;
use common::{Harness, GATEWAY_KEY, OPENAI_UPSTREAM_KEY};

#[test]
fn openai_responses_passthrough_strips_and_injects() {
    let gw = Harness::start();
    // OpenAI Responses API request shape: `model` + `input` (not `messages`).
    let body = br#"{"model":"gpt-4o","input":"Tell me a joke"}"#;

    let resp = gw.send(
        &Request::post("/v1/responses", body)
            .header("authorization", &format!("Bearer {GATEWAY_KEY}")),
    );

    assert_eq!(resp.status, 200, "body: {}", resp.body_str());
    assert_eq!(resp.header("x-observed-method"), Some("POST"));
    assert_eq!(
        resp.header("x-observed-path"),
        Some("/v1/responses"),
        "Responses path must be forwarded verbatim"
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
fn openai_responses_without_gateway_key_is_rejected() {
    let gw = Harness::start();
    let body = br#"{"model":"gpt-4o","input":"hi"}"#;

    let resp = gw.send(&Request::post("/v1/responses", body));

    assert_eq!(resp.status, 401);
    // Responses shares the OpenAI error envelope (T8 adapter collapse).
    let json: serde_json::Value = serde_json::from_slice(&resp.body).unwrap();
    assert_eq!(json["error"]["type"], "invalid_request_error");
    assert_eq!(json["error"]["code"], "invalid_api_key");
}

#[test]
fn openai_responses_unknown_model_returns_native_not_found() {
    let gw = Harness::start();
    let body = br#"{"model":"does-not-exist","input":"hi"}"#;

    let resp = gw.send(
        &Request::post("/v1/responses", body)
            .header("authorization", &format!("Bearer {GATEWAY_KEY}")),
    );

    assert_eq!(resp.status, 404, "no route maps to 404");
    let json: serde_json::Value = serde_json::from_slice(&resp.body).unwrap();
    assert_eq!(json["error"]["type"], "invalid_request_error");
    assert_eq!(json["error"]["code"], "model_not_found");
}
