//! `pingogate-keyvault` - AES-GCM provider-key encryption/decryption (constitution
//! XX security core).
//!
//! The Rust kernel's [`AesGcmKeyVault`] encrypts provider keys for at-rest
//! storage in the Go control plane DB and decrypts them once per hot-path
//! request. The master key (`PINGO_MKEK`, 32 bytes) is loaded at startup; the
//! AES-GCM nonce is **random per encryption** (12 bytes, prefixed to ciphertext:
//! `ct = nonce || ciphertext`) - a fixed nonce with a static key would be a
//! nonce-reuse catastrophe. [`KeyVaultGrpcService`] exposes Encrypt/Decrypt over
//! gRPC for the Go control plane (mTLS + internal token, spec §12A).
//!
//! This crate depends on no other internal crate (it owns its own proto
//! codegen); `pingogate-storage` wraps [`AesGcmKeyVault`] to satisfy the
//! `KeyVault` trait used on the hot path (one-way dependency: storage -> keyvault).
//!
//! S3 adds the [`OwnerLookup`] trait + dual-defense check inside
//! [`KeyVaultGrpcService::decrypt`]: `view_plaintext` decrypts require
//! `requester == owner_of(key_id)` (constitution XX "1Password model"). The
//! production `OwnerLookup` impl lives in `pingogate-core::grpc`
//! (`SnapshotOwnerLookup`) and reads `key_owners` from the live
//! `RuntimeSnapshot` via `Arc<SnapshotHolder>` - Rust holds the owner mapping
//! independently of Go (AWS KMS pattern).

pub mod crypto;
pub mod grpc_service;

/// Generated protobuf types + service traits (mirrors `pingogate-core` codegen).
pub mod proto {
    include!(concat!(env!("OUT_DIR"), "/pingogate.rs"));
}

pub use crypto::{AesGcmKeyVault, KeyError};
pub use grpc_service::{
    INTENT_HOT_PATH_INJECT, INTENT_VIEW_PLAINTEXT, KeyVaultGrpcService, OwnerLookup,
};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encrypt_decrypt_roundtrip() {
        let kv = AesGcmKeyVault::from_master_key(&[42u8; 32]);
        let pt = b"sk-openai-test-key";
        let ct = kv.encrypt(pt).unwrap();
        assert_ne!(&ct[..], pt);
        let decrypted = kv.decrypt(&ct).unwrap();
        assert_eq!(&decrypted[..], pt);
    }

    #[test]
    fn decrypt_wrong_key_fails() {
        let kv1 = AesGcmKeyVault::from_master_key(&[1u8; 32]);
        let kv2 = AesGcmKeyVault::from_master_key(&[2u8; 32]);
        let ct = kv1.encrypt(b"secret").unwrap();
        assert!(kv2.decrypt(&ct).is_err());
    }

    #[test]
    fn nonce_is_random_no_reuse() {
        // Same plaintext encrypted twice yields different ciphertexts (nonce is
        // random, preventing nonce reuse under a static key).
        let kv = AesGcmKeyVault::from_master_key(&[42u8; 32]);
        let ct1 = kv.encrypt(b"same-plaintext").unwrap();
        let ct2 = kv.encrypt(b"same-plaintext").unwrap();
        assert_ne!(ct1, ct2, "nonce must be random - ciphertexts must differ");
        // Both decrypt back to the original plaintext.
        assert_eq!(kv.decrypt(&ct1).unwrap(), b"same-plaintext");
        assert_eq!(kv.decrypt(&ct2).unwrap(), b"same-plaintext");
    }
}
