//! Immutable [`RuntimeSnapshot`] and the [`SnapshotHolder`] that the data path
//! reads through `ArcSwap` for lock-free hot reload (constitution XII).
//!
//! `build` is the only way to construct a snapshot: it validates semantics,
//! resolves every secret reference eagerly (so a missing key fails the build,
//! not a live request — FR-022), and freezes the result behind `Arc`.

use std::collections::HashMap;
use std::sync::Arc;

use arc_swap::ArcSwap;
use pingo_core::{AuthMethod, CapabilityFamily, ProviderKind, SecretString};
use pingo_storage::SecretResolver;

use crate::model::{AuthMethodKind, GatewayConfig};
use crate::validate::{validate_semantics, ConfigError};

/// A provider with its secret resolved and auth method normalized.
#[derive(Debug)]
pub struct ResolvedProvider {
    pub name: String,
    pub kind: ProviderKind,
    pub base_url: String,
    pub anthropic_version: Option<String>,
    pub auth_method: AuthMethod,
    pub key: SecretString,
    pub capability_families: Vec<CapabilityFamily>,
}

/// A model-alias route resolved to a provider + upstream model.
#[derive(Debug, Clone)]
pub struct ResolvedRoute {
    pub alias: String,
    pub provider: String,
    pub upstream_model: String,
}

/// Metadata for an enabled gateway key (the secret itself is the map key).
#[derive(Debug, Clone)]
pub struct GatewayKeyEntry {
    pub name: String,
}

/// An immutable view of runtime configuration. Cloned cheaply via `Arc`.
#[derive(Debug)]
pub struct RuntimeSnapshot {
    version: u64,
    providers: HashMap<String, ResolvedProvider>,
    routes: HashMap<String, ResolvedRoute>,
    /// Keyed by the gateway-key plaintext. In-memory only; never logged (XX).
    gateway_keys: HashMap<String, GatewayKeyEntry>,
}

impl RuntimeSnapshot {
    /// Validate, resolve secrets, and freeze a snapshot at `version`.
    pub fn build(
        config: &GatewayConfig,
        resolver: &dyn SecretResolver,
        version: u64,
    ) -> Result<Arc<Self>, ConfigError> {
        validate_semantics(config)?;

        let mut providers = HashMap::with_capacity(config.providers.len());
        for (i, p) in config.providers.iter().enumerate() {
            let key = resolver
                .resolve(&p.auth.key_ref)
                .map_err(|e| ConfigError::Secret {
                    path: format!("providers[{i}].auth.key_ref"),
                    source: e,
                })?;
            providers.insert(
                p.name.clone(),
                ResolvedProvider {
                    name: p.name.clone(),
                    kind: p.kind,
                    base_url: p.base_url.clone(),
                    anthropic_version: p.anthropic_version.clone(),
                    auth_method: map_auth(p.auth.method),
                    key,
                    capability_families: p.capability_families.clone(),
                },
            );
        }

        let mut routes = HashMap::with_capacity(config.routes.len());
        for r in &config.routes {
            routes.insert(
                r.alias.clone(),
                ResolvedRoute {
                    alias: r.alias.clone(),
                    provider: r.provider.clone(),
                    upstream_model: r.upstream_model.clone(),
                },
            );
        }

        let mut gateway_keys = HashMap::new();
        for (i, gk) in config.gateway_keys.iter().enumerate() {
            if !gk.enabled {
                continue;
            }
            let secret = resolver
                .resolve(&gk.secret_ref)
                .map_err(|e| ConfigError::Secret {
                    path: format!("gateway_keys[{i}].secret_ref"),
                    source: e,
                })?;
            gateway_keys.insert(
                secret.expose().to_string(),
                GatewayKeyEntry {
                    name: gk.name.clone(),
                },
            );
        }

        Ok(Arc::new(Self {
            version,
            providers,
            routes,
            gateway_keys,
        }))
    }

    pub fn version(&self) -> u64 {
        self.version
    }

    pub fn route(&self, alias: &str) -> Option<&ResolvedRoute> {
        self.routes.get(alias)
    }

    pub fn provider(&self, name: &str) -> Option<&ResolvedProvider> {
        self.providers.get(name)
    }

    /// Match a presented gateway-key secret to its named entry (FR-013).
    pub fn authenticate_gateway_key(&self, presented: &str) -> Option<&GatewayKeyEntry> {
        self.gateway_keys.get(presented)
    }

    pub fn route_count(&self) -> usize {
        self.routes.len()
    }

    pub fn provider_count(&self) -> usize {
        self.providers.len()
    }
}

fn map_auth(kind: AuthMethodKind) -> AuthMethod {
    match kind {
        AuthMethodKind::Bearer => AuthMethod::Bearer,
        AuthMethodKind::ApiKeyHeader => AuthMethod::ApiKeyHeader,
        AuthMethodKind::QueryKey => AuthMethod::QueryKey,
    }
}

/// Holds the active snapshot for lock-free reads and atomic hot swap (XII).
pub struct SnapshotHolder {
    inner: ArcSwap<RuntimeSnapshot>,
}

impl SnapshotHolder {
    pub fn new(initial: Arc<RuntimeSnapshot>) -> Self {
        Self {
            inner: ArcSwap::from(initial),
        }
    }

    /// Load the active snapshot. In-flight requests keep their loaded `Arc`
    /// even after a swap, so a reload never disturbs them (FR-021).
    pub fn load(&self) -> Arc<RuntimeSnapshot> {
        self.inner.load_full()
    }

    /// Atomically install a new snapshot.
    pub fn store(&self, next: Arc<RuntimeSnapshot>) {
        self.inner.store(next);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pingo_storage::EnvSecretResolver;

    fn build_snapshot() -> Arc<RuntimeSnapshot> {
        std::env::set_var("PINGO_TEST_PROVIDER_KEY", "sk-test");
        std::env::set_var("PINGO_TEST_GW_KEY", "pg-test");
        let yaml = r#"
listeners:
  public: { address: "0.0.0.0:8080" }
  admin: { address: "127.0.0.1:9090" }
gateway_keys:
  - { name: "team-alpha", secret_ref: "env:PINGO_TEST_GW_KEY" }
  - { name: "disabled-team", secret_ref: "env:NONEXISTENT", enabled: false }
providers:
  - name: "openai-main"
    kind: "openai-compatible"
    base_url: "https://api.openai.com"
    auth: { method: "bearer", key_ref: "env:PINGO_TEST_PROVIDER_KEY" }
    capability_families: ["generation.stateless"]
routes:
  - { alias: "gpt-4o", provider: "openai-main", upstream_model: "gpt-4o" }
"#;
        let cfg = GatewayConfig::from_yaml(yaml).unwrap();
        RuntimeSnapshot::build(&cfg, &EnvSecretResolver, 1).unwrap()
    }

    #[test]
    fn builds_snapshot_with_routes_and_providers() {
        let snap = build_snapshot();
        assert_eq!(snap.version(), 1);
        assert_eq!(snap.route("gpt-4o").unwrap().upstream_model, "gpt-4o");
        assert_eq!(
            snap.provider("openai-main").unwrap().auth_method,
            AuthMethod::Bearer
        );
    }

    #[test]
    fn disabled_gateway_key_is_skipped_and_secret_not_required() {
        let snap = build_snapshot();
        assert_eq!(
            snap.authenticate_gateway_key("pg-test").unwrap().name,
            "team-alpha"
        );
        assert!(snap.authenticate_gateway_key("wrong").is_none());
    }

    #[test]
    fn missing_provider_secret_fails_build_not_runtime() {
        let yaml = r#"
listeners:
  public: { address: "0.0.0.0:8080" }
  admin: { address: "127.0.0.1:9090" }
providers:
  - name: "p"
    kind: "openai-compatible"
    base_url: "https://x"
    auth: { method: "bearer", key_ref: "env:PINGO_TEST_MISSING_PROVIDER_KEY" }
    capability_families: ["generation.stateless"]
"#;
        let cfg = GatewayConfig::from_yaml(yaml).unwrap();
        let err = RuntimeSnapshot::build(&cfg, &EnvSecretResolver, 1).unwrap_err();
        assert!(matches!(err, ConfigError::Secret { .. }));
    }

    #[test]
    fn holder_swaps_snapshot_atomically() {
        let first = build_snapshot();
        let holder = SnapshotHolder::new(first.clone());
        assert_eq!(holder.load().version(), 1);
        // A second snapshot at version 2 replaces the first.
        std::env::set_var("PINGO_TEST_PROVIDER_KEY", "sk-test");
        std::env::set_var("PINGO_TEST_GW_KEY", "pg-test");
        let second = build_snapshot();
        let second = Arc::new(rebuild_at_version(&second, 2));
        holder.store(second);
        assert_eq!(holder.load().version(), 2);
    }

    fn rebuild_at_version(_prev: &RuntimeSnapshot, version: u64) -> RuntimeSnapshot {
        let yaml = r#"
listeners:
  public: { address: "0.0.0.0:8080" }
  admin: { address: "127.0.0.1:9090" }
gateway_keys:
  - { name: "team-alpha", secret_ref: "env:PINGO_TEST_GW_KEY" }
providers:
  - name: "openai-main"
    kind: "openai-compatible"
    base_url: "https://api.openai.com"
    auth: { method: "bearer", key_ref: "env:PINGO_TEST_PROVIDER_KEY" }
    capability_families: ["generation.stateless"]
routes:
  - { alias: "gpt-4o", provider: "openai-main", upstream_model: "gpt-4o" }
"#;
        let cfg = GatewayConfig::from_yaml(yaml).unwrap();
        Arc::try_unwrap(RuntimeSnapshot::build(&cfg, &EnvSecretResolver, version).unwrap())
            .expect("unique")
    }
}
