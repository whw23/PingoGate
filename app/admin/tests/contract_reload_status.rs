//! US3 contract test (T045): `/reload/status` reflects the last reload outcome.
//!
//! Before any reload the status is `pending` at version 1. After a successful
//! reload it reports `success` at the new version with a rollback target; after
//! a rejected reload it reports `rejected`, carries the errors, and the active
//! version is unchanged (constitution XII).
#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use common::{admin, base_config, body_json, config_with_routes, fixture};

#[test]
fn status_starts_pending_at_initial_version() {
    let fx = fixture("status-init", &base_config());
    let resp = fx.handler.dispatch(&admin(), "GET", "/reload/status", b"");
    assert_eq!(resp.status(), 200);
    let body = body_json(&resp);
    assert_eq!(body["last_result"], "pending");
    assert_eq!(body["active_version"], 1);
    assert!(body["timestamp"].as_str().is_some());
}

#[test]
fn status_reflects_successful_reload() {
    let fx = fixture("status-ok", &base_config());
    let _ = fx.handler.dispatch(&admin(), "POST", "/reload", b"");

    let resp = fx.handler.dispatch(&admin(), "GET", "/reload/status", b"");
    let body = body_json(&resp);
    assert_eq!(body["last_result"], "success");
    assert_eq!(body["active_version"], 2);
    assert_eq!(body["rollback_target"], 1);
    assert!(body["errors"].as_array().unwrap().is_empty());
}

#[test]
fn status_reflects_rejected_reload() {
    let fx = fixture("status-bad", &base_config());
    let bad =
        config_with_routes("  - { alias: \"a\", provider: \"ghost\", upstream_model: \"m\" }\n");
    std::fs::write(&fx.config_path, bad).unwrap();
    let _ = fx.handler.dispatch(&admin(), "POST", "/reload", b"");

    let resp = fx.handler.dispatch(&admin(), "GET", "/reload/status", b"");
    let body = body_json(&resp);
    assert_eq!(body["last_result"], "rejected");
    // The rejected reload did not advance the active version.
    assert_eq!(body["active_version"], 1);
    assert!(!body["errors"].as_array().unwrap().is_empty());
}
