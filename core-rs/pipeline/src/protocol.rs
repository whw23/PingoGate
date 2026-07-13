//! Inbound protocol identification by request line.
//!
//! Detection matches path *suffixes* with a version-prefix wildcard, so
//! `{ver}/chat/completions`, `/v1/chat/completions`, `/v2/chat/completions`,
//! and a prefix-less `/chat/completions` all resolve to the same protocol
//! (research finding: LiteLLM double-registers `/v1beta/...` and `/...`; we
//! implement this as a suffix match so `/v1/`, `/v2/`, and no-prefix all work).
//! No `/v1` is hardcoded: the version segment is provider-specific and
//! variable (OpenAI/Anthropic `/v1`, Gemini `/v1beta`, subject to change).
//!
//! A recognized-but-unsupported capability surface (Realtime/Embeddings/Batch)
//! is reported distinctly from a wholly unidentified request, so the former can
//! mirror the provider's native error shape while the latter falls back to
//! PingoGate-native. The `protocol` field on `UnsupportedCapability` records
//! which provider namespace the surface belongs to, so error rendering can pick
//! the correct native shape (carry-over fix from T7: T7 dropped this field,
//! T8's error mirroring needs it).

use pingogate_core_types::ProtocolKind;

/// A request matched a supported provider-native generation endpoint.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Detected {
    pub protocol: ProtocolKind,
    /// True when the request line alone proves a streaming response (Gemini
    /// `:streamGenerateContent`). OpenAI/Anthropic streaming is read from the
    /// body `stream` flag separately.
    pub streaming_by_path: bool,
}

/// Outcome of identifying the inbound request against known provider surfaces.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Detection {
    /// A supported `generation.stateless`/`generation.stateful` endpoint;
    /// proceed to routing.
    Supported(Detected),
    /// A recognized provider surface but an unsupported capability family
    /// (Realtime/Embeddings/Batch…); short-circuit with an explicit error in
    /// `protocol`'s native shape.
    UnsupportedCapability {
        protocol: ProtocolKind,
        family: String,
    },
    /// No known provider-native shape; PingoGate-native error.
    Unidentified,
}

impl Detection {
    /// Returns the matched protocol when supported, else `None`.
    pub fn protocol_if_supported(&self) -> Option<ProtocolKind> {
        match self {
            Detection::Supported(d) => Some(d.protocol),
            _ => None,
        }
    }
}

/// Identify the inbound protocol from the request method and path.
///
/// Version-prefix wildcard: matches path *suffixes*, so `/v1/`, `/v2/`,
/// `/v1beta/`, and a prefix-less path all resolve. No `/v1` is hardcoded.
pub fn detect(method: &str, path: &str) -> Detection {
    let path = path.split_once('?').map(|(p, _)| p).unwrap_or(path);
    let is_post = method.eq_ignore_ascii_case("POST");

    // OpenAI Chat Completions: {ver}/chat/completions or /chat/completions
    if is_post && path.ends_with("/chat/completions") {
        return supported(ProtocolKind::OpenAiCompatible, false);
    }
    // OpenAI Responses: {ver}/responses or /responses
    if is_post && path.ends_with("/responses") {
        return supported(ProtocolKind::OpenAiResponses, false);
    }
    // Anthropic Messages: {ver}/messages or /messages
    if is_post && path.ends_with("/messages") {
        return supported(ProtocolKind::Anthropic, false);
    }
    // Gemini streamGenerateContent (streaming proven by path)
    if path.contains(":streamGenerateContent") {
        return supported(ProtocolKind::Gemini, true);
    }
    // Gemini generateContent
    if path.contains(":generateContent") {
        return supported(ProtocolKind::Gemini, false);
    }
    // Gemini Interactions API action
    if path.contains(":interact") {
        return supported(ProtocolKind::GeminiInteractions, false);
    }

    // Recognized-but-unsupported capability families
    if let Some((protocol, family)) = unsupported_family(path) {
        return Detection::UnsupportedCapability { protocol, family };
    }

    Detection::Unidentified
}

fn supported(protocol: ProtocolKind, streaming_by_path: bool) -> Detection {
    Detection::Supported(Detected {
        protocol,
        streaming_by_path,
    })
}

/// Recognize provider surfaces that are out of scope this phase. Returns the
/// provider namespace (for error-shape selection) and the capability family.
fn unsupported_family(path: &str) -> Option<(ProtocolKind, String)> {
    // OpenAI-namespace unsupported surfaces
    if path.ends_with("/realtime") {
        return Some((ProtocolKind::OpenAiCompatible, "realtime.live".into()));
    }
    if path.ends_with("/embeddings") {
        return Some((ProtocolKind::OpenAiCompatible, "embedding".into()));
    }
    if path.ends_with("/batches") {
        return Some((ProtocolKind::OpenAiCompatible, "batch".into()));
    }
    // Gemini-namespace unsupported surfaces
    if path.contains(":countTokens") {
        return Some((ProtocolKind::Gemini, "platform.admin".into()));
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use pingogate_core_types::ProtocolKind;

    #[test]
    fn detects_openai_chat_with_any_version_prefix() {
        assert!(matches!(detect("POST", "/v1/chat/completions"), Detection::Supported(_)));
        assert!(matches!(detect("POST", "/v2/chat/completions"), Detection::Supported(_)));
        assert!(matches!(detect("POST", "/chat/completions"), Detection::Supported(_)));
    }

    #[test]
    fn detects_openai_responses() {
        assert!(matches!(detect("POST", "/v1/responses").protocol_if_supported(), Some(ProtocolKind::OpenAiResponses)));
    }

    #[test]
    fn detects_anthropic_messages() {
        let d = detect("POST", "/v1/messages");
        assert!(matches!(d, Detection::Supported(Detected { protocol: ProtocolKind::Anthropic, .. })));
    }

    #[test]
    fn detects_gemini_generate_content() {
        let d = detect("POST", "/v1beta/models/gemini-1.5-pro:generateContent");
        assert!(matches!(d, Detection::Supported(Detected { protocol: ProtocolKind::Gemini, streaming_by_path: false })));
    }

    #[test]
    fn detects_gemini_stream_generate_content() {
        let d = detect("POST", "/v1beta/models/gemini-1.5-pro:streamGenerateContent");
        assert!(matches!(d, Detection::Supported(Detected { protocol: ProtocolKind::Gemini, streaming_by_path: true })));
    }

    #[test]
    fn detects_gemini_interactions() {
        let d = detect("POST", "/v1beta/models/gemini-3.5-flash:interact");
        assert!(matches!(d, Detection::Supported(Detected { protocol: ProtocolKind::GeminiInteractions, .. })));
    }

    #[test]
    fn unidentified_for_unknown_path() {
        assert!(matches!(detect("GET", "/unknown"), Detection::Unidentified));
    }

    #[test]
    fn unsupported_capability_for_realtime() {
        let d = detect("POST", "/v1/realtime");
        assert!(matches!(
            d,
            Detection::UnsupportedCapability {
                protocol: ProtocolKind::OpenAiCompatible,
                family,
            } if family == "realtime.live"
        ));
    }

    #[test]
    fn unsupported_capability_count_tokens_is_gemini() {
        let d = detect("POST", "/v1beta/models/gemini-1.5-pro:countTokens");
        assert!(matches!(
            d,
            Detection::UnsupportedCapability {
                protocol: ProtocolKind::Gemini,
                family,
            } if family == "platform.admin"
        ));
    }
}
