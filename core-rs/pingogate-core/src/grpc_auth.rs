//! gRPC `x-internal-token` interceptor (spec §12A).
//!
//! The Rust gRPC server only accepts calls from the Go control plane that
//! carry the shared internal token in metadata. mTLS already authenticates the
//! peer certificate, but the token is a second defense layer: any local
//! process holding the CA+client cert (e.g. a misconfigured sidecar) still
//! cannot call `KeyVault.Decrypt` without the token. Constant-time comparison
//! avoids timing oracles on the shared secret (constitution XX).

use subtle::ConstantTimeEq;
use tonic::service::Interceptor;
use tonic::{Request, Status};

/// Metadata header carrying the shared internal token (spec §12A).
pub const INTERNAL_TOKEN_HEADER: &str = "x-internal-token";

/// Interceptor that rejects any request whose `x-internal-token` metadata does
/// not match the expected shared secret. Comparison is constant-time.
#[derive(Clone)]
pub struct InternalTokenInterceptor {
    expected: Vec<u8>,
}

impl InternalTokenInterceptor {
    /// Construct from the `PINGO_INTERNAL_TOKEN` value loaded at bootstrap.
    pub fn new(expected: String) -> Self {
        Self {
            expected: expected.into_bytes(),
        }
    }
}

impl Interceptor for InternalTokenInterceptor {
    fn call(&mut self, request: Request<()>) -> Result<Request<()>, Status> {
        let presented = request
            .metadata()
            .get(INTERNAL_TOKEN_HEADER)
            .and_then(|value| value.to_str().ok())
            .map(|s| s.as_bytes().to_vec());

        match presented {
            Some(token) if token.len() == self.expected.len() && {
                token.ct_eq(&self.expected).into()
            } =>
            {
                Ok(request)
            }
            _ => Err(Status::unauthenticated("invalid or missing internal token")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tonic::Request;

    fn request_with_token(token: Option<&str>) -> Request<()> {
        let mut req = Request::new(());
        if let Some(t) = token {
            req.metadata_mut()
                .insert(INTERNAL_TOKEN_HEADER, t.parse().unwrap());
        }
        req
    }

    #[test]
    fn accepts_matching_token() {
        let mut interceptor = InternalTokenInterceptor::new("secret-shared".to_string());
        let req = request_with_token(Some("secret-shared"));
        assert!(interceptor.call(req).is_ok());
    }

    #[test]
    fn rejects_missing_token() {
        let mut interceptor = InternalTokenInterceptor::new("secret-shared".to_string());
        let req = request_with_token(None);
        let err = interceptor.call(req).unwrap_err();
        assert_eq!(err.code(), tonic::Code::Unauthenticated);
    }

    #[test]
    fn rejects_wrong_token() {
        let mut interceptor = InternalTokenInterceptor::new("secret-shared".to_string());
        let req = request_with_token(Some("wrong"));
        let err = interceptor.call(req).unwrap_err();
        assert_eq!(err.code(), tonic::Code::Unauthenticated);
    }

    #[test]
    fn rejects_empty_token_when_expected_nonempty() {
        let mut interceptor = InternalTokenInterceptor::new("secret-shared".to_string());
        let req = request_with_token(Some(""));
        assert!(interceptor.call(req).is_err());
    }
}
