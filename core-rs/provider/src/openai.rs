//! OpenAI-native error body renderer (FR-035; contract provider-passthrough.md):
//! `{"error": {"message", "type", "code"}}`.
//!
//! Shared by both OpenAI protocol surfaces - Chat Completions
//! (`OpenAiCompatible`) and Responses (`OpenAiResponses`) - because the
//! Responses API uses the same error envelope as Chat Completions. See
//! [`crate::responses`] and [`crate::adapter`].

use pingogate_core_types::AppError;
use serde_json::{json, Value};

use crate::error_shape::classify;

/// Render `err` as an OpenAI-shaped error body.
pub fn render_error(err: &AppError) -> Value {
    let c = classify(err);
    json!({
        "error": {
            "message": err.to_string(),
            "type": c.openai_type,
            "code": c.openai_code,
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn auth_failure_renders_openai_shape() {
        let v = render_error(&AppError::AuthFailed);
        assert_eq!(v["error"]["type"], "invalid_request_error");
        assert_eq!(v["error"]["code"], "invalid_api_key");
        assert!(v["error"]["message"].is_string());
    }

    #[test]
    fn no_route_uses_model_not_found_code() {
        let v = render_error(&AppError::NoRoute { alias: "x".into() });
        assert_eq!(v["error"]["code"], "model_not_found");
    }
}
