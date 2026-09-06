//! Immutable [`RuntimeSnapshot`] and the [`SnapshotHolder`] that the data path
//! reads through `ArcSwap` for lock-free hot reload (constitution XII).
//!
//! `build` is the only way to construct a snapshot: it validates semantics,
//! resolves every secret reference eagerly (so a missing key fails the build,
//! not a live request - FR-022), and freezes the result behind `Arc`.
//!
//! **Config hierarchy resolution** (model > provider > global): for each model
//! under a provider, the resolved [`Route`] carries the *effective* value of
//! each parameter (model override ?? provider default). The hot path reads
//! these effective values directly from the snapshot without re-resolving.

use std::collections::HashMap;
use std::sync::Arc;

use arc_swap::{ArcSwap, Guard};
use pingogate_core_types::{
    AuthMethod, CapabilityFamily, ProviderKind, SecretResolver, SecretString,
};

use crate::model::{AuthMethodKind, GatewayConfig, ModelCfg, Upstream};
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
    /// request via the KeyVault (constitution XX).
    pub key: SecretString,
    /// Platform mode only: AES-GCM ciphertext of the provider key (`nonce ||
    /// ciphertext`, 12-byte random nonce prefix). `None` in standalone mode.
    pub encrypted_key: Option<Vec<u8>>,
    pub anthropic_version: Option<String>,
    pub capability_families: Vec<CapabilityFamily>,
    /// Provider-level timeout default (ms). `None` = use global.
    pub timeout_ms: Option<u64>,
    /// Provider-level upstream path template. `None` = forward inbound path.
    pub upstream_path: Option<String>,
    /// Provider-level HTTP-traffic proxy. `None` = inherit global.
    pub http_proxy: Option<String>,
    /// Provider-level HTTPS-traffic proxy. `None` = inherit global.
    pub https_proxy: Option<String>,
    /// Shell command whose stdout is the access token (`TokenCommand`).
    /// Standalone: the Rust kernel executes it; platform: Go injects instead.
    pub auth_command: Option<String>,
    /// Token cache TTL seconds (`TokenCommand`); `None` = default 1800.
    pub token_ttl_secs: Option<u64>,
}

/// A resolved model route with effective parameter values (model override ??
/// provider default). The hot path reads these directly without re-resolving.
#[derive(Debug, Clone)]
pub struct Route {
    pub alias: String,
    pub provider: String,
    pub upstream_model: String,
    /// Effective upstream protocol kind (model ?? provider). For future
    /// protocol conversion (constitution VI: "转换引擎").
    pub kind: ProviderKind,
    /// Effective auth method (model ?? provider).
    pub auth_method: AuthMethod,
    /// Effective anthropic_version (model ?? provider).
    pub anthropic_version: Option<String>,
    /// Effective timeout (model ?? provider). `None` = use global.
    pub timeout_ms: Option<u64>,
    /// Effective upstream path template (model ?? provider). `None` = forward
    /// inbound path.
    pub upstream_path: Option<String>,
    /// Effective HTTP-traffic proxy (model ?? provider ?? global). `None` = no
    /// proxy. Used for non-TLS upstreams.
    pub http_proxy: Option<String>,
    /// Effective HTTPS-traffic proxy (model ?? provider ?? global). Used for
    /// TLS upstreams.
    pub https_proxy: Option<String>,
}

/// An enabled gateway key with its secret resolved (in-memory only; never logged).
#[derive(Debug, Clone)]
pub struct GatewayKey {
    pub name: String,
    pub secret: SecretString,
}

/// A virtual key entry (platform mode, S3). Mirrors the proto `VirtualKeyEntry`.
#[derive(Debug, Clone)]
pub struct VirtualKeyEntry {
    pub id: String,
    pub token_hash: String,
    pub owner_user_id: String,
    pub provider_key_id: String,
    pub allowed_models: Vec<String>,
    pub allowed_providers: Vec<String>,
    pub expires_at: i64,
    pub max_concurrency: i32,
    pub enabled: bool,
}

/// Resolved upstream connection defaults.
#[derive(Debug, Clone)]
pub struct UpstreamConfig {
    pub timeout_ms: u64,
    /// Global HTTP-traffic proxy (`host:port` or `http://host:port`).
    pub http_proxy: Option<String>,
    /// Global HTTPS-traffic proxy.
    pub https_proxy: Option<String>,
}

/// An immutable view of runtime configuration. Cloned cheaply via `Arc`.
#[derive(Debug)]
pub struct RuntimeSnapshot {
    pub version: u64,
    pub providers: Vec<ResolvedProvider>,
    pub routes: Vec<Route>,
    pub gateway_keys: Vec<GatewayKey>,
    pub virtual_keys: Vec<VirtualKeyEntry>,
    pub key_owners: HashMap<String, String>,
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
        let routes = resolve_routes(&providers, config);
        let gateway_keys = resolve_gateway_keys(config, resolver)?;
        Ok(Arc::new(Self {
            version,
            providers,
            routes,
            gateway_keys,
            virtual_keys: Vec::new(),
            key_owners: HashMap::new(),
            encrypted_keys: HashMap::new(),
            upstream: UpstreamConfig {
                timeout_ms: config.upstream.timeout_ms,
                http_proxy: config.upstream.http_proxy.clone(),
                https_proxy: config.upstream.https_proxy.clone(),
            },
        }))
    }

    /// Look up a route by alias (client-visible model name).
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
        AuthMethodKind::TokenCommand => AuthMethod::TokenCommand,
    }
}

/// Resolve every provider's secret reference and normalize auth method.
fn resolve_providers(
    config: &GatewayConfig,
    resolver: &dyn SecretResolver,
) -> Result<Vec<ResolvedProvider>, ConfigError> {
    let mut providers = Vec::with_capacity(config.providers.len());
    for (i, p) in config.providers.iter().enumerate() {
        // `TokenCommand` providers have no static key - the token comes from
        // the auth command; resolve only when a key_ref is present.
        let key = match p.auth.key_ref.as_deref() {
            Some(reference) => resolver
                .resolve(reference)
                .map_err(|e| ConfigError::Secret {
                    path: format!("providers[{i}].auth.key_ref"),
                    source: e,
                })?,
            None => SecretString::new(String::new()),
        };
        providers.push(ResolvedProvider {
            name: p.name.clone(),
            kind: p.kind,
            base_url: p.base_url.clone(),
            auth_method: map_auth(p.auth.method),
            key,
            encrypted_key: None,
            anthropic_version: p.anthropic_version.clone(),
            capability_families: p.capability_families.clone(),
            timeout_ms: p.timeout_ms,
            upstream_path: p.upstream_path.clone(),
            http_proxy: p.http_proxy.clone(),
            https_proxy: p.https_proxy.clone(),
            auth_command: p.auth.command.clone(),
            token_ttl_secs: p.auth.token_ttl_secs,
        });
    }
    Ok(providers)
}

/// Resolve all model routes from the config, applying the model > provider
/// override chain for each parameter.
fn resolve_routes(providers: &[ResolvedProvider], config: &GatewayConfig) -> Vec<Route> {
    let mut routes = Vec::new();
    for p in &config.providers {
        // Find the resolved provider to get defaults.
        let resolved = providers.iter().find(|rp| rp.name == p.name);
        let resolved = match resolved {
            Some(r) => r,
            None => continue, // validate_semantics should have caught this
        };
        for m in &p.models {
            let route = resolve_one_route(m, resolved, &config.upstream);
            routes.push(route);
        }
    }
    routes
}

/// Resolve a single model route, applying the model > provider > global
/// override chain for each parameter.
fn resolve_one_route(m: &ModelCfg, provider: &ResolvedProvider, global: &Upstream) -> Route {
    Route {
        alias: m.alias.clone(),
        provider: provider.name.clone(),
        upstream_model: m.upstream_model.clone(),
        kind: m.kind.unwrap_or(provider.kind),
        auth_method: m
            .auth_method
            .map(map_auth)
            .unwrap_or(provider.auth_method),
        anthropic_version: m
            .anthropic_version
            .clone()
            .or(provider.anthropic_version.clone()),
        timeout_ms: m.timeout_ms.or(provider.timeout_ms),
        upstream_path: m.upstream_path.clone().or(provider.upstream_path.clone()),
        http_proxy: m
            .http_proxy
            .clone()
            .or(provider.http_proxy.clone())
            .or(global.http_proxy.clone()),
        https_proxy: m
            .https_proxy
            .clone()
            .or(provider.https_proxy.clone())
            .or(global.https_proxy.clone()),
    }
}

/// Resolve secrets for enabled gateway keys; disabled ones are skipped.
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
                upstream: UpstreamConfig {
                    timeout_ms: 60_000,
                    http_proxy: None,
                    https_proxy: None,
                },
            })),
        }
    }

    pub fn load(&self) -> Guard<Arc<RuntimeSnapshot>> {
        self.inner.load()
    }

    pub fn load_full(&self) -> Arc<RuntimeSnapshot> {
        self.inner.load_full()
    }

    pub fn store(&self, next: Arc<RuntimeSnapshot>) {
        self.inner.store(next);
    }
}

#[cfg(test)]
#[path = "snapshot_tests.rs"]
mod tests;
