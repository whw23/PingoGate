//! Shared domain enums: inbound protocol, provider family, capability family,
//! and the upstream authentication-method descriptor.

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};

/// Upstream provider interface declared in configuration (`kind:` in YAML).
///
/// Precise per *interface*, not per vendor family: the protocol-conversion
/// engine (constitution VI) maps between the inbound protocol and this declared
/// upstream interface, so `gemini` alone would be ambiguous (Gemini has several
/// wire interfaces with different request/response shapes). Gemini exposes two
/// interfaces this phase: the stateless `generateContent` /
/// `streamGenerateContent` (`kind: gemini`) and the stateful Interactions API
/// (`kind: gemini-interactions`, `POST /v1beta/interactions`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ProviderKind {
    /// OpenAI Chat Completions (`/chat/completions`).
    OpenaiCompatible,
    /// OpenAI Responses API (`/responses`).
    OpenaiResponses,
    /// Anthropic Messages (`/messages`).
    Anthropic,
    /// Google Gemini generateContent / streamGenerateContent (`:generateContent`).
    Gemini,
    /// Google Gemini Interactions API (`POST /v1beta/interactions`).
    GeminiInteractions,
}

impl ProviderKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::OpenaiCompatible => "openai-compatible",
            Self::OpenaiResponses => "openai-responses",
            Self::Anthropic => "anthropic",
            Self::Gemini => "gemini",
            Self::GeminiInteractions => "gemini-interactions",
        }
    }
}

impl fmt::Display for ProviderKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Inbound wire protocol identified from the request line/headers (research R3).
///
/// OpenAI Chat/Responses and Anthropic Messages are stateless request-response;
/// Gemini is reached through two surfaces: the stateless `generateContent` /
/// `streamGenerateContent` (`ProtocolKind::Gemini`) and the stateful
/// Interactions API (`ProtocolKind::GeminiInteractions`,
/// `POST /v1beta/interactions`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProtocolKind {
    /// OpenAI Chat Completions (`/v1/chat/completions`).
    OpenAiCompatible,
    /// OpenAI Responses API (`/v1/responses`).
    OpenAiResponses,
    Anthropic,
    /// Google Gemini `generateContent` / `streamGenerateContent` (`:generateContent`).
    Gemini,
    /// Google Gemini Interactions API (`/v1beta/interactions`).
    GeminiInteractions,
}

impl ProtocolKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::OpenAiCompatible => "openai-compatible",
            Self::OpenAiResponses => "openai-responses",
            Self::Anthropic => "anthropic",
            Self::Gemini => "gemini",
            Self::GeminiInteractions => "gemini-interactions",
        }
    }
}

impl fmt::Display for ProtocolKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Capability family a provider exposes. This phase supports only
/// `generation.stateless`; any other value is rejected at parse time so
/// unsupported families fail fast with a clear error (constitution XI).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub enum CapabilityFamily {
    GenerationStateless,
}

impl CapabilityFamily {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::GenerationStateless => "generation.stateless",
        }
    }
}

impl FromStr for CapabilityFamily {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "generation.stateless" => Ok(Self::GenerationStateless),
            other => Err(format!("unsupported capability family: {other}")),
        }
    }
}

impl TryFrom<String> for CapabilityFamily {
    type Error = String;
    fn try_from(value: String) -> Result<Self, Self::Error> {
        value.parse()
    }
}

impl From<CapabilityFamily> for String {
    fn from(value: CapabilityFamily) -> Self {
        value.as_str().to_string()
    }
}

impl fmt::Display for CapabilityFamily {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// How the gateway injects credentials into the upstream request (FR-012).
/// The Anthropic version header value is carried at the provider level.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuthMethod {
    Bearer,
    ApiKeyHeader,
    QueryKey,
    /// Token obtained from an external command (standalone: the Rust kernel
    /// runs `auth.command` via the shell and injects `Authorization: Bearer
    /// <token>`; the command is provider-specific so different vendors can
    /// plug different token strategies). In platform mode the Go control plane
    /// performs the token exchange and injects the credential instead.
    TokenCommand,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn provider_kind_serde_uses_kebab_case() {
        let k: ProviderKind = serde_json::from_str("\"openai-compatible\"").unwrap();
        assert_eq!(k, ProviderKind::OpenaiCompatible);
        assert_eq!(ProviderKind::Anthropic.as_str(), "anthropic");
    }

    #[test]
    fn capability_family_parses_supported_and_rejects_unknown() {
        assert_eq!(
            "generation.stateless".parse::<CapabilityFamily>().unwrap(),
            CapabilityFamily::GenerationStateless
        );
        assert!("generation.stateful".parse::<CapabilityFamily>().is_err());
    }

    #[test]
    fn capability_family_serde_roundtrip_and_rejection() {
        let f: CapabilityFamily = serde_json::from_str("\"generation.stateless\"").unwrap();
        assert_eq!(
            serde_json::to_string(&f).unwrap(),
            "\"generation.stateless\""
        );
        assert!(serde_json::from_str::<CapabilityFamily>("\"embeddings\"").is_err());
    }

    #[test]
    fn protocol_kind_new_variants_have_stable_tokens() {
        assert_eq!(ProtocolKind::OpenAiResponses.as_str(), "openai-responses");
        assert_eq!(ProtocolKind::Gemini.as_str(), "gemini");
        assert_eq!(
            ProtocolKind::GeminiInteractions.as_str(),
            "gemini-interactions"
        );
    }

    #[test]
    fn protocol_kind_display_matches_as_str() {
        for p in [
            ProtocolKind::OpenAiCompatible,
            ProtocolKind::OpenAiResponses,
            ProtocolKind::Anthropic,
            ProtocolKind::Gemini,
            ProtocolKind::GeminiInteractions,
        ] {
            assert_eq!(p.to_string(), p.as_str());
        }
    }
}
