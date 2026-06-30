//! Upstream target resolution (FR-016/FR-017; research R1/R4).
//!
//! Routing selects a provider and resolves its base URL into a connectable
//! peer target; the inbound path/body are forwarded unchanged this phase, with
//! the provider base path (if any) prepended. This module is pure so it is unit
//! testable without a live Pingora session.

use http::Uri;
use pingo_config::RuntimeSnapshot;
use pingo_core::{AppError, CapabilityFamily};

/// A provider base URL resolved to a connectable peer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UpstreamTarget {
    /// `host:port`, DNS-resolved by Pingora at connect time.
    pub addr: String,
    /// Whether to dial over TLS (verified upstream cert; constitution XX).
    pub tls: bool,
    /// TLS SNI and the `Host` header sent upstream.
    pub sni: String,
    /// Path prefix from the base URL (`""` or `/prefix`, no trailing slash),
    /// prepended to the forwarded inbound path.
    pub base_path: String,
}

/// Parse a provider `base_url` into an [`UpstreamTarget`].
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

/// Resolve a model alias to its provider name and connectable target, enforcing
/// the provider's declared capability gate (FR-006/FR-007).
pub fn resolve_route(
    snapshot: &RuntimeSnapshot,
    alias: &str,
) -> Result<(String, UpstreamTarget), AppError> {
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
    Ok((route.provider.clone(), target))
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
}
