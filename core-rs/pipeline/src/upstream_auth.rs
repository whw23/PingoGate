//! Upstream credential injection plan (FR-010/FR-011/FR-012; research R5).
//!
//! The pipeline strips the client's gateway key and injects the provider's own
//! credential according to its declared auth method. The resolved key lives on
//! the snapshot's `ResolvedProvider`; adapters never see plaintext.
//!
//! Standalone mode ([`UpstreamAuth::build`]) reads env-resolved plaintext from
//! `ResolvedProvider::key`. Platform mode ([`UpstreamAuth::build_platform`])
//! decrypts `ResolvedProvider::encrypted_key` via the [`KeyVault`] per request
//! (constitution XX: "明文仅在 `KeyVault::decrypt()` 返回的 `SecretString` 中
//! 存活"); the returned [`SecretString`] is dropped at the end of
//! `upstream_request_filter`, clearing plaintext from memory.

use std::sync::Arc;

use pingogate_core_types::{AuthMethod, SecretString};
use pingogate_provider::TokenSource;
use pingogate_snapshot::ResolvedProvider;
use pingogate_storage::{KeyError, KeyVault};
use percent_encoding::{utf8_percent_encode, NON_ALPHANUMERIC};
use pingora::{Error, ErrorType, Result};

/// Inbound header names that carry a gateway key; stripped before forwarding.
pub const GATEWAY_KEY_HEADERS: [&str; 2] = ["authorization", "x-api-key"];

/// A credential mutation to apply to the upstream request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UpstreamAuth {
    /// `Authorization: <value>`, where value is the full `Bearer <key>`.
    BearerHeader(String),
    /// `x-api-key: <key>` plus optional `anthropic-version: <version>`.
    ApiKeyHeader {
        key: String,
        version: Option<String>,
    },
    /// URL query parameter `key=<key>` appended to the upstream path.
    QueryKey(String),
}

impl UpstreamAuth {
    /// Build the injection plan from a provider's auth method and resolved key.
    /// `TokenCommand` is handled before this (the token comes from an external
    /// source, injected as a Bearer header), so it is unreachable here.
    pub fn build(method: AuthMethod, key: &str, anthropic_version: Option<&str>) -> Self {
        match method {
            AuthMethod::Bearer => Self::BearerHeader(format!("Bearer {key}")),
            AuthMethod::ApiKeyHeader => Self::ApiKeyHeader {
                key: key.to_string(),
                version: anthropic_version.map(|v| v.to_string()),
            },
            AuthMethod::QueryKey => Self::QueryKey(key.to_string()),
            AuthMethod::TokenCommand => {
                unreachable!("TokenCommand is resolved before UpstreamAuth::build")
            }
        }
    }

    /// Platform-mode build: decrypt `encrypted_key` via the [`KeyVault`] once,
    /// then inject the plaintext per the auth method. The plaintext lives only
    /// in the returned [`SecretString`]; the caller must drop it at the end of
    /// the request phase (constitution XX: "明文仅在 `KeyVault::decrypt()`
    /// 返回的 `SecretString` 中存活").
    ///
    /// Per S1 research: AES-GCM decrypt is ~0.2µs, negligible vs the 5 ms p50
    /// budget. No caching: each request decrypts fresh (constitution XX:
    /// "不缓存、不常驻").
    pub fn build_platform(
        method: AuthMethod,
        encrypted_key: &[u8],
        keyvault: &dyn KeyVault,
        anthropic_version: Option<&str>,
    ) -> Result<Self, KeyError> {
        // Per-request decrypt; plaintext lives only in `plaintext` which drops
        // at the end of this function. The returned UpstreamAuth carries the
        // plaintext as an owned String (injected into the upstream header); it
        // must be dropped by the caller at request end.
        let plaintext: SecretString = keyvault.decrypt(encrypted_key)?;
        let key = plaintext.expose();
        Ok(Self::build(method, key, anthropic_version))
    }
}

/// Build the upstream auth plan, branching on mode (constitution XII/XX).
/// Platform: per-request decrypt of `encrypted_key`. Standalone: env-resolved
/// plaintext from `provider.key`. `token_source` is required when the auth
/// method is `TokenCommand` (external command token).
pub async fn build_upstream_auth(
    keyvault: &Option<Arc<dyn KeyVault>>,
    provider: &ResolvedProvider,
    token_source: Option<&dyn TokenSource>,
) -> Result<UpstreamAuth> {
    build_upstream_auth_with_method(
        keyvault,
        provider,
        provider.auth_method,
        provider.anthropic_version.as_deref(),
        token_source,
    )
    .await
}

/// Build the upstream auth plan with explicit auth method + anthropic_version
/// (issue 1: per-route auth override + model > provider hierarchy). The key
/// material comes from the provider (env-resolved in standalone, KeyVault-
/// decrypted in platform); the auth *method* and *anthropic_version* can be
/// overridden per route so a single provider can serve multiple protocols with
/// different auth styles.
pub async fn build_upstream_auth_with_method(
    keyvault: &Option<Arc<dyn KeyVault>>,
    provider: &ResolvedProvider,
    method: AuthMethod,
    anthropic_version: Option<&str>,
    token_source: Option<&dyn TokenSource>,
) -> Result<UpstreamAuth> {
    // TokenCommand: the access token comes from an external source (standalone:
    // the Rust kernel runs `auth.command`; platform: the Go control plane
    // injects it), then injected as a Bearer header.
    if method == AuthMethod::TokenCommand {
        let source = token_source.ok_or_else(|| {
            Error::explain(ErrorType::InternalError, "TokenCommand auth requires a token source")
        })?;
        let token = source.access_token().await.map_err(|e| {
            Error::explain(ErrorType::InternalError, format!("token fetch failed: {e}"))
        })?;
        return Ok(UpstreamAuth::BearerHeader(format!("Bearer {token}")));
    }
    match keyvault {
        Some(kv) => {
            let encrypted = provider.encrypted_key.as_ref().ok_or_else(|| {
                Error::explain(ErrorType::InternalError, "platform-mode provider missing encrypted_key")
            })?;
            let plaintext: SecretString = kv
                .decrypt(encrypted)
                .map_err(|e| Error::explain(ErrorType::InternalError, format!("keyvault decrypt failed: {e}")))?;
            Ok(UpstreamAuth::build(method, plaintext.expose(), anthropic_version))
        }
        None => Ok(UpstreamAuth::build(
            method,
            provider.key.expose(),
            anthropic_version,
        )),
    }
}

/// Append `key=<value>` to an origin-form path-and-query string (Gemini).
///
/// The key is percent-encoded with [`NON_ALPHANUMERIC`] so special characters
/// (`&`, `=`, `#`, space, ...) cannot break the query or inject extra params.
pub fn append_query_key(path_and_query: &str, key: &str) -> String {
    let enc = utf8_percent_encode(key, NON_ALPHANUMERIC).to_string();
    if path_and_query.contains('?') {
        format!("{path_and_query}&key={enc}")
    } else {
        format!("{path_and_query}?key={enc}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    #[test]
    fn bearer_wraps_key() {
        assert_eq!(
            UpstreamAuth::build(AuthMethod::Bearer, "sk-1", None),
            UpstreamAuth::BearerHeader("Bearer sk-1".to_string())
        );
    }

    #[test]
    fn api_key_header_carries_version() {
        assert_eq!(
            UpstreamAuth::build(AuthMethod::ApiKeyHeader, "sk-2", Some("2023-06-01")),
            UpstreamAuth::ApiKeyHeader {
                key: "sk-2".to_string(),
                version: Some("2023-06-01".to_string())
            }
        );
    }

    #[test]
    fn query_key_appends_with_correct_separator() {
        assert_eq!(
            append_query_key("/v1beta/x:generateContent", "k"),
            "/v1beta/x:generateContent?key=k"
        );
        assert_eq!(
            append_query_key("/v1beta/x:generateContent?alt=sse", "k"),
            "/v1beta/x:generateContent?alt=sse&key=k"
        );
    }

    #[test]
    fn query_key_percent_encodes_special_chars() {
        // `&` and `=` would otherwise inject/break query params; `#` would
        // start a fragment; space is illegal in a query value.
        assert_eq!(
            append_query_key("/v1beta/x:generateContent", "a&b=c#d e"),
            "/v1beta/x:generateContent?key=a%26b%3Dc%23d%20e"
        );
        // Pure ASCII alphanum is left untouched (regression guard).
        assert_eq!(
            append_query_key("/v1beta/x:generateContent?alt=sse", "sk123"),
            "/v1beta/x:generateContent?alt=sse&key=sk123"
        );
    }

    /// Mock KeyVault that counts `decrypt` calls and returns a fixed plaintext.
    /// Verifies the platform hot path calls `decrypt` exactly once per request
    /// and that the plaintext is correctly injected into the auth plan.
    struct CountingKeyVault {
        calls: Arc<AtomicUsize>,
        plaintext: &'static str,
    }

    impl KeyVault for CountingKeyVault {
        fn encrypt(&self, _plaintext: &[u8]) -> Result<Vec<u8>, KeyError> {
            Err(KeyError::EncryptFailed)
        }
        fn decrypt(&self, _ciphertext: &[u8]) -> Result<SecretString, KeyError> {
            self.calls.fetch_add(1, Ordering::Relaxed);
            Ok(SecretString::new(self.plaintext))
        }
    }

    #[test]
    fn build_platform_decrypts_per_request_bearer() {
        let calls = Arc::new(AtomicUsize::new(0));
        let kv = CountingKeyVault {
            calls: calls.clone(),
            plaintext: "sk-openai-real",
        };
        let ct = b"ciphertext-bytes";
        let auth = UpstreamAuth::build_platform(AuthMethod::Bearer, ct, &kv, None)
            .expect("decrypt must succeed");
        assert_eq!(auth, UpstreamAuth::BearerHeader("Bearer sk-openai-real".to_string()));
        // Exactly one decrypt per request (no cache; constitution XX).
        assert_eq!(calls.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn build_platform_decrypts_per_request_api_key_with_version() {
        let calls = Arc::new(AtomicUsize::new(0));
        let kv = CountingKeyVault {
            calls: calls.clone(),
            plaintext: "sk-anthropic",
        };
        let auth = UpstreamAuth::build_platform(
            AuthMethod::ApiKeyHeader,
            b"ct",
            &kv,
            Some("2023-06-01"),
        )
        .expect("decrypt must succeed");
        assert_eq!(
            auth,
            UpstreamAuth::ApiKeyHeader {
                key: "sk-anthropic".to_string(),
                version: Some("2023-06-01".to_string())
            }
        );
        assert_eq!(calls.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn build_platform_decrypts_per_request_query_key() {
        let calls = Arc::new(AtomicUsize::new(0));
        let kv = CountingKeyVault {
            calls: calls.clone(),
            plaintext: "sk-gemini",
        };
        let auth = UpstreamAuth::build_platform(AuthMethod::QueryKey, b"ct", &kv, None)
            .expect("decrypt must succeed");
        assert_eq!(auth, UpstreamAuth::QueryKey("sk-gemini".to_string()));
        assert_eq!(calls.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn build_platform_propagates_decrypt_error() {
        struct FailingKeyVault;
        impl KeyVault for FailingKeyVault {
            fn encrypt(&self, _: &[u8]) -> Result<Vec<u8>, KeyError> {
                Err(KeyError::EncryptFailed)
            }
            fn decrypt(&self, _: &[u8]) -> Result<SecretString, KeyError> {
                Err(KeyError::DecryptFailed)
            }
        }
        let err = UpstreamAuth::build_platform(
            AuthMethod::Bearer,
            b"bad-ct",
            &FailingKeyVault,
            None,
        )
        .unwrap_err();
        assert!(matches!(err, KeyError::DecryptFailed));
    }

    #[test]
    fn build_platform_decrypts_twice_for_two_requests_no_cache() {
        // Verifies no caching: two back-to-back builds each call decrypt.
        let calls = Arc::new(AtomicUsize::new(0));
        let kv = CountingKeyVault {
            calls: calls.clone(),
            plaintext: "sk-x",
        };
        let _a = UpstreamAuth::build_platform(AuthMethod::Bearer, b"ct", &kv, None).unwrap();
        let _b = UpstreamAuth::build_platform(AuthMethod::Bearer, b"ct", &kv, None).unwrap();
        assert_eq!(calls.load(Ordering::Relaxed), 2);
    }

    #[tokio::test]
    async fn token_command_injects_bearer_from_source() {
        use async_trait::async_trait;
        use pingogate_core_types::ProviderKind;
        use pingogate_provider::TokenError;

        struct FixedToken(&'static str);
        #[async_trait]
        impl TokenSource for FixedToken {
            async fn access_token(&self) -> Result<String, TokenError> {
                Ok(self.0.to_string())
            }
        }

        let provider = ResolvedProvider {
            name: "vertex-ai".into(),
            kind: ProviderKind::Gemini,
            base_url: "https://aiplatform.googleapis.com".into(),
            auth_method: AuthMethod::TokenCommand,
            key: SecretString::new(String::new()),
            encrypted_key: None,
            anthropic_version: None,
            capability_families: vec![],
            timeout_ms: None,
            upstream_path: None,
            http_proxy: None,
            https_proxy: None,
            auth_command: Some("gcloud auth application-default print-access-token".into()),
            token_ttl_secs: None,
        };
        let src = FixedToken("ya29-test-token");
        let auth = build_upstream_auth_with_method(
            &None,
            &provider,
            AuthMethod::TokenCommand,
            None,
            Some(&src as &dyn TokenSource),
        )
        .await
        .unwrap();
        assert_eq!(
            auth,
            UpstreamAuth::BearerHeader("Bearer ya29-test-token".into())
        );
    }
}
