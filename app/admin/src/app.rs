//! Admin API application (constitution XX; admin-api contract).
//!
//! Every request is authenticated into a [`Principal`] and then authorized
//! through [`AuthContext::authorize`] — the bootstrap admin token is no
//! exception, so there is never a global-token equality shortcut as the sole
//! gate on a privileged action. This phase serves liveness (`/healthz`) and
//! readiness (`/readyz`); config validation and reload land in User Story 2/3.

use std::sync::Arc;

use async_trait::async_trait;
use http::Response;
use pingo_config::SnapshotHolder;
use pingo_core::{Action, AuthContext, Principal, Resource, SecretString};
use pingora::apps::http_app::ServeHttp;
use pingora::protocols::http::ServerSession;
use serde_json::{json, Value};

/// The Admin API surface, backed by the live snapshot holder.
pub struct AdminApp {
    admin_token: SecretString,
    holder: Arc<SnapshotHolder>,
}

impl AdminApp {
    pub fn new(admin_token: SecretString, holder: Arc<SnapshotHolder>) -> Self {
        Self {
            admin_token,
            holder,
        }
    }

    /// Authenticate the bootstrap admin credential into a [`Principal`].
    ///
    /// This only *authenticates* (verifies the presented token). Authorization
    /// is always decided afterwards by [`AuthContext::authorize`]; this check is
    /// never the sole gate on a privileged action (constitution XX).
    fn authenticate(&self, session: &ServerSession) -> Option<Principal> {
        let value = session.req_header().headers.get("authorization")?;
        let token = value.to_str().ok()?.strip_prefix("Bearer ")?.trim();
        // NOTE: constant-time comparison is a hardening item for the security
        // review (T065); a direct compare is acceptable for the bootstrap path.
        if !token.is_empty() && token == self.admin_token.expose() {
            Some(Principal::admin("bootstrap"))
        } else {
            None
        }
    }
}

#[async_trait]
impl ServeHttp for AdminApp {
    async fn response(&self, session: &mut ServerSession) -> Response<Vec<u8>> {
        let Some(principal) = self.authenticate(session) else {
            return json_response(
                401,
                json!({"error": {"kind": "pingogate.auth_failed", "message": "missing or invalid admin credential"}}),
            );
        };
        let auth = AuthContext::new(principal);

        let (method, path) = {
            let h = session.req_header();
            (h.method.as_str().to_string(), h.uri.path().to_string())
        };

        match (method.as_str(), path.as_str()) {
            ("GET", "/healthz") => guarded(&auth, "/healthz", || {
                json_response(200, json!({"status": "ok"}))
            }),
            ("GET", "/readyz") => guarded(&auth, "/readyz", || {
                let version = self.holder.load().version();
                json_response(200, json!({"status": "ready", "active_version": version}))
            }),
            // Mutating control-plane endpoints arrive with User Story 2/3.
            (_, "/config/validate") | (_, "/reload") | (_, "/reload/status") => json_response(
                501,
                json!({"error": {"kind": "pingogate.not_implemented", "message": "endpoint implemented in a later milestone"}}),
            ),
            _ => json_response(
                404,
                json!({"error": {"kind": "pingogate.not_found", "message": "no such admin endpoint"}}),
            ),
        }
    }
}

/// Run `handler` only if the admin principal is authorized to read `endpoint`;
/// otherwise return `403` from the same boundary (constitution XX).
fn guarded(
    auth: &AuthContext,
    endpoint: &str,
    handler: impl FnOnce() -> Response<Vec<u8>>,
) -> Response<Vec<u8>> {
    match auth.authorize(Action::AdminRead, &Resource::admin_endpoint(endpoint)) {
        Ok(()) => handler(),
        Err(_) => json_response(
            403,
            json!({"error": {"kind": "pingogate.unauthorized", "message": "not permitted"}}),
        ),
    }
}

/// Build a JSON response with the given status, never panicking on serialization.
///
/// `Content-Length` is set explicitly: the Pingora `ServeHttp` writer does not
/// frame the body for us, so without it an HTTP/1.1 keep-alive client would
/// hang waiting for the connection to close.
fn json_response(status: u16, body: Value) -> Response<Vec<u8>> {
    let bytes = serde_json::to_vec(&body).unwrap_or_else(|_| b"{}".to_vec());
    Response::builder()
        .status(status)
        .header("content-type", "application/json")
        .header("content-length", bytes.len().to_string())
        .body(bytes)
        .unwrap_or_else(|_| Response::new(Vec::new()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn json_response_sets_status_and_content_type() {
        let r = json_response(200, json!({"status": "ok"}));
        assert_eq!(r.status(), 200);
        assert_eq!(r.headers().get("content-type").unwrap(), "application/json");
        assert!(!r.body().is_empty());
    }

    #[test]
    fn guarded_blocks_non_admin_principal() {
        // A gateway-key principal must never pass the admin read boundary.
        let auth = AuthContext::new(Principal::gateway_key("k"));
        let r = guarded(&auth, "/healthz", || json_response(200, json!({})));
        assert_eq!(r.status(), 403);
    }

    #[test]
    fn guarded_allows_admin_principal() {
        let auth = AuthContext::new(Principal::admin("ops"));
        let r = guarded(&auth, "/readyz", || json_response(200, json!({"ok": true})));
        assert_eq!(r.status(), 200);
    }
}
