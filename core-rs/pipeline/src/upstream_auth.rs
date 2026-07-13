//! Upstream credential injection plan (FR-010/FR-011/FR-012; research R5).
//!
//! The pipeline strips the client's gateway key and injects the provider's own
//! credential according to its declared auth method. The resolved key lives on
//! the snapshot's `ResolvedProvider`; adapters never see plaintext.

use pingogate_core_types::AuthMethod;

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
    pub fn build(method: AuthMethod, key: &str, anthropic_version: Option<&str>) -> Self {
        match method {
            AuthMethod::Bearer => Self::BearerHeader(format!("Bearer {key}")),
            AuthMethod::ApiKeyHeader => Self::ApiKeyHeader {
                key: key.to_string(),
                version: anthropic_version.map(|v| v.to_string()),
            },
            AuthMethod::QueryKey => Self::QueryKey(key.to_string()),
        }
    }
}

/// Append `key=<value>` to an origin-form path-and-query string (Gemini).
pub fn append_query_key(path_and_query: &str, key: &str) -> String {
    if path_and_query.contains('?') {
        format!("{path_and_query}&key={key}")
    } else {
        format!("{path_and_query}?key={key}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
