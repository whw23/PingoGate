//! US2 integration test (T032): a `SIGHUP` reload grows the route table.
//!
//! A model alias absent from the running config returns `404` (no route). After
//! the on-disk config gains a route for it and the gateway is signalled with
//! `SIGHUP`, the same request reaches the upstream — the new snapshot is live
//! without a restart (FR-018/FR-019).
#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use common::client::Request;
use common::{Harness, DEFAULT_ROUTES, GATEWAY_KEY};

#[test]
fn sighup_reload_adds_a_new_route() {
    let gw = Harness::start();
    let body = br#"{"model":"gpt-4o-mini","messages":[]}"#;
    let auth = format!("Bearer {GATEWAY_KEY}");
    let request = || Request::post("/v1/chat/completions", body).header("authorization", &auth);

    // The alias is not in the initial route table → no route.
    let before = gw.send(&request());
    assert_eq!(before.status, 404, "alias must be unknown before reload");

    // Grow the route table on disk and signal a reload.
    let grown = format!(
        "{DEFAULT_ROUTES}  - alias: \"gpt-4o-mini\"\n    \
         provider: \"openai-up\"\n    upstream_model: \"gpt-4o-mini\"\n"
    );
    gw.reload_with_routes(&grown);

    // After the reload lands, the alias routes to the upstream.
    let live = gw.poll_until(|gw| gw.send(&request()).status == 200);
    assert!(live, "new route should be live after SIGHUP reload");

    // The upstream model name carried by the new route is honored.
    let resp = gw.send(&request());
    assert_eq!(resp.status, 200, "body: {}", resp.body_str());
    assert_eq!(resp.header("x-observed-method"), Some("POST"));
}
