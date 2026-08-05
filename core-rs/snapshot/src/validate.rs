//! Schema and semantic validation for [`GatewayConfig`] (config-schema contract).
//!
//! Schema-level rejection (unknown fields, type errors, unknown capability
//! families) happens during parse. This module adds cross-field semantic checks
//! and reports every issue with a concrete path so a rejected reload names what
//! is wrong (FR-020). Secret-resolvability is enforced at snapshot build.

use std::collections::HashSet;

use pingogate_core_types::{ProviderKind, SecretError};

use crate::model::GatewayConfig;

/// A single semantic problem, located by a dotted config path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValidationError {
    pub path: String,
    pub message: String,
}

impl ValidationError {
    fn new(path: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            path: path.into(),
            message: message.into(),
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error("failed to parse configuration: {0}")]
    Parse(String),

    #[error("configuration is invalid: {} issue(s)", .0.len())]
    Invalid(Vec<ValidationError>),

    #[error("secret resolution failed at {path}: {source}")]
    Secret {
        path: String,
        #[source]
        source: SecretError,
    },
}

/// Run all semantic checks, collecting every issue before failing.
pub fn validate_semantics(cfg: &GatewayConfig) -> Result<(), ConfigError> {
    let mut errs = Vec::new();

    check_unique(
        cfg.gateway_keys.iter().map(|k| k.name.as_str()),
        "gateway_keys",
        &mut errs,
    );
    check_unique(
        cfg.providers.iter().map(|p| p.name.as_str()),
        "providers",
        &mut errs,
    );

    // Model aliases must be unique across ALL providers (the client sends a
    // model name; it must resolve to exactly one provider+model).
    let mut global_aliases: HashSet<&str> = HashSet::new();
    for (i, p) in cfg.providers.iter().enumerate() {
        for (j, m) in p.models.iter().enumerate() {
            if !global_aliases.insert(m.alias.as_str()) {
                errs.push(ValidationError::new(
                    format!("providers[{i}].models[{j}].alias"),
                    format!("duplicate model alias across providers: {}", m.alias),
                ));
            }
        }
    }

    for (i, p) in cfg.providers.iter().enumerate() {
        // Anthropic kind requires anthropic_version (at provider or model level).
        // The effective check is at resolution time; here we only check the
        // provider-level default when the kind is anthropic and no model
        // overrides it. We defer the full effective check to snapshot build.
        if p.kind == ProviderKind::Anthropic && p.anthropic_version.is_none() {
            // Check if all models also lack anthropic_version override.
            let all_missing = p.models.iter().all(|m| m.anthropic_version.is_none());
            if all_missing && !p.models.is_empty() {
                errs.push(ValidationError::new(
                    format!("providers[{i}].anthropic_version"),
                    "anthropic provider requires anthropic_version (set at provider or model level)",
                ));
            }
        }
        if p.capability_families.is_empty() {
            errs.push(ValidationError::new(
                format!("providers[{i}].capability_families"),
                "at least one capability family is required",
            ));
        }

        // Validate model-level overrides.
        for (j, m) in p.models.iter().enumerate() {
            // Effective kind = model ?? provider.
            let effective_kind = m.kind.unwrap_or(p.kind);

            // Anthropic kind with effective anthropic_version check.
            if effective_kind == ProviderKind::Anthropic {
                let effective_version = m
                    .anthropic_version
                    .as_deref()
                    .or(p.anthropic_version.as_deref());
                if effective_version.is_none() {
                    errs.push(ValidationError::new(
                        format!("providers[{i}].models[{j}].anthropic_version"),
                        format!(
                            "model {} has effective anthropic kind but no anthropic_version (set at provider or model level)",
                            m.alias
                        ),
                    ));
                }
            }
        }
    }

    if errs.is_empty() {
        Ok(())
    } else {
        Err(ConfigError::Invalid(errs))
    }
}

fn check_unique<'a>(
    items: impl Iterator<Item = &'a str>,
    section: &str,
    errs: &mut Vec<ValidationError>,
) {
    let mut seen = HashSet::new();
    for name in items {
        if !seen.insert(name) {
            errs.push(ValidationError::new(
                section.to_string(),
                format!("duplicate name: {name}"),
            ));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::GatewayConfig;

    fn cfg(yaml: &str) -> GatewayConfig {
        GatewayConfig::from_yaml(yaml).unwrap()
    }

    const BASE_LISTENERS: &str = r#"
listeners:
  public: { address: "0.0.0.0:8080" }
  admin: { address: "127.0.0.1:9090" }
"#;

    #[test]
    fn duplicate_alias_across_providers_is_rejected() {
        let yaml = format!(
            r#"{BASE_LISTENERS}providers:
  - name: "p1"
    kind: "openai-compatible"
    base_url: "https://x"
    auth: {{ method: "bearer", key_ref: "env:K" }}
    capability_families: ["generation.stateless"]
    models:
      - {{ alias: "dup", upstream_model: "a" }}
  - name: "p2"
    kind: "openai-compatible"
    base_url: "https://y"
    auth: {{ method: "bearer", key_ref: "env:K2" }}
    capability_families: ["generation.stateless"]
    models:
      - {{ alias: "dup", upstream_model: "b" }}
"#
        );
        let err = validate_semantics(&cfg(&yaml)).unwrap_err();
        match err {
            ConfigError::Invalid(v) => {
                assert!(v.iter().any(|e| e.message.contains("duplicate model alias")));
            }
            _ => panic!("expected Invalid"),
        }
    }

    #[test]
    fn anthropic_without_version_is_rejected() {
        let yaml = format!(
            r#"{BASE_LISTENERS}providers:
  - name: "a"
    kind: "anthropic"
    base_url: "https://x"
    auth: {{ method: "api_key_header", key_ref: "env:K" }}
    capability_families: ["generation.stateless"]
    models:
      - {{ alias: "claude", upstream_model: "claude-3" }}
"#
        );
        let err = validate_semantics(&cfg(&yaml)).unwrap_err();
        assert!(
            matches!(err, ConfigError::Invalid(v) if v.iter().any(|e| e.path.ends_with("anthropic_version")))
        );
    }

    #[test]
    fn anthropic_model_override_satisfies_version() {
        let yaml = format!(
            r#"{BASE_LISTENERS}providers:
  - name: "a"
    kind: "anthropic"
    base_url: "https://x"
    auth: {{ method: "api_key_header", key_ref: "env:K" }}
    capability_families: ["generation.stateless"]
    models:
      - alias: "claude"
        upstream_model: "claude-3"
        anthropic_version: "2023-06-01"
"#
        );
        assert!(validate_semantics(&cfg(&yaml)).is_ok());
    }

    #[test]
    fn minimal_valid_config_passes() {
        let yaml = format!(
            r#"{BASE_LISTENERS}providers:
  - name: "openai-main"
    kind: "openai-compatible"
    base_url: "https://api.openai.com"
    auth: {{ method: "bearer", key_ref: "env:K" }}
    capability_families: ["generation.stateless"]
    models:
      - {{ alias: "gpt-4o", upstream_model: "gpt-4o" }}
"#
        );
        assert!(validate_semantics(&cfg(&yaml)).is_ok());
    }
}
