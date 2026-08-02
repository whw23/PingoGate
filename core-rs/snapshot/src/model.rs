//! Serde schema for `pingogate.yaml` (config-schema contract).
//!
//! Unknown fields are rejected (`deny_unknown_fields`) so typos fail validation
//! rather than being silently ignored. These are raw parsed structs; semantic
//! validation lives in [`crate::validate`] and resolution in [`crate::snapshot`].

use pingogate_core_types::{CapabilityFamily, ProviderKind};
use serde::Deserialize;

use crate::validate::ConfigError;

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GatewayConfig {
    pub listeners: Listeners,
    #[serde(default)]
    pub gateway_keys: Vec<GatewayKeyCfg>,
    #[serde(default)]
    pub providers: Vec<ProviderCfg>,
    #[serde(default)]
    pub routes: Vec<RouteCfg>,
    #[serde(default)]
    pub observability: Observability,
    #[serde(default)]
    pub upstream: Upstream,
}

impl GatewayConfig {
    /// Parse YAML into the schema (no semantic validation yet).
    pub fn from_yaml(yaml: &str) -> Result<Self, ConfigError> {
        serde_yaml::from_str(yaml).map_err(|e| ConfigError::Parse(e.to_string()))
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Listeners {
    pub public: ListenerCfg,
    pub admin: ListenerCfg,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ListenerCfg {
    pub address: String,
    #[serde(default)]
    pub tls: Option<TlsCfg>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TlsCfg {
    pub cert: String,
    pub key_ref: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GatewayKeyCfg {
    pub name: String,
    pub secret_ref: String,
    #[serde(default = "default_true")]
    pub enabled: bool,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProviderCfg {
    pub name: String,
    pub kind: ProviderKind,
    pub base_url: String,
    #[serde(default)]
    pub anthropic_version: Option<String>,
    pub auth: AuthCfg,
    pub capability_families: Vec<CapabilityFamily>,
    /// Per-provider upstream timeout in milliseconds (issue 3: timeout should
    /// follow the provider, not be global). Overrides `upstream.timeout_ms`
    /// when set. Different providers have different latency profiles (e.g.
    /// Anthropic may need longer than OpenAI).
    #[serde(default)]
    pub timeout_ms: Option<u64>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AuthCfg {
    pub method: AuthMethodKind,
    pub key_ref: String,
}

/// Wire form of the upstream auth method (`auth.method` in YAML).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AuthMethodKind {
    Bearer,
    ApiKeyHeader,
    QueryKey,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RouteCfg {
    pub alias: String,
    pub provider: String,
    pub upstream_model: String,
    /// Upstream path template (issue 2: paths are not hardcoded). Supports
    /// `{model}` placeholder substituted with `upstream_model` at request time.
    /// When set, the forwarded path is rewritten to this template (prepended
    /// with the provider base_path) instead of the original inbound path.
    /// Example: "/v1/chat/completions" or "/api/v2/models/{model}/generate".
    #[serde(default)]
    pub upstream_path: Option<String>,
    /// Auth method override (issue 1: one provider, multiple protocols). When
    /// set, uses this auth method instead of the provider's default. Allows a
    /// single upstream to serve different protocols with different auth styles
    /// (e.g. OpenAI-compatible Bearer for Chat, Anthropic api_key_header for
    /// Messages).
    #[serde(default)]
    pub auth_method: Option<AuthMethodKind>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Observability {
    #[serde(default)]
    pub metrics: Metrics,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Metrics {
    #[serde(default)]
    pub label_by_principal: bool,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Upstream {
    #[serde(default = "default_timeout_ms")]
    pub timeout_ms: u64,
}

impl Default for Upstream {
    fn default() -> Self {
        Self {
            timeout_ms: default_timeout_ms(),
        }
    }
}

fn default_true() -> bool {
    true
}

fn default_timeout_ms() -> u64 {
    60_000
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_minimal_config_with_defaults() {
        let yaml = r#"
listeners:
  public: { address: "0.0.0.0:8080" }
  admin: { address: "127.0.0.1:9090" }
gateway_keys:
  - { name: "a", secret_ref: "env:PINGO_KEY_ALPHA" }
providers:
  - name: "openai-main"
    kind: "openai-compatible"
    base_url: "https://api.openai.com"
    auth: { method: "bearer", key_ref: "env:OPENAI_API_KEY" }
    capability_families: ["generation.stateless"]
routes:
  - { alias: "gpt-4o", provider: "openai-main", upstream_model: "gpt-4o" }
"#;
        let cfg = GatewayConfig::from_yaml(yaml).unwrap();
        assert_eq!(cfg.providers[0].kind, ProviderKind::OpenaiCompatible);
        assert!(cfg.gateway_keys[0].enabled, "enabled defaults to true");
        assert_eq!(cfg.upstream.timeout_ms, 60_000, "timeout default applied");
    }

    #[test]
    fn unknown_field_is_rejected() {
        let yaml = r#"
listeners:
  public: { address: "0.0.0.0:8080" }
  admin: { address: "127.0.0.1:9090" }
bogus_top_level: true
"#;
        assert!(matches!(
            GatewayConfig::from_yaml(yaml).unwrap_err(),
            ConfigError::Parse(_)
        ));
    }

    #[test]
    fn unknown_capability_family_is_rejected_at_parse() {
        let yaml = r#"
listeners:
  public: { address: "0.0.0.0:8080" }
  admin: { address: "127.0.0.1:9090" }
providers:
  - name: "p"
    kind: "openai-compatible"
    base_url: "https://x"
    auth: { method: "bearer", key_ref: "env:K" }
    capability_families: ["generation.stateful"]
"#;
        assert!(GatewayConfig::from_yaml(yaml).is_err());
    }
}
