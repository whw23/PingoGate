//! Gemini native error body renderer (FR-035): `{"error": {"code", "message",
//! "status"}}`, where `code` is the numeric HTTP status and `status` is the
//! canonical UPPER_SNAKE code.

use pingo_core::AppError;
use serde_json::{json, Value};

use crate::error_shape::classify;

/// Render `err` as a Gemini-shaped error body.
pub fn render_error(err: &AppError) -> Value {
    let c = classify(err);
    json!({
        "error": {
            "code": err.http_status(),
            "message": err.to_string(),
            "status": c.gemini_status,
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn auth_failure_renders_gemini_shape() {
        let v = render_error(&AppError::AuthFailed);
        assert_eq!(v["error"]["code"], 401);
        assert_eq!(v["error"]["status"], "UNAUTHENTICATED");
        assert!(v["error"]["message"].is_string());
    }

    #[test]
    fn no_route_uses_not_found_status() {
        let v = render_error(&AppError::NoRoute { alias: "x".into() });
        assert_eq!(v["error"]["code"], 404);
        assert_eq!(v["error"]["status"], "NOT_FOUND");
    }
}
