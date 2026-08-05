//! Tests for [`VirtualKeyAuth`] (constitution XX; S3).
//!
//! Extracted from `virtual_key_auth.rs` to keep that module under the
//! constitution V file-size limit (≤300 lines).

use super::*;
use pingogate_snapshot::{UpstreamConfig, VirtualKeyEntry};
use std::collections::HashMap;

fn vkey(
    id: &str,
    token: &str,
    owner: &str,
    max_concurrency: i32,
    expires_at: i64,
    enabled: bool,
) -> VirtualKeyEntry {
    VirtualKeyEntry {
        id: id.to_string(),
        token_hash: hex_sha256(token.as_bytes()),
        owner_user_id: owner.to_string(),
        provider_key_id: "pk-1".to_string(),
        allowed_models: Vec::new(),
        allowed_providers: Vec::new(),
        expires_at,
        max_concurrency,
        enabled,
    }
}

fn snapshot_with(vkeys: Vec<VirtualKeyEntry>) -> RuntimeSnapshot {
    RuntimeSnapshot {
        version: 1,
        providers: Vec::new(),
        routes: Vec::new(),
        gateway_keys: Vec::new(),
        virtual_keys: vkeys,
        key_owners: HashMap::new(),
        encrypted_keys: HashMap::new(),
        upstream: UpstreamConfig {
            timeout_ms: 30_000,
            http_proxy: None,
            https_proxy: None,
        },
    }
}

#[test]
fn authenticates_valid_token_returns_principal() {
    let snap = snapshot_with(vec![vkey("vk-1", "pg_vkey_secret", "user-7", 0, 0, true)]);
    let auth = VirtualKeyAuth::new();
    let p = auth
        .authenticate("pg_vkey_secret", &snap)
        .expect("valid token");
    assert_eq!(p.id, "vk-1");
    assert_eq!(p.kind, PrincipalKind::VirtualKey);
    assert_eq!(p.owner_user_id.as_deref(), Some("user-7"));
}

#[test]
fn rejects_wrong_token() {
    let snap = snapshot_with(vec![vkey("vk-1", "pg_vkey_secret", "user-7", 0, 0, true)]);
    let auth = VirtualKeyAuth::new();
    assert!(auth.authenticate("wrong", &snap).is_none());
}

#[test]
fn rejects_disabled_key() {
    let snap = snapshot_with(vec![vkey("vk-1", "tok", "u", 0, 0, false)]);
    let auth = VirtualKeyAuth::new();
    assert!(auth.authenticate("tok", &snap).is_none());
}

#[test]
fn rejects_expired_key() {
    // expires_at in the past (1 second ago).
    let past = now_unix_secs() as i64 - 1;
    let snap = snapshot_with(vec![vkey("vk-1", "tok", "u", 0, past, true)]);
    let auth = VirtualKeyAuth::new();
    assert!(auth.authenticate("tok", &snap).is_none());
}

#[test]
fn accepts_non_expired_key() {
    // expires_at 1 hour in the future.
    let future = now_unix_secs() as i64 + 3600;
    let snap = snapshot_with(vec![vkey("vk-1", "tok", "u", 0, future, true)]);
    let auth = VirtualKeyAuth::new();
    assert!(auth.authenticate("tok", &snap).is_some());
}

#[test]
fn max_concurrency_zero_means_unlimited() {
    let snap = snapshot_with(vec![vkey("vk-1", "tok", "u", 0, 0, true)]);
    let auth = VirtualKeyAuth::new();
    // Authenticate many times without release; all should succeed.
    for _ in 0..100 {
        assert!(auth.authenticate("tok", &snap).is_some());
    }
    // Counter was never incremented (max_concurrency == 0).
    assert_eq!(auth.in_flight("vk-1"), 0);
}

#[test]
fn enforces_concurrency_quota() {
    let snap = snapshot_with(vec![vkey("vk-1", "tok", "u", 2, 0, true)]);
    let auth = VirtualKeyAuth::new();
    // Two in-flight: both succeed.
    let p1 = auth.authenticate("tok", &snap).expect("first");
    let p2 = auth.authenticate("tok", &snap).expect("second");
    assert_eq!(auth.in_flight("vk-1"), 2);
    // Third is rejected (quota exhausted).
    assert!(auth.authenticate("tok", &snap).is_none());
    // Release one; the next auth succeeds.
    auth.release(&p1);
    assert_eq!(auth.in_flight("vk-1"), 1);
    let _p3 = auth.authenticate("tok", &snap).expect("after release");
    assert_eq!(auth.in_flight("vk-1"), 2);
    // Cleanup.
    auth.release(&p2);
}

#[test]
fn release_decrements_counter() {
    let snap = snapshot_with(vec![vkey("vk-1", "tok", "u", 5, 0, true)]);
    let auth = VirtualKeyAuth::new();
    let p = auth.authenticate("tok", &snap).expect("auth");
    assert_eq!(auth.in_flight("vk-1"), 1);
    auth.release(&p);
    assert_eq!(auth.in_flight("vk-1"), 0);
}

#[test]
fn release_clamps_at_zero_on_double_release() {
    let snap = snapshot_with(vec![vkey("vk-1", "tok", "u", 5, 0, true)]);
    let auth = VirtualKeyAuth::new();
    let p = auth.authenticate("tok", &snap).expect("auth");
    auth.release(&p);
    // Stray double-release must not underflow.
    auth.release(&p);
    assert_eq!(auth.in_flight("vk-1"), 0);
}

#[test]
fn release_ignores_non_virtual_key_principal() {
    let snap = snapshot_with(vec![vkey("vk-1", "tok", "u", 5, 0, true)]);
    let auth = VirtualKeyAuth::new();
    let _p = auth.authenticate("tok", &snap).expect("auth");
    assert_eq!(auth.in_flight("vk-1"), 1);
    // A gateway-key principal (standalone-mode) must not touch the counter.
    let gk = Principal::gateway_key("team-alpha");
    auth.release(&gk);
    assert_eq!(auth.in_flight("vk-1"), 1);
}

#[test]
fn hex_sha256_matches_known_value() {
    // SHA-256("abc") = ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad
    assert_eq!(
        hex_sha256(b"abc"),
        "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
    );
}

#[test]
fn authenticates_one_of_multiple_vkeys() {
    let snap = snapshot_with(vec![
        vkey("vk-1", "tok-a", "u-a", 0, 0, true),
        vkey("vk-2", "tok-b", "u-b", 0, 0, true),
    ]);
    let auth = VirtualKeyAuth::new();
    let p = auth.authenticate("tok-b", &snap).expect("found");
    assert_eq!(p.id, "vk-2");
    assert_eq!(p.owner_user_id.as_deref(), Some("u-b"));
}

#[test]
fn empty_snapshot_returns_none() {
    let snap = snapshot_with(Vec::new());
    let auth = VirtualKeyAuth::new();
    assert!(auth.authenticate("any", &snap).is_none());
}
