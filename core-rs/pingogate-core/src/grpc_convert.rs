//! Proto `Snapshot` -> [`RuntimeSnapshot`] conversion (platform mode).
//!
//! Extracted from `grpc.rs` to keep file sizes under the constitution V limit.
//! This module is proto-aware (depends on the tonic-generated `pingogate`
//! types); the rest of `grpc.rs` only needs the proto types for service traits.

use std::collections::HashMap;
use std::str::FromStr;
use std::sync::Arc;

use pingogate_core_types::{AuthMethod, CapabilityFamily, ProviderKind, SecretString};
use pingogate_snapshot::{ResolvedProvider, Route, RuntimeSnapshot, UpstreamConfig, VirtualKeyEntry};
use tonic::Status;

use super::pingogate::Snapshot;

/// Convert a proto `Snapshot` into an immutable [`RuntimeSnapshot`] ready for
/// `ArcSwap`.
///
/// Platform-mode semantics (constitution XX):
/// - Each `ProviderEntry.encrypted_key_ref` is resolved against
///   `Snapshot.encrypted_keys` to obtain the AES-GCM ciphertext (`nonce ||
///   ciphertext`). The ciphertext is stored verbatim in
///   [`ResolvedProvider::encrypted_key`] AND aggregated into the snapshot-level
///   `encrypted_keys` map (key_id -> ciphertext) so the hot path can decrypt
///   by id without re-scanning providers.
/// - `key_owners` is derived from `encrypted_keys`: for each
///   `EncryptedProviderKey`, `key_owners[id] = owner_user_id` (S3 dual-defense
///   per constitution XX: the Rust kernel can reject a virtual key whose
///   `provider_key_id` is not owned by the virtual key's `owner_user_id`
///   without a round-trip to Go).
/// - `virtual_keys` (S3) is mirrored verbatim from the proto.
/// - `gateway_keys` is empty in platform mode (S2/S3 uses virtual keys).
/// - `upstream.timeout_ms` is not carried by the proto yet; the S1 default
///   (60_000 ms) is used until the proto is extended.
///
/// Validation mirrors `validate_semantics` for the standalone path: unknown
/// provider kinds / auth methods / capability families are rejected, and
/// `encrypted_key_ref` must resolve to an existing `EncryptedProviderKey.id`.
#[allow(clippy::result_large_err)] // tonic::Status is ~176 bytes; boxing adds indirection for no gain
pub(super) fn build_runtime_snapshot(snap: &Snapshot) -> Result<Arc<RuntimeSnapshot>, Status> {
    // Index encrypted keys by id for O(1) ref resolution + owner lookup.
    let mut key_map: HashMap<&str, &[u8]> = HashMap::new();
    let mut key_owners: HashMap<String, String> = HashMap::new();
    let mut encrypted_keys: HashMap<String, Vec<u8>> = HashMap::new();
    for ek in &snap.encrypted_keys {
        if key_map.insert(ek.id.as_str(), ek.ciphertext.as_slice()).is_some() {
            return Err(Status::invalid_argument(format!(
                "duplicate encrypted_key id: {}",
                ek.id
            )));
        }
        // S3 dual-defense: key_id -> owner_user_id (constitution XX). An
        // empty owner_user_id is accepted (the field is reserved for S3; S2
        // may push an empty string for bootstrap providers).
        key_owners.insert(ek.id.clone(), ek.owner_user_id.clone());
        encrypted_keys.insert(ek.id.clone(), ek.ciphertext.clone());
    }

    let mut providers = Vec::with_capacity(snap.providers.len());
    let mut provider_names: HashMap<&str, ()> = HashMap::new();
    // Store provider-level defaults for route resolution (model > provider
    // override chain).
    let mut provider_defaults: HashMap<String, ProviderDefaults> = HashMap::new();
    for p in &snap.providers {
        if provider_names.insert(p.name.as_str(), ()).is_some() {
            return Err(Status::invalid_argument(format!(
                "duplicate provider name: {}",
                p.name
            )));
        }
        let kind = parse_provider_kind(&p.kind)
            .map_err(|e| Status::invalid_argument(format!("provider {}: {e}", p.name)))?;
        let auth_method = parse_auth_method(&p.auth_method)
            .map_err(|e| Status::invalid_argument(format!("provider {}: {e}", p.name)))?;
        let capability_families = parse_capability_families(&p.capability_families).map_err(|e| {
            Status::invalid_argument(format!("provider {}: {e}", p.name))
        })?;
        if capability_families.is_empty() {
            return Err(Status::invalid_argument(format!(
                "provider {}: capability_families must not be empty",
                p.name
            )));
        }

        let ciphertext = key_map.get(p.encrypted_key_ref.as_str()).copied().ok_or_else(|| {
            Status::invalid_argument(format!(
                "provider {} references unknown encrypted_key_ref: {}",
                p.name, p.encrypted_key_ref
            ))
        })?;

        let anthropic_version = if p.anthropic_version.is_empty() {
            if kind == ProviderKind::Anthropic {
                return Err(Status::invalid_argument(format!(
                    "anthropic provider {} requires anthropic_version",
                    p.name
                )));
            }
            None
        } else {
            Some(p.anthropic_version.clone())
        };

        // Save defaults for route resolution.
        provider_defaults.insert(
            p.name.clone(),
            ProviderDefaults {
                kind,
                auth_method,
                anthropic_version: anthropic_version.clone(),
                timeout_ms: p.timeout_ms,
                upstream_path: p.upstream_path.clone(),
            },
        );

        providers.push(ResolvedProvider {
            name: p.name.clone(),
            kind,
            base_url: p.base_url.clone(),
            auth_method,
            key: SecretString::new(""),
            encrypted_key: Some(ciphertext.to_vec()),
            anthropic_version,
            capability_families,
            timeout_ms: p.timeout_ms,
            upstream_path: p.upstream_path.clone(),
            // Platform mode: proxy routing is not yet part of the gRPC
            // snapshot contract; standalone YAML config carries it.
            http_proxy: None,
            https_proxy: None,
            // Platform mode: `TokenCommand` token exchange is performed by the
            // Go control plane (future work); the snapshot does not yet carry
            // the auth command.
            auth_command: None,
            token_ttl_secs: None,
        });
    }

    // Routes: resolve effective values (model override ?? provider default).
    let mut routes = Vec::with_capacity(snap.routes.len());
    for r in &snap.routes {
        let defaults = provider_defaults.get(&r.provider).ok_or_else(|| {
            Status::invalid_argument(format!(
                "route {} references unknown provider: {}",
                r.alias, r.provider
            ))
        })?;
        routes.push(Route {
            alias: r.alias.clone(),
            provider: r.provider.clone(),
            upstream_model: r.upstream_model.clone(),
            kind: r
                .kind
                .as_deref()
                .and_then(|s| parse_provider_kind(s).ok())
                .unwrap_or(defaults.kind),
            auth_method: r
                .auth_method
                .as_deref()
                .and_then(|s| parse_auth_method(s).ok())
                .unwrap_or(defaults.auth_method),
            anthropic_version: r
                .anthropic_version
                .clone()
                .or(defaults.anthropic_version.clone()),
            timeout_ms: r.timeout_ms.or(defaults.timeout_ms),
            upstream_path: r.upstream_path.clone().or(defaults.upstream_path.clone()),
            // Platform mode: per-route proxy overrides are not yet part of the
            // gRPC snapshot contract.
            http_proxy: None,
            https_proxy: None,
        });
    }

    // Mirror proto virtual keys verbatim into the snapshot (S3). The hot path
    // authenticates by hash + checks owner against `key_owners` (constitution
    // XX dual-defense). Empty in S2 bootstrap pushes.
    let virtual_keys: Vec<VirtualKeyEntry> = snap
        .virtual_keys
        .iter()
        .map(|vk| VirtualKeyEntry {
            id: vk.id.clone(),
            token_hash: vk.token_hash.clone(),
            owner_user_id: vk.owner_user_id.clone(),
            provider_key_id: vk.provider_key_id.clone(),
            allowed_models: vk.allowed_models.clone(),
            allowed_providers: vk.allowed_providers.clone(),
            expires_at: vk.expires_at,
            max_concurrency: vk.max_concurrency,
            enabled: vk.enabled,
        })
        .collect();

    Ok(Arc::new(RuntimeSnapshot {
        version: snap.version,
        providers,
        routes,
        gateway_keys: Vec::new(),
        virtual_keys,
        key_owners,
        encrypted_keys,
        upstream: UpstreamConfig {
            timeout_ms: 60_000,
            http_proxy: None,
            https_proxy: None,
        },
    }))
}

fn parse_provider_kind(s: &str) -> Result<ProviderKind, String> {
    match s {
        "openai-compatible" => Ok(ProviderKind::OpenaiCompatible),
        "openai-responses" => Ok(ProviderKind::OpenaiResponses),
        "anthropic" => Ok(ProviderKind::Anthropic),
        "gemini" => Ok(ProviderKind::Gemini),
        "gemini-interactions" => Ok(ProviderKind::GeminiInteractions),
        other => Err(format!("unknown provider kind: {other}")),
    }
}

fn parse_auth_method(s: &str) -> Result<AuthMethod, String> {
    match s {
        "bearer" => Ok(AuthMethod::Bearer),
        "api_key_header" => Ok(AuthMethod::ApiKeyHeader),
        "query_key" => Ok(AuthMethod::QueryKey),
        "token_command" => Ok(AuthMethod::TokenCommand),
        other => Err(format!("unknown auth method: {other}")),
    }
}

fn parse_capability_families(families: &[String]) -> Result<Vec<CapabilityFamily>, String> {
    families.iter().map(|f| CapabilityFamily::from_str(f)).collect()
}

/// Provider-level defaults stored for route resolution (model > provider
/// override chain). The Go builder populates RouteEntry with effective values
/// when possible; this struct lets the Rust side resolve any field the builder
/// left unset.
struct ProviderDefaults {
    kind: ProviderKind,
    auth_method: AuthMethod,
    anthropic_version: Option<String>,
    timeout_ms: Option<u64>,
    upstream_path: Option<String>,
}
