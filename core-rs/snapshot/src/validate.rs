//! Schema and semantic validation for [`GatewayConfig`] (config-schema contract).
//!
//! Schema-level rejection (unknown fields, type errors, unknown capability
//! families) happens during parse. This module adds cross-field semantic checks
//! and reports every issue with a concrete path so a rejected reload names what
//! is wrong (FR-020). Secret-resolvability is enforced at snapshot build.

use std::collections::HashSet;

use pingogate_core_types::{ProviderKind, SecretError};

use crate::model::{AuthMethodKind, GatewayConfig};

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
    check_unique(
        cfg.routes.iter().map(|r| r.alias.as_str()),
        "routes",
        &mut errs,
    );

    for (i, p) in cfg.providers.iter().enumerate() {
        if !auth_compatible(p.kind, p.auth.method) {
            errs.push(ValidationError::new(
                format!("providers[{i}].auth.method"),
                format!("auth method is not compatible with kind {}", p.kind),
            ));
        }
        if p.kind == ProviderKind::Anthropic && p.anthropic_version.is_none() {
            errs.push(ValidationError::new(
                format!("providers[{i}].anthropic_version"),
                "anthropic provider requires anthropic_version",
            ));
        }
        if p.capability_families.is_empty() {
            errs.push(ValidationError::new(
                format!("providers[{i}].capability_families"),
                "at least one capability family is required",
            ));
        }
    }

    let provider_names: HashSet<&str> = cfg.providers.iter().map(|p| p.name.as_str()).collect();
    for (i, r) in cfg.routes.iter().enumerate() {
        if !provider_names.contains(r.provider.as_str()) {
            errs.push(ValidationError::new(
                format!("routes[{i}].provider"),
                format!("unknown provider: {}", r.provider),
            ));
        }
    }

    if errs.is_empty() {
        Ok(())
    } else {
        Err(ConfigError::Invalid(errs))
    }
}

fn auth_compatible(kind: ProviderKind, method: AuthMethodKind) -> bool {
    matches!(
        (kind, method),
        (ProviderKind::OpenaiCompatible, AuthMethodKind::Bearer)
            | (ProviderKind::Anthropic, AuthMethodKind::ApiKeyHeader)
            | (ProviderKind::Gemini, AuthMethodKind::QueryKey)
    )
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
    fn route_referencing_unknown_provider_is_rejected() {
        let yaml = format!(
            "{BASE_LISTENERS}routes:\n  - {{ alias: \"a\", provider: \"missing\", upstream_model: \"m\" }}\n"
        );
        let err = validate_semantics(&cfg(&yaml)).unwrap_err();
        match err {
            ConfigError::Invalid(v) => assert_eq!(v[0].path, "routes[0].provider"),
            _ => panic!("expected Invalid"),
        }
    }

    #[test]
    fn anthropic_without_version_is_rejected() {
        let yaml = format!(
            "{BASE_LISTENERS}providers:\n  - name: \"a\"\n    kind: \"anthropic\"\n    base_url: \"https://x\"\n    auth: {{ method: \"api_key_header\", key_ref: \"env:K\" }}\n    capability_families: [\"generation.stateless\"]\n"
        );
        let err = validate_semantics(&cfg(&yaml)).unwrap_err();
        assert!(
            matches!(err, ConfigError::Invalid(v) if v.iter().any(|e| e.path.ends_with("anthropic_version")))
        );
    }

    #[test]
    fn incompatible_auth_method_is_rejected() {
        let yaml = format!(
            "{BASE_LISTENERS}providers:\n  - name: \"a\"\n    kind: \"openai-compatible\"\n    base_url: \"https://x\"\n    auth: {{ method: \"query_key\", key_ref: \"env:K\" }}\n    capability_families: [\"generation.stateless\"]\n"
        );
        assert!(validate_semantics(&cfg(&yaml)).is_err());
    }

    #[test]
    fn minimal_valid_config_passes() {
        let yaml = format!(
            "{BASE_LISTENERS}providers:\n  - name: \"openai-main\"\n    kind: \"openai-compatible\"\n    base_url: \"https://api.openai.com\"\n    auth: {{ method: \"bearer\", key_ref: \"env:K\" }}\n    capability_families: [\"generation.stateless\"]\nroutes:\n  - {{ alias: \"gpt-4o\", provider: \"openai-main\", upstream_model: \"gpt-4o\" }}\n"
        );
        assert!(validate_semantics(&cfg(&yaml)).is_ok());
    }
}
