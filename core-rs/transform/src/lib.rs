//! Transform engine: stateless body rewriting, protocol bridging, declarative
//! TransformPlan compilation. This phase ships the minimal real transform --
//! injecting `stream_options.include_usage: true` into OpenAI Chat Completions
//! streaming requests so the provider returns usage in the terminal chunk
//! (constitution VI; research R2; consumed by `usage_extractor`).

use pingogate_core_types::ProtocolKind;

/// Inject `stream_options.include_usage: true` into an OpenAI Chat Completions
/// streaming request body. Returns the body unchanged when:
/// * the request is not streaming, or
/// * the protocol is not `OpenAiCompatible` (Anthropic/Gemini carry usage
///   natively in their stream delimiters), or
/// * the body is not valid JSON (partial chunk, malformed).
///
/// Idempotent: re-applying to an already-injected body is a no-op (sets the
/// flag to `true` again). The pipeline calls this from `request_body_filter`
/// with the full retry-buffered body (single chunk), so the JSON parse sees
/// the complete request.
pub fn inject_include_usage(body: &[u8], protocol: ProtocolKind, streaming: bool) -> Vec<u8> {
    if !streaming || protocol != ProtocolKind::OpenAiCompatible {
        return body.to_vec();
    }
    // Parse once; on any parse failure (partial chunk, malformed) return the
    // original bytes verbatim. The pipeline only calls this with the full
    // retry-buffered body, so a parse failure means a genuinely malformed
    // request -- let the upstream return its own error rather than a confusing
    // JSON-rewrite failure.
    let mut v: serde_json::Value = match serde_json::from_slice(body) {
        Ok(v) => v,
        Err(_) => return body.to_vec(),
    };
    // Set `stream_options.include_usage = true`, preserving any sibling fields
    // a client may already have set (e.g. `stream_options.include_usage` was
    // false -- override to true; other stream_options keys stay).
    if let Some(opts) = v
        .as_object_mut()
        .map(|o| o.entry("stream_options").or_insert(serde_json::Value::Object(Default::default())))
    {
        if let Some(opts_obj) = opts.as_object_mut() {
            opts_obj.insert("include_usage".to_string(), serde_json::Value::Bool(true));
        }
    }
    // Reserialize; on any failure (shouldn't happen for valid JSON) fall back
    // to the original bytes so the request still proceeds.
    serde_json::to_vec(&v).unwrap_or_else(|_| body.to_vec())
}

#[cfg(test)]
mod tests {
    use super::*;
    use pingogate_core_types::ProtocolKind;

    #[test]
    fn injects_include_usage_for_openai_chat_stream() {
        let body = br#"{"model":"gpt-4o","messages":[],"stream":true}"#;
        let transformed = inject_include_usage(body, ProtocolKind::OpenAiCompatible, true);
        let v: serde_json::Value = serde_json::from_slice(&transformed).unwrap();
        assert_eq!(v["stream_options"]["include_usage"], true);
        // Original fields preserved.
        assert_eq!(v["model"], "gpt-4o");
        assert_eq!(v["stream"], true);
    }

    #[test]
    fn no_inject_for_non_stream() {
        let body = br#"{"model":"gpt-4o","messages":[]}"#;
        let transformed = inject_include_usage(body, ProtocolKind::OpenAiCompatible, false);
        assert_eq!(transformed, body);
    }

    #[test]
    fn no_inject_for_non_openai() {
        // Anthropic carries usage in `message_delta` natively; no injection.
        let body = br#"{"model":"claude-3-5-sonnet","messages":[],"stream":true}"#;
        let transformed = inject_include_usage(body, ProtocolKind::Anthropic, true);
        assert_eq!(transformed, body);
        // Gemini generateContent likewise carries usageMetadata natively.
        let body2 = br#"{"contents":[],"stream":true}"#;
        let transformed2 = inject_include_usage(body2, ProtocolKind::Gemini, true);
        assert_eq!(transformed2, body2);
        // Gemini Interactions carries usage natively in the interaction object.
        let body3 = br#"{"model":"gemini-3.5-flash","input":"ping","stream":true}"#;
        let transformed3 = inject_include_usage(body3, ProtocolKind::GeminiInteractions, true);
        assert_eq!(transformed3, body3);
    }

    #[test]
    fn idempotent_when_already_injected() {
        let body = br#"{"model":"gpt-4o","stream":true,"stream_options":{"include_usage":true}}"#;
        let transformed = inject_include_usage(body, ProtocolKind::OpenAiCompatible, true);
        let v: serde_json::Value = serde_json::from_slice(&transformed).unwrap();
        assert_eq!(v["stream_options"]["include_usage"], true);
    }

    #[test]
    fn preserves_existing_stream_options() {
        // If the client already set other stream_options fields, preserve them.
        let body = br#"{"model":"gpt-4o","stream":true,"stream_options":{"include_usage":false}}"#;
        let transformed = inject_include_usage(body, ProtocolKind::OpenAiCompatible, true);
        let v: serde_json::Value = serde_json::from_slice(&transformed).unwrap();
        assert_eq!(v["stream_options"]["include_usage"], true); // overridden to true
    }

    #[test]
    fn no_inject_for_invalid_json() {
        // Partial / malformed body is returned verbatim (no panic).
        let body = br#"not json"#;
        let transformed = inject_include_usage(body, ProtocolKind::OpenAiCompatible, true);
        assert_eq!(transformed, body);
    }
}
