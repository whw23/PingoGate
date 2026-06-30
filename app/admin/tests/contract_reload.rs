//! US3 contract test (T044): `/reload` applies a valid candidate, rejects a bad one.
//!
//! A successful reload returns `200 {result:"success", active_version, rollback_target}`
//! and advances the active version; a rejected candidate returns
//! `422 {result:"rejected", errors}` and leaves the active snapshot serving
//! (constitution XII — a bad reload never takes the gateway down).
#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use common::{admin, base_config, body_json, config_with_routes, fixture, gateway_key};

#[test]
fn valid_reload_succeeds_and_advances_version() {
    let fx = fixture("reload-ok", &base_config());
    let resp = fx.handler.dispatch(&admin(), "POST", "/reload", b"");
    assert_eq!(resp.status(), 200);
    let body = body_json(&resp);
    assert_eq!(body["result"], "success");
    assert_eq!(body["active_version"], 2);
    assert_eq!(body["rollback_target"], 1);

    // The active snapshot now reports the new version.
    let ready = fx.handler.dispatch(&admin(), "GET", "/readyz", b"");
    assert_eq!(body_json(&ready)["active_version"], 2);
}

#[test]
fn rejected_reload_keeps_active_snapshot() {
    let fx = fixture("reload-bad", &base_config());
    // Rewrite the on-disk config the reloader reads from to an invalid candidate
    // (a route pointing at a provider that does not exist).
    let bad =
        config_with_routes("  - { alias: \"a\", provider: \"ghost\", upstream_model: \"m\" }\n");
    std::fs::write(&fx.config_path, bad).unwrap();

    let resp = fx.handler.dispatch(&admin(), "POST", "/reload", b"");
    assert_eq!(resp.status(), 422);
    let body = body_json(&resp);
    assert_eq!(body["result"], "rejected");
    assert!(!body["errors"].as_array().unwrap().is_empty());

    // The active snapshot is untouched — still version 1.
    let ready = fx.handler.dispatch(&admin(), "GET", "/readyz", b"");
    assert_eq!(body_json(&ready)["active_version"], 1);
}

#[test]
fn reload_requires_admin_principal() {
    let fx = fixture("reload-authz", &base_config());
    let resp = fx.handler.dispatch(&gateway_key(), "POST", "/reload", b"");
    assert_eq!(resp.status(), 403);
}
