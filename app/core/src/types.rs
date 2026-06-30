//! Shared domain enums: inbound protocol, provider family, capability family,
//! and the upstream authentication-method descriptor.

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};

/// Upstream provider family declared in configuration (`kind:` in YAML).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ProviderKind {
    OpenaiCompatible,
    Anthropic,
    Gemini,
}

impl ProviderKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::OpenaiCompatible => "openai-compatible",
            Self::Anthropic => "anthropic",
            Self::Gemini => "gemini",
        }
    }
}

impl fmt::Display for ProviderKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Inbound wire protocol identified from the request line/headers (research R3).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProtocolKind {
    OpenAiCompatible,
    Anthropic,
    Gemini,
}

impl ProtocolKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::OpenAiCompatible => "openai-compatible",
            Self::Anthropic => "anthropic",
            Self::Gemini => "gemini",
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
}
