//! US2 contract test (T035): `pingogate.yaml` semantic validation.
//!
//! Exercises the config-schema contract through the crate's public surface: a
//! valid document builds, and each class of semantic error is reported as
//! `ConfigError::Invalid` with a concrete dotted path so a rejected reload can
//! name exactly what is wrong (FR-020). Schema-level rejections (unknown fields,
//! unknown capability families) surface as `ConfigError::Parse` during parse.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use pingo_config::validate::validate_semantics;
use pingo_config::{ConfigError, GatewayConfig};

const LISTENERS: &str = r#"
listeners:
  public: { address: "0.0.0.0:8080" }
  admin: { address: "127.0.0.1:9090" }
"#;

fn parse(yaml: &str) -> GatewayConfig {
    GatewayConfig::from_yaml(yaml).expect("schema-valid YAML must parse")
}

/// Assert that `yaml` fails semantic validation and that some issue's path
/// matches `path_suffix`.
fn assert_invalid_at(yaml: &str, path_suffix: &str) {
    match validate_semantics(&parse(yaml)).unwrap_err() {
        ConfigError::Invalid(issues) => assert!(
            issues.iter().any(|e| e.path.ends_with(path_suffix)),
            "expected an issue at *{path_suffix}, got {issues:?}"
        ),
        other => panic!("expected ConfigError::Invalid, got {other:?}"),
    }
}

#[test]
fn fully_valid_config_passes_validation() {
    let yaml = format!(
        "{LISTENERS}\
providers:
  - name: \"openai-main\"
    kind: \"openai-compatible\"
    base_url: \"https://api.openai.com\"
    auth: {{ method: \"bearer\", key_ref: \"env:K\" }}
    capability_families: [\"generation.stateless\"]
routes:
  - {{ alias: \"gpt-4o\", provider: \"openai-main\", upstream_model: \"gpt-4o\" }}
"
    );
    assert!(validate_semantics(&parse(&yaml)).is_ok());
}

#[test]
fn route_to_unknown_provider_is_rejected_with_path() {
    let yaml = format!(
        "{LISTENERS}\
routes:
  - {{ alias: \"a\", provider: \"ghost\", upstream_model: \"m\" }}
"
    );
    assert_invalid_at(&yaml, "routes[0].provider");
}

#[test]
fn duplicate_provider_names_are_rejected() {
    let yaml = format!(
        "{LISTENERS}\
providers:
  - name: \"dup\"
    kind: \"openai-compatible\"
    base_url: \"https://x\"
    auth: {{ method: \"bearer\", key_ref: \"env:K\" }}
    capability_families: [\"generation.stateless\"]
  - name: \"dup\"
    kind: \"openai-compatible\"
    base_url: \"https://y\"
    auth: {{ method: \"bearer\", key_ref: \"env:K2\" }}
    capability_families: [\"generation.stateless\"]
"
    );
    assert_invalid_at(&yaml, "providers");
}

#[test]
fn duplicate_route_aliases_are_rejected() {
    let yaml = format!(
        "{LISTENERS}\
providers:
  - name: \"p\"
    kind: \"openai-compatible\"
    base_url: \"https://x\"
    auth: {{ method: \"bearer\", key_ref: \"env:K\" }}
    capability_families: [\"generation.stateless\"]
routes:
  - {{ alias: \"same\", provider: \"p\", upstream_model: \"m\" }}
  - {{ alias: \"same\", provider: \"p\", upstream_model: \"m2\" }}
"
    );
    assert_invalid_at(&yaml, "routes");
}

#[test]
fn anthropic_provider_without_version_is_rejected() {
    let yaml = format!(
        "{LISTENERS}\
providers:
  - name: \"a\"
    kind: \"anthropic\"
    base_url: \"https://x\"
    auth: {{ method: \"api_key_header\", key_ref: \"env:K\" }}
    capability_families: [\"generation.stateless\"]
"
    );
    assert_invalid_at(&yaml, "anthropic_version");
}

#[test]
fn incompatible_auth_method_is_rejected_with_path() {
    let yaml = format!(
        "{LISTENERS}\
providers:
  - name: \"a\"
    kind: \"openai-compatible\"
    base_url: \"https://x\"
    auth: {{ method: \"query_key\", key_ref: \"env:K\" }}
    capability_families: [\"generation.stateless\"]
"
    );
    assert_invalid_at(&yaml, "auth.method");
}

#[test]
fn empty_capability_families_is_rejected() {
    let yaml = format!(
        "{LISTENERS}\
providers:
  - name: \"a\"
    kind: \"openai-compatible\"
    base_url: \"https://x\"
    auth: {{ method: \"bearer\", key_ref: \"env:K\" }}
    capability_families: []
"
    );
    assert_invalid_at(&yaml, "capability_families");
}

#[test]
fn unknown_capability_family_is_a_parse_error() {
    let yaml = format!(
        "{LISTENERS}\
providers:
  - name: \"a\"
    kind: \"openai-compatible\"
    base_url: \"https://x\"
    auth: {{ method: \"bearer\", key_ref: \"env:K\" }}
    capability_families: [\"not.a.family\"]
"
    );
    assert!(matches!(
        GatewayConfig::from_yaml(&yaml),
        Err(ConfigError::Parse(_))
    ));
}
