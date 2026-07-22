//! gRPC `KeyVaultService` server: exposes Encrypt/Decrypt to the Go control
//! plane over mTLS + internal token (spec §12A).
//!
//! S2 ships Encrypt/Decrypt backed by [`AesGcmKeyVault`]; S3 adds owner-vs-
//! `created_by` enforcement as a second defense line (spec §12A L3). The audit
//! log records every decrypt (`key_id`, `requester`, `intent`) - plaintext
//! never appears in logs (constitution XX).
//!
//! ## S3 Dual-Defense (constitution XX "1Password model")
//!
//! Two independent checks gate `view_plaintext` decrypts:
//!
//! 1. **Go L1** (control plane): `created_by == requester` check at the
//!    `Reveal` handler. Rejects non-creators with 404 before the gRPC call.
//! 2. **Rust L2** (this module): `requester == owner_of(key_id)` check inside
//!    `decrypt`, using the [`OwnerLookup`] trait backed by the live
//!    [`RuntimeSnapshot`](pingogate_snapshot::RuntimeSnapshot). Rust holds the
//!    owner mapping **independently** (AWS KMS pattern: do not trust the
//!    caller to pass its own authorization); Go cannot bypass this check by
//!    crafting the request. Mismatch yields `permission_denied`.
//!
//! `hot_path_inject` decrypts skip the owner check: the hot path has already
//! authenticated the caller via a virtual key whose `owner_user_id` equals the
//! provider key's owner (T25 snapshot invariants), so re-checking here would
//! be redundant. Every decrypt - regardless of intent - emits an audit log
//! line with `key_id` / `requester` / `intent` / decision (constitution XX).

use std::sync::Arc;

use tonic::{Request, Response, Status};

use crate::crypto::AesGcmKeyVault;
use crate::proto::{
    key_vault_service_server::KeyVaultService, DecryptRequest, DecryptResponse, EncryptRequest,
    EncryptResponse,
};

/// Intent tag for a Decrypt call (spec §12A).
///
/// - `view_plaintext`: a human is asking to see the raw key once (Reveal
///   endpoint). Subject to the owner check (Rust L2 dual-defense).
/// - `hot_path_inject`: the data plane is decrypting once per request to
///   inject into an upstream call. Already authenticated via a virtual key;
///   owner check skipped.
pub const INTENT_VIEW_PLAINTEXT: &str = "view_plaintext";
pub const INTENT_HOT_PATH_INJECT: &str = "hot_path_inject";

/// Owner-of-key lookup used by [`KeyVaultGrpcService`] for the S3 dual-defense
/// check (constitution XX: Rust holds the owner mapping **independently** of
/// the caller; AWS KMS pattern).
///
/// The production impl (in `pingogate-core::grpc`) wraps an
/// `Arc<SnapshotHolder>` and reads `key_owners` from the live
/// [`RuntimeSnapshot`](pingogate_snapshot::RuntimeSnapshot) on each Decrypt.
/// This trait lives in `pingogate-keyvault` (not `pingogate-snapshot`) so the
/// keyvault crate stays decoupled from the snapshot crate (constitution VIII:
/// crate boundaries via traits; storage -> keyvault is the only internal dep
/// direction).
///
/// Returning `Option<String>` (owned) keeps the trait object-safe and avoids
/// borrowing from the snapshot's `HashMap` across an `Arc<dyn>` boundary.
pub trait OwnerLookup: Send + Sync {
    /// Return the `owner_user_id` of `key_id`, or `None` if the key is not in
    /// the live snapshot. An empty `owner_user_id` (bootstrap providers pushed
    /// before S3) is returned as-is; the dual-defense check then rejects
    /// `view_plaintext` unless the requester is also empty (which never
    /// happens for an authenticated user).
    fn owner_of(&self, key_id: &str) -> Option<String>;
}

/// gRPC `KeyVaultService` backed by [`AesGcmKeyVault`].
///
/// `kv` is shared via `Arc` so the same keyvault serves gRPC (control plane)
/// and the hot-path `KeyVault` trait wrapper (see `pingogate-storage`).
/// `owners` provides the independent owner map for the S3 dual-defense check
/// (constitution XX); it reads from the live `RuntimeSnapshot` so the check
/// uses the latest pushed owner mapping, not a stale copy.
pub struct KeyVaultGrpcService {
    pub kv: Arc<AesGcmKeyVault>,
    pub owners: Arc<dyn OwnerLookup>,
}

impl KeyVaultGrpcService {
    /// Construct a new service with the given keyvault and owner lookup. The
    /// owner lookup is typically a `SnapshotOwnerLookup` wrapping the same
    /// `SnapshotHolder` the `SnapshotService` swaps into (platform mode).
    pub fn new(kv: Arc<AesGcmKeyVault>, owners: Arc<dyn OwnerLookup>) -> Self {
        Self { kv, owners }
    }
}

#[tonic::async_trait]
impl KeyVaultService for KeyVaultGrpcService {
    async fn encrypt(
        &self,
        req: Request<EncryptRequest>,
    ) -> Result<Response<EncryptResponse>, Status> {
        let ct = self
            .kv
            .encrypt(&req.into_inner().plaintext)
            .map_err(|e| Status::internal(e.to_string()))?;
        Ok(Response::new(EncryptResponse {
            ciphertext: ct,
            error: String::new(),
        }))
    }

    async fn decrypt(
        &self,
        req: Request<DecryptRequest>,
    ) -> Result<Response<DecryptResponse>, Status> {
        let req = req.into_inner();

        // S3 dual-defense (constitution XX): for `view_plaintext`, verify
        // requester == owner using the independent owner map. `hot_path_inject`
        // skips this check (the pipeline already authenticated the caller via
        // a virtual key whose owner matches the provider key's owner, T25).
        if req.intent == INTENT_VIEW_PLAINTEXT {
            match self.owners.owner_of(&req.key_id) {
                Some(owner) if owner == req.requester_user_id => {
                    tracing::info!(
                        key_id = %req.key_id,
                        requester = %req.requester_user_id,
                        intent = %req.intent,
                        "keyvault decrypt: owner check passed"
                    );
                }
                Some(owner) => {
                    // Mismatch: log the decision (no plaintext, constitution XX)
                    // and reject with permission_denied. The audit log records
                    // who tried to decrypt whose key.
                    tracing::warn!(
                        key_id = %req.key_id,
                        requester = %req.requester_user_id,
                        owner = %owner,
                        intent = %req.intent,
                        "keyvault decrypt DENIED: requester is not owner"
                    );
                    return Err(Status::permission_denied(
                        "not key owner (Rust L2 dual-defense)",
                    ));
                }
                None => {
                    // Key id not in the live snapshot. This is either a stale
                    // key id (deleted but still in Go's DB) or a bug. Reject:
                    // we cannot verify ownership, so we do not decrypt.
                    tracing::warn!(
                        key_id = %req.key_id,
                        requester = %req.requester_user_id,
                        intent = %req.intent,
                        "keyvault decrypt DENIED: key_id not in live snapshot"
                    );
                    return Err(Status::permission_denied(
                        "key_id not found in live snapshot (Rust L2 dual-defense)",
                    ));
                }
            }
        }

        let pt = self
            .kv
            .decrypt(&req.ciphertext)
            .map_err(|e| Status::internal(e.to_string()))?;
        // Audit log: every decrypt (both intents) records key_id / requester /
        // intent (constitution XX). Plaintext is never logged.
        tracing::info!(
            key_id = %req.key_id,
            requester = %req.requester_user_id,
            intent = %req.intent,
            "keyvault decrypt: ok"
        );
        Ok(Response::new(DecryptResponse {
            plaintext: pt,
            error: String::new(),
        }))
    }
}

#[cfg(test)]
#[path = "grpc_service_tests.rs"]
mod tests;
