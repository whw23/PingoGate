//! Bootstrap admin credential provider (constitution XX; R1 single-token).
//!
//! Maps a presented `Authorization: Bearer <token>` to an admin [`Principal`]
//! only when it matches the configured admin token. This is *authentication*
//! only - whether the principal may perform a given action is always decided
//! by [`AuthContext::authorize`], so token equality is never the sole gate.
//!
//! R1: standalone-mode admin auth is a single token from `PINGO_ADMIN_TOKEN`.
//! When the env var is unset at startup, `admin_token` is `None` and every
//! authed endpoint rejects (401); `/healthz` and `/metrics` are unaffected.

use pingogate_core_types::{Principal, SecretString};
use subtle::ConstantTimeEq;

/// Verifies the bootstrap admin credential and issues an admin [`Principal`].
pub struct BootstrapAuth {
    admin_token: Option<SecretString>,
}

impl BootstrapAuth {
    /// Create with the resolved admin token, or `None` when unset (R1: all
    /// authed endpoints will reject).
    pub fn new(admin_token: Option<SecretString>) -> Self {
        Self { admin_token }
    }

    /// Authenticate a raw `Authorization` header value into an admin principal.
    /// Returns `None` when the header is absent, malformed, the token is unset,
    /// or the token does not match. The caller turns `None` into a `401`
    /// without revealing which condition triggered it (SC-006).
    pub fn authenticate(&self, authorization: Option<&str>) -> Option<Principal> {
        let expected = self.admin_token.as_ref()?;
        let presented = authorization?.strip_prefix("Bearer ")?.trim();
        if presented.is_empty() {
            return None;
        }
        // Constant-time comparison avoids timing side-channels on the secret
        // (constitution XX), matching the gateway-key comparison in StaticKeyAuth.
        if bool::from(presented.as_bytes().ct_eq(expected.expose().as_bytes())) {
            Some(Principal::admin("bootstrap"))
        } else {
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pingogate_core_types::PrincipalKind;

    fn auth(token: Option<&str>) -> BootstrapAuth {
        BootstrapAuth::new(token.map(SecretString::new))
    }

    #[test]
    fn valid_bearer_token_authenticates_as_admin() {
        let p = auth(Some("s3cret"))
            .authenticate(Some("Bearer s3cret"))
            .unwrap();
        assert_eq!(p.kind, PrincipalKind::Admin);
    }

    #[test]
    fn wrong_token_is_rejected() {
        assert!(auth(Some("s3cret"))
            .authenticate(Some("Bearer nope"))
            .is_none());
    }

    #[test]
    fn missing_or_malformed_header_is_rejected() {
        let a = auth(Some("s3cret"));
        assert!(a.authenticate(None).is_none());
        assert!(a.authenticate(Some("s3cret")).is_none()); // no scheme
        assert!(a.authenticate(Some("Bearer ")).is_none()); // empty token
    }

    #[test]
    fn unset_token_rejects_all_auth_attempts() {
        let a = auth(None);
        assert!(a.authenticate(Some("Bearer s3cret")).is_none());
        assert!(a.authenticate(None).is_none());
    }
}
