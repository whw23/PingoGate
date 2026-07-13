//! Admin API `ServeHttp` glue (constitution XX; admin-api contract).
//!
//! Thin transport adapter: route `/healthz` and `/metrics` without auth
//! (probes/scrape, R1), authenticate the bootstrap credential for all other
//! endpoints into a [`Principal`] (via [`BootstrapAuth`]), read the request
//! body, then delegate to [`AdminHandler`] which enforces the `authorize`
//! boundary per endpoint.

use std::sync::Arc;

use async_trait::async_trait;
use http::Response;
use pingogate_core_types::{AuthContext, SecretString};
use pingogate_pipeline::Metrics;
use pingogate_snapshot::SnapshotHolder;
use pingora::apps::http_app::ServeHttp;
use pingora::protocols::http::ServerSession;
use pingora::services::listening::Service;
use serde_json::json;

use crate::admin::auth::BootstrapAuth;
use crate::admin::handler::{json_response, AdminHandler};
use crate::admin::reload::Reloader;
use crate::admin::AdminServiceConfig;

/// Largest admin request body accepted (candidate configs for `/config/validate`).
const MAX_ADMIN_BODY: usize = 1024 * 1024;

/// The Admin API surface: authentication + the control-plane handlers.
pub struct AdminApp {
    bootstrap: BootstrapAuth,
    handler: AdminHandler,
}

impl AdminApp {
    pub fn new(
        admin_token: Option<SecretString>,
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
        let (method, path) = {
            let h = session.req_header();
            (h.method.as_str().to_string(), h.uri.path().to_string())
        };

        // Unauthenticated endpoints (R1): probes and scrape bypass auth.
        if path == "/healthz" {
            return self.handler.healthz();
        }
        if path == "/metrics" {
            return self.handler.metrics();
        }

        // Authenticated endpoints: Bearer token -> Principal -> authorize.
        let authorization = session
            .req_header()
            .headers
            .get("authorization")
            .and_then(|v| v.to_str().ok())
            .map(str::to_string);
        let Some(principal) = self.bootstrap.authenticate(authorization.as_deref()) else {
            return json_response(
                401,
                json!({"error": {"kind": "pingogate.auth_failed", "message": "missing or invalid admin credential"}}),
            );
        };
        let auth_ctx = AuthContext::new(principal);

        let body = match read_body(session).await {
            Ok(body) => body,
            Err(response) => return response,
        };

        match (method.as_str(), path.as_str()) {
            ("GET", "/readyz") => self.handler.readyz(&auth_ctx),
            ("POST", "/reload") => self.handler.reload(&auth_ctx),
            ("GET", "/reload/status") => self.handler.reload_status(&auth_ctx),
            ("POST", "/config/validate") => self.handler.validate(&auth_ctx, &body),
            _ => json_response(
                404,
                json!({"error": {"kind": "pingogate.not_found", "message": "no such admin endpoint"}}),
            ),
        }
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

/// Build the Admin API service from its configuration.
pub fn build_admin_service(config: AdminServiceConfig) -> Service<AdminApp> {
    let mut service = Service::new(
        "pingogate-admin".to_string(),
        AdminApp::new(config.admin_token, config.holder, config.reloader, config.metrics),
    );
    service.add_tcp(&config.address);
    service
}
