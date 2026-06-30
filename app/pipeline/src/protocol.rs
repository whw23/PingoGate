//! Inbound protocol identification (research R3; FR-001/FR-006/FR-007).
//!
//! Identification is by request line, not by an artificial `/openai/...` prefix,
//! so official SDKs work by only changing base URL/token/model (SC-001). A
//! recognized-but-out-of-scope capability surface is reported distinctly from a
//! wholly unidentified request, so the former can mirror the provider's native
//! error shape while the latter falls back to PingoGate-native (FR-036).

use pingo_core::ProtocolKind;

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
    /// A supported `generation.stateless` endpoint; proceed to routing.
    Supported(Detected),
    /// A recognized provider surface but an unsupported capability family
    /// (Responses/Realtime/Embeddings/Batch…); short-circuit with an explicit
    /// error in `protocol`'s native shape (FR-007/SC-010).
    UnsupportedCapability {
        protocol: ProtocolKind,
        family: String,
    },
    /// No known provider-native shape; PingoGate-native error (FR-036).
    Unidentified,
}

/// Identify the inbound protocol from the request method and path.
pub fn detect(method: &str, path: &str) -> Detection {
    let path = path.split_once('?').map(|(p, _)| p).unwrap_or(path);
    let is_post = method.eq_ignore_ascii_case("POST");

    if is_post && path == "/v1/chat/completions" {
        return supported(ProtocolKind::OpenAiCompatible, false);
    }
    if is_post && path == "/v1/messages" {
        return supported(ProtocolKind::Anthropic, false);
    }
    if path.contains(":streamGenerateContent") {
        return supported(ProtocolKind::Gemini, true);
    }
    if path.contains(":generateContent") {
        return supported(ProtocolKind::Gemini, false);
    }

    if let Some(family) = openai_unsupported_family(path) {
        return Detection::UnsupportedCapability {
            protocol: ProtocolKind::OpenAiCompatible,
            family,
        };
    }
    if let Some(family) = gemini_unsupported_family(path) {
        return Detection::UnsupportedCapability {
            protocol: ProtocolKind::Gemini,
            family,
        };
    }
    Detection::Unidentified
}

fn supported(protocol: ProtocolKind, streaming_by_path: bool) -> Detection {
    Detection::Supported(Detected {
        protocol,
        streaming_by_path,
    })
}

/// Recognize OpenAI-family surfaces that are out of scope this phase.
fn openai_unsupported_family(path: &str) -> Option<String> {
    let family = match path {
        "/v1/responses" => "generation.stateful",
        "/v1/realtime" => "realtime.live",
        "/v1/embeddings" => "embedding",
        "/v1/batches" => "batch",
        "/v1/completions" => "generation.legacy",
        _ if path.starts_with("/v1/audio/") => "audio",
        _ if path.starts_with("/v1/images/") => "image",
        _ => return None,
    };
    Some(family.to_string())
}

/// Recognize Gemini surfaces that are out of scope this phase.
fn gemini_unsupported_family(path: &str) -> Option<String> {
    if path.contains(":embedContent") || path.contains(":batchEmbedContents") {
        Some("embedding".to_string())
    } else if path.contains(":batchGenerateContent") {
        Some("batch".to_string())
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_three_supported_protocols() {
        assert_eq!(
            detect("POST", "/v1/chat/completions"),
            supported(ProtocolKind::OpenAiCompatible, false)
        );
        assert_eq!(
            detect("post", "/v1/messages"),
            supported(ProtocolKind::Anthropic, false)
        );
        assert_eq!(
            detect("POST", "/v1beta/models/gemini-1.5-pro:generateContent"),
            supported(ProtocolKind::Gemini, false)
        );
    }

    #[test]
    fn gemini_stream_variant_marks_streaming_by_path() {
        let d = detect(
            "POST",
            "/v1beta/models/gemini-1.5-pro:streamGenerateContent?alt=sse",
        );
        assert_eq!(d, supported(ProtocolKind::Gemini, true));
    }

    #[test]
    fn recognized_unsupported_capability_keeps_protocol() {
        assert_eq!(
            detect("POST", "/v1/embeddings"),
            Detection::UnsupportedCapability {
                protocol: ProtocolKind::OpenAiCompatible,
                family: "embedding".to_string()
            }
        );
        assert!(matches!(
            detect("POST", "/v1/responses"),
            Detection::UnsupportedCapability { .. }
        ));
    }

    #[test]
    fn unknown_path_is_unidentified() {
        assert_eq!(detect("GET", "/"), Detection::Unidentified);
        assert_eq!(detect("POST", "/totally/unknown"), Detection::Unidentified);
    }
}
