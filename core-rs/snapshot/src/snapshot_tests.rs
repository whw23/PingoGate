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
fn token_command_provider_carries_command_and_ttl() {
    let yaml = r#"
listeners:
  public: { address: "0.0.0.0:8080" }
  admin: { address: "127.0.0.1:9090" }
gateway_keys:
  - { name: "k", secret_ref: "plain:pg" }
providers:
  - name: "vertex-ai"
    kind: "gemini"
    base_url: "https://aiplatform.googleapis.com"
    auth:
      method: "token_command"
      command: "gcloud auth application-default print-access-token"
      token_ttl_secs: 900
    capability_families: ["generation.stateless"]
    models:
      - alias: "v-flash"
        upstream_model: "gemini-3.5-flash"
"#;
    let cfg = GatewayConfig::from_yaml(yaml).unwrap();
    let snap = RuntimeSnapshot::build(&cfg, &EnvResolver, 1).unwrap();
    let p = snap.provider("vertex-ai").unwrap();
    assert_eq!(p.auth_method, AuthMethod::TokenCommand);
    assert_eq!(
        p.auth_command.as_deref(),
        Some("gcloud auth application-default print-access-token")
    );
    assert_eq!(p.token_ttl_secs, Some(900));
    // No key_ref needed: the token comes from the command, not a static secret.
    assert_eq!(p.key.expose(), "");
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

/// The checked-in `pingogate-core.example.yaml` must always parse with the
/// current schema (prevents doc/schema drift). Semantic validation is skipped
/// here (it requires resolving env/plain secrets), but the schema-level parse
/// is enforced.
#[test]
fn example_config_parses_with_current_schema() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(|p| p.parent()) // core-rs/snapshot -> core-rs -> repo root
        .unwrap()
        .join("pingogate-core.example.yaml");
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

/// Proxy fields resolve through the model > provider > global override chain:
/// a model-level value beats provider-level, which beats the global default.
#[test]
fn proxy_override_chain_model_wins_over_provider_wins_over_global() {
    std::env::set_var("PINGO_TEST_PROXY_KEY", "sk-test");
    let yaml = r#"
listeners:
  public: { address: "0.0.0.0:8080" }
  admin: { address: "127.0.0.1:9090" }
gateway_keys:
  - { name: "k", secret_ref: "plain:pg" }
upstream:
  timeout_ms: 30000
  http_proxy: "http://global-http:3128"
  https_proxy: "http://global-https:3128"
providers:
  - name: "p"
    kind: "openai-compatible"
    base_url: "https://upstream.example"
    auth: { method: "bearer", key_ref: "plain:sk-test" }
    capability_families: ["generation.stateless"]
    http_proxy: "http://provider-http:3128"
    https_proxy: "http://provider-https:3128"
    models:
      # inherits provider-level https_proxy
      - alias: "inherits"
        upstream_model: "inherits"
      # overrides both levels
      - alias: "overrides"
        upstream_model: "overrides"
        http_proxy: "http://model-http:8080"
        https_proxy: "http://model-https:8080"
"#;
    let cfg = GatewayConfig::from_yaml(yaml).unwrap();
    let snap = RuntimeSnapshot::build(&cfg, &EnvResolver, 1).unwrap();

    let inherited = snap.route("inherits").unwrap();
    assert_eq!(inherited.http_proxy.as_deref(), Some("http://provider-http:3128"));
    assert_eq!(inherited.https_proxy.as_deref(), Some("http://provider-https:3128"));

    let overridden = snap.route("overrides").unwrap();
    assert_eq!(overridden.http_proxy.as_deref(), Some("http://model-http:8080"));
    assert_eq!(overridden.https_proxy.as_deref(), Some("http://model-https:8080"));
}

/// A model with no proxy config inherits the global `upstream` default when
/// neither the model nor its provider set one.
#[test]
fn proxy_inherits_global_when_no_provider_or_model_value() {
    std::env::set_var("PINGO_TEST_PROXY_KEY2", "sk-test");
    let yaml = r#"
listeners:
  public: { address: "0.0.0.0:8080" }
  admin: { address: "127.0.0.1:9090" }
gateway_keys:
  - { name: "k", secret_ref: "plain:pg" }
upstream:
  https_proxy: "http://global-https:3128"
providers:
  - name: "p"
    kind: "openai-compatible"
    base_url: "https://upstream.example"
    auth: { method: "bearer", key_ref: "plain:sk-test" }
    capability_families: ["generation.stateless"]
    models:
      - alias: "m"
        upstream_model: "m"
"#;
    let cfg = GatewayConfig::from_yaml(yaml).unwrap();
    let snap = RuntimeSnapshot::build(&cfg, &EnvResolver, 1).unwrap();

    let route = snap.route("m").unwrap();
    assert_eq!(route.https_proxy.as_deref(), Some("http://global-https:3128"));
    // No global http_proxy set -> resolves to None (no proxy for HTTP traffic).
    assert_eq!(route.http_proxy, None);
}
