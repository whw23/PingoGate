//! Cross-provider classification of a gateway [`AppError`] into the fields each
//! provider's native error body needs (research R4).
//!
//! Keeping the mapping in one place lets the three renderers stay DRY and
//! consistent: the gateway's own errors are mirrored into the target provider's
//! native shape so existing SDK error parsing keeps working (SC-002/FR-035).

use pingogate_core_types::AppError;

/// Provider-neutral classification of a gateway error into native field values.
pub struct ErrorClass {
    /// OpenAI `error.type`.
    pub openai_type: &'static str,
    /// OpenAI `error.code`.
    pub openai_code: &'static str,
    /// Anthropic `error.type`.
    pub anthropic_type: &'static str,
    /// Gemini `error.status` (canonical UPPER_SNAKE code).
    pub gemini_status: &'static str,
}

/// Map a gateway [`AppError`] to its per-provider native error classifiers.
///
/// `#[rustfmt::skip]` keeps the one-arm-per-line data table compact so the
/// function stays well within the constitution V length budget (≤ 50 lines);
/// the body is a pure lookup with no branching complexity.
#[rustfmt::skip]
pub(crate) fn classify(err: &AppError) -> ErrorClass {
    match err {
        AppError::UnknownProtocol => ErrorClass { openai_type: "invalid_request_error", openai_code: "unknown_protocol", anthropic_type: "invalid_request_error", gemini_status: "INVALID_ARGUMENT" },
        AppError::NoRoute { .. } => ErrorClass { openai_type: "invalid_request_error", openai_code: "model_not_found", anthropic_type: "not_found_error", gemini_status: "NOT_FOUND" },
        AppError::UnsupportedCapability { .. } => ErrorClass { openai_type: "invalid_request_error", openai_code: "unsupported_capability", anthropic_type: "invalid_request_error", gemini_status: "INVALID_ARGUMENT" },
        AppError::AuthFailed => ErrorClass { openai_type: "invalid_request_error", openai_code: "invalid_api_key", anthropic_type: "authentication_error", gemini_status: "UNAUTHENTICATED" },
        AppError::Unauthorized { .. } => ErrorClass { openai_type: "invalid_request_error", openai_code: "forbidden", anthropic_type: "permission_error", gemini_status: "PERMISSION_DENIED" },
        AppError::UpstreamUnavailable { .. } => ErrorClass { openai_type: "api_error", openai_code: "upstream_unavailable", anthropic_type: "api_error", gemini_status: "UNAVAILABLE" },
        AppError::UpstreamTimeout { .. } => ErrorClass { openai_type: "api_error", openai_code: "upstream_timeout", anthropic_type: "api_error", gemini_status: "DEADLINE_EXCEEDED" },
        AppError::Validation { .. } => ErrorClass { openai_type: "invalid_request_error", openai_code: "invalid_config", anthropic_type: "invalid_request_error", gemini_status: "INVALID_ARGUMENT" },
        AppError::Internal { .. } => ErrorClass { openai_type: "api_error", openai_code: "internal_error", anthropic_type: "api_error", gemini_status: "INTERNAL" },
    }
}
