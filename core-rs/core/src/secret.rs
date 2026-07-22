//! Secret material + secret-reference resolution (constitution XX).
//!
//! [`SecretString`] wraps resolved provider keys and gateway-key secrets so an
//! accidental `{:?}`/`{}` in a log line cannot leak plaintext. [`SecretResolver`]
//! abstracts how an opaque reference (e.g. `env:VAR`) becomes material at
//! snapshot-build time.
//!
//! Both live in the cycle-breaker `core` crate (constitution VIII): the
//! `pingogate-snapshot` crate needs `SecretResolver` for `RuntimeSnapshot::build`
//! and the `pingogate-storage` crate needs it for `EnvSecretResolver`/`FileSnapshotSource`;
//! keeping the trait here avoids a snapshot <-> storage dependency cycle.

use std::fmt;

#[derive(Clone, PartialEq, Eq)]
pub struct SecretString(String);

impl SecretString {
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    /// Controlled access to the plaintext. Call sites must not log the result.
    pub fn expose(&self) -> &str {
        &self.0
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

impl fmt::Debug for SecretString {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("SecretString([REDACTED])")
    }
}

impl fmt::Display for SecretString {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("[REDACTED]")
    }
}

impl From<String> for SecretString {
    fn from(value: String) -> Self {
        Self(value)
    }
}

impl Drop for SecretString {
    /// Zero the plaintext bytes on drop (constitution XX: "明文请求结束清零").
    ///
    /// `String`'s heap bytes are normally returned to the allocator without
    /// being wiped, leaving plaintext recoverable via heap inspection (core
    /// dump / `/proc/<pid>/mem`). This overwrites the bytes with zeros before
    /// the `String` is dropped.
    fn drop(&mut self) {
        // Zero the plaintext bytes before the String deallocates (constitution
        // XX: "明文请求结束清零"). This is an explicit unsafe review per
        // constitution III: we write only into the String's own valid byte
        // buffer (no aliasing borrow outstanding; drop takes &mut self). The
        // bytes become non-UTF-8 (zeros) but the String is about to drop and
        // will not be re-parsed. Safe-Rust alternative as_mut_vec also requires
        // unsafe for the same reason.
        let bytes = unsafe { self.0.as_bytes_mut() };
        for b in bytes {
            *b = 0;
        }
    }
}

/// Errors raised while resolving an opaque secret reference to material.
#[derive(Debug, thiserror::Error)]
pub enum SecretError {
    #[error("secret reference is empty")]
    Empty,
    #[error("unsupported secret reference scheme: {0}")]
    UnsupportedScheme(String),
    #[error("environment variable not set: {0}")]
    MissingEnv(String),
    #[error("resolved secret is empty: {0}")]
    EmptyValue(String),
}

/// Resolves an opaque secret reference (e.g. `env:OPENAI_API_KEY`) to material.
///
/// Implementations live in `pingogate-storage` (`EnvSecretResolver`); the trait
/// stays here so both `pingogate-snapshot` and `pingogate-storage` can name it
/// without a cyclic dependency.
pub trait SecretResolver: Send + Sync {
    fn resolve(&self, reference: &str) -> Result<SecretString, SecretError>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn debug_and_display_never_reveal_plaintext() {
        let s = SecretString::new("sk-super-secret");
        assert_eq!(format!("{s:?}"), "SecretString([REDACTED])");
        assert_eq!(format!("{s}"), "[REDACTED]");
        assert!(!format!("{s:?}{s}").contains("super-secret"));
    }

    #[test]
    fn expose_returns_plaintext_for_controlled_use() {
        let s = SecretString::new("value");
        assert_eq!(s.expose(), "value");
        assert!(!s.is_empty());
    }

    #[test]
    fn drop_does_not_panic_and_clears_without_leak() {
        // constitution XX: SecretString drops cleanly (zeroing in Drop impl).
        // We cannot safely read freed heap to assert zeroing, but verify Drop
        // runs without panic across many instances (regression for Drop impl).
        for _ in 0..1000 {
            let s = SecretString::new("sk-test-secret-material-for-drop");
            assert_eq!(s.expose(), "sk-test-secret-material-for-drop");
            // s drops here; Drop zeroes the backing buffer.
        }
    }
}
