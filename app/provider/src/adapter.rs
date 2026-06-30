//! Provider adapter abstraction (constitution XI).
//!
//! An adapter declares its provider family, the namespace it occupies, the
//! capability families it supports (only `generation.stateless` this phase),
//! and how the gateway injects upstream credentials. Adapters never hold a
//! plaintext key — the pipeline injects auth in `upstream_request_filter`.

use pingo_core::{CapabilityFamily, ProviderKind};

pub use pingo_core::AuthMethod;

/// A provider family adapter.
pub trait ProviderAdapter: Send + Sync {
    fn kind(&self) -> ProviderKind;
    fn namespace(&self) -> &'static str;
    fn capability_families(&self) -> &'static [CapabilityFamily];
    fn auth_method(&self) -> AuthMethod;

    /// Whether this adapter supports `family`. Unsupported families must be
    /// rejected with an explicit error (FR-007).
    fn supports(&self, family: CapabilityFamily) -> bool {
        self.capability_families().contains(&family)
    }
}

const STATELESS: &[CapabilityFamily] = &[CapabilityFamily::GenerationStateless];

pub struct OpenAiAdapter;
impl ProviderAdapter for OpenAiAdapter {
    fn kind(&self) -> ProviderKind {
        ProviderKind::OpenaiCompatible
    }
    fn namespace(&self) -> &'static str {
        "openai"
    }
    fn capability_families(&self) -> &'static [CapabilityFamily] {
        STATELESS
    }
    fn auth_method(&self) -> AuthMethod {
        AuthMethod::Bearer
    }
}

pub struct AnthropicAdapter;
impl ProviderAdapter for AnthropicAdapter {
    fn kind(&self) -> ProviderKind {
        ProviderKind::Anthropic
    }
    fn namespace(&self) -> &'static str {
        "anthropic"
    }
    fn capability_families(&self) -> &'static [CapabilityFamily] {
        STATELESS
    }
    fn auth_method(&self) -> AuthMethod {
        AuthMethod::ApiKeyHeader
    }
}

pub struct GeminiAdapter;
impl ProviderAdapter for GeminiAdapter {
    fn kind(&self) -> ProviderKind {
        ProviderKind::Gemini
    }
    fn namespace(&self) -> &'static str {
        "gemini"
    }
    fn capability_families(&self) -> &'static [CapabilityFamily] {
        STATELESS
    }
    fn auth_method(&self) -> AuthMethod {
        AuthMethod::QueryKey
    }
}

/// Return the adapter for a provider family.
pub fn adapter_for(kind: ProviderKind) -> Box<dyn ProviderAdapter> {
    match kind {
        ProviderKind::OpenaiCompatible => Box::new(OpenAiAdapter),
        ProviderKind::Anthropic => Box::new(AnthropicAdapter),
        ProviderKind::Gemini => Box::new(GeminiAdapter),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_adapters_declare_only_generation_stateless() {
        for kind in [
            ProviderKind::OpenaiCompatible,
            ProviderKind::Anthropic,
            ProviderKind::Gemini,
        ] {
            let a = adapter_for(kind);
            assert!(a.supports(CapabilityFamily::GenerationStateless));
            assert_eq!(a.capability_families(), STATELESS);
        }
    }

    #[test]
    fn auth_method_matches_family() {
        assert_eq!(OpenAiAdapter.auth_method(), AuthMethod::Bearer);
        assert_eq!(AnthropicAdapter.auth_method(), AuthMethod::ApiKeyHeader);
        assert_eq!(GeminiAdapter.auth_method(), AuthMethod::QueryKey);
    }

    #[test]
    fn namespaces_are_distinct() {
        assert_eq!(OpenAiAdapter.namespace(), "openai");
        assert_eq!(AnthropicAdapter.namespace(), "anthropic");
        assert_eq!(GeminiAdapter.namespace(), "gemini");
    }
}
