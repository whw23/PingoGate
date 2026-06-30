//! US3 contract test (T046): the admin authentication + authorization boundary.
//!
//! Two layers, tested at their seams (constitution XX, FR-024/FR-025):
//!   1. `BootstrapAuth` *authenticates*: a correct bearer token yields an admin
//!      principal; a missing, malformed, or wrong token yields `None` (the live
//!      server turns that into `401`) — never a partial trust.
//!   2. Every privileged endpoint *authorizes* through `AuthContext::authorize`,
//!      so a data-plane gateway-key principal is rejected with `403` on every
//!      route — there is no global-token shortcut.
#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use common::{base_config, fixture, gateway_key};

use pingo_admin::BootstrapAuth;
use pingo_core::{PrincipalKind, SecretString};

fn bootstrap() -> BootstrapAuth {
    BootstrapAuth::new(SecretString::new("admin-secret"))
}

#[test]
fn correct_token_authenticates_as_admin() {
    let principal = bootstrap()
        .authenticate(Some("Bearer admin-secret"))
        .expect("correct token authenticates");
    assert_eq!(principal.kind, PrincipalKind::Admin);
}

#[test]
fn missing_or_wrong_credentials_are_rejected() {
    let auth = bootstrap();
    assert!(
        auth.authenticate(None).is_none(),
        "absent header → no principal"
    );
    assert!(
        auth.authenticate(Some("admin-secret")).is_none(),
        "no scheme → no principal"
    );
    assert!(
        auth.authenticate(Some("Bearer ")).is_none(),
        "empty token → no principal"
    );
    assert!(
        auth.authenticate(Some("Bearer wrong")).is_none(),
        "wrong token → no principal"
    );
}

#[test]
fn gateway_key_principal_is_forbidden_on_every_endpoint() {
    let fx = fixture("authz-all", &base_config());
    let endpoints = [
        ("GET", "/healthz"),
        ("GET", "/readyz"),
        ("POST", "/config/validate"),
        ("POST", "/reload"),
        ("GET", "/reload/status"),
    ];
    for (method, path) in endpoints {
        let resp = fx
            .handler
            .dispatch(&gateway_key(), method, path, base_config().as_bytes());
        assert_eq!(
            resp.status(),
            403,
            "{method} {path} must reject a gateway-key principal"
        );
    }
}
