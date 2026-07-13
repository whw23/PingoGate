//! Provider adapter abstraction (constitution XI).
//!
//! An adapter declares its provider family, the namespace it occupies, the
//! capability families it supports (only `generation.stateless` this phase),
//! and how the gateway injects upstream credentials. Adapters never hold a
//! plaintext key - the pipeline injects auth in `upstream_request_filter`.
//!
//! ## Six interfaces, three adapters (YAGNI / constitution II)
//!
//! S1 is homomorphic passthrough: an adapter's job is to declare the auth
//! method and the native error-body shape for its protocol surface - it does
//! NOT transform request bodies. Because auth + error shape are identical
//! within each provider family, the six inbound interfaces collapse to three
//! adapter variants via [`ProviderAdapter::for_protocol`]:
//!
//! | Protocol (`ProtocolKind`)            | Adapter variant   | Auth          | Error shape |
//! |--------------------------------------|-------------------|---------------|-------------|
//! | `OpenAiCompatible` (Chat Completions)| `OpenAi`          | `Bearer`      | OpenAI      |
//! | `OpenAiResponses` (Responses API)    | `OpenAi`          | `Bearer`      | OpenAI      |
//! | `Anthropic` (Messages)               | `Anthropic`       | `ApiKeyHeader`| Anthropic   |
//! | `Gemini` (generateContent / stream)  | `Gemini`          | `QueryKey`    | Gemini      |
//! | `GeminiInteractions` (Interactions)  | `Gemini`          | `QueryKey`    | Gemini      |
//!
//! `streamGenerateContent` is the same Gemini protocol with a streaming flag
//! (handled in protocol detection, T7); it shares `ProtocolKind::Gemini`.

use pingogate_core_types::{AuthMethod, CapabilityFamily, ProtocolKind, ProviderKind};
use serde_json::Value;

use pingogate_core_types::AppError;

/// Capability families supported this phase (constitution XI - fail fast on
/// unsupported families).
const STATELESS: &[CapabilityFamily] = &[CapabilityFamily::GenerationStateless];

/// A provider family adapter. Enum (not trait) because the adapter set is fixed
/// at three families for S1; this avoids boxing, gives exhaustiveness on
/// protocol dispatch, and matches the `for_protocol -> Self` constructor the
/// spec calls for. Adding a new family is a one-variant extension.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProviderAdapter {
    /// OpenAI family: Chat Completions + Responses. Auth: `Authorization:
    /// Bearer <key>`. Error body: OpenAI shape (`openai::render_error`).
    OpenAi,
    /// Anthropic family: Messages. Auth: `x-api-key: <key>` +
    /// `anthropic-version`. Error body: Anthropic shape.
    Anthropic,
    /// Gemini family: generateContent / streamGenerateContent / Interactions.
    /// Auth: `x-goog-api-key: <key>` or `?key=<key>`. Error body: Gemini shape.
    Gemini,
}

impl ProviderAdapter {
    /// Return the adapter for an inbound [`ProtocolKind`]. Related protocols
    /// (OpenAI Chat/Responses, Gemini generateContent/Interactions) share an
    /// adapter because their auth method and error body shape are identical
    /// (constitution II - do not duplicate).
    pub fn for_protocol(protocol: ProtocolKind) -> Self {
        match protocol {
            ProtocolKind::OpenAiCompatible | ProtocolKind::OpenAiResponses => Self::OpenAi,
            ProtocolKind::Anthropic => Self::Anthropic,
            ProtocolKind::Gemini | ProtocolKind::GeminiInteractions => Self::Gemini,
        }
    }

    /// The upstream provider family this adapter represents.
    pub fn provider_kind(&self) -> ProviderKind {
        match self {
            Self::OpenAi => ProviderKind::OpenaiCompatible,
            Self::Anthropic => ProviderKind::Anthropic,
            Self::Gemini => ProviderKind::Gemini,
        }
    }

    /// Provider namespace segment (route prefix / metric label).
    pub fn namespace(&self) -> &'static str {
        match self {
            Self::OpenAi => "openai",
            Self::Anthropic => "anthropic",
            Self::Gemini => "gemini",
        }
    }

    /// Capability families this adapter supports (S1: `generation.stateless`).
    pub fn capability_families(&self) -> &'static [CapabilityFamily] {
        STATELESS
    }

    /// How the gateway injects credentials into the upstream request (FR-012).
    pub fn auth_method(&self) -> AuthMethod {
        match self {
            Self::OpenAi => AuthMethod::Bearer,
            Self::Anthropic => AuthMethod::ApiKeyHeader,
            Self::Gemini => AuthMethod::QueryKey,
        }
    }

    /// Whether this adapter supports `family`. Unsupported families must be
    /// rejected with an explicit error (FR-007).
    pub fn supports(&self, family: CapabilityFamily) -> bool {
        self.capability_families().contains(&family)
    }

    /// Render a gateway [`AppError`] into this adapter's native error body
    /// shape (FR-035). Dispatches to the per-family renderer.
    pub fn render_error(&self, err: &AppError) -> Value {
        match self {
            Self::OpenAi => crate::openai::render_error(err),
            Self::Anthropic => crate::anthropic::render_error(err),
            Self::Gemini => crate::gemini::render_error(err),
        }
    }
}

/// Return the adapter for a provider family (by [`ProviderKind`]).
pub fn adapter_for(kind: ProviderKind) -> ProviderAdapter {
    match kind {
        ProviderKind::OpenaiCompatible => ProviderAdapter::OpenAi,
        ProviderKind::Anthropic => ProviderAdapter::Anthropic,
        ProviderKind::Gemini => ProviderAdapter::Gemini,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn for_protocol_maps_five_protocols_to_three_adapters() {
        assert_eq!(
            ProviderAdapter::for_protocol(ProtocolKind::OpenAiCompatible),
            ProviderAdapter::OpenAi
        );
        assert_eq!(
            ProviderAdapter::for_protocol(ProtocolKind::OpenAiResponses),
            ProviderAdapter::OpenAi
        );
        assert_eq!(
            ProviderAdapter::for_protocol(ProtocolKind::Anthropic),
            ProviderAdapter::Anthropic
        );
        assert_eq!(
            ProviderAdapter::for_protocol(ProtocolKind::Gemini),
            ProviderAdapter::Gemini
        );
        assert_eq!(
            ProviderAdapter::for_protocol(ProtocolKind::GeminiInteractions),
            ProviderAdapter::Gemini
        );
    }

    #[test]
    fn all_adapters_declare_only_generation_stateless() {
        for adapter in [
            ProviderAdapter::OpenAi,
            ProviderAdapter::Anthropic,
            ProviderAdapter::Gemini,
        ] {
            assert!(adapter.supports(CapabilityFamily::GenerationStateless));
            assert_eq!(adapter.capability_families(), STATELESS);
        }
    }

    #[test]
    fn auth_method_matches_family() {
        assert_eq!(ProviderAdapter::OpenAi.auth_method(), AuthMethod::Bearer);
        assert_eq!(
            ProviderAdapter::Anthropic.auth_method(),
            AuthMethod::ApiKeyHeader
        );
        assert_eq!(ProviderAdapter::Gemini.auth_method(), AuthMethod::QueryKey);
    }

    #[test]
    fn namespaces_are_distinct() {
        assert_eq!(ProviderAdapter::OpenAi.namespace(), "openai");
        assert_eq!(ProviderAdapter::Anthropic.namespace(), "anthropic");
        assert_eq!(ProviderAdapter::Gemini.namespace(), "gemini");
    }

    #[test]
    fn provider_kind_round_trips_through_adapter_for() {
        for kind in [
            ProviderKind::OpenaiCompatible,
            ProviderKind::Anthropic,
            ProviderKind::Gemini,
        ] {
            assert_eq!(adapter_for(kind).provider_kind(), kind);
        }
    }

    #[test]
    fn render_error_dispatches_to_native_shape() {
        let err = AppError::AuthFailed;
        assert_eq!(
            ProviderAdapter::OpenAi.render_error(&err)["error"]["type"],
            "invalid_request_error"
        );
        assert_eq!(
            ProviderAdapter::Anthropic.render_error(&err)["error"]["type"],
            "authentication_error"
        );
        assert_eq!(
            ProviderAdapter::Gemini.render_error(&err)["error"]["status"],
            "UNAUTHENTICATED"
        );
    }
}
