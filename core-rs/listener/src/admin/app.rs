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

    /// Route a request to the right handler. Bypass endpoints (`/healthz`,
    /// `/metrics`) skip auth entirely (R1); all others require a valid bootstrap
    /// credential. Extracted from [`ServeHttp::response`] so the bypass-vs-auth
    /// ordering is unit-testable without a live `ServerSession` (a regression
    /// moving `/healthz` after the auth check would be caught here).
    fn route(
        &self,
        method: &str,
        path: &str,
        authorization: Option<&str>,
        body: &[u8],
    ) -> Response<Vec<u8>> {
        // Unauthenticated endpoints (R1): probes and scrape bypass auth.
        if path == "/healthz" {
            return self.handler.healthz();
        }
        if path == "/metrics" {
            return self.handler.metrics();
        }

        // Authenticated endpoints: Bearer token -> Principal -> authorize.
        let Some(principal) = self.bootstrap.authenticate(authorization) else {
            return json_response(
                401,
                json!({"error": {"kind": "pingogate.auth_failed", "message": "missing or invalid admin credential"}}),
            );
        };
        let auth_ctx = AuthContext::new(principal);

        match (method, path) {
            ("GET", "/readyz") => self.handler.readyz(&auth_ctx),
            ("POST", "/reload") => self.handler.reload(&auth_ctx),
            ("GET", "/reload/status") => self.handler.reload_status(&auth_ctx),
            ("POST", "/config/validate") => self.handler.validate(&auth_ctx, body),
            _ => json_response(
                404,
                json!({"error": {"kind": "pingogate.not_found", "message": "no such admin endpoint"}}),
            ),
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
        let authorization = session
            .req_header()
            .headers
            .get("authorization")
            .and_then(|v| v.to_str().ok())
            .map(str::to_string);

        // Bypass endpoints (R1) skip body reading so a liveness probe always
        // answers promptly; all other paths read the body before routing.
        let body = if is_bypass(&path) {
            Vec::new()
        } else {
            match read_body(session).await {
                Ok(body) => body,
                Err(response) => return response,
            }
        };

        self.route(&method, &path, authorization.as_deref(), &body)
    }
}

/// Whether `path` is an unauthenticated bypass endpoint (R1: probes/scrape).
fn is_bypass(path: &str) -> bool {
    path == "/healthz" || path == "/metrics"
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::admin::status::ReloadStatusStore;
    use pingogate_snapshot::{GatewayConfig, RuntimeSnapshot};
    use pingogate_storage::{EnvSecretResolver, SnapshotError, SnapshotSource};

    /// A `SnapshotSource` that never builds. The bypass-routing tests never
    /// trigger a reload, so the reloader's source is never exercised.
    struct StubSource;
    impl SnapshotSource for StubSource {
        type Snapshot = RuntimeSnapshot;
        fn build_snapshot(&self, _version: u64) -> Result<Arc<RuntimeSnapshot>, SnapshotError> {
            Err(SnapshotError::Parse("stub source".to_string()))
        }
    }

    /// Build an `AdminApp` with no admin token (R1: every authed endpoint
    /// rejects). The holder/reloader/metrics are real but minimal - bypass
    /// endpoints never touch them, and authed endpoints return `401` before
    /// reaching the handler, so the stubs are never exercised.
    fn app_without_token() -> AdminApp {
        std::env::set_var("PINGO_APP_TEST_KEY", "sk-test");
        let yaml = r#"listeners:
  public: { address: "0.0.0.0:8080" }
  admin: { address: "127.0.0.1:9090" }
providers:
  - name: "p"
    kind: "openai-compatible"
    base_url: "https://x"
    auth: { method: "bearer", key_ref: "env:PINGO_APP_TEST_KEY" }
    capability_families: ["generation.stateless"]
    models:
      - { alias: "a", upstream_model: "m" }
"#;
        let config = GatewayConfig::from_yaml(yaml).unwrap();
        let snapshot = RuntimeSnapshot::build(&config, &EnvSecretResolver, 1).unwrap();
        let holder = Arc::new(SnapshotHolder::new(snapshot));
        let status = Arc::new(ReloadStatusStore::new(1));
        let reloader = Arc::new(Reloader::new(holder.clone(), status, Arc::new(StubSource)));
        let metrics = Arc::new(Metrics::new());
        AdminApp::new(None, holder, reloader, metrics)
    }

    // ---- R1: bypass endpoints return 200 even with no credential ----
    // If the auth check ran before the bypass check, these would be 401.

    #[test]
    fn healthz_bypasses_auth_without_token() {
        let app = app_without_token();
        let resp = app.route("GET", "/healthz", None, &[]);
        assert_eq!(resp.status(), 200);
    }

    #[test]
    fn metrics_bypasses_auth_without_token() {
        let app = app_without_token();
        let resp = app.route("GET", "/metrics", None, &[]);
        assert_eq!(resp.status(), 200);
    }

    // ---- R1: authenticated endpoints return 401 without a credential ----

    #[test]
    fn readyz_rejects_without_token() {
        let app = app_without_token();
        let resp = app.route("GET", "/readyz", None, &[]);
        assert_eq!(resp.status(), 401);
    }

    #[test]
    fn reload_rejects_without_token() {
        let app = app_without_token();
        let resp = app.route("POST", "/reload", None, &[]);
        assert_eq!(resp.status(), 401);
    }

    #[test]
    fn reload_status_rejects_without_token() {
        let app = app_without_token();
        let resp = app.route("GET", "/reload/status", None, &[]);
        assert_eq!(resp.status(), 401);
    }

    #[test]
    fn config_validate_rejects_without_token() {
        let app = app_without_token();
        let resp = app.route("POST", "/config/validate", None, &[]);
        assert_eq!(resp.status(), 401);
    }
}
