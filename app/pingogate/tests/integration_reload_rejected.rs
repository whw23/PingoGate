//! US2 integration test (T033): a rejected candidate leaves the active config.
//!
//! When a `SIGHUP` reload points at a semantically invalid config (a route to a
//! provider that does not exist), the rebuild fails its validation. The active
//! snapshot is untouched: the original route keeps serving and the bad alias
//! never becomes live (FR-020 — failure-atomic reload).
#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use std::thread::sleep;
use std::time::Duration;

use common::client::Request;
use common::{Harness, GATEWAY_KEY};

#[test]
fn rejected_candidate_keeps_active_snapshot() {
    let gw = Harness::start();
    let auth = format!("Bearer {GATEWAY_KEY}");
    let good = || {
        Request::post(
            "/v1/chat/completions",
            br#"{"model":"gpt-4o","messages":[]}"#,
        )
        .header("authorization", &auth)
    };
    let bad = || {
        Request::post(
            "/v1/chat/completions",
            br#"{"model":"ghost-model","messages":[]}"#,
        )
        .header("authorization", &auth)
    };

    // Baseline: the default route works.
    assert_eq!(gw.send(&good()).status, 200);

    // Candidate references an unknown provider → semantic validation fails.
    gw.reload_with_routes(
        "  - alias: \"ghost-model\"\n    provider: \"does-not-exist\"\n    upstream_model: \"m\"\n",
    );

    // Give the signal handler time to attempt (and reject) the candidate.
    sleep(Duration::from_millis(400));

    // The original route still serves — the active snapshot was never swapped.
    assert_eq!(
        gw.send(&good()).status,
        200,
        "active route must survive a rejected reload"
    );
    // The bad candidate never took effect.
    assert_eq!(
        gw.send(&bad()).status,
        404,
        "rejected route must not become live"
    );
}
