//! Gateway-key authentication abstraction (FR-009/FR-013; constitution XX).
//!
//! [`KeyAuth`] decouples credential verification from the pipeline: the pipeline
//! extracts the presented key from headers ([`crate::auth_filter`]), then hands
//! it to a `KeyAuth` impl together with the pinned snapshot. S1 ships
//! [`StaticKeyAuth`] (standalone mode: match against the snapshot's enabled
//! gateway keys). S2 will add a platform-mode impl that consults virtual keys.
//!
//! The trait returns a [`Principal`] on success so the pipeline can record the
//! authenticated identity without knowing how the match was performed. The
//! snapshot only contains enabled keys (disabled ones are filtered at build
//! time in `pingogate-snapshot`), so no `enabled` check is needed here.

use pingogate_core_types::Principal;
use pingogate_snapshot::RuntimeSnapshot;
use subtle::ConstantTimeEq;

/// Verifies a presented gateway-key credential against the active snapshot.
pub trait KeyAuth: Send + Sync {
    /// Returns the authenticated [`Principal`] when `credential` matches a
    /// configured gateway key, otherwise `None`.
    fn authenticate(&self, credential: &str, snapshot: &RuntimeSnapshot) -> Option<Principal>;
}

/// Standalone-mode authenticator: matches the presented credential against the
/// snapshot's enabled gateway keys using constant-time comparison
/// ([`subtle::ConstantTimeEq`]) to avoid timing side-channels on the secret
/// comparison (constitution XX).
pub struct StaticKeyAuth;

impl KeyAuth for StaticKeyAuth {
    fn authenticate(&self, credential: &str, snapshot: &RuntimeSnapshot) -> Option<Principal> {
        snapshot
            .gateway_keys
            .iter()
            .find(|k| bool::from(credential.as_bytes().ct_eq(k.secret.expose().as_bytes())))
            .map(|k| Principal::gateway_key(&k.name))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pingogate_core_types::SecretString;
    use pingogate_snapshot::{GatewayKey, RuntimeSnapshot, UpstreamConfig};

    fn snapshot_with_keys(keys: &[(&str, &str)]) -> RuntimeSnapshot {
        let gateway_keys = keys
            .iter()
            .map(|(name, secret)| GatewayKey {
                name: (*name).to_string(),
                secret: SecretString::new((*secret).to_string()),
            })
            .collect();
        RuntimeSnapshot {
            version: 1,
            providers: Vec::new(),
            routes: Vec::new(),
            gateway_keys,
            upstream: UpstreamConfig { timeout_ms: 30000 },
        }
    }

    #[test]
    fn matches_known_key_returns_principal() {
        let snap = snapshot_with_keys(&[("team-alpha", "pg-secret-1")]);
        let auth = StaticKeyAuth;
        let principal = auth.authenticate("pg-secret-1", &snap).unwrap();
        assert_eq!(principal.id, "team-alpha");
    }

    #[test]
    fn unknown_credential_returns_none() {
        let snap = snapshot_with_keys(&[("team-alpha", "pg-secret-1")]);
        let auth = StaticKeyAuth;
        assert!(auth.authenticate("wrong", &snap).is_none());
    }

    #[test]
    fn empty_snapshot_returns_none() {
        let snap = snapshot_with_keys(&[]);
        let auth = StaticKeyAuth;
        assert!(auth.authenticate("anything", &snap).is_none());
    }

    #[test]
    fn matches_first_of_multiple_keys() {
        let snap = snapshot_with_keys(&[("a", "k1"), ("b", "k2")]);
        let auth = StaticKeyAuth;
        let p = auth.authenticate("k2", &snap).unwrap();
        assert_eq!(p.id, "b");
    }
}
