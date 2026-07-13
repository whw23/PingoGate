//! S1 e2e test: SSE streaming passthrough.
//!
//! A streaming request (`"stream": true`) is forwarded and the upstream
//! `text/event-stream` response is relayed back to the client unchanged
//! (SC-006: token-by-token passthrough needs no buffering).
//!
//! Ported from 001 (`app/pingogate/tests/us1_streaming.rs`).
#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use common::client::Request;
use common::{Harness, GATEWAY_KEY};

#[test]
fn openai_streaming_response_is_relayed() {
    let gw = Harness::start();
    let body = br#"{"model":"gpt-4o","stream":true,"messages":[{"role":"user","content":"hi"}]}"#;

    let resp = gw.send(
        &Request::post("/v1/chat/completions", body)
            .header("authorization", &format!("Bearer {GATEWAY_KEY}")),
    );

    assert_eq!(resp.status, 200);
    assert_eq!(
        resp.header("content-type"),
        Some("text/event-stream"),
        "SSE content-type relayed"
    );
    let text = resp.body_str();
    assert!(text.contains("data:"), "SSE frames relayed: {text}");
    assert!(text.contains("[DONE]"), "stream terminator relayed: {text}");
}
