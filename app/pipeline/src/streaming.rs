//! Streaming (SSE) detection for the observability marker (research R2).
//!
//! Passthrough itself needs no buffering — Pingora forwards body chunks as they
//! arrive, so a streamed response is relayed token-by-token (SC-006). This
//! module only *records* whether a request/response is a stream, so the log line
//! can carry a `streaming` flag without inspecting bodies elsewhere.

/// Whether the client asked for a streamed response. Gemini proves it from the
/// path (`:streamGenerateContent`); OpenAI/Anthropic carry `"stream": true` in
/// the JSON body.
pub fn request_is_streaming(streaming_by_path: bool, body: &[u8]) -> bool {
    if streaming_by_path {
        return true;
    }
    serde_json::from_slice::<serde_json::Value>(body)
        .ok()
        .and_then(|v| v.get("stream").and_then(|s| s.as_bool()))
        .unwrap_or(false)
}

/// Whether the upstream response is a stream, by its `content-type`.
pub fn response_is_streaming(content_type: Option<&str>) -> bool {
    content_type
        .map(|c| c.contains("text/event-stream"))
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn path_streaming_short_circuits_body() {
        assert!(request_is_streaming(true, b""));
    }

    #[test]
    fn body_stream_flag_is_read() {
        assert!(request_is_streaming(false, br#"{"stream":true}"#));
        assert!(!request_is_streaming(false, br#"{"stream":false}"#));
        assert!(!request_is_streaming(false, br#"{"messages":[]}"#));
        assert!(!request_is_streaming(false, b"not json"));
    }

    #[test]
    fn response_streaming_reads_content_type() {
        assert!(response_is_streaming(Some(
            "text/event-stream; charset=utf-8"
        )));
        assert!(!response_is_streaming(Some("application/json")));
        assert!(!response_is_streaming(None));
    }
}
