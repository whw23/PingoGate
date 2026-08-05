//! S1 e2e test: Gemini Interactions API passthrough.
//!
//! `POST /v1beta/interactions` (top-level resource) is identified as
//! `ProtocolKind::GeminiInteractions`, authenticated by gateway key (Bearer),
//! routed by the `model` field in the JSON body, and forwarded with the gateway
//! key stripped and the upstream credential injected as a `?key=` query
//! parameter - body byte-for-byte unchanged. The legacy Gemini
//! `generateContent`/`streamGenerateContent` and `:interact` surfaces were
//! removed; the Interactions API is Gemini's primary interface.
//!
//! The mock upstream echoes the request body verbatim, which lets us assert the
//! body was forwarded unchanged. A real Gemini Interactions endpoint would
//! return an interaction response; the echo is a stronger assertion for
//! passthrough verification (constitution IX: upstream responses are passed
//! through untouched, FR-008).
#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use common::client::Request;
use common::{Harness, GATEWAY_KEY, GEMINI_UPSTREAM_KEY};

#[test]
fn gemini_interactions_passthrough_appends_query_key() {
    let gw = Harness::start();
    let body = br#"{"model":"gemini-1.5-pro","input":{"contents":[{"parts":[{"text":"hi"}]}]}}"#;
    let path = "/v1beta/interactions";

    let resp = gw.send(
        &Request::post(path, body)
            .header("authorization", &format!("Bearer {GATEWAY_KEY}")),
    );

    assert_eq!(resp.status, 200, "body: {}", resp.body_str());
    assert_eq!(resp.header("x-observed-method"), Some("POST"));
    // Upstream key appended as a query parameter (FR-012, Gemini query-key auth).
    // The gateway percent-encodes the key value (security: prevents query
    // injection). Hyphens encode to %2D under NON_ALPHANUMERIC.
    let expected_key = GEMINI_UPSTREAM_KEY.replace('-', "%2D");
    let observed_path = resp.header("x-observed-path").unwrap_or("");
    assert!(
        observed_path.starts_with(path),
        "interactions path preserved: {observed_path}"
    );
    assert!(
        observed_path.contains(&format!("key={expected_key}")),
        "upstream key appended as query param (percent-encoded): {observed_path}"
    );
    // Gateway bearer key must NOT be forwarded upstream.
    assert_eq!(resp.header("x-observed-authorization"), Some(""));
    // Body forwarded unchanged (research R1: no body transform this phase).
    assert_eq!(resp.body, body);
}

#[test]
fn gemini_interactions_without_gateway_key_is_rejected() {
    let gw = Harness::start();
    let body = br#"{"model":"gemini-1.5-pro","input":{"contents":[{"parts":[{"text":"hi"}]}]}}"#;
    let path = "/v1beta/interactions";

    let resp = gw.send(&Request::post(path, body));

    assert_eq!(resp.status, 401);
    // Interactions shares the Gemini error envelope (T8 adapter collapse).
    let json: serde_json::Value = serde_json::from_slice(&resp.body).unwrap();
    assert_eq!(json["error"]["status"], "UNAUTHENTICATED");
}

#[test]
fn gemini_interactions_unknown_model_returns_native_not_found() {
    let gw = Harness::start();
    let body = br#"{"model":"does-not-exist","input":{"contents":[{"parts":[{"text":"hi"}]}]}}"#;
    let path = "/v1beta/interactions";

    let resp = gw.send(
        &Request::post(path, body)
            .header("authorization", &format!("Bearer {GATEWAY_KEY}")),
    );

    assert_eq!(resp.status, 404, "no route maps to 404");
    let json: serde_json::Value = serde_json::from_slice(&resp.body).unwrap();
    assert_eq!(json["error"]["status"], "NOT_FOUND");
}
