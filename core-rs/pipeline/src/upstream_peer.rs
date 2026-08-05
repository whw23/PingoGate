//! Upstream target resolution (FR-016/FR-017; research R1/R4).
//!
//! Routing selects a provider and resolves its base URL into a connectable
//! peer target; the inbound path/body are forwarded unchanged this phase, with
//! the provider base path (if any) prepended. This module is pure so it is unit
//! testable without a live Pingora session.

use http::Uri;
use pingogate_core_types::{AppError, AuthMethod, CapabilityFamily, ProviderKind};
use pingogate_snapshot::RuntimeSnapshot;

/// A provider base URL resolved to a connectable peer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UpstreamTarget {
    pub addr: String,
    pub tls: bool,
    pub sni: String,
    pub base_path: String,
}

pub fn parse_base_url(base_url: &str) -> Result<UpstreamTarget, AppError> {
    let uri: Uri = base_url.parse().map_err(|_| AppError::Internal {
        message: format!("invalid provider base_url: {base_url}"),
    })?;
    let tls = match uri.scheme_str() {
        Some("https") | None => true,
        Some("http") => false,
        Some(other) => {
            return Err(AppError::Internal {
                message: format!("unsupported scheme '{other}' in base_url: {base_url}"),
            })
        }
    };
    let host = uri
        .host()
        .ok_or_else(|| AppError::Internal {
            message: format!("base_url has no host: {base_url}"),
        })?
        .to_string();
    let port = uri.port_u16().unwrap_or(if tls { 443 } else { 80 });
    let base_path = uri.path().trim_end_matches('/').to_string();
    Ok(UpstreamTarget {
        addr: format!("{host}:{port}"),
        tls,
        sni: host,
        base_path,
    })
}

/// Full routing resolution with effective params (model > provider > global).
#[derive(Debug)]
pub struct RouteResolution {
    pub provider: String,
    pub upstream_model: String,
    pub target: UpstreamTarget,
    pub timeout_ms: u64,
    /// Effective upstream path template (model ?? provider). `None` = forward
    /// inbound path.
    pub upstream_path: Option<String>,
    /// Effective auth method (model ?? provider).
    pub auth_override: Option<AuthMethod>,
    /// Effective upstream kind (model ?? provider). For future protocol
    /// conversion (constitution VI).
    pub kind: ProviderKind,
    /// Effective anthropic_version (model ?? provider).
    pub anthropic_version: Option<String>,
}

/// Resolve a model alias to its provider, upstream model, connectable target,
/// and all effective parameters.
pub fn resolve_route(
    snapshot: &RuntimeSnapshot,
    alias: &str,
) -> Result<RouteResolution, AppError> {
    let route = snapshot.route(alias).ok_or_else(|| AppError::NoRoute {
        alias: alias.to_string(),
    })?;
    let provider = snapshot
        .provider(&route.provider)
        .ok_or_else(|| AppError::Internal {
            message: format!(
                "route '{alias}' references unknown provider '{}'",
                route.provider
            ),
        })?;
    if !provider
        .capability_families
        .iter()
        .any(|f| matches!(f, CapabilityFamily::GenerationStateless))
    {
        return Err(AppError::UnsupportedCapability {
            family: "generation.stateless".to_string(),
        });
    }
    let target = parse_base_url(&provider.base_url)?;
    let timeout_ms = route
        .timeout_ms
        .unwrap_or(snapshot.upstream.timeout_ms);
    Ok(RouteResolution {
        provider: route.provider.clone(),
        upstream_model: route.upstream_model.clone(),
        target,
        timeout_ms,
        upstream_path: route.upstream_path.clone(),
        // The route already has effective auth_method; we pass it as an override
        // over the provider's default (which is the same value after resolution,
        // but this keeps the proxy logic simple: always use the route's auth).
        auth_override: Some(route.auth_method),
        kind: route.kind,
        anthropic_version: route.anthropic_version.clone(),
    })
}

/// Expand an upstream_path template by substituting `{model}`.
pub fn expand_upstream_path(template: &str, upstream_model: &str) -> String {
    template.replace("{model}", upstream_model)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn https_origin_defaults_to_443_and_no_prefix() {
        let t = parse_base_url("https://api.openai.com").unwrap();
        assert_eq!(t.addr, "api.openai.com:443");
        assert!(t.tls);
        assert_eq!(t.sni, "api.openai.com");
        assert_eq!(t.base_path, "");
    }

    #[test]
    fn http_with_port_and_path_prefix() {
        let t = parse_base_url("http://127.0.0.1:8080/openai/").unwrap();
        assert_eq!(t.addr, "127.0.0.1:8080");
        assert!(!t.tls);
        assert_eq!(t.base_path, "/openai");
    }

    #[test]
    fn rejects_non_http_scheme() {
        assert!(parse_base_url("ftp://example.com").is_err());
        assert!(parse_base_url("not a url").is_err());
    }

    #[test]
    fn expand_upstream_path_substitutes_model() {
        assert_eq!(
            expand_upstream_path("/v1/chat/completions", "gpt-4o"),
            "/v1/chat/completions"
        );
        assert_eq!(
            expand_upstream_path("/api/v2/models/{model}/generate", "my-custom-model"),
            "/api/v2/models/my-custom-model/generate"
        );
    }
}
