//! S3 cross-validation contract (spec §11 S3 falsification): Rust side.
//!
//! Paired with `ctrl-go/internal/usage/s3_contract_test.go`. Hermetic tests
//! verifying the four S3 falsification criteria observable from the Rust
//! kernel: VirtualKeyAuth validates (T26), usage extractor O(1) memory (T27),
//! KeyVault owner intercept denies non-owner (T30), include_usage injected
//! for OpenAI Chat streaming (T28). S3 runs this file first; if any test
//! fails, S2/T26-T31 output is broken.
//!
//! Hermetic: no network, no files, no external processes.
#![cfg(test)]

use std::collections::HashMap;
use std::sync::Arc;

use pingogate_core_types::{now_unix_secs, PrincipalKind, ProtocolKind};
use pingogate_keyvault::grpc_service::{
    INTENT_HOT_PATH_INJECT, INTENT_VIEW_PLAINTEXT, KeyVaultGrpcService, OwnerLookup,
};
use pingogate_keyvault::proto::key_vault_service_server::KeyVaultService;
use pingogate_keyvault::AesGcmKeyVault;
use pingogate_pipeline::usage_extractor::UsageExtractor;
use pingogate_pipeline::virtual_key_auth::VirtualKeyAuth;
use pingogate_pipeline::KeyAuth;
use pingogate_snapshot::{RuntimeSnapshot, UpstreamConfig, VirtualKeyEntry};
use pingogate_transform::inject_include_usage;
use tonic::{Request, Status};

// ---------------------------------------------------------------------------
// 1. VirtualKeyAuth validates (T26)
// ---------------------------------------------------------------------------

/// A valid virtual-key token authenticates; the returned Principal carries
/// the vkey id + owner_user_id (the hot path uses owner to bind a decrypt
/// to the BYOK user). Wrong token / disabled / expired are rejected. This
/// is the Rust-side half of SC-3 / SC-4 (the Go half is in
/// `ctrl-go/internal/usage/s3_contract_test.go`).
#[test]
fn virtual_key_auth_validates() {
    let snap = snapshot_with(vec![vkey(
        "vk-1",
        "pg_vkey_secret_s3",
        "user-s3-owner",
        0,
        0,
        true,
    )]);
    let auth = VirtualKeyAuth::new();

    // Valid token -> Principal{id=vk-1, owner=user-s3-owner}.
    let p = auth
        .authenticate("pg_vkey_secret_s3", &snap)
        .expect("valid token must authenticate");
    assert_eq!(p.id, "vk-1");
    assert_eq!(p.kind, PrincipalKind::VirtualKey);
    assert_eq!(p.owner_user_id.as_deref(), Some("user-s3-owner"));

    // Wrong token -> None (SC-4: invalid key rejected at auth stage).
    assert!(auth.authenticate("wrong-token", &snap).is_none());

    // Disabled -> None (SC-4: revoked key rejected).
    let snap_disabled = snapshot_with(vec![vkey("vk-2", "tok", "u", 0, 0, false)]);
    assert!(VirtualKeyAuth::new().authenticate("tok", &snap_disabled).is_none());

    // Expired -> None (SC-4: expired key rejected).
    let past = now_unix_secs() as i64 - 1;
    let snap_expired = snapshot_with(vec![vkey("vk-3", "tok", "u", 0, past, true)]);
    assert!(VirtualKeyAuth::new().authenticate("tok", &snap_expired).is_none());
}

// ---------------------------------------------------------------------------
// 2. Usage extractor O(1) memory (T27)
// ---------------------------------------------------------------------------

/// Feeding 1000 non-marker SSE chunks does NOT accumulate them: each chunk
/// ends with `\n` so the residual stays at 0. Even under pathological
/// no-newline input, the residual is capped at MAX_RESIDUAL (64 KiB), not
/// the full feed. Constitution XXI: hot-path memory budget bounded.
#[test]
fn usage_extractor_o1_memory() {
    let mut ex = UsageExtractor::new(ProtocolKind::OpenAiCompatible);

    // 1000 complete SSE lines without usage: residual stays 0 (each ends \n).
    let chunk = b"data: {\"choices\":[{\"delta\":{\"content\":\"tok\"}}]}\n";
    for _ in 0..1000 {
        ex.on_body_chunk(chunk, false);
        assert_eq!(
            ex.residual_len(),
            0,
            "residual must stay 0 after each complete-line chunk"
        );
    }

    // Final chunk carries usage and is still extracted correctly (T27 contract).
    ex.on_body_chunk(
        b"data: {\"choices\":[],\"usage\":{\"prompt_tokens\":7,\"completion_tokens\":3}}\n",
        true,
    );
    let tokens = ex.finalize().expect("usage must be extracted");
    assert_eq!(tokens.input, 7);
    assert_eq!(tokens.output, 3);

    // Pathological: no newlines for many chunks -> residual capped, not unbounded.
    let mut ex2 = UsageExtractor::new(ProtocolKind::OpenAiCompatible);
    let big = vec![b'x'; 1024];
    for _ in 0..1000 {
        ex2.on_body_chunk(&big, false);
        // Cap enforced at every push (constitution XXI: < 64 KiB residual).
        assert!(ex2.residual_len() <= 64 * 1024, "residual exceeded cap");
    }
}

// ---------------------------------------------------------------------------
// 3. KeyVault owner intercept denies non-owner (T30 dual-defense L2)
// ---------------------------------------------------------------------------

/// In-memory `OwnerLookup` for the contract test (production impl reads the
/// live `RuntimeSnapshot` via `Arc<SnapshotHolder>`; here we substitute a
/// static map to stay hermetic).
struct MapOwnerLookup {
    owners: HashMap<String, String>,
}

impl OwnerLookup for MapOwnerLookup {
    fn owner_of(&self, key_id: &str) -> Option<String> {
        self.owners.get(key_id).cloned()
    }
}

/// Rust L2 dual-defense: `view_plaintext` decrypt with requester != owner is
/// rejected with `permission_denied` (not just audited). `hot_path_inject`
/// bypasses the owner check (the hot path authenticates via virtual key
/// scope; T25 snapshot invariants). This is the S3 contract for SC-2
/// (admin-blind) + spec §12A (Rust independent trust domain).
#[tokio::test]
async fn keyvault_owner_intercept_denies_non_owner() {
    let kv = Arc::new(AesGcmKeyVault::from_master_key(&[42u8; 32]));
    let mut owners = HashMap::new();
    owners.insert("k-s3".to_string(), "user-A".to_string());
    let lookup: Arc<dyn OwnerLookup> = Arc::new(MapOwnerLookup { owners });
    let svc = KeyVaultGrpcService::new(kv.clone(), lookup);

    let ct = kv
        .encrypt(b"sk-byok-user-a-s3-contract")
        .expect("encrypt must succeed");

    // Non-owner (user-B) requests view_plaintext -> permission_denied (SC-2).
    let err = svc
        .decrypt(Request::new(pingogate_keyvault::proto::DecryptRequest {
            ciphertext: ct.clone(),
            requester_user_id: "user-B".to_string(),
            key_id: "k-s3".to_string(),
            intent: INTENT_VIEW_PLAINTEXT.to_string(),
        }))
        .await
        .expect_err("non-owner view_plaintext must be rejected");
    assert_eq!(err.code(), Status::permission_denied("").code());
    assert!(err.message().contains("not key owner"));

    // Owner (user-A) requests view_plaintext -> succeeds, plaintext returned.
    let resp = svc
        .decrypt(Request::new(pingogate_keyvault::proto::DecryptRequest {
            ciphertext: ct.clone(),
            requester_user_id: "user-A".to_string(),
            key_id: "k-s3".to_string(),
            intent: INTENT_VIEW_PLAINTEXT.to_string(),
        }))
        .await
        .expect("owner view_plaintext must succeed");
    assert_eq!(resp.into_inner().plaintext, b"sk-byok-user-a-s3-contract");

    // hot_path_inject bypasses owner check (requester != owner, still succeeds).
    let resp = svc
        .decrypt(Request::new(pingogate_keyvault::proto::DecryptRequest {
            ciphertext: ct,
            requester_user_id: "user-B".to_string(),
            key_id: "k-s3".to_string(),
            intent: INTENT_HOT_PATH_INJECT.to_string(),
        }))
        .await
        .expect("hot_path_inject must skip owner check");
    assert_eq!(resp.into_inner().plaintext, b"sk-byok-user-a-s3-contract");
}

// ---------------------------------------------------------------------------
// 4. include_usage injected for OpenAI Chat streaming (T28)
// ---------------------------------------------------------------------------

/// The transform injects `stream_options.include_usage: true` into an OpenAI
/// Chat streaming request body so the provider returns usage in the terminal
/// SSE chunk (consumed by `UsageExtractor`). Non-streaming / non-OpenAI /
/// already-injected bodies are passed through unchanged. SC-10 contract
/// (Rust kernel must extract usage from every interface that carries it;
/// OpenAI Chat only carries usage in stream when `include_usage` is set).
#[test]
fn include_usage_injected_for_openai_stream() {
    let body = br#"{"model":"gpt-4o","messages":[],"stream":true}"#;
    let out = inject_include_usage(body, ProtocolKind::OpenAiCompatible, true);
    let v: serde_json::Value = serde_json::from_slice(&out).expect("valid JSON");
    assert_eq!(v["stream_options"]["include_usage"], true);
    assert_eq!(v["model"], "gpt-4o");
    assert_eq!(v["stream"], true);

    // Non-streaming -> no injection.
    let non_stream = br#"{"model":"gpt-4o","messages":[]}"#;
    assert_eq!(
        inject_include_usage(non_stream, ProtocolKind::OpenAiCompatible, false),
        non_stream,
    );

    // Non-OpenAI -> no injection (Anthropic / Gemini carry usage natively).
    let anth = br#"{"model":"claude-3-5","messages":[],"stream":true}"#;
    assert_eq!(
        inject_include_usage(anth, ProtocolKind::Anthropic, true),
        anth,
    );

    // Already-injected -> idempotent (flag stays true, no duplicate keys).
    let already = br#"{"stream":true,"stream_options":{"include_usage":true}}"#;
    let out = inject_include_usage(already, ProtocolKind::OpenAiCompatible, true);
    let v: serde_json::Value = serde_json::from_slice(&out).expect("valid JSON");
    assert_eq!(v["stream_options"]["include_usage"], true);
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn vkey(id: &str, token: &str, owner: &str, mc: i32, exp: i64, en: bool) -> VirtualKeyEntry {
    VirtualKeyEntry {
        id: id.to_string(),
        token_hash: hex_sha256_for_test(token.as_bytes()),
        owner_user_id: owner.to_string(),
        provider_key_id: "pk-s3".to_string(),
        allowed_models: Vec::new(),
        allowed_providers: Vec::new(),
        expires_at: exp,
        max_concurrency: mc,
        enabled: en,
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

/// SHA-256 -> lowercase hex (matches `pipeline::virtual_key_auth::hex_sha256`).
/// Reimplemented here because that helper is private to the `pipeline` crate.
fn hex_sha256_for_test(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    let d = Sha256::new().chain_update(bytes).finalize();
    let mut out = String::with_capacity(64);
    for b in d {
        for n in [b >> 4, b & 0x0f] {
            out.push(if n < 10 { (b'0' + n) as char } else { (b'a' + n - 10) as char });
        }
    }
    out
}
