//! Bootstrap admin credential provider (constitution XX; FR-024/FR-025).
//!
//! Maps a presented `Authorization: Bearer <token>` to a [`Principal`] only when
//! it matches the configured bootstrap admin token. This is *authentication*
//! only — it establishes *who* the caller is. Whether that principal may perform
//! a given action is always decided afterwards by [`AuthContext::authorize`], so
//! token equality is never the sole gate on a privileged action. The shape is a
//! deliberate seam: a future OAuth/OIDC provider slots in behind the same
//! `authenticate -> Principal` boundary without touching the handlers.

use pingo_core::Principal;
use pingo_core::SecretString;

/// Verifies the bootstrap admin credential and issues an admin [`Principal`].
pub struct BootstrapAuth {
    admin_token: SecretString,
}

impl BootstrapAuth {
    pub fn new(admin_token: SecretString) -> Self {
        Self { admin_token }
    }

    /// Authenticate a raw `Authorization` header value into an admin principal.
    /// Returns `None` when the header is absent, malformed, or does not match —
    /// the caller turns that into a `401` without revealing which it was (SC-006).
    pub fn authenticate(&self, authorization: Option<&str>) -> Option<Principal> {
        let token = authorization?.strip_prefix("Bearer ")?.trim();
        // NOTE: constant-time comparison is a hardening item for the security
        // review (T065); a direct compare is acceptable for the bootstrap path.
        if !token.is_empty() && token == self.admin_token.expose() {
            Some(Principal::admin("bootstrap"))
        } else {
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pingo_core::PrincipalKind;

    fn auth() -> BootstrapAuth {
        BootstrapAuth::new(SecretString::new("s3cret"))
    }

    #[test]
    fn valid_bearer_token_authenticates_as_admin() {
        let p = auth().authenticate(Some("Bearer s3cret")).unwrap();
        assert_eq!(p.kind, PrincipalKind::Admin);
    }

    #[test]
    fn wrong_token_is_rejected() {
        assert!(auth().authenticate(Some("Bearer nope")).is_none());
    }

    #[test]
    fn missing_or_malformed_header_is_rejected() {
        assert!(auth().authenticate(None).is_none());
        assert!(auth().authenticate(Some("s3cret")).is_none()); // no scheme
        assert!(auth().authenticate(Some("Bearer ")).is_none()); // empty token
    }
}
