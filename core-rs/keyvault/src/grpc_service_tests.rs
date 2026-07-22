//! Tests for [`KeyVaultGrpcService`] dual-defense behavior (S3, constitution
//! XX). Extracted from `grpc_service.rs` to keep that file under
//! constitution V's 300-line limit. Same module, compiles as part of the
//! parent test binary.
//!
//! These tests use a static `MapOwnerLookup` instead of the production
//! `SnapshotOwnerLookup` (which reads from the live `RuntimeSnapshot`). The
//! trait is the stable contract; the production impl is exercised in
//! `pingogate-core/tests/security_no_leak.rs` and `s2_contract.rs`.

use std::collections::HashMap;
use std::sync::Arc;

use tonic::{Request, Status};

use crate::proto::key_vault_service_server::KeyVaultService;
use super::{
    AesGcmKeyVault, DecryptRequest, EncryptRequest, INTENT_HOT_PATH_INJECT,
    INTENT_VIEW_PLAINTEXT, KeyVaultGrpcService, OwnerLookup,
};

/// In-memory `OwnerLookup` for tests: a static `key_id -> owner_user_id`
/// map. The production impl reads from the live `RuntimeSnapshot`; tests
/// substitute this to avoid depending on the snapshot crate.
#[derive(Default)]
struct MapOwnerLookup {
    owners: HashMap<String, String>,
}

impl MapOwnerLookup {
    fn new(owners: HashMap<String, String>) -> Self {
        Self { owners }
    }
}

impl OwnerLookup for MapOwnerLookup {
    fn owner_of(&self, key_id: &str) -> Option<String> {
        self.owners.get(key_id).cloned()
    }
}

fn service_with_owners(owners: HashMap<String, String>) -> KeyVaultGrpcService {
    let kv = Arc::new(AesGcmKeyVault::from_master_key(&[42u8; 32]));
    let lookup: Arc<dyn OwnerLookup> = Arc::new(MapOwnerLookup::new(owners));
    KeyVaultGrpcService::new(kv, lookup)
}

fn encrypt_for_test(svc: &KeyVaultGrpcService, plaintext: &[u8]) -> Vec<u8> {
    svc.kv.encrypt(plaintext).expect("encrypt must succeed")
}

/// owner = user-A; requester = user-B; intent = view_plaintext. Rust must
/// REJECT (permission_denied), not just audit.
#[tokio::test]
async fn decrypt_denied_for_non_owner_view_plaintext() {
    let mut owners = HashMap::new();
    owners.insert("k1".to_string(), "user-A".to_string());
    let svc = service_with_owners(owners);

    let ct = encrypt_for_test(&svc, b"sk-test-key-12345678");
    let req = Request::new(DecryptRequest {
        ciphertext: ct,
        requester_user_id: "user-B".to_string(),
        key_id: "k1".to_string(),
        intent: INTENT_VIEW_PLAINTEXT.to_string(),
    });
    let err = svc.decrypt(req).await.expect_err("must reject non-owner");
    assert_eq!(err.code(), tonic::Code::PermissionDenied);
    assert!(
        err.message().contains("not key owner"),
        "error message must mention owner check: {}",
        err.message()
    );
}

/// requester == owner -> decrypt succeeds, plaintext returned.
#[tokio::test]
async fn decrypt_allowed_for_owner_view_plaintext() {
    let mut owners = HashMap::new();
    owners.insert("k1".to_string(), "user-A".to_string());
    let svc = service_with_owners(owners);

    let plaintext = b"sk-owner-can-see-12345";
    let ct = encrypt_for_test(&svc, plaintext);
    let req = Request::new(DecryptRequest {
        ciphertext: ct,
        requester_user_id: "user-A".to_string(),
        key_id: "k1".to_string(),
        intent: INTENT_VIEW_PLAINTEXT.to_string(),
    });
    let resp = svc.decrypt(req).await.expect("owner decrypt must succeed");
    assert_eq!(resp.into_inner().plaintext, plaintext.to_vec());
}

/// Key id not in owner map -> reject (cannot verify ownership).
#[tokio::test]
async fn decrypt_denied_when_key_id_not_in_snapshot() {
    let svc = service_with_owners(HashMap::new());
    let ct = encrypt_for_test(&svc, b"sk-unknown-key-1234567");
    let req = Request::new(DecryptRequest {
        ciphertext: ct,
        requester_user_id: "user-A".to_string(),
        key_id: "missing-key".to_string(),
        intent: INTENT_VIEW_PLAINTEXT.to_string(),
    });
    let err = svc.decrypt(req).await.expect_err("must reject unknown key");
    assert_eq!(err.code(), tonic::Code::PermissionDenied);
}

/// intent = hot_path_inject -> no owner check even if requester != owner.
/// The hot path authenticates via virtual key scope (T25); re-checking here
/// would be redundant.
#[tokio::test]
async fn hot_path_inject_skips_owner_check() {
    let mut owners = HashMap::new();
    owners.insert("k1".to_string(), "user-A".to_string());
    let svc = service_with_owners(owners);

    let plaintext = b"sk-hot-path-inject-12";
    let ct = encrypt_for_test(&svc, plaintext);
    // Requester is user-B (not the owner) but intent is hot_path_inject.
    let req = Request::new(DecryptRequest {
        ciphertext: ct,
        requester_user_id: "user-B".to_string(),
        key_id: "k1".to_string(),
        intent: INTENT_HOT_PATH_INJECT.to_string(),
    });
    let resp = svc
        .decrypt(req)
        .await
        .expect("hot_path_inject must skip owner check");
    assert_eq!(resp.into_inner().plaintext, plaintext.to_vec());
}

/// hot_path_inject does not consult the owner map at all, so an unknown
/// key_id is fine (the hot path has its own auth chain via virtual keys).
#[tokio::test]
async fn hot_path_inject_succeeds_even_for_unknown_key_id() {
    let svc = service_with_owners(HashMap::new());
    let plaintext = b"sk-hot-path-unknown-id";
    let ct = encrypt_for_test(&svc, plaintext);
    let req = Request::new(DecryptRequest {
        ciphertext: ct,
        requester_user_id: "user-A".to_string(),
        key_id: "never-pushed".to_string(),
        intent: INTENT_HOT_PATH_INJECT.to_string(),
    });
    let resp = svc
        .decrypt(req)
        .await
        .expect("hot_path_inject ignores owner map entirely");
    assert_eq!(resp.into_inner().plaintext, plaintext.to_vec());
}

/// Sanity: the gRPC Encrypt path still produces ciphertext the Decrypt path
/// can reverse (hot_path_inject bypasses owner check).
#[tokio::test]
async fn encrypt_round_trips_through_grpc_service() {
    let svc = service_with_owners(HashMap::new());
    let plaintext = b"sk-grpc-roundtrip-1234";

    let enc_resp = svc
        .encrypt(Request::new(EncryptRequest {
            plaintext: plaintext.to_vec(),
        }))
        .await
        .expect("encrypt must succeed");
    let ct = enc_resp.into_inner().ciphertext;
    assert!(!ct.is_empty());
    assert_ne!(&ct[..], plaintext);

    let dec_resp = svc
        .decrypt(Request::new(DecryptRequest {
            ciphertext: ct,
            requester_user_id: "u1".to_string(),
            key_id: "any".to_string(),
            intent: INTENT_HOT_PATH_INJECT.to_string(),
        }))
        .await
        .expect("decrypt must succeed");
    assert_eq!(dec_resp.into_inner().plaintext, plaintext.to_vec());
}

/// Ensure the `Status` type is reachable even when the test binary's
/// imports are pruned (defensive; the compiler usually keeps it via the
/// `expect_err` call sites).
#[allow(dead_code)]
fn _status_type_anchor(_s: Status) {}
