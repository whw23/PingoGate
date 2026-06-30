//! US2 integration test (T034): an in-flight request binds its snapshot.
//!
//! A request parked at the upstream holds the `Arc<RuntimeSnapshot>` it loaded
//! when it began. A `SIGHUP` reload that removes that very route swaps the
//! active snapshot, but the parked request still completes successfully against
//! the snapshot it bound — a reload never disturbs in-flight traffic
//! (constitution XII / FR-021). A fresh request afterward sees the new snapshot.
#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use std::thread;
use std::time::Duration;

use common::client::{self, Request};
use common::{Harness, GATEWAY_KEY};

#[test]
fn in_flight_request_binds_old_snapshot_across_reload() {
    let (gw, gate) = Harness::start_gated();

    // Fire a request that routes to the (gated) upstream and parks there. The
    // client runs on its own thread with owned data so it outlives this frame.
    let port = gw.public_port();
    let auth = format!("Bearer {GATEWAY_KEY}");
    let inflight = thread::spawn(move || {
        let req = Request::post(
            "/v1/chat/completions",
            br#"{"model":"gpt-4o","messages":[]}"#,
        )
        .header("authorization", &auth);
        client::send(port, &req)
    });
    gate.wait_for_request(); // the request is now provably in-flight upstream

    // Reload to a config that drops the gpt-4o route, then let it land.
    gw.reload_with_routes(
        "  - alias: \"claude-3-5-sonnet\"\n    provider: \"anthropic-up\"\n    \
         upstream_model: \"claude-3-5-sonnet\"\n",
    );
    thread::sleep(Duration::from_millis(400));

    // Release the parked upstream; the in-flight request completes on the
    // snapshot it bound, even though that route no longer exists.
    gate.release();
    let resp = inflight.join().expect("in-flight thread");
    assert_eq!(
        resp.status,
        200,
        "in-flight request must complete on its bound snapshot: {}",
        resp.body_str()
    );

    // A fresh request sees the new snapshot: gpt-4o is gone.
    let after = gw.poll_until(|gw| {
        gw.send(
            &Request::post(
                "/v1/chat/completions",
                br#"{"model":"gpt-4o","messages":[]}"#,
            )
            .header("authorization", &format!("Bearer {GATEWAY_KEY}")),
        )
        .status
            == 404
    });
    assert!(after, "removed route must be gone from the active snapshot");
}
