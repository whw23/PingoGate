//! Sensitive-value redaction for logs and metrics (constitution XX; FR-034).
//!
//! Gateway keys, provider keys, and tokens MUST NOT appear in plaintext in any
//! observability output. [`SecretString`](crate::SecretString) already guards
//! values held in the snapshot; this module redacts the *other* place a secret
//! can leak - request headers and query strings echoed into a log line - by
//! collapsing credential material to [`REDACTED`] while keeping the surrounding,
//! non-secret structure (header name, auth scheme, other query params) readable.

/// Placeholder substituted for any redacted secret.
pub const REDACTED: &str = "***";

/// Header names whose values carry credentials and must never be logged raw.
const SENSITIVE_HEADERS: &[&str] = &["authorization", "x-api-key", "x-goog-api-key", "api-key"];

/// Whether a header name carries credential material (case-insensitive).
pub fn is_sensitive_header(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    SENSITIVE_HEADERS.iter().any(|h| *h == lower)
}

/// Redact a header value for safe logging. A non-sensitive header is returned
/// unchanged; `Authorization: Bearer <token>` keeps only its `Bearer` scheme;
/// any other credential header collapses entirely to [`REDACTED`].
pub fn redact_header_value(name: &str, value: &str) -> String {
    if !is_sensitive_header(name) {
        return value.to_string();
    }
    match value.split_once(' ') {
        Some((scheme, _)) if scheme.eq_ignore_ascii_case("bearer") => format!("Bearer {REDACTED}"),
        _ => REDACTED.to_string(),
    }
}

/// Redact a bare secret/token to the placeholder. Takes the value only so call
/// sites read intentionally (the value is deliberately discarded).
pub fn redact_secret(_secret: &str) -> &'static str {
    REDACTED
}

/// Replace the value of a `key=<secret>` query parameter (Gemini query-key auth)
/// with the placeholder, leaving the path and any other parameters intact.
pub fn redact_query_key(path: &str) -> String {
    let Some((base, query)) = path.split_once('?') else {
        return path.to_string();
    };
    let redacted = query
        .split('&')
        .map(|param| match param.split_once('=') {
            Some((k, _)) if k.eq_ignore_ascii_case("key") => format!("{k}={REDACTED}"),
            _ => param.to_string(),
        })
        .collect::<Vec<_>>()
        .join("&");
    format!("{base}?{redacted}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bearer_header_keeps_scheme_but_hides_token() {
        let out = redact_header_value("Authorization", "Bearer sk-super-secret");
        assert_eq!(out, "Bearer ***");
        assert!(!out.contains("sk-super-secret"));
    }

    #[test]
    fn api_key_header_is_fully_redacted() {
        assert_eq!(redact_header_value("x-api-key", "sk-secret"), "***");
        assert_eq!(redact_header_value("X-Goog-Api-Key", "AIza-secret"), "***");
    }

    #[test]
    fn non_sensitive_header_is_unchanged() {
        assert_eq!(
            redact_header_value("content-type", "application/json"),
            "application/json"
        );
    }

    #[test]
    fn sensitive_header_detection_is_case_insensitive() {
        assert!(is_sensitive_header("AUTHORIZATION"));
        assert!(is_sensitive_header("x-api-key"));
        assert!(!is_sensitive_header("user-agent"));
    }

    #[test]
    fn query_key_value_is_redacted_in_place() {
        let out = redact_query_key("/v1beta/models/m:generateContent?key=AIzaSecret&alt=sse");
        assert_eq!(out, "/v1beta/models/m:generateContent?key=***&alt=sse");
        assert!(!out.contains("AIzaSecret"));
    }

    #[test]
    fn query_key_passthrough_without_query() {
        assert_eq!(
            redact_query_key("/v1/chat/completions"),
            "/v1/chat/completions"
        );
    }

    #[test]
    fn redact_secret_discards_value() {
        assert_eq!(redact_secret("anything"), "***");
    }
}
