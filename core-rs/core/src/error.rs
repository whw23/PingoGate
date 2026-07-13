//! Error taxonomy (constitution IX).
//!
//! [`AppError`] carries a layered classification ([`ErrorLayer`]); HTTP status
//! codes are derived only at the request boundary, never inside domain logic.
//! [`AppError::kind`] is the stable machine token used by the PingoGate-native
//! mirrored error body (`pingogate.<reason>`).

/// Architectural layer an error originates from (constitution IX).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErrorLayer {
    Domain,
    Application,
    Infrastructure,
    Validation,
}

/// The gateway's own error type. Upstream provider errors are passed through
/// untouched and are NOT represented here (FR-008/FR-037).
#[derive(Debug, thiserror::Error)]
pub enum AppError {
    #[error("protocol not recognized")]
    UnknownProtocol,

    #[error("no route for model alias: {alias}")]
    NoRoute { alias: String },

    #[error("capability family not supported: {family}")]
    UnsupportedCapability { family: String },

    #[error("gateway authentication failed")]
    AuthFailed,

    #[error("not authorized for action: {action}")]
    Unauthorized { action: String },

    #[error("upstream unavailable: {provider}")]
    UpstreamUnavailable { provider: String },

    #[error("upstream timed out: {provider}")]
    UpstreamTimeout { provider: String },

    #[error("configuration invalid: {message}")]
    Validation { message: String },

    #[error("internal error: {message}")]
    Internal { message: String },
}

impl AppError {
    /// Originating architectural layer.
    pub fn layer(&self) -> ErrorLayer {
        match self {
            Self::UnknownProtocol | Self::NoRoute { .. } | Self::UnsupportedCapability { .. } => {
                ErrorLayer::Domain
            }
            Self::AuthFailed | Self::Unauthorized { .. } => ErrorLayer::Application,
            Self::UpstreamUnavailable { .. }
            | Self::UpstreamTimeout { .. }
            | Self::Internal { .. } => ErrorLayer::Infrastructure,
            Self::Validation { .. } => ErrorLayer::Validation,
        }
    }

    /// HTTP status for the boundary response. Call only when rendering a
    /// response - domain/application code must not branch on this.
    pub fn http_status(&self) -> u16 {
        match self {
            Self::UnknownProtocol | Self::UnsupportedCapability { .. } => 400,
            Self::AuthFailed => 401,
            Self::Unauthorized { .. } => 403,
            Self::NoRoute { .. } => 404,
            Self::UpstreamUnavailable { .. } => 502,
            Self::UpstreamTimeout { .. } => 504,
            Self::Validation { .. } => 400,
            Self::Internal { .. } => 500,
        }
    }

    /// Stable machine token for the PingoGate-native error body (FR-036).
    pub fn kind(&self) -> &'static str {
        match self {
            Self::UnknownProtocol => "pingogate.unknown_protocol",
            Self::NoRoute { .. } => "pingogate.no_route",
            Self::UnsupportedCapability { .. } => "pingogate.unsupported_capability",
            Self::AuthFailed => "pingogate.auth_failed",
            Self::Unauthorized { .. } => "pingogate.unauthorized",
            Self::UpstreamUnavailable { .. } => "pingogate.upstream_unavailable",
            Self::UpstreamTimeout { .. } => "pingogate.upstream_timeout",
            Self::Validation { .. } => "pingogate.invalid_config",
            Self::Internal { .. } => "pingogate.internal",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn domain_errors_map_to_domain_layer() {
        assert_eq!(AppError::UnknownProtocol.layer(), ErrorLayer::Domain);
        assert_eq!(
            AppError::NoRoute { alias: "x".into() }.layer(),
            ErrorLayer::Domain
        );
    }

    #[test]
    fn application_and_infra_layers_classified() {
        assert_eq!(AppError::AuthFailed.layer(), ErrorLayer::Application);
        assert_eq!(
            AppError::UpstreamTimeout {
                provider: "p".into()
            }
            .layer(),
            ErrorLayer::Infrastructure
        );
        assert_eq!(
            AppError::Validation {
                message: "m".into()
            }
            .layer(),
            ErrorLayer::Validation
        );
    }

    #[test]
    fn http_status_only_resolved_at_boundary() {
        assert_eq!(AppError::AuthFailed.http_status(), 401);
        assert_eq!(
            AppError::Unauthorized {
                action: "reload".into()
            }
            .http_status(),
            403
        );
        assert_eq!(AppError::NoRoute { alias: "x".into() }.http_status(), 404);
        assert_eq!(
            AppError::UpstreamUnavailable {
                provider: "p".into()
            }
            .http_status(),
            502
        );
        assert_eq!(
            AppError::UpstreamTimeout {
                provider: "p".into()
            }
            .http_status(),
            504
        );
    }

    #[test]
    fn kind_tokens_are_namespaced() {
        assert!(AppError::AuthFailed.kind().starts_with("pingogate."));
        assert_eq!(
            AppError::NoRoute { alias: "x".into() }.kind(),
            "pingogate.no_route"
        );
    }
}
