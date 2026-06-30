//! Gateway-key extraction (FR-009/FR-013).
//!
//! This module only *locates* the key the client presented; matching it against
//! the snapshot (and mapping to a `Principal`) is done by the pipeline. The
//! gateway key is strictly isolated from upstream provider keys (FR-010).

/// Extract the gateway key the client presented: `Authorization: Bearer <k>`
/// (OpenAI/Gemini style) or `x-api-key: <k>` (Anthropic style). Returns `None`
/// when neither is present or the value is blank.
pub fn presented_key(authorization: Option<&str>, x_api_key: Option<&str>) -> Option<String> {
    if let Some(auth) = authorization {
        if let Some(rest) = auth.strip_prefix("Bearer ") {
            let k = rest.trim();
            if !k.is_empty() {
                return Some(k.to_string());
            }
        }
    }
    match x_api_key {
        Some(k) if !k.trim().is_empty() => Some(k.trim().to_string()),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_bearer_authorization() {
        assert_eq!(
            presented_key(Some("Bearer pg-abc"), None).as_deref(),
            Some("pg-abc")
        );
    }

    #[test]
    fn reads_x_api_key_when_no_bearer() {
        assert_eq!(
            presented_key(None, Some("pg-xyz")).as_deref(),
            Some("pg-xyz")
        );
    }

    #[test]
    fn blank_or_missing_yields_none() {
        assert_eq!(presented_key(None, None), None);
        assert_eq!(presented_key(Some("Bearer    "), None), None);
        assert_eq!(presented_key(Some("Basic abc"), None), None);
        assert_eq!(presented_key(None, Some("   ")), None);
    }
}
