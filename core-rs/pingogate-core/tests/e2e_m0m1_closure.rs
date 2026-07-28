//! S3 Task 32: end-to-end M0+M1 closure tests (BYOK + virtual key + admin-
//! blind + usage). Three `#[ignore]` integration tests requiring BOTH the Go
//! control plane (`pingogate-ctrl`) and the Rust kernel (`pingogate-core`)
//! running in platform mode with mTLS certs.
//!
//! ## Prerequisites
//! 1. mTLS certs at `ctrl-go/internal/grpcmtls/certs/` (`go test
//!    ./internal/grpcmtls/ -run TestEnsureCerts` or start pingogate-ctrl once).
//! 2. Go binary: `cd ctrl-go && go build -o pingogate-ctrl ./cmd/pingogate-ctrl`.
//! 3. Run: `cargo test -p pingogate-core --test e2e_m0m1_closure -- --ignored`.
//!
//! ## Platform data plane (wired in commit 3c49b93)
//!
//! `run_platform` starts BOTH the gRPC server (background) AND the Pingora data
//! plane (public + admin listeners, foreground): VirtualKeyAuth + per-request
//! KeyVault decrypt + usage reporter. The Go builder pushes providers + virtual
//! keys + encrypted keys but NOT routes (`Routes: nil`). Without a route, step 5
//! asserts 404 NoRoute (not 200). This still proves the data plane is wired:
//! the request reaches the public listener, vkey auth succeeds (not 401), and
//! the routing stage returns a structured error. When routes are added, step 5
//! should assert 200 + mock body, and step 9 should assert a usage row exists.

#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use std::time::Duration;

use common::closure::{
    http_get_json, http_post_json, http_post_to_rust, query_usage_count, spawn_go_ctrl,
    spawn_rust_platform, wait_for_http_ready, ClosureEnv, BOOTSTRAP_ADMIN_TOKEN,
    USER_A_PROVIDER_KEY_PLAINTEXT,
};
use common::mock::MockUpstream;

// ---------------------------------------------------------------------------
// Test 1: full M0+M1 closure (BYOK + virtual key + admin-blind + usage)
// ---------------------------------------------------------------------------

/// Drive the 9-step M0+M1 closure flow end-to-end. `#[ignore]` (requires
/// binaries + certs; see module docstring).
///
/// Steps: 1) start Go+Rust+mock. 2) bootstrap admin -> create user-A.
/// 3) user-A records BYOK key (Rust KeyVault encrypt). 4) user-A issues vkey.
/// 5) user-A calls Rust public listener with vkey -> VirtualKeyAuth validates
///    -> routing. Currently 404 NoRoute (no routes in snapshot); when routes
///    land, assert 200. 6) admin GET key -> 404 (ownership). 7) user-A Reveal
///    -> 200. 8) admin Reveal -> 404. 9) usage table (empty until routes land).
#[tokio::test]
#[ignore = "requires mTLS certs + Go binary built; see module docstring for setup"]
async fn full_closure_user_calls_with_byok_key_admin_blind() {
    let env = ClosureEnv::setup();
    let _rust = spawn_rust_platform(&env);
    let _go = spawn_go_ctrl(&env);
    wait_for_http_ready(&env.http_addr(), Duration::from_secs(30));

    // Mock upstream (echoes request body + observed headers back as 200).
    let mock = MockUpstream::start();
    let mock_base_url = format!("http://127.0.0.1:{}", mock.port);

    // Step 2a: bootstrap admin is created by Go on first start (from
    // PINGO_BOOTSTRAP_ADMIN_TOKEN). Verify it can authenticate.
    let admin_token = BOOTSTRAP_ADMIN_TOKEN;
    let users = http_get_json(&env, "/api/users", admin_token);
    assert_eq!(users.status, 200, "admin list users: {}", users.body_str());
    let users_arr: serde_json::Value =
        serde_json::from_slice(&users.body).expect("users JSON");
    let initial_count = users_arr.as_array().map(|a| a.len()).unwrap_or(0);
    assert_eq!(initial_count, 1, "bootstrap admin should be the only user");

    // Step 2b: admin creates user-A.
    let create_user = http_post_json(
        &env,
        "/api/users",
        admin_token,
        r#"{"email":"user-a@example.com"}"#,
    );
    assert_eq!(
        create_user.status, 201,
        "create user-A: {}",
        create_user.body_str()
    );
    let user_a: serde_json::Value =
        serde_json::from_slice(&create_user.body).expect("user-A JSON");
    let user_a_token = user_a["token"].as_str().expect("user-A token").to_string();
    let user_a_id = user_a["id"].as_str().expect("user-A id").to_string();

    // Step 3: user-A records a BYOK provider key. Go calls Rust KeyVault.Encrypt
    // (gRPC) -> ciphertext stored in DB. base_url points to the mock upstream
    // so the data plane can forward to it when routes are added.
    let create_key = http_post_json(
        &env,
        "/api/provider-keys",
        &user_a_token,
        &format!(
            r#"{{"provider_type":"openai","plaintext_key":"{}","base_url":"{}"}}"#,
            USER_A_PROVIDER_KEY_PLAINTEXT, mock_base_url
        ),
    );
    assert_eq!(
        create_key.status, 201,
        "user-A record provider key: {}",
        create_key.body_str()
    );
    let pk: serde_json::Value =
        serde_json::from_slice(&create_key.body).expect("provider-key JSON");
    let pk_id = pk["id"].as_str().expect("provider-key id").to_string();
    let pk_last4 = pk["key_last4"].as_str().expect("key_last4").to_string();
    let body_str = create_key.body_str();
    assert!(
        !body_str.contains("encrypted_key"),
        "response must not contain encrypted_key field: {body_str}"
    );
    assert!(
        !body_str.contains(USER_A_PROVIDER_KEY_PLAINTEXT),
        "response must not contain plaintext (LEAKMARKER): {body_str}"
    );
    assert_eq!(
        pk_last4,
        &USER_A_PROVIDER_KEY_PLAINTEXT[USER_A_PROVIDER_KEY_PLAINTEXT.len() - 4..],
        "key_last4 must be the last 4 chars of the plaintext"
    );

    // Step 4: user-A issues a virtual key linked to the BYOK provider key.
    let issue_vkey = http_post_json(
        &env,
        "/api/virtual-keys",
        &user_a_token,
        &format!(r#"{{"provider_key_id":"{}"}}"#, pk_id),
    );
    assert_eq!(
        issue_vkey.status, 201,
        "user-A issue vkey: {}",
        issue_vkey.body_str()
    );
    let vkey_body = issue_vkey.body_str();
    assert!(
        !vkey_body.contains("token_hash"),
        "vkey response must not contain token_hash: {vkey_body}"
    );
    let vkey: serde_json::Value =
        serde_json::from_slice(&issue_vkey.body).expect("vkey JSON");
    let vkey_token = vkey["token"]
        .as_str()
        .expect("vkey response must include token")
        .to_string();

    // Step 5: user-A uses the vkey through the Rust public data-plane listener.
    // VirtualKeyAuth validates the vkey (SHA-256 hash + snapshot lookup). The
    // Go builder does not push routes, so resolve_route returns NoRoute (404).
    // This proves the data plane is wired: request reaches the public listener,
    // vkey auth succeeds (not 401), error is structured (not conn-refused/500).
    // When routes land: assert_eq!(resp.status, 200) + body contains mock echo.
    let chat_body = r#"{"model":"gpt-4o","messages":[{"role":"user","content":"hi"}]}"#;
    let resp = http_post_to_rust(&env, "/v1/chat/completions", &vkey_token, chat_body);
    assert_ne!(
        resp.status, 401,
        "vkey must authenticate (data plane wired): {}",
        resp.body_str()
    );
    assert_ne!(
        resp.status, 0,
        "Rust public listener must be reachable (data plane wired)"
    );
    // NoRoute = 404 (the Go builder pushes providers but no routes yet).
    assert_eq!(
        resp.status, 404,
        "expected NoRoute (no routes in snapshot); got {}: {}",
        resp.status,
        resp.body_str()
    );
    let resp_body = resp.body_str();
    assert!(
        resp_body.contains("no_route") || resp_body.contains("NoRoute"),
        "error body should be a NoRoute error: {resp_body}"
    );

    // Step 6: admin GET /api/provider-keys/:id -> 404 (non-owner). Go handler
    // enforces ownership (404 not 200+last4, to avoid leaking key existence).
    let admin_get = http_get_json(
        &env,
        &format!("/api/provider-keys/{}", pk_id),
        admin_token,
    );
    assert_eq!(
        admin_get.status, 404,
        "admin GET non-owned key should be 404 (existence-hidden): {}",
        admin_get.body_str()
    );
    let admin_get_body = admin_get.body_str();
    assert!(
        !admin_get_body.contains(USER_A_PROVIDER_KEY_PLAINTEXT),
        "admin GET must not leak plaintext: {admin_get_body}"
    );

    // Step 7: user-A Reveal -> 200 + plaintext. Go L1 (created_by) + Rust L2
    // (owner via snapshot.key_owners) both pass.
    let user_a_reveal = http_get_json(
        &env,
        &format!("/api/provider-keys/{}/reveal", pk_id),
        &user_a_token,
    );
    assert_eq!(
        user_a_reveal.status, 200,
        "user-A reveal (creator): {}",
        user_a_reveal.body_str()
    );
    let reveal: serde_json::Value =
        serde_json::from_slice(&user_a_reveal.body).expect("reveal JSON");
    assert_eq!(
        reveal["plaintext_key"].as_str().expect("plaintext_key"),
        USER_A_PROVIDER_KEY_PLAINTEXT,
        "user-A reveal must return the original plaintext"
    );
    assert_eq!(
        reveal["key_id"].as_str().expect("key_id"),
        pk_id,
        "reveal must echo the key_id"
    );

    // Step 8: admin Reveal -> 404 (Go L1 rejects non-creator). 404 not 403 to
    // avoid leaking key existence. Rust L2 never reached (Go short-circuits).
    let admin_reveal = http_get_json(
        &env,
        &format!("/api/provider-keys/{}/reveal", pk_id),
        admin_token,
    );
    assert_eq!(
        admin_reveal.status, 404,
        "admin reveal (non-creator) should be 404 (existence-hidden): {}",
        admin_reveal.body_str()
    );
    let admin_reveal_body = admin_reveal.body_str();
    assert!(
        !admin_reveal_body.contains(USER_A_PROVIDER_KEY_PLAINTEXT),
        "admin reveal must not leak plaintext: {admin_reveal_body}"
    );

    // Step 9: verify usage table. The request reached the data plane but failed
    // at routing (NoRoute), so no UsageEvent was pushed. Assert empty.
    // When routes land: assert!(count >= 1).
    let usage_count = query_usage_count(env.db_path(), &user_a_id);
    assert_eq!(
        usage_count, 0,
        "usage table should be empty (NoRoute -> no UsageEvent pushed)"
    );
}

// ---------------------------------------------------------------------------
// Test 2: six interfaces homomorphic passthrough (skeleton)
// ---------------------------------------------------------------------------

/// Six interfaces pass through the platform-mode gateway unchanged
/// (homomorphic passthrough). `#[ignore]` until routes are pushed in the
/// snapshot. The S1 e2e_* tests cover standalone mode; this is the
/// platform-mode dual (VirtualKeyAuth + per-request KeyVault decrypt).
#[tokio::test]
#[ignore = "requires routes in snapshot (Go builder does not push routes yet)"]
async fn six_interfaces_homomorphic_passthrough() {
    eprintln!(
        "six_interfaces_homomorphic_passthrough: SKIPPED - requires routes \
         in snapshot. S1 e2e_* tests cover standalone mode."
    );
}

// ---------------------------------------------------------------------------
// Test 3: usage extracted and persisted (skeleton)
// ---------------------------------------------------------------------------

/// OpenAI Chat streaming (T28 include_usage injection) -> T27 inline usage
/// extraction -> T31 Go UsageService persistence. `#[ignore]` until routes
/// are pushed. Go-side persistence covered by usage_test.go; this is the
/// end-to-end dual.
#[tokio::test]
#[ignore = "requires routes in snapshot (Go builder does not push routes yet)"]
async fn usage_extracted_and_persisted() {
    eprintln!(
        "usage_extracted_and_persisted: SKIPPED - requires routes in \
         snapshot. Go-side persistence covered by usage_test.go."
    );
}
