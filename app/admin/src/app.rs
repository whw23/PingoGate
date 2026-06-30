//! Admin API application — the `ServeHttp` glue (constitution XX; admin-api).
//!
//! Thin transport adapter: authenticate the bootstrap credential into a
//! [`Principal`] (via [`BootstrapAuth`]), read the request body, then delegate
//! routing, authorization, and endpoint logic to [`AdminHandler`]. Keeping the
//! Pingora-facing surface minimal makes the contract logic unit-testable without
//! a live connection.

use std::sync::Arc;

use async_trait::async_trait;
use http::Response;
use pingo_config::SnapshotHolder;
use pingo_core::{AuthContext, SecretString};
use pingo_pipeline::Metrics;
use pingora::apps::http_app::ServeHttp;
use pingora::protocols::http::ServerSession;
use serde_json::json;

use crate::bootstrap::BootstrapAuth;
use crate::handler::{json_response, AdminHandler};
use crate::reload::Reloader;

/// Largest admin request body accepted (candidate configs for `/config/validate`).
const MAX_ADMIN_BODY: usize = 1024 * 1024;

/// The Admin API surface: authentication + the control-plane handlers.
pub struct AdminApp {
    bootstrap: BootstrapAuth,
    handler: AdminHandler,
}

impl AdminApp {
    pub fn new(
        admin_token: SecretString,
        holder: Arc<SnapshotHolder>,
        reloader: Arc<Reloader>,
        metrics: Arc<Metrics>,
    ) -> Self {
        Self {
            bootstrap: BootstrapAuth::new(admin_token),
            handler: AdminHandler::new(holder, reloader, metrics),
        }
    }
}

#[async_trait]
impl ServeHttp for AdminApp {
    async fn response(&self, session: &mut ServerSession) -> Response<Vec<u8>> {
        let authorization = session
            .req_header()
            .headers
            .get("authorization")
            .and_then(|v| v.to_str().ok())
            .map(str::to_string);

        // Authenticate first; an unauthenticated caller never reaches a handler
        // and learns nothing about which endpoints exist (SC-006).
        let Some(principal) = self.bootstrap.authenticate(authorization.as_deref()) else {
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

        let body = match read_body(session).await {
            Ok(body) => body,
            Err(response) => return response,
        };
        self.handler.dispatch(&auth, &method, &path, &body)
    }
}

/// Read the request body up to [`MAX_ADMIN_BODY`], returning a `413` response if
/// it is exceeded. An I/O error yields an empty body so a GET still proceeds.
async fn read_body(session: &mut ServerSession) -> Result<Vec<u8>, Response<Vec<u8>>> {
    let mut body = Vec::new();
    while let Ok(Some(chunk)) = session.read_request_body().await {
        if body.len() + chunk.len() > MAX_ADMIN_BODY {
            return Err(json_response(
                413,
                json!({"error": {"kind": "pingogate.payload_too_large", "message": "admin request body too large"}}),
            ));
        }
        body.extend_from_slice(&chunk);
    }
    Ok(body)
}
