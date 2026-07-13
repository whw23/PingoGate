//! KeyVault: provider key encryption/decryption. S1 stub; S2 implements AES-GCM.
//!
//! In platform mode the Go control plane stores provider keys as ciphertext and
//! pushes them via gRPC snapshot; the Rust kernel decrypts once per hot-path
//! request inside [`KeyVault::decrypt`], returning a [`SecretString`] whose
//! plaintext lives only for the request and is zeroed on drop (constitution XX:
//! "明文仅在 `KeyVault::decrypt()` 返回的 `SecretString` 中存活"). S1 ships
//! [`StubKeyVault`] which returns [`KeyError::NotImplemented`]; S2 ships
//! [`AesGcmKeyVault`] (a thin wrapper over `pingogate_keyvault::AesGcmKeyVault`)
//! keyed by a per-instance master key (`PINGO_MKEK`, 32 bytes).

use pingogate_core_types::SecretString;
use pingogate_keyvault::AesGcmKeyVault as InnerKv;

#[derive(Debug, thiserror::Error)]
pub enum KeyError {
    #[error("keyvault not implemented in S1")]
    NotImplemented,
    #[error("encrypt failed")]
    EncryptFailed,
    #[error("decrypt failed")]
    DecryptFailed,
}

/// Provider-key encryption/decryption boundary (constitution XX, security core).
pub trait KeyVault: Send + Sync {
    /// Encrypt `plaintext` provider-key material to ciphertext for storage.
    fn encrypt(&self, plaintext: &[u8]) -> Result<Vec<u8>, KeyError>;
    /// Decrypt `ciphertext` to a [`SecretString`]. Plaintext must not escape
    /// the returned [`SecretString`] or be logged.
    fn decrypt(&self, ciphertext: &[u8]) -> Result<SecretString, KeyError>;
}

/// S1 stub; S2 replaces with `AesGcmKeyVault`.
pub struct StubKeyVault;

impl KeyVault for StubKeyVault {
    fn encrypt(&self, _plaintext: &[u8]) -> Result<Vec<u8>, KeyError> {
        Err(KeyError::NotImplemented)
    }
    fn decrypt(&self, _ciphertext: &[u8]) -> Result<SecretString, KeyError> {
        Err(KeyError::NotImplemented)
    }
}

/// S2 AES-GCM KeyVault: wraps `pingogate_keyvault::AesGcmKeyVault` to satisfy
/// the hot-path [`KeyVault`] trait. The inner keyvault is the single source of
/// crypto; this adapter only maps errors and lifts `Vec<u8>` to [`SecretString`]
/// (provider keys are UTF-8; lossy conversion guards against corruption).
pub struct AesGcmKeyVault(pub InnerKv);

impl KeyVault for AesGcmKeyVault {
    fn encrypt(&self, plaintext: &[u8]) -> Result<Vec<u8>, KeyError> {
        self.0
            .encrypt(plaintext)
            .map_err(|_| KeyError::EncryptFailed)
    }
    fn decrypt(&self, ciphertext: &[u8]) -> Result<SecretString, KeyError> {
        let pt = self
            .0
            .decrypt(ciphertext)
            .map_err(|_| KeyError::DecryptFailed)?;
        Ok(SecretString::new(String::from_utf8_lossy(&pt).into_owned()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stub_encrypt_returns_not_implemented() {
        let vault = StubKeyVault;
        let err = vault.encrypt(b"sk-test").unwrap_err();
        assert!(matches!(err, KeyError::NotImplemented));
    }

    #[test]
    fn stub_decrypt_returns_not_implemented() {
        let vault = StubKeyVault;
        let err = vault.decrypt(b"cipher").unwrap_err();
        assert!(matches!(err, KeyError::NotImplemented));
    }

    #[test]
    fn aes_gcm_wrapper_roundtrip() {
        let inner = InnerKv::from_master_key(&[7u8; 32]);
        let vault = AesGcmKeyVault(inner);
        let ct = vault.encrypt(b"sk-provider-key").unwrap();
        let pt = vault.decrypt(&ct).unwrap();
        assert_eq!(pt.expose(), "sk-provider-key");
    }
}
