//! A secret string that never reveals its contents through `Debug`/`Display`.
//!
//! Resolved provider keys and gateway-key secrets are wrapped here so that an
//! accidental `{:?}` or `{}` in a log line cannot leak plaintext (constitution
//! XX). Access the plaintext only through [`SecretString::expose`].

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
