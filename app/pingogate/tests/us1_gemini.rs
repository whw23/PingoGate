//! US1 contract test (T019): Gemini generateContent passthrough.
//!
//! Gemini is routed by the `{model}` in the path; the gateway injects the
//! upstream credential as a `?key=` query parameter and forwards the body
//! unchanged. The gateway key is presented as a bearer token here.
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
    let observed_path = resp.header("x-observed-path").unwrap_or("");
    assert!(
        observed_path.starts_with(path),
        "path prefix preserved: {observed_path}"
    );
    assert!(
        observed_path.contains(&format!("key={GEMINI_UPSTREAM_KEY}")),
        "upstream key appended as query param: {observed_path}"
    );
    // Gateway bearer key must NOT be forwarded upstream.
    assert_eq!(resp.header("x-observed-authorization"), Some(""));
    assert_eq!(resp.body, body);
}
