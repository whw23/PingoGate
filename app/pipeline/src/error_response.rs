//! Boundary error rendering (constitution IX; FR-007/FR-036).
//!
//! HTTP status codes are assigned **only here**, at the protocol boundary. The
//! body mirrors the target provider's native error shape when the protocol is
//! known (so SDK error handling keeps working), and falls back to a
//! PingoGate-native envelope when the request was never identified.

use pingo_core::{AppError, ProtocolKind};
use serde_json::json;

/// Render a gateway error as `(http_status, json_body_bytes)`.
///
/// `protocol` is `Some(_)` once the inbound protocol was identified (mirror its
/// native shape) and `None` for an unidentified request (PingoGate-native).
pub fn render(protocol: Option<ProtocolKind>, err: &AppError) -> (u16, Vec<u8>) {
    let body = match protocol {
        Some(ProtocolKind::OpenAiCompatible) => pingo_provider::openai::render_error(err),
        Some(ProtocolKind::Anthropic) => pingo_provider::anthropic::render_error(err),
        Some(ProtocolKind::Gemini) => pingo_provider::gemini::render_error(err),
        None => json!({
            "error": { "message": err.to_string(), "kind": err.kind() }
        }),
    };
    let bytes = serde_json::to_vec(&body).unwrap_or_else(|_| b"{}".to_vec());
    (err.http_status(), bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_protocol_mirrors_native_shape() {
        let (status, bytes) = render(Some(ProtocolKind::OpenAiCompatible), &AppError::AuthFailed);
        assert_eq!(status, 401);
        let v: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(v["error"]["type"], "invalid_request_error");
        assert_eq!(v["error"]["code"], "invalid_api_key");
    }

    #[test]
    fn anthropic_error_uses_error_wrapper() {
        let (status, bytes) = render(
            Some(ProtocolKind::Anthropic),
            &AppError::NoRoute {
                alias: "gpt-4o".to_string(),
            },
        );
        assert_eq!(status, 404);
        let v: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(v["type"], "error");
        assert_eq!(v["error"]["type"], "not_found_error");
    }

    #[test]
    fn unidentified_falls_back_to_native_envelope() {
        let (status, bytes) = render(None, &AppError::UnknownProtocol);
        assert_eq!(status, 400);
        let v: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(v["error"]["kind"], "pingogate.unknown_protocol");
    }
}
