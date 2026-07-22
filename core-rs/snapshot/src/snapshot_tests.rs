//! Tests for [`RuntimeSnapshot`] and [`SnapshotHolder`] (ported from 001 config).

use super::*;
use pingogate_core_types::{SecretError, SecretResolver as ResolverTrait};

/// Test-only env resolver (snapshot crate does not depend on storage).
struct EnvResolver;

impl ResolverTrait for EnvResolver {
    fn resolve(&self, reference: &str) -> Result<SecretString, SecretError> {
        let reference = reference.trim();
        let (scheme, rest) = reference
            .split_once(':')
            .ok_or_else(|| SecretError::UnsupportedScheme(reference.to_string()))?;
        assert_eq!(scheme, "env", "test resolver only supports env:");
        let var = rest.trim();
        let value = std::env::var(var).map_err(|_| SecretError::MissingEnv(var.to_string()))?;
        Ok(SecretString::new(value))
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
routes:
  - { alias: "gpt-4o", provider: "openai-main", upstream_model: "gpt-4o" }
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
/// (`virtual_keys`, `key_owners`, `encrypted_keys`) empty (constitution X:
/// standalone has no ciphertext, no virtual keys, no owner mapping).
#[test]
fn standalone_build_leaves_platform_fields_empty() {
    let snap = build_snapshot();
    assert!(snap.virtual_keys.is_empty(), "standalone has no virtual keys");
    assert!(
        snap.key_owners.is_empty(),
        "standalone has no key_owners (no encrypted keys)"
    );
    assert!(
        snap.encrypted_keys.is_empty(),
        "standalone resolves env refs into ResolvedProvider::key, not ciphertext"
    );
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
    // A second snapshot at version 2 replaces the first.
    let second = rebuild_at_version(2);
    holder.store(second);
    assert_eq!(holder.load().version, 2);
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
routes:
  - { alias: "gpt-4o", provider: "openai-main", upstream_model: "gpt-4o" }
"#;
    let cfg = GatewayConfig::from_yaml(yaml).unwrap();
    RuntimeSnapshot::build(&cfg, &EnvResolver, version).unwrap()
}
