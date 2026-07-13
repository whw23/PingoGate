//! Immutable [`RuntimeSnapshot`] and the [`SnapshotHolder`] that the data path
//! reads through `ArcSwap` for lock-free hot reload (constitution XII).
//!
//! `build` is the only way to construct a snapshot: it validates semantics,
//! resolves every secret reference eagerly (so a missing key fails the build,
//! not a live request - FR-022), and freezes the result behind `Arc`.
//!
//! S1 ships standalone-mode plaintext keys (env-resolved). Platform-mode
//! ciphertext provider keys arrive in S2 via the KeyVault; the `key` field on
//! [`ResolvedProvider`] is the single home for the resolved [`SecretString`] in
//! either mode (constitution XX).

use std::sync::Arc;

use arc_swap::{ArcSwap, Guard};
use pingogate_core_types::{
    AuthMethod, CapabilityFamily, ProviderKind, SecretResolver, SecretString,
};

use crate::model::{AuthMethodKind, GatewayConfig};
use crate::validate::{validate_semantics, ConfigError};

/// A provider with its secret resolved and auth method normalized.
#[derive(Debug)]
pub struct ResolvedProvider {
    pub name: String,
    pub kind: ProviderKind,
    pub base_url: String,
    pub auth_method: AuthMethod,
    /// Standalone mode: env-resolved plaintext. Platform mode (S2): ciphertext
    /// decrypted once per request by KeyVault into this same `SecretString`.
    pub key: SecretString,
    pub anthropic_version: Option<String>,
    pub capability_families: Vec<CapabilityFamily>,
}

/// A model-alias route resolved to a provider + upstream model.
#[derive(Debug, Clone)]
pub struct Route {
    pub alias: String,
    pub provider: String,
    pub upstream_model: String,
}

/// An enabled gateway key with its secret resolved (in-memory only; never logged).
#[derive(Debug, Clone)]
pub struct GatewayKey {
    pub name: String,
    /// The resolved gateway-key plaintext. Keyed implicitly by this secret for
    /// authentication (`authenticate_gateway_key` compares against it).
    pub secret: SecretString,
}

/// Resolved upstream connection defaults.
#[derive(Debug, Clone)]
pub struct UpstreamConfig {
    pub timeout_ms: u64,
}

/// An immutable view of runtime configuration. Cloned cheaply via `Arc`.
///
/// Fields are public per the S1 contract; the data path reads them directly on
/// the hot path. Lookup helpers (`route`, `provider`, `authenticate_gateway_key`)
/// are provided for convenience but iterate the vectors (small N).
#[derive(Debug)]
pub struct RuntimeSnapshot {
    pub version: u64,
    pub providers: Vec<ResolvedProvider>,
    pub routes: Vec<Route>,
    pub gateway_keys: Vec<GatewayKey>,
    pub upstream: UpstreamConfig,
}

impl RuntimeSnapshot {
    /// Validate, resolve secrets, and freeze a snapshot at `version`.
    pub fn build(
        config: &GatewayConfig,
        resolver: &dyn SecretResolver,
        version: u64,
    ) -> Result<Arc<Self>, ConfigError> {
        validate_semantics(config)?;
        let providers = resolve_providers(config, resolver)?;
        let routes = config
            .routes
            .iter()
            .map(|r| Route {
                alias: r.alias.clone(),
                provider: r.provider.clone(),
                upstream_model: r.upstream_model.clone(),
            })
            .collect();
        let gateway_keys = resolve_gateway_keys(config, resolver)?;
        Ok(Arc::new(Self {
            version,
            providers,
            routes,
            gateway_keys,
            upstream: UpstreamConfig {
                timeout_ms: config.upstream.timeout_ms,
            },
        }))
    }

    /// Look up a route by alias.
    pub fn route(&self, alias: &str) -> Option<&Route> {
        self.routes.iter().find(|r| r.alias == alias)
    }

    /// Look up a provider by name.
    pub fn provider(&self, name: &str) -> Option<&ResolvedProvider> {
        self.providers.iter().find(|p| p.name == name)
    }

    /// Match a presented gateway-key secret to its named entry (FR-013).
    pub fn authenticate_gateway_key(&self, presented: &str) -> Option<&GatewayKey> {
        self.gateway_keys
            .iter()
            .find(|gk| gk.secret.expose() == presented)
    }
}

fn map_auth(kind: AuthMethodKind) -> AuthMethod {
    match kind {
        AuthMethodKind::Bearer => AuthMethod::Bearer,
        AuthMethodKind::ApiKeyHeader => AuthMethod::ApiKeyHeader,
        AuthMethodKind::QueryKey => AuthMethod::QueryKey,
    }
}

/// Resolve every provider's secret reference and normalize auth method.
fn resolve_providers(
    config: &GatewayConfig,
    resolver: &dyn SecretResolver,
) -> Result<Vec<ResolvedProvider>, ConfigError> {
    let mut providers = Vec::with_capacity(config.providers.len());
    for (i, p) in config.providers.iter().enumerate() {
        let key = resolver
            .resolve(&p.auth.key_ref)
            .map_err(|e| ConfigError::Secret {
                path: format!("providers[{i}].auth.key_ref"),
                source: e,
            })?;
        providers.push(ResolvedProvider {
            name: p.name.clone(),
            kind: p.kind,
            base_url: p.base_url.clone(),
            auth_method: map_auth(p.auth.method),
            key,
            anthropic_version: p.anthropic_version.clone(),
            capability_families: p.capability_families.clone(),
        });
    }
    Ok(providers)
}

/// Resolve secrets for enabled gateway keys; disabled ones are skipped (and
/// their secret reference is never resolved, so it need not exist).
fn resolve_gateway_keys(
    config: &GatewayConfig,
    resolver: &dyn SecretResolver,
) -> Result<Vec<GatewayKey>, ConfigError> {
    let mut gateway_keys = Vec::new();
    for gk in &config.gateway_keys {
        if !gk.enabled {
            continue;
        }
        let secret = resolver.resolve(&gk.secret_ref).map_err(|e| ConfigError::Secret {
            path: format!("gateway_keys[{}].secret_ref", gk.name),
            source: e,
        })?;
        gateway_keys.push(GatewayKey {
            name: gk.name.clone(),
            secret,
        });
    }
    Ok(gateway_keys)
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

    /// Load the active snapshot as a cheap `Guard` lease. The guard derefs to
    /// `RuntimeSnapshot`, so callers read fields directly. In-flight requests
    /// that need to outlive a swap should take an `Arc` via `load_full`.
    pub fn load(&self) -> Guard<Arc<RuntimeSnapshot>> {
        self.inner.load()
    }

    /// Load the active snapshot as a full `Arc` (keeps it alive across swaps).
    pub fn load_full(&self) -> Arc<RuntimeSnapshot> {
        self.inner.load_full()
    }

    /// Atomically install a new snapshot.
    pub fn store(&self, next: Arc<RuntimeSnapshot>) {
        self.inner.store(next);
    }
}

#[cfg(test)]
#[path = "snapshot_tests.rs"]
mod tests;

