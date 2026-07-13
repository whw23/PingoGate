//! gRPC `KeyVaultService` server: exposes Encrypt/Decrypt to the Go control
//! plane over mTLS + internal token (spec §12A).
//!
//! S2 ships Encrypt/Decrypt backed by [`AesGcmKeyVault`]; S3 adds owner-vs-
//! `created_by` enforcement as a second defense line (spec §12A L3). The audit
//! log records every decrypt (`key_id`, `requester`, `intent`) - plaintext
//! never appears in logs (constitution XX).

use std::sync::Arc;

use tonic::{Request, Response, Status};

use crate::crypto::AesGcmKeyVault;
use crate::proto::{
    key_vault_service_server::KeyVaultService, DecryptRequest, DecryptResponse, EncryptRequest,
    EncryptResponse,
};

/// gRPC `KeyVaultService` backed by [`AesGcmKeyVault`].
///
/// `kv` is shared via `Arc` so the same keyvault serves gRPC (control plane)
/// and the hot-path `KeyVault` trait wrapper (see `pingogate-storage`). S3 will
/// add an owner-map field for the second defense line; S2 trusts Go's mTLS.
pub struct KeyVaultGrpcService {
    pub kv: Arc<AesGcmKeyVault>,
}

impl KeyVaultGrpcService {
    pub fn new(kv: Arc<AesGcmKeyVault>) -> Self {
        Self { kv }
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
        // S3: verify requester_user_id == owner (S2 placeholder, trusts Go mTLS).
        let pt = self
            .kv
            .decrypt(&req.ciphertext)
            .map_err(|e| Status::internal(e.to_string()))?;
        // Audit log (S2 already, spec §12A L3). Plaintext is never logged.
        tracing::info!(
            key_id = %req.key_id,
            requester = %req.requester_user_id,
            intent = %req.intent,
            "keyvault decrypt"
        );
        Ok(Response::new(DecryptResponse {
            plaintext: pt,
            error: String::new(),
        }))
    }
}
