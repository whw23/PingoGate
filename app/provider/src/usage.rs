//! Upstream token-usage extraction (FR-032; constitution XIX/XX).
//!
//! Reads the `usage` block from a non-streaming upstream JSON response in the
//! shape native to each protocol. Only integer counts are read — never any
//! prompt or response content (constitution XX) — so this can run on the hot
//! path without persisting message bodies. A missing block, unparseable body, or
//! streaming response yields `None`; absent sub-fields default to zero/`None`.

use pingo_core::ProtocolKind;
use serde_json::Value;

/// Token counts surfaced by an upstream response. `input`/`output` are always
/// present (zero if the provider omitted them); the remaining fields appear only
/// when the provider reports them (FR-032 — reasoning / cache when available).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TokenUsage {
    pub input: u64,
    pub output: u64,
    pub reasoning: Option<u64>,
    pub cache_read: Option<u64>,
    pub cache_write: Option<u64>,
}

impl TokenUsage {
    /// Whether no countable usage was reported (all directions zero/absent).
    pub fn is_empty(&self) -> bool {
        self.input == 0
            && self.output == 0
            && self.reasoning.is_none()
            && self.cache_read.is_none()
            && self.cache_write.is_none()
    }
}

/// Parse token usage from a full upstream response body for `protocol`.
pub fn parse_usage(protocol: ProtocolKind, body: &[u8]) -> Option<TokenUsage> {
    let value: Value = serde_json::from_slice(body).ok()?;
    match protocol {
        ProtocolKind::OpenAiCompatible => parse_openai(&value),
        ProtocolKind::Anthropic => parse_anthropic(&value),
        ProtocolKind::Gemini => parse_gemini(&value),
    }
}

/// Read an unsigned integer field, treating absent/non-integer as missing.
fn u64_at(value: &Value, key: &str) -> Option<u64> {
    value.get(key).and_then(Value::as_u64)
}

fn parse_openai(value: &Value) -> Option<TokenUsage> {
    let usage = value.get("usage")?;
    Some(TokenUsage {
        input: u64_at(usage, "prompt_tokens").unwrap_or(0),
        output: u64_at(usage, "completion_tokens").unwrap_or(0),
        reasoning: usage
            .get("completion_tokens_details")
            .and_then(|d| u64_at(d, "reasoning_tokens")),
        cache_read: usage
            .get("prompt_tokens_details")
            .and_then(|d| u64_at(d, "cached_tokens")),
        cache_write: None,
    })
}

fn parse_anthropic(value: &Value) -> Option<TokenUsage> {
    let usage = value.get("usage")?;
    Some(TokenUsage {
        input: u64_at(usage, "input_tokens").unwrap_or(0),
        output: u64_at(usage, "output_tokens").unwrap_or(0),
        reasoning: None,
        cache_read: u64_at(usage, "cache_read_input_tokens"),
        cache_write: u64_at(usage, "cache_creation_input_tokens"),
    })
}

fn parse_gemini(value: &Value) -> Option<TokenUsage> {
    let usage = value.get("usageMetadata")?;
    Some(TokenUsage {
        input: u64_at(usage, "promptTokenCount").unwrap_or(0),
        output: u64_at(usage, "candidatesTokenCount").unwrap_or(0),
        reasoning: u64_at(usage, "thoughtsTokenCount"),
        cache_read: u64_at(usage, "cachedContentTokenCount"),
        cache_write: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn openai_usage_includes_reasoning_and_cache_details() {
        let body = br#"{"usage":{"prompt_tokens":11,"completion_tokens":22,
            "completion_tokens_details":{"reasoning_tokens":5},
            "prompt_tokens_details":{"cached_tokens":7}}}"#;
        let usage = parse_usage(ProtocolKind::OpenAiCompatible, body).unwrap();
        assert_eq!(usage.input, 11);
        assert_eq!(usage.output, 22);
        assert_eq!(usage.reasoning, Some(5));
        assert_eq!(usage.cache_read, Some(7));
        assert_eq!(usage.cache_write, None);
    }

    #[test]
    fn anthropic_usage_maps_cache_fields() {
        let body = br#"{"usage":{"input_tokens":3,"output_tokens":9,
            "cache_read_input_tokens":2,"cache_creation_input_tokens":4}}"#;
        let usage = parse_usage(ProtocolKind::Anthropic, body).unwrap();
        assert_eq!(usage.input, 3);
        assert_eq!(usage.output, 9);
        assert_eq!(usage.cache_read, Some(2));
        assert_eq!(usage.cache_write, Some(4));
    }

    #[test]
    fn gemini_usage_reads_metadata_block() {
        let body = br#"{"usageMetadata":{"promptTokenCount":12,"candidatesTokenCount":8,
            "thoughtsTokenCount":6,"cachedContentTokenCount":1}}"#;
        let usage = parse_usage(ProtocolKind::Gemini, body).unwrap();
        assert_eq!(usage.input, 12);
        assert_eq!(usage.output, 8);
        assert_eq!(usage.reasoning, Some(6));
        assert_eq!(usage.cache_read, Some(1));
    }

    #[test]
    fn missing_usage_block_yields_none() {
        assert!(parse_usage(ProtocolKind::OpenAiCompatible, br#"{"choices":[]}"#).is_none());
        assert!(parse_usage(ProtocolKind::Anthropic, b"not json").is_none());
    }

    #[test]
    fn empty_usage_is_reported_empty() {
        let usage = parse_usage(ProtocolKind::OpenAiCompatible, br#"{"usage":{}}"#).unwrap();
        assert!(usage.is_empty());
    }
}
