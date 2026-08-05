//! Tests for [`RuntimeSnapshot`] and [`SnapshotHolder`] (ported from 001 config).

use super::*;
use pingogate_core_types::{SecretError, SecretResolver as ResolverTrait};

/// Test-only resolver supporting env: and plain: schemes.
struct EnvResolver;

impl ResolverTrait for EnvResolver {
    fn resolve(&self, reference: &str) -> Result<SecretString, SecretError> {
        let reference = reference.trim();
        let (scheme, rest) = reference
            .split_once(':')
            .ok_or_else(|| SecretError::UnsupportedScheme(reference.to_string()))?;
        match scheme {
            "env" => {
                let var = rest.trim();
                let value =
                    std::env::var(var).map_err(|_| SecretError::MissingEnv(var.to_string()))?;
                Ok(SecretString::new(value))
            }
            "plain" => Ok(SecretString::new(rest.trim())),
            other => Err(SecretError::UnsupportedScheme(other.to_string())),
        }
    }
}

fn build_snapshot() -> Arc<RuntimeSnapshot> {
    std::env::set_var("PINGO_TEST_PROVIDER_KEY", "sk-test");
    std::env::set_var("PINGO_TEST_GW_KEY", "pg-test");
    let yaml = r#"
listeners:
  public: { address: "0.0.0.0:8080" }
  admin: { address: "127.0.0.1:9090" }
gateway_keys:
  - { name: "team-alpha", secret_ref: "env:PINGO_TEST_GW_KEY" }
  - { name: "disabled-team", secret_ref: "env:NONEXISTENT", enabled: false }
providers:
  - name: "openai-main"
    kind: "openai-compatible"
    base_url: "https://api.openai.com"
    auth: { method: "bearer", key_ref: "env:PINGO_TEST_PROVIDER_KEY" }
    capability_families: ["generation.stateless"]
    models:
      - alias: "gpt-4o"
        upstream_model: "gpt-4o"
"#;
    let cfg = GatewayConfig::from_yaml(yaml).unwrap();
    RuntimeSnapshot::build(&cfg, &EnvResolver, 1).unwrap()
}

#[test]
fn builds_snapshot_with_routes_and_providers() {
    let snap = build_snapshot();
    assert_eq!(snap.version, 1);
    assert_eq!(snap.route("gpt-4o").unwrap().upstream_model, "gpt-4o");
    assert_eq!(
        snap.provider("openai-main").unwrap().auth_method,
        AuthMethod::Bearer
    );
}

/// Standalone-mode `RuntimeSnapshot::build` leaves the platform-mode fields
/// empty (constitution X: standalone has no ciphertext, no virtual keys).
#[test]
fn standalone_build_leaves_platform_fields_empty() {
    let snap = build_snapshot();
    assert!(snap.virtual_keys.is_empty(), "standalone has no virtual keys");
    assert!(snap.key_owners.is_empty());
    assert!(snap.encrypted_keys.is_empty());
}

#[test]
fn disabled_gateway_key_is_skipped_and_secret_not_required() {
    let snap = build_snapshot();
    assert_eq!(
        snap.authenticate_gateway_key("pg-test").unwrap().name,
        "team-alpha"
    );
    assert!(snap.authenticate_gateway_key("wrong").is_none());
}

#[test]
fn missing_provider_secret_fails_build_not_runtime() {
    let yaml = r#"
listeners:
  public: { address: "0.0.0.0:8080" }
  admin: { address: "127.0.0.1:9090" }
providers:
  - name: "p"
    kind: "openai-compatible"
    base_url: "https://x"
    auth: { method: "bearer", key_ref: "env:PINGO_TEST_MISSING_PROVIDER_KEY" }
    capability_families: ["generation.stateless"]
"#;
    let cfg = GatewayConfig::from_yaml(yaml).unwrap();
    let err = RuntimeSnapshot::build(&cfg, &EnvResolver, 1).unwrap_err();
    assert!(matches!(err, ConfigError::Secret { .. }));
}

#[test]
fn holder_swaps_snapshot_atomically() {
    let first = build_snapshot();
    let holder = SnapshotHolder::new(first.clone());
    assert_eq!(holder.load().version, 1);
    let second = rebuild_at_version(2);
    holder.store(second);
    assert_eq!(holder.load().version, 2);
}

#[test]
fn model_overrides_provider_defaults() {
    std::env::set_var("PINGO_TEST_PROVIDER_KEY", "sk-test");
    let yaml = r#"
listeners:
  public: { address: "0.0.0.0:8080" }
  admin: { address: "127.0.0.1:9090" }
providers:
  - name: "multi"
    kind: "openai-compatible"
    base_url: "https://api.openai.com"
    auth: { method: "bearer", key_ref: "env:PINGO_TEST_PROVIDER_KEY" }
    capability_families: ["generation.stateless"]
    timeout_ms: 120000
    upstream_path: "/v1/chat/completions"
    models:
      - alias: "gpt-4o"
        upstream_model: "gpt-4o"
      - alias: "o1-pro"
        upstream_model: "o1-pro"
        timeout_ms: 300000
        upstream_path: "/v1/responses"
"#;
    let cfg = GatewayConfig::from_yaml(yaml).unwrap();
    let snap = RuntimeSnapshot::build(&cfg, &EnvResolver, 1).unwrap();

    // gpt-4o: inherits all provider defaults.
    let r1 = snap.route("gpt-4o").unwrap();
    assert_eq!(r1.timeout_ms, Some(120_000));
    assert_eq!(r1.upstream_path.as_deref(), Some("/v1/chat/completions"));

    // o1-pro: overrides timeout and upstream_path.
    let r2 = snap.route("o1-pro").unwrap();
    assert_eq!(r2.timeout_ms, Some(300_000));
    assert_eq!(r2.upstream_path.as_deref(), Some("/v1/responses"));
}

fn rebuild_at_version(version: u64) -> Arc<RuntimeSnapshot> {
    std::env::set_var("PINGO_TEST_PROVIDER_KEY", "sk-test");
    std::env::set_var("PINGO_TEST_GW_KEY", "pg-test");
    let yaml = r#"
listeners:
  public: { address: "0.0.0.0:8080" }
  admin: { address: "127.0.0.1:9090" }
gateway_keys:
  - { name: "team-alpha", secret_ref: "env:PINGO_TEST_GW_KEY" }
providers:
  - name: "openai-main"
    kind: "openai-compatible"
    base_url: "https://api.openai.com"
    auth: { method: "bearer", key_ref: "env:PINGO_TEST_PROVIDER_KEY" }
    capability_families: ["generation.stateless"]
    models:
      - alias: "gpt-4o"
        upstream_model: "gpt-4o"
"#;
    let cfg = GatewayConfig::from_yaml(yaml).unwrap();
    RuntimeSnapshot::build(&cfg, &EnvResolver, version).unwrap()
}

/// The checked-in `pingogate-core.yaml.example` must always parse with the
/// current schema (prevents doc/schema drift). Semantic validation is skipped
/// here (it requires resolving env/plain secrets), but the schema-level parse
/// is enforced.
#[test]
fn example_config_parses_with_current_schema() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(|p| p.parent()) // core-rs/snapshot -> core-rs -> repo root
        .unwrap()
        .join("pingogate-core.yaml.example");
    let yaml = std::fs::read_to_string(&path)
        .unwrap_or_else(|_| panic!("cannot read example at {}", path.display()));
    let cfg = GatewayConfig::from_yaml(&yaml)
        .unwrap_or_else(|e| panic!("example config failed to parse: {e}"));
    // Every provider must have at least one model (models are how routing works).
    assert!(
        !cfg.providers.is_empty(),
        "example should define providers"
    );
    let total_models: usize = cfg.providers.iter().map(|p| p.models.len()).sum();
    assert!(
        total_models > 0,
        "example should define at least one model nested under a provider"
    );
}
