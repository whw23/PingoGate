//! AES-GCM provider-key encryption/decryption (constitution XX security core).
//!
//! The master key (`PINGO_MKEK`, 32 bytes) is loaded once at bootstrap and held
//! in [`AesGcmKeyVault`]. Each [`AesGcmKeyVault::encrypt`] call draws a fresh
//! random 12-byte nonce via `rand::thread_rng()` and prefixes it to the
//! ciphertext (`ct = nonce || ciphertext`); [`AesGcmKeyVault::decrypt`] reads
//! the first 12 bytes as the nonce. AES-GCM with a **fixed** nonce under a
//! static key is a catastrophic nonce-reuse failure, so randomness is mandatory.

use aes_gcm::aead::{Aead, KeyInit};
use aes_gcm::{Aes256Gcm, Key, Nonce};
use rand::RngCore;

#[derive(Debug, thiserror::Error)]
pub enum KeyError {
    #[error("encrypt failed")]
    EncryptFailed,
    #[error("decrypt failed")]
    DecryptFailed,
}

/// AES-256-GCM KeyVault keyed by a 32-byte master key (`PINGO_MKEK`).
///
/// Nonce is random per encryption (12 bytes, prefixed to ciphertext). Safe for
/// concurrent use: `Aes256Gcm` is `Sync` and stateless after construction.
pub struct AesGcmKeyVault {
    cipher: Aes256Gcm,
}

impl AesGcmKeyVault {
    /// Construct from a 32-byte master key. The caller must guarantee `mkek`
    /// entropy (loaded from `PINGO_MKEK` at startup; missing/malformed = fatal).
    pub fn from_master_key(mkek: &[u8; 32]) -> Self {
        let key = Key::<Aes256Gcm>::from_slice(mkek);
        Self {
            cipher: Aes256Gcm::new(key),
        }
    }

    /// Encrypt `plaintext` to `nonce || ciphertext` (12-byte random nonce prefix).
    pub fn encrypt(&self, plaintext: &[u8]) -> Result<Vec<u8>, KeyError> {
        // Random nonce (12 bytes), prefixed to ciphertext: ct = nonce || ciphertext.
        // AES-GCM with a fixed nonce + static key = nonce-reuse catastrophe, so
        // the nonce MUST be random per encryption.
        let mut nonce_bytes = [0u8; 12];
        rand::thread_rng().fill_bytes(&mut nonce_bytes);
        let nonce = Nonce::from_slice(&nonce_bytes);
        let ct = self
            .cipher
            .encrypt(nonce, plaintext)
            .map_err(|_| KeyError::EncryptFailed)?;
        let mut out = Vec::with_capacity(12 + ct.len());
        out.extend_from_slice(&nonce_bytes);
        out.extend_from_slice(&ct);
        Ok(out)
    }

    /// Decrypt `nonce || ciphertext`: read the first 12 bytes as the nonce, the
    /// remainder as the AES-GCM ciphertext. Returns the plaintext bytes.
    pub fn decrypt(&self, ciphertext: &[u8]) -> Result<Vec<u8>, KeyError> {
        // Read the first 12 bytes as the nonce, the remainder as ciphertext.
        if ciphertext.len() < 12 {
            return Err(KeyError::DecryptFailed);
        }
        let (nonce_bytes, ct) = ciphertext.split_at(12);
        let nonce = Nonce::from_slice(nonce_bytes);
        self.cipher
            .decrypt(nonce, ct)
            .map_err(|_| KeyError::DecryptFailed)
    }
}
