//! Tests for `grpc.rs` service impls + `grpc_convert.rs` conversion.

use super::*;
use super::grpc_convert::build_runtime_snapshot;
use pingogate::ProviderEntry;

fn make_provider_entry(name: &str, kind: &str, auth: &str, key_ref: &str) -> ProviderEntry {
    ProviderEntry {
        name: name.to_string(),
        kind: kind.to_string(),
        base_url: "https://upstream.example.com".to_string(),
        auth_method: auth.to_string(),
        encrypted_key_ref: key_ref.to_string(),
        anthropic_version: String::new(),
        capability_families: vec!["generation.stateless".to_string()],
        timeout_ms: None,
    }
}

#[test]
fn build_runtime_snapshot_resolves_encrypted_key_ref() {
    use pingogate::{EncryptedProviderKey, RouteEntry, Snapshot};
    let snap = Snapshot {
        version: 7,
        providers: vec![make_provider_entry("openai-main", "openai-compatible", "bearer", "k1")],
        routes: vec![RouteEntry {
            alias: "gpt-4o".to_string(),
            provider: "openai-main".to_string(),
            upstream_model: "gpt-4o".to_string(),
            upstream_path: None,
            auth_method: None,
        }],
        virtual_keys: vec![],
        encrypted_keys: vec![EncryptedProviderKey {
            id: "k1".to_string(),
            ciphertext: vec![0u8; 24],
            owner_user_id: "u1".to_string(),
            created_by: "u1".to_string(),
        }],
    };
    let rt = build_runtime_snapshot(&snap).unwrap();
    assert_eq!(rt.version, 7);
    assert_eq!(rt.providers.len(), 1);
    let p = &rt.providers[0];
    assert_eq!(p.name, "openai-main");
    assert!(p.key.expose().is_empty(), "platform mode key is empty placeholder");
    assert_eq!(p.encrypted_key.as_ref().unwrap().len(), 24);
    assert_eq!(rt.routes.len(), 1);
    assert!(rt.gateway_keys.is_empty());
    // S3 dual-defense: encrypted_keys + key_owners are populated from the
    // proto EncryptedProviderKey list (constitution XX).
    assert_eq!(rt.encrypted_keys.len(), 1);
    assert_eq!(rt.encrypted_keys["k1"].len(), 24);
    assert_eq!(rt.key_owners.get("k1").unwrap(), "u1");
    assert!(rt.virtual_keys.is_empty(), "no virtual_keys pushed");
}

#[test]
fn build_runtime_snapshot_builds_key_owners_and_virtual_keys() {
    use pingogate::{EncryptedProviderKey, Snapshot, VirtualKeyEntry as ProtoVKey};
    let snap = Snapshot {
        version: 3,
        providers: vec![make_provider_entry("p", "openai-compatible", "bearer", "k1")],
        routes: vec![],
        virtual_keys: vec![ProtoVKey {
            id: "vk1".to_string(),
            token_hash: "hash-vk1".to_string(),
            owner_user_id: "u1".to_string(),
            provider_key_id: "k1".to_string(),
            allowed_models: vec!["gpt-4o".to_string()],
            allowed_providers: vec!["p".to_string()],
            expires_at: 1_700_000_000,
            max_concurrency: 4,
            enabled: true,
        }],
        encrypted_keys: vec![
            EncryptedProviderKey {
                id: "k1".to_string(),
                ciphertext: vec![1u8; 24],
                owner_user_id: "u1".to_string(),
                created_by: "u1".to_string(),
            },
            EncryptedProviderKey {
                id: "k2".to_string(),
                ciphertext: vec![2u8; 24],
                owner_user_id: "u2".to_string(),
                created_by: "u2".to_string(),
            },
        ],
    };
    let rt = build_runtime_snapshot(&snap).unwrap();

    // key_owners derived from encrypted_keys.owner_user_id.
    assert_eq!(rt.key_owners.len(), 2);
    assert_eq!(rt.key_owners.get("k1").unwrap(), "u1");
    assert_eq!(rt.key_owners.get("k2").unwrap(), "u2");

    // encrypted_keys aggregated by id.
    assert_eq!(rt.encrypted_keys.len(), 2);
    assert_eq!(rt.encrypted_keys["k1"], vec![1u8; 24]);
    assert_eq!(rt.encrypted_keys["k2"], vec![2u8; 24]);

    // virtual_keys mirrored verbatim.
    assert_eq!(rt.virtual_keys.len(), 1);
    let vk = &rt.virtual_keys[0];
    assert_eq!(vk.id, "vk1");
    assert_eq!(vk.token_hash, "hash-vk1");
    assert_eq!(vk.owner_user_id, "u1");
    assert_eq!(vk.provider_key_id, "k1");
    assert_eq!(vk.allowed_models, vec!["gpt-4o".to_string()]);
    assert_eq!(vk.allowed_providers, vec!["p".to_string()]);
    assert_eq!(vk.expires_at, 1_700_000_000);
    assert_eq!(vk.max_concurrency, 4);
    assert!(vk.enabled);
}

#[test]
fn build_runtime_snapshot_rejects_unknown_encrypted_key_ref() {
    use pingogate::Snapshot;
    let snap = Snapshot {
        version: 1,
        providers: vec![make_provider_entry("p", "openai-compatible", "bearer", "missing")],
        routes: vec![],
        virtual_keys: vec![],
        encrypted_keys: vec![],
    };
    let err = build_runtime_snapshot(&snap).unwrap_err();
    assert_eq!(err.code(), tonic::Code::InvalidArgument);
    assert!(err.message().contains("unknown encrypted_key_ref"));
}

#[test]
fn build_runtime_snapshot_rejects_incompatible_auth() {
    use pingogate::{EncryptedProviderKey, Snapshot};
    let snap = Snapshot {
        version: 1,
        providers: vec![make_provider_entry("p", "openai-compatible", "query_key", "k1")],
        routes: vec![],
        virtual_keys: vec![],
        encrypted_keys: vec![EncryptedProviderKey {
            id: "k1".to_string(),
            ciphertext: vec![0u8; 24],
            owner_user_id: "u".to_string(),
            created_by: "u".to_string(),
        }],
    };
    let err = build_runtime_snapshot(&snap).unwrap_err();
    assert_eq!(err.code(), tonic::Code::InvalidArgument);
    assert!(err.message().contains("not compatible"));
}

#[test]
fn build_runtime_snapshot_rejects_anthropic_without_version() {
    use pingogate::{EncryptedProviderKey, Snapshot};
    let snap = Snapshot {
        version: 1,
        providers: vec![make_provider_entry("p", "anthropic", "api_key_header", "k1")],
        routes: vec![],
        virtual_keys: vec![],
        encrypted_keys: vec![EncryptedProviderKey {
            id: "k1".to_string(),
            ciphertext: vec![0u8; 24],
            owner_user_id: "u".to_string(),
            created_by: "u".to_string(),
        }],
    };
    let err = build_runtime_snapshot(&snap).unwrap_err();
    assert_eq!(err.code(), tonic::Code::InvalidArgument);
    assert!(err.message().contains("anthropic_version"));
}

#[test]
fn build_runtime_snapshot_rejects_empty_capability_families() {
    use pingogate::{EncryptedProviderKey, Snapshot};
    let mut entry = make_provider_entry("p", "openai-compatible", "bearer", "k1");
    entry.capability_families = vec![];
    let snap = Snapshot {
        version: 1,
        providers: vec![entry],
        routes: vec![],
        virtual_keys: vec![],
        encrypted_keys: vec![EncryptedProviderKey {
            id: "k1".to_string(),
            ciphertext: vec![0u8; 24],
            owner_user_id: "u".to_string(),
            created_by: "u".to_string(),
        }],
    };
    let err = build_runtime_snapshot(&snap).unwrap_err();
    assert_eq!(err.code(), tonic::Code::InvalidArgument);
    assert!(err.message().contains("capability_families must not be empty"));
}

#[test]
fn build_runtime_snapshot_rejects_route_to_unknown_provider() {
    use pingogate::{EncryptedProviderKey, RouteEntry, Snapshot};
    let snap = Snapshot {
        version: 1,
        providers: vec![make_provider_entry("p", "openai-compatible", "bearer", "k1")],
        routes: vec![RouteEntry {
            alias: "a".to_string(),
            provider: "nonexistent".to_string(),
            upstream_model: "m".to_string(),
            upstream_path: None,
            auth_method: None,
        }],
        virtual_keys: vec![],
        encrypted_keys: vec![EncryptedProviderKey {
            id: "k1".to_string(),
            ciphertext: vec![0u8; 24],
            owner_user_id: "u".to_string(),
            created_by: "u".to_string(),
        }],
    };
    let err = build_runtime_snapshot(&snap).unwrap_err();
    assert_eq!(err.code(), tonic::Code::InvalidArgument);
    assert!(err.message().contains("unknown provider"));
}

#[tokio::test]
async fn health_service_reports_not_ready_until_first_snapshot() {
    let holder = Arc::new(SnapshotHolder::empty());
    let svc = HealthServiceImpl::new(holder);
    // version 0 -> not ready
    let resp = svc
        .check(Request::new(HealthRequest { current_version: 0 }))
        .await
        .unwrap()
        .into_inner();
    assert!(!resp.ready);
    assert_eq!(resp.version, 0);
}

#[tokio::test]
async fn health_service_reports_ready_after_snapshot_applied() {
    use pingogate::{EncryptedProviderKey, Snapshot};
    let holder = Arc::new(SnapshotHolder::empty());
    let source = Arc::new(GrpcSnapshotSource::new(holder.clone()));
    let snap = Snapshot {
        version: 5,
        providers: vec![make_provider_entry("p", "openai-compatible", "bearer", "k1")],
        routes: vec![],
        virtual_keys: vec![],
        encrypted_keys: vec![EncryptedProviderKey {
            id: "k1".to_string(),
            ciphertext: vec![0u8; 24],
            owner_user_id: "u".to_string(),
            created_by: "u".to_string(),
        }],
    };
    let rt = build_runtime_snapshot(&snap).unwrap();
    source.apply(rt);

    let svc = HealthServiceImpl::new(holder);
    let resp = svc
        .check(Request::new(HealthRequest { current_version: 0 }))
        .await
        .unwrap()
        .into_inner();
    assert!(resp.ready);
    assert_eq!(resp.version, 5);
}
