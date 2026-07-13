//! Anthropic-native error body renderer (FR-035): the top-level object is
//! `{"type": "error", "error": {"type", "message"}}`.

use pingogate_core_types::AppError;
use serde_json::{json, Value};

use crate::error_shape::classify;

/// Render `err` as an Anthropic-shaped error body.
pub fn render_error(err: &AppError) -> Value {
    let c = classify(err);
    json!({
        "type": "error",
        "error": {
            "type": c.anthropic_type,
            "message": err.to_string(),
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn auth_failure_renders_anthropic_shape() {
        let v = render_error(&AppError::AuthFailed);
        assert_eq!(v["type"], "error");
        assert_eq!(v["error"]["type"], "authentication_error");
        assert!(v["error"]["message"].is_string());
    }

    #[test]
    fn no_route_uses_not_found_error() {
        let v = render_error(&AppError::NoRoute { alias: "x".into() });
        assert_eq!(v["error"]["type"], "not_found_error");
    }
}
