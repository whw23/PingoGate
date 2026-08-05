//! S1 e2e tests: Gemini generateContent / streamGenerateContent passthrough.
//!
//! Gemini is routed by the `{model}` in the path (`/models/{model}:action`);
//! the gateway injects the upstream credential as a `?key=` query parameter and
//! forwards the body unchanged. The gateway key is presented as a bearer token
//! here. Both surfaces share `ProtocolKind::Gemini`; `:streamGenerateContent`
//! additionally marks the request as streaming by path.
//!
//! Ported from 001 (`app/pingogate/tests/us1_gemini.rs`).
#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use common::client::Request;
use common::{Harness, GATEWAY_KEY, GEMINI_UPSTREAM_KEY};

#[test]
fn gemini_generate_content_passthrough_appends_query_key() {
    let gw = Harness::start();
    let body = br#"{"contents":[{"parts":[{"text":"hi"}]}]}"#;
    let path = "/v1beta/models/gemini-1.5-pro:generateContent";

    let resp = gw
        .send(&Request::post(path, body).header("authorization", &format!("Bearer {GATEWAY_KEY}")));

    assert_eq!(resp.status, 200, "body: {}", resp.body_str());
    // Upstream key appended as a query parameter (FR-012, Gemini query-key auth).
    // The gateway percent-encodes the key value (security: prevents query
    // injection). Hyphens encode to %2D under NON_ALPHANUMERIC.
    let expected_key = GEMINI_UPSTREAM_KEY.replace('-', "%2D");
    let observed_path = resp.header("x-observed-path").unwrap_or("");
    assert!(
        observed_path.starts_with(path),
        "path prefix preserved: {observed_path}"
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
fn gemini_stream_generate_content_passthrough_is_relayed() {
    let gw = Harness::start();
    let body = br#"{"contents":[{"parts":[{"text":"hi"}]}],"stream":true}"#;
    let path = "/v1beta/models/gemini-1.5-pro:streamGenerateContent";

    let resp = gw
        .send(&Request::post(path, body).header("authorization", &format!("Bearer {GATEWAY_KEY}")));

    assert_eq!(resp.status, 200, "body: {}", resp.body_str());
    // The streaming response is relayed as SSE (SC-006: no buffering). The
    // mock's streaming branch does not echo the observed path, so auth/query-key
    // verification is covered by the non-streaming generateContent test above.
    assert_eq!(
        resp.header("content-type"),
        Some("text/event-stream"),
        "SSE content-type relayed"
    );
    let text = resp.body_str();
    assert!(text.contains("data:"), "SSE frames relayed: {text}");
    assert!(text.contains("[DONE]"), "stream terminator relayed: {text}");
}

#[test]
fn gemini_generate_content_without_gateway_key_is_rejected() {
    let gw = Harness::start();
    let body = br#"{"contents":[{"parts":[{"text":"hi"}]}]}"#;
    let path = "/v1beta/models/gemini-1.5-pro:generateContent";

    let resp = gw.send(&Request::post(path, body));

    assert_eq!(resp.status, 401);
    // Gemini error envelope shared by generateContent and Interactions.
    let json: serde_json::Value = serde_json::from_slice(&resp.body).unwrap();
    assert_eq!(json["error"]["status"], "UNAUTHENTICATED");
}
