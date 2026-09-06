//! E2E: `AuthMethod::TokenCommand` injects a Bearer header from an external
//! shell command (standalone Vertex AI auth path).
//!
//! A `token_command` provider declares `command: echo e2e-token`; the Rust
//! kernel runs it (via the shell) at auth time, caches the token for the TTL,
//! and injects `Authorization: Bearer e2e-token` upstream - no static key_ref.
//! This mirrors the real `gcloud auth application-default print-access-token`
//! command used for Vertex AI, with a fixed echo standing in for the token.
#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use common::client::Request;
use common::{Harness, GATEWAY_KEY};

#[test]
fn token_command_injects_bearer_from_shell() {
    let gw = Harness::start();
    let body = br#"{"model":"token-cmd-model","messages":[]}"#;
    let path = "/v1/chat/completions";

    let resp = gw.send(
        &Request::post(path, body)
            .header("authorization", &format!("Bearer {GATEWAY_KEY}")),
    );

    assert_eq!(resp.status, 200, "body: {}", resp.body_str());
    // Upstream saw the command's token as the Bearer credential.
    assert_eq!(
        resp.header("x-observed-authorization"),
        Some("Bearer e2e-token"),
        "shell command token injected as Bearer"
    );
    // Gateway bearer key must NOT be forwarded upstream.
    assert!(
        !resp
            .header("x-observed-authorization")
            .unwrap_or("")
            .contains(GATEWAY_KEY),
        "gateway key must not leak upstream"
    );
    // Body forwarded unchanged (research R1: no body transform this phase).
    assert_eq!(resp.body, body);
}
