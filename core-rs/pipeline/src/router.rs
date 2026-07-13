//! Model-alias extraction for routing (FR-016; research R3).
//!
//! The model name is read from the body (OpenAI/Anthropic `model`) or the path
//! (Gemini `{model}`). This phase does **not** rewrite the model: the resolved
//! route only selects the upstream provider; the body/path is forwarded
//! unchanged (research R1). Route lookup against the snapshot is done by the
//! pipeline via `RuntimeSnapshot::route`.

use pingogate_core_types::ProtocolKind;

/// Extract the model alias from an OpenAI/Anthropic JSON request body.
pub fn model_from_body(body: &[u8]) -> Option<String> {
    let value: serde_json::Value = serde_json::from_slice(body).ok()?;
    value.get("model")?.as_str().map(|s| s.to_string())
}

/// Extract the model alias from a Gemini path `/v1beta/models/{model}:action`.
pub fn model_from_gemini_path(path: &str) -> Option<String> {
    let path = path.split_once('?').map(|(p, _)| p).unwrap_or(path);
    let after = path.rsplit_once("/models/")?.1;
    let model = after.split(':').next().unwrap_or("");
    if model.is_empty() {
        None
    } else {
        Some(model.to_string())
    }
}

/// Extract the model alias appropriate for `protocol`.
///
/// OpenAI Chat Completions, OpenAI Responses, and Anthropic Messages all carry
/// the model in the JSON body; Gemini `generateContent`/`streamGenerateContent`
/// and Gemini Interactions both encode it in the path.
pub fn extract_model(protocol: ProtocolKind, path: &str, body: &[u8]) -> Option<String> {
    match protocol {
        ProtocolKind::OpenAiCompatible
        | ProtocolKind::OpenAiResponses
        | ProtocolKind::Anthropic => model_from_body(body),
        ProtocolKind::Gemini | ProtocolKind::GeminiInteractions => {
            model_from_gemini_path(path)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_model_from_json_body() {
        let body = br#"{"model":"gpt-4o","messages":[]}"#;
        assert_eq!(model_from_body(body).as_deref(), Some("gpt-4o"));
    }

    #[test]
    fn malformed_body_yields_none() {
        assert_eq!(model_from_body(b"not json"), None);
        assert_eq!(model_from_body(br#"{"no_model":true}"#), None);
    }

    #[test]
    fn reads_model_from_gemini_path() {
        let p = "/v1beta/models/gemini-1.5-pro:generateContent";
        assert_eq!(model_from_gemini_path(p).as_deref(), Some("gemini-1.5-pro"));
        let s = "/v1beta/models/gemini-1.5-flash:streamGenerateContent?alt=sse";
        assert_eq!(
            model_from_gemini_path(s).as_deref(),
            Some("gemini-1.5-flash")
        );
    }

    #[test]
    fn reads_model_from_gemini_interactions_path() {
        let p = "/v1beta/models/gemini-3.5-flash:interact";
        assert_eq!(
            model_from_gemini_path(p).as_deref(),
            Some("gemini-3.5-flash")
        );
    }

    #[test]
    fn extract_model_dispatches_by_protocol() {
        let body = br#"{"model":"claude-3-5-sonnet"}"#;
        assert_eq!(
            extract_model(ProtocolKind::Anthropic, "/v1/messages", body).as_deref(),
            Some("claude-3-5-sonnet")
        );
        assert_eq!(
            extract_model(
                ProtocolKind::Gemini,
                "/v1beta/models/g:generateContent",
                b""
            )
            .as_deref(),
            Some("g")
        );
        assert_eq!(
            extract_model(
                ProtocolKind::GeminiInteractions,
                "/v1beta/models/gi:interact",
                b""
            )
            .as_deref(),
            Some("gi")
        );
        assert_eq!(
            extract_model(
                ProtocolKind::OpenAiResponses,
                "/v1/responses",
                br#"{"model":"gpt-4o"}"#
            )
            .as_deref(),
            Some("gpt-4o")
        );
    }
}
