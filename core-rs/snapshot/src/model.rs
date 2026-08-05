//! Serde schema for `pingogate.yaml` (config-schema contract).
//!
//! Unknown fields are rejected (`deny_unknown_fields`) so typos fail validation
//! rather than being silently ignored. These are raw parsed structs; semantic
//! validation lives in [`crate::validate`] and resolution in [`crate::snapshot`].
//!
//! **Config hierarchy** (issue: model > provider > global):
//! - Provider-level fields (kind, auth, timeout_ms, upstream_path,
//!   anthropic_version) are defaults for all models under that provider.
//! - Model-level fields are optional overrides; `None` = inherit from provider.
//! - The resolved [`crate::snapshot::Route`] carries effective values after
//!   applying the override chain.

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

/// Provider-level configuration. All fields except `name`/`kind`/`base_url`/
/// `auth`/`capability_families` are defaults that models can override.
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
    /// Provider-level timeout default (ms). Overridden by model.timeout_ms.
    #[serde(default)]
    pub timeout_ms: Option<u64>,
    /// Provider-level upstream path template (supports `{model}`). Overridden
    /// by model.upstream_path.
    #[serde(default)]
    pub upstream_path: Option<String>,
    /// Models under this provider. Each model inherits provider defaults and
    /// can override any of them.
    #[serde(default)]
    pub models: Vec<ModelCfg>,
}

/// Model-level configuration nested under a provider. `alias` and
/// `upstream_model` are required; all other fields are optional overrides
/// (`None` = inherit from the parent provider).
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ModelCfg {
    /// Client-visible model name (what the client sends in the `model` field).
    pub alias: String,
    /// Actual model name at the upstream provider.
    pub upstream_model: String,
    /// Override the provider's kind for this model (for protocol conversion).
    #[serde(default)]
    pub kind: Option<ProviderKind>,
    /// Override the provider's auth method for this model.
    #[serde(default)]
    pub auth_method: Option<AuthMethodKind>,
    /// Override the provider's anthropic_version for this model.
    #[serde(default)]
    pub anthropic_version: Option<String>,
    /// Override the provider's timeout_ms for this model.
    #[serde(default)]
    pub timeout_ms: Option<u64>,
    /// Override the provider's upstream_path template for this model.
    #[serde(default)]
    pub upstream_path: Option<String>,
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
    fn parses_nested_models_with_overrides() {
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
    timeout_ms: 120000
    models:
      - alias: "gpt-4o"
        upstream_model: "gpt-4o"
      - alias: "o1-pro"
        upstream_model: "o1-pro"
        timeout_ms: 300000
        upstream_path: "/v1/responses"
"#;
        let cfg = GatewayConfig::from_yaml(yaml).unwrap();
        assert_eq!(cfg.providers[0].models.len(), 2);
        assert_eq!(cfg.providers[0].models[0].alias, "gpt-4o");
        assert_eq!(cfg.providers[0].models[1].timeout_ms, Some(300_000));
        assert_eq!(
            cfg.providers[0].models[1].upstream_path.as_deref(),
            Some("/v1/responses")
        );
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
