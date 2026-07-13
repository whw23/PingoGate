//! S1 e2e tests: error-path behavior.
//!
//! Covers no-route, unsupported capability, and unidentified protocol. Each
//! error is rendered in the target protocol's native shape when the protocol is
//! known, and the PingoGate-native envelope when it is not - with the HTTP
//! status assigned only at the boundary (constitution IX).
//!
//! Ported from 001 (`app/pingogate/tests/us1_errors.rs`).
#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use common::client::Request;
use common::{Harness, GATEWAY_KEY};

#[test]
fn unknown_model_returns_native_not_found() {
    let gw = Harness::start();
    let body = br#"{"model":"does-not-exist","messages":[]}"#;

    let resp = gw.send(
        &Request::post("/v1/chat/completions", body)
            .header("authorization", &format!("Bearer {GATEWAY_KEY}")),
    );

    assert_eq!(resp.status, 404, "no route maps to 404");
    let json: serde_json::Value = serde_json::from_slice(&resp.body).unwrap();
    assert_eq!(json["error"]["type"], "invalid_request_error");
    assert_eq!(json["error"]["code"], "model_not_found");
}

#[test]
fn unsupported_capability_returns_native_error() {
    let gw = Harness::start();
    let body = br#"{"model":"gpt-4o","input":"hi"}"#;

    // /v1/embeddings is a recognized OpenAI surface but out of scope this phase.
    let resp = gw.send(
        &Request::post("/v1/embeddings", body)
            .header("authorization", &format!("Bearer {GATEWAY_KEY}")),
    );

    assert_eq!(
        resp.status, 400,
        "unsupported capability maps to 400"
    );
    let json: serde_json::Value = serde_json::from_slice(&resp.body).unwrap();
    assert_eq!(json["error"]["type"], "invalid_request_error");
    assert_eq!(json["error"]["code"], "unsupported_capability");
}

#[test]
fn unidentified_request_returns_pingogate_native_error() {
    let gw = Harness::start();

    let resp = gw.send(&Request::get("/not/an/api/surface"));

    assert_eq!(resp.status, 400, "unidentified protocol maps to 400");
    let json: serde_json::Value = serde_json::from_slice(&resp.body).unwrap();
    // No provider protocol to mirror -> PingoGate-native envelope (FR-036).
    assert_eq!(json["error"]["kind"], "pingogate.unknown_protocol");
}
