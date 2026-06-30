//! US3 contract test (T042): liveness and readiness are machine-readable.
//!
//! `GET /healthz` reports liveness; `GET /readyz` reports readiness plus the
//! active config version (SC-007). Both pass through the authorization boundary.
#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use common::{admin, base_config, body_json, fixture};

#[test]
fn healthz_returns_ok_status() {
    let fx = fixture("health", &base_config());
    let resp = fx.handler.dispatch(&admin(), "GET", "/healthz", b"");
    assert_eq!(resp.status(), 200);
    assert_eq!(body_json(&resp)["status"], "ok");
}

#[test]
fn readyz_returns_ready_with_active_version() {
    let fx = fixture("ready", &base_config());
    let resp = fx.handler.dispatch(&admin(), "GET", "/readyz", b"");
    assert_eq!(resp.status(), 200);
    let body = body_json(&resp);
    assert_eq!(body["status"], "ready");
    assert_eq!(body["active_version"], 1);
}

#[test]
fn unknown_endpoint_returns_not_found() {
    let fx = fixture("nf", &base_config());
    let resp = fx.handler.dispatch(&admin(), "GET", "/nope", b"");
    assert_eq!(resp.status(), 404);
}
