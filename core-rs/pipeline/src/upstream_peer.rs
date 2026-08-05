//! Upstream target resolution (FR-016/FR-017; research R1/R4).
//!
//! Routing selects a provider and resolves its base URL into a connectable
//! peer target; the inbound path/body are forwarded unchanged this phase, with
//! the provider base path (if any) prepended. This module is pure so it is unit
//! testable without a live Pingora session.

use std::time::Duration;

use async_trait::async_trait;
use http::Uri;
use pingogate_core_types::{AppError, AuthMethod, CapabilityFamily, ProviderKind};
use pingogate_snapshot::RuntimeSnapshot;
use pingora::connectors::L4Connect;
use pingora::protocols::l4::stream::Stream as IoStream;
use pingora::{Error, ErrorType, Result};
use tokio::net::TcpStream;

/// A provider base URL resolved to a connectable peer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UpstreamTarget {
    pub addr: String,
    pub tls: bool,
    pub sni: String,
    pub base_path: String,
    /// Upstream port (from the base_url, defaulted by scheme).
    pub port: u16,
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
        port,
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
    /// Effective HTTP CONNECT proxy for this upstream (http or https, chosen
    /// by the target's scheme). `None` = direct connection.
    pub upstream_proxy: Option<ProxyTarget>,
}

/// A parsed HTTP CONNECT proxy address (`host:port`, optional `http://` prefix).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProxyTarget {
    pub host: String,
    pub port: u16,
}

/// Parse a proxy address from config (`http://host:port` or `host:port`).
pub fn parse_proxy(proxy: &str) -> Result<ProxyTarget, AppError> {
    let s = proxy.trim();
    let s = s
        .strip_prefix("http://")
        .or_else(|| s.strip_prefix("https://"))
        .unwrap_or(s);
    let (host, port) = s.rsplit_once(':').ok_or_else(|| AppError::Validation {
        message: format!("invalid proxy address (expected host:port): '{proxy}'"),
    })?;
    if host.is_empty() {
        return Err(AppError::Validation {
            message: format!("invalid proxy address (missing host): '{proxy}'"),
        });
    }
    let port: u16 = port
        .parse()
        .map_err(|_| AppError::Validation {
            message: format!("invalid proxy port in '{proxy}'"),
        })?;
    Ok(ProxyTarget {
        host: host.to_string(),
        port,
    })
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
    // Pick the proxy by the upstream's scheme: HTTPS traffic uses `https_proxy`,
    // plain HTTP uses `http_proxy` (curl semantics). Both already carry the
    // effective model ?? provider ?? global override.
    let proxy = if target.tls {
        route.https_proxy.as_deref()
    } else {
        route.http_proxy.as_deref()
    };
    let upstream_proxy = match proxy {
        Some(p) => Some(parse_proxy(p)?),
        None => None,
    };
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
        upstream_proxy,
    })
}

/// Expand an upstream_path template by substituting `{model}`.
pub fn expand_upstream_path(template: &str, upstream_model: &str) -> String {
    template.replace("{model}", upstream_model)
}

/// Custom L4 connector that establishes an HTTP CONNECT tunnel through a proxy.
///
/// Pingora's built-in `HttpPeer::new_proxy` only supports Unix-socket proxies
/// (and panics on Windows), so TCP CONNECT proxies are injected via
/// `PeerOptions.custom_l4` (the `L4Connect` trait). Pingora's l7 layer then
/// performs the TLS handshake to the upstream on the returned tunnel stream.
#[derive(Debug)]
pub struct ProxyL4Connector {
    pub proxy: ProxyTarget,
    /// Hostname used in the CONNECT line (the upstream's SNI).
    pub connect_host: String,
    pub connect_port: u16,
    pub timeout: Duration,
}

#[async_trait]
impl L4Connect for ProxyL4Connector {
    async fn connect(
        &self,
        _addr: &pingora::protocols::l4::socket::SocketAddr,
    ) -> Result<IoStream> {
        let fut = async {
            let tcp = TcpStream::connect((self.proxy.host.as_str(), self.proxy.port))
                .await
                .map_err(|e| {
                    Error::explain(
                        ErrorType::ConnectError,
                        format!(
                            "connect CONNECT proxy {}:{}: {e}",
                            self.proxy.host, self.proxy.port
                        ),
                    )
                })?;
            tcp.set_nodelay(true).map_err(|e| {
                Error::explain(ErrorType::ConnectError, format!("set_nodelay on proxy: {e}"))
            })?;

            let req = format!(
                "CONNECT {}:{} HTTP/1.1\r\nHost: {}:{}\r\n\r\n",
                self.connect_host, self.connect_port, self.connect_host, self.connect_port
            );
            tcp.writable().await.map_err(|e| {
                Error::explain(ErrorType::ConnectError, format!("proxy writable: {e}"))
            })?;
            tcp.try_write(req.as_bytes()).map_err(|e| {
                Error::explain(ErrorType::ConnectError, format!("write CONNECT: {e}"))
            })?;

            // Read the response header (ends at CRLFCRLF); require a 2xx status.
            let mut buf = [0u8; 4096];
            let mut header: Vec<u8> = Vec::with_capacity(1024);
            loop {
                if header.len() > 16 * 1024 {
                    return Err(Error::explain(
                        ErrorType::ConnectError,
                        "proxy CONNECT response header too large",
                    ));
                }
                tcp.readable().await.map_err(|e| {
                    Error::explain(ErrorType::ConnectError, format!("proxy readable: {e}"))
                })?;
                match tcp.try_read(&mut buf) {
                    Ok(0) => {
                        return Err(Error::explain(
                            ErrorType::ConnectError,
                            "proxy closed connection during CONNECT",
                        ))
                    }
                    Ok(n) => {
                        header.extend_from_slice(&buf[..n]);
                        if let Some(end) = find_header_end(&header) {
                            let status = parse_status(&header[..end]);
                            if !(200..300).contains(&status) {
                                return Err(Error::explain(
                                    ErrorType::ConnectError,
                                    format!("CONNECT proxy rejected: HTTP {status}"),
                                ));
                            }
                            break;
                        }
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => continue,
                    Err(e) => {
                        return Err(Error::explain(
                            ErrorType::ConnectError,
                            format!("proxy read error: {e}"),
                        ))
                    }
                }
            }

            Ok(IoStream::from(tcp))
        };
        match tokio::time::timeout(self.timeout, fut).await {
            Ok(inner) => inner,
            Err(_) => Err(Error::explain(
                ErrorType::ConnectTimedout,
                format!(
                    "CONNECT proxy {}:{} timed out",
                    self.proxy.host, self.proxy.port
                ),
            )),
        }
    }
}

/// Index just past the end of the HTTP header block (after `\r\n\r\n`).
fn find_header_end(buf: &[u8]) -> Option<usize> {
    buf.windows(4).position(|w| w == b"\r\n\r\n").map(|p| p + 4)
}

/// Parse the 3-digit status code from a response head, e.g. `HTTP/1.1 200`.
fn parse_status(head: &[u8]) -> u16 {
    let s = String::from_utf8_lossy(head);
    let mut parts = s.split_whitespace();
    let _version = parts.next();
    parts.next().and_then(|c| c.parse().ok()).unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

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

    #[test]
    fn parse_proxy_accepts_prefixed_and_bare_addresses() {
        let p = parse_proxy("http://127.0.0.1:7890").unwrap();
        assert_eq!(p.host, "127.0.0.1");
        assert_eq!(p.port, 7890);
        let p = parse_proxy("proxy.example:3128").unwrap();
        assert_eq!(p.host, "proxy.example");
        assert_eq!(p.port, 3128);
        // https:// prefix on the proxy address is tolerated (it still speaks TCP).
        let p = parse_proxy("https://p:8443").unwrap();
        assert_eq!(p.host, "p");
        assert_eq!(p.port, 8443);
    }

    #[test]
    fn parse_proxy_rejects_malformed_addresses() {
        assert!(parse_proxy("").is_err());
        assert!(parse_proxy("no-port").is_err());
        assert!(parse_proxy("host:notaport").is_err());
        assert!(parse_proxy("http://:7890").is_err());
    }

    /// A test resolver that accepts any reference verbatim.
    struct TestResolver;
    impl pingogate_core_types::SecretResolver for TestResolver {
        fn resolve(
            &self,
            reference: &str,
        ) -> Result<pingogate_core_types::SecretString, pingogate_core_types::SecretError>
        {
            Ok(pingogate_core_types::SecretString::new(
                reference.trim().to_string(),
            ))
        }
    }

    fn snapshot_with_global_proxies() -> Arc<RuntimeSnapshot> {
        let yaml = r#"
listeners:
  public: { address: "0.0.0.0:8080" }
  admin: { address: "127.0.0.1:9090" }
gateway_keys:
  - { name: "k", secret_ref: "plain:pg" }
upstream:
  http_proxy: "http://global-http:3128"
  https_proxy: "http://global-https:3128"
providers:
  - name: "https-p"
    kind: "openai-compatible"
    base_url: "https://api.example.com"
    auth: { method: "bearer", key_ref: "plain:sk" }
    capability_families: ["generation.stateless"]
    models:
      - alias: "https-m"
        upstream_model: "https-m"
  - name: "http-p"
    kind: "openai-compatible"
    base_url: "http://127.0.0.1:9000"
    auth: { method: "bearer", key_ref: "plain:sk" }
    capability_families: ["generation.stateless"]
    models:
      - alias: "http-m"
        upstream_model: "http-m"
"#;
        let cfg = pingogate_snapshot::GatewayConfig::from_yaml(yaml).unwrap();
        RuntimeSnapshot::build(&cfg, &TestResolver, 1).unwrap()
    }

    #[test]
    fn resolve_route_picks_proxy_by_upstream_scheme() {
        let snap = snapshot_with_global_proxies();
        // HTTPS upstream -> https_proxy.
        let https = resolve_route(&snap, "https-m").unwrap();
        assert_eq!(https.upstream_proxy.unwrap().host, "global-https");
        // Plain-HTTP upstream -> http_proxy.
        let http = resolve_route(&snap, "http-m").unwrap();
        assert_eq!(http.upstream_proxy.unwrap().host, "global-http");
    }

    /// End-to-end CONNECT tunnel through a mock proxy: the connector dials the
    /// proxy, sends `CONNECT <upstream>`, and returns a stream whose subsequent
    /// bytes flow through the tunnel.
    #[tokio::test]
    async fn proxy_connector_establishes_connect_tunnel() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let proxy_addr = listener.local_addr().unwrap();

        let proxy_task = tokio::spawn(async move {
            let (mut sock, _) = listener.accept().await.unwrap();
            let mut head = Vec::new();
            let mut buf = [0u8; 1024];
            loop {
                let n = sock.read(&mut buf).await.unwrap();
                head.extend_from_slice(&buf[..n]);
                if head.windows(4).any(|w| w == b"\r\n\r\n") {
                    break;
                }
            }
            let head_str = String::from_utf8_lossy(&head);
            assert!(
                head_str.starts_with("CONNECT example.com:443 "),
                "unexpected CONNECT line: {head_str}"
            );
            sock.write_all(b"HTTP/1.1 200 Connection established\r\n\r\n")
                .await
                .unwrap();
            // Keep the connection alive briefly so the client-side read cannot
            // race the socket close.
            tokio::time::sleep(Duration::from_millis(100)).await;
            sock.write_all(b"tunnel-ready").await.unwrap();
            tokio::time::sleep(Duration::from_millis(500)).await;
        });

        let connector = ProxyL4Connector {
            proxy: ProxyTarget {
                host: "127.0.0.1".to_string(),
                port: proxy_addr.port(),
            },
            connect_host: "example.com".to_string(),
            connect_port: 443,
            timeout: Duration::from_secs(5),
        };
        let addr = pingora::protocols::l4::socket::SocketAddr::from(
            "127.0.0.1:443".parse::<std::net::SocketAddr>().unwrap(),
        );
        let mut stream = connector
            .connect(&addr)
            .await
            .expect("CONNECT tunnel should be established");
        let mut out = [0u8; 12];
        let n = tokio::time::timeout(Duration::from_secs(5), stream.read(&mut out))
            .await
            .expect("read from tunnel should complete")
            .expect("read from tunnel should not error");
        assert_eq!(&out[..n], b"tunnel-ready");
        proxy_task.await.unwrap();
    }
}
