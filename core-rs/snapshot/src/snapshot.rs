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

use std::collections::HashMap;
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
    /// Standalone mode: env-resolved plaintext (set at build time). Platform
    /// mode: empty placeholder - the hot path decrypts `encrypted_key` per
    /// request via the KeyVault into a transient `SecretString` (constitution XX:
    /// "明文仅在 `KeyVault::decrypt()` 返回的 `SecretString` 中存活").
    pub key: SecretString,
    /// Platform mode only: AES-GCM ciphertext of the provider key (`nonce ||
    /// ciphertext`, 12-byte random nonce prefix). `None` in standalone mode.
    /// The Go control plane pushes this; the Rust kernel decrypts once per
    /// hot-path request. Ciphertext is arbitrary bytes (not UTF-8), so it lives
    /// here as `Vec<u8>` rather than in `key: SecretString`.
    pub encrypted_key: Option<Vec<u8>>,
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

/// A virtual key entry (platform mode, S3). Mirrors the proto `VirtualKeyEntry`
/// (T24): an opaque token presented by the caller, authenticated by hash
/// comparison, scoped to a provider key + model/provider allowlists. The Rust
/// kernel reads this from the [`RuntimeSnapshot`] on the hot path; the Go
/// control plane pushes it via gRPC (constitution X).
#[derive(Debug, Clone)]
pub struct VirtualKeyEntry {
    pub id: String,
    /// Hash (bcrypt or SHA-256) of the presented token. Compared in constant
    /// time on the hot path (constitution XX).
    pub token_hash: String,
    /// Owner user id (BYOK visibility: only this user can view the plaintext
    /// provider key the virtual key references; constitution XX).
    pub owner_user_id: String,
    /// References `EncryptedProviderKey.id` in the snapshot's `encrypted_keys`.
    pub provider_key_id: String,
    pub allowed_models: Vec<String>,
    pub allowed_providers: Vec<String>,
    /// Unix timestamp; 0 = never expires.
    pub expires_at: i64,
    /// Concurrency quota; 0 = unlimited.
    pub max_concurrency: i32,
    pub enabled: bool,
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
    /// Platform mode (S3): virtual keys pushed by the Go control plane. Empty
    /// in standalone mode (which uses `gateway_keys`).
    pub virtual_keys: Vec<VirtualKeyEntry>,
    /// key_id -> owner_user_id (S3 dual-defense: Rust hot path can reject a
    /// virtual key whose `provider_key_id` is not owned by the virtual key's
    /// `owner_user_id` without a round-trip to Go). Built from
    /// `encrypted_keys` at snapshot build time (constitution XX).
    pub key_owners: HashMap<String, String>,
    /// key_id -> AES-GCM ciphertext (`nonce || ciphertext`). The hot path
    /// decrypts per request via the KeyVault (constitution XX: "明文仅在
    /// `KeyVault::decrypt()` 返回的 `SecretString` 中存活"). Empty in
    /// standalone mode (which resolves plaintext env refs into
    /// `ResolvedProvider::key`).
    pub encrypted_keys: HashMap<String, Vec<u8>>,
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
            // Standalone mode has no virtual keys, no ciphertext, no owner
            // mapping (constitution X: platform-mode fields stay empty).
            virtual_keys: Vec::new(),
            key_owners: HashMap::new(),
            encrypted_keys: HashMap::new(),
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
            encrypted_key: None,
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

    /// Create a holder with an empty version-0 snapshot (platform mode). The
    /// holder is "not ready" until the Go control plane pushes the first real
    /// snapshot via gRPC and `store` swaps it in (spec §12B: readyz not-ready
    /// until first snapshot). `version == 0` is the sentinel for "no snapshot
    /// applied yet"; [`HealthService`](../grpc/struct.HealthServiceImpl.html)
    /// reports `ready: false` while it sees version 0.
    pub fn empty() -> Self {
        Self {
            inner: ArcSwap::from(Arc::new(RuntimeSnapshot {
                version: 0,
                providers: Vec::new(),
                routes: Vec::new(),
                gateway_keys: Vec::new(),
                virtual_keys: Vec::new(),
                key_owners: HashMap::new(),
                encrypted_keys: HashMap::new(),
                upstream: UpstreamConfig { timeout_ms: 60_000 },
            })),
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

