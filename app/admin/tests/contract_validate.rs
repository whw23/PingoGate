//! US3 contract test (T043): `/config/validate` accepts/rejects without mutating.
//!
//! A valid candidate returns `{valid:true}`; an invalid one returns `422` with
//! structured `{path,message}` errors. Crucially, validation MUST NOT alter the
//! active runtime: the active snapshot version is unchanged afterward (FR-026).
#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use common::{admin, base_config, body_json, config_with_routes, fixture, gateway_key};

#[test]
fn valid_candidate_is_accepted() {
    let fx = fixture("val-ok", &base_config());
    let candidate =
        config_with_routes("  - { alias: \"b\", provider: \"p\", upstream_model: \"m2\" }\n");
    let resp = fx
        .handler
        .dispatch(&admin(), "POST", "/config/validate", candidate.as_bytes());
    assert_eq!(resp.status(), 200);
    assert_eq!(body_json(&resp)["valid"], true);
}

#[test]
fn invalid_candidate_is_rejected_with_structured_errors() {
    let fx = fixture("val-bad", &base_config());
    let candidate =
        config_with_routes("  - { alias: \"b\", provider: \"ghost\", upstream_model: \"m\" }\n");
    let resp = fx
        .handler
        .dispatch(&admin(), "POST", "/config/validate", candidate.as_bytes());
    assert_eq!(resp.status(), 422);
    let body = body_json(&resp);
    assert_eq!(body["valid"], false);
    assert!(body["errors"]
        .as_array()
        .unwrap()
        .iter()
        .any(|e| e["path"].as_str().unwrap_or("").ends_with("provider")));
}

#[test]
fn validation_does_not_touch_active_runtime() {
    let fx = fixture("val-pure", &base_config());
    // Validate a config that, if applied, would change the route table.
    let candidate = config_with_routes(
        "  - { alias: \"different\", provider: \"p\", upstream_model: \"m\" }\n",
    );
    let _ = fx
        .handler
        .dispatch(&admin(), "POST", "/config/validate", candidate.as_bytes());
    // The active snapshot still reports version 1 (no swap occurred).
    let resp = fx.handler.dispatch(&admin(), "GET", "/readyz", b"");
    assert_eq!(body_json(&resp)["active_version"], 1);
}

#[test]
fn config_validate_requires_admin_principal() {
    let fx = fixture("val-authz", &base_config());
    let resp = fx.handler.dispatch(
        &gateway_key(),
        "POST",
        "/config/validate",
        base_config().as_bytes(),
    );
    assert_eq!(resp.status(), 403);
}
