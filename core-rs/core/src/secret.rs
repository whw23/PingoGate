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
}
