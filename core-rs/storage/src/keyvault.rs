//! KeyVault: provider key encryption/decryption. S1 stub; S2 implements AES-GCM.
//!
//! In platform mode the Go control plane stores provider keys as ciphertext and
//! pushes them via gRPC snapshot; the Rust kernel decrypts once per hot-path
//! request inside [`KeyVault::decrypt`], returning a [`SecretString`] whose
//! plaintext lives only for the request and is zeroed on drop (constitution XX:
//! "明文仅在 `KeyVault::decrypt()` 返回的 `SecretString` 中存活"). S1 ships
//! [`StubKeyVault`] which returns [`KeyError::NotImplemented`]; S2 replaces it
//! with an AES-GCM impl keyed by a per-instance master key.

use pingogate_core_types::SecretString;

#[derive(Debug, thiserror::Error)]
pub enum KeyError {
    #[error("keyvault not implemented in S1")]
    NotImplemented,
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
}
