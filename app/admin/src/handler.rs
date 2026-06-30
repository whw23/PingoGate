//! Admin API request router and endpoint handlers (admin-api contract).
//!
//! [`AdminHandler::dispatch`] routes an already-authenticated request to one of
//! the control-plane endpoints. Every endpoint passes through
//! [`AuthContext::authorize`] via [`guard`] before doing any work, so the
//! authorization boundary is uniform and there is no global-token shortcut
//! (constitution XX). Config validation and reload never mutate the active
//! snapshot except through the shared [`Reloader`] (FR-026).

use std::sync::Arc;

use http::Response;
use pingo_config::validate::validate_semantics;
use pingo_config::{ConfigError, GatewayConfig, SnapshotHolder};
use pingo_core::{Action, AuthContext, Resource};
use pingo_pipeline::Metrics;
use serde_json::{json, Value};

use crate::metrics_endpoint::render_metrics;
use crate::reload::Reloader;
use crate::status::{ReloadOutcome, ReloadStatus};

/// The control-plane endpoint handlers, backed by the live snapshot holder, the
/// shared reload orchestrator, and the metrics registry.
pub struct AdminHandler {
    holder: Arc<SnapshotHolder>,
    reloader: Arc<Reloader>,
    metrics: Arc<Metrics>,
}

impl AdminHandler {
    pub fn new(
        holder: Arc<SnapshotHolder>,
        reloader: Arc<Reloader>,
        metrics: Arc<Metrics>,
    ) -> Self {
        Self {
            holder,
            reloader,
            metrics,
        }
    }

    /// Route an authenticated request to its endpoint. `body` is the raw request
    /// body (used only by `/config/validate`).
    pub fn dispatch(
        &self,
        auth: &AuthContext,
        method: &str,
        path: &str,
        body: &[u8],
    ) -> Response<Vec<u8>> {
        match (method, path) {
            ("GET", "/healthz") => self.healthz(auth),
            ("GET", "/readyz") => self.readyz(auth),
            ("GET", "/metrics") => self.metrics(auth),
            ("POST", "/config/validate") => self.validate(auth, body),
            ("POST", "/reload") => self.reload(auth),
            ("GET", "/reload/status") => self.reload_status(auth),
            _ => json_response(
                404,
                json!({"error": {"kind": "pingogate.not_found", "message": "no such admin endpoint"}}),
            ),
        }
    }

    fn healthz(&self, auth: &AuthContext) -> Response<Vec<u8>> {
        guard(
            auth,
            Action::AdminRead,
            &Resource::admin_endpoint("/healthz"),
            || json_response(200, json!({"status": "ok"})),
        )
    }

    fn readyz(&self, auth: &AuthContext) -> Response<Vec<u8>> {
        guard(
            auth,
            Action::AdminRead,
            &Resource::admin_endpoint("/readyz"),
            || {
                let version = self.holder.load().version();
                json_response(200, json!({"status": "ready", "active_version": version}))
            },
        )
    }

    /// Expose the Prometheus metrics exposition (FR-032), behind the same
    /// authorization boundary as every other admin endpoint (constitution XX).
    fn metrics(&self, auth: &AuthContext) -> Response<Vec<u8>> {
        guard(
            auth,
            Action::AdminRead,
            &Resource::admin_endpoint("/metrics"),
            || render_metrics(&self.metrics),
        )
    }

    /// Validate a candidate config without touching the active runtime (FR-026).
    fn validate(&self, auth: &AuthContext, body: &[u8]) -> Response<Vec<u8>> {
        guard(auth, Action::AdminValidate, &Resource::config(), || {
            let yaml = candidate_yaml(body);
            match validate_candidate(&yaml) {
                Ok(()) => json_response(200, json!({"valid": true})),
                Err(errors) => json_response(422, json!({"valid": false, "errors": errors})),
            }
        })
    }

    /// Trigger a reload through the shared orchestrator (same path as SIGHUP).
    fn reload(&self, auth: &AuthContext) -> Response<Vec<u8>> {
        guard(
            auth,
            Action::AdminReload,
            &Resource::config(),
            || match self.reloader.reload() {
                Ok(version) => json_response(
                    200,
                    json!({
                        "result": "success",
                        "active_version": version,
                        "rollback_target": version.saturating_sub(1),
                    }),
                ),
                Err(err) => json_response(
                    422,
                    json!({"result": "rejected", "errors": error_list(&err)}),
                ),
            },
        )
    }

    fn reload_status(&self, auth: &AuthContext) -> Response<Vec<u8>> {
        guard(auth, Action::AdminRead, &Resource::config(), || {
            json_response(200, status_json(&self.reloader.status().current()))
        })
    }
}

/// Run `handler` only if `auth` permits `action` on `resource`; otherwise return
/// `403` from the same boundary (constitution XX).
fn guard(
    auth: &AuthContext,
    action: Action,
    resource: &Resource,
    handler: impl FnOnce() -> Response<Vec<u8>>,
) -> Response<Vec<u8>> {
    match auth.authorize(action, resource) {
        Ok(()) => handler(),
        Err(_) => json_response(
            403,
            json!({"error": {"kind": "pingogate.unauthorized", "message": "not permitted"}}),
        ),
    }
}

/// Extract the candidate YAML from the body: either a JSON `{ "config": "..." }`
/// envelope or the raw body treated as YAML directly.
fn candidate_yaml(body: &[u8]) -> String {
    if let Ok(Value::Object(map)) = serde_json::from_slice::<Value>(body) {
        if let Some(Value::String(cfg)) = map.get("config") {
            return cfg.clone();
        }
    }
    String::from_utf8_lossy(body).into_owned()
}

/// Parse and semantically validate a candidate config, returning structured
/// errors. Never resolves secrets or swaps the active snapshot.
fn validate_candidate(yaml: &str) -> Result<(), Vec<Value>> {
    let config = GatewayConfig::from_yaml(yaml).map_err(|e| error_list(&e))?;
    validate_semantics(&config).map_err(|e| error_list(&e))
}

/// Render a [`ConfigError`] as the contract's `errors` array of `{path,message}`.
fn error_list(err: &ConfigError) -> Vec<Value> {
    match err {
        ConfigError::Invalid(issues) => issues
            .iter()
            .map(|e| json!({"path": e.path, "message": e.message}))
            .collect(),
        other => vec![json!({"path": "", "message": other.to_string()})],
    }
}

/// Map the in-memory [`ReloadStatus`] to the `/reload/status` contract shape.
fn status_json(status: &ReloadStatus) -> Value {
    let last_result = match status.outcome {
        ReloadOutcome::Pending => "pending",
        ReloadOutcome::Success => "success",
        ReloadOutcome::Failure => "rejected",
    };
    let rollback_target = (status.active_version > 1).then(|| status.active_version - 1);
    let errors = match status.outcome {
        ReloadOutcome::Failure => vec![json!({"path": "", "message": status.message})],
        _ => Vec::new(),
    };
    json!({
        "last_result": last_result,
        "active_version": status.active_version,
        "rollback_target": rollback_target,
        "timestamp": status.timestamp,
        "errors": errors,
    })
}

/// Build a JSON response with an explicit `Content-Length` (the `ServeHttp`
/// writer does not frame the body, so HTTP/1.1 keep-alive clients need it).
pub(crate) fn json_response(status: u16, body: Value) -> Response<Vec<u8>> {
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
    use pingo_core::Principal;

    #[test]
    fn candidate_yaml_unwraps_json_envelope() {
        let body = br#"{"config": "listeners: {}"}"#;
        assert_eq!(candidate_yaml(body), "listeners: {}");
    }

    #[test]
    fn candidate_yaml_passes_through_raw_body() {
        let body = b"listeners:\n  public: {}";
        assert_eq!(candidate_yaml(body), "listeners:\n  public: {}");
    }

    #[test]
    fn guard_blocks_non_admin_for_every_action() {
        let ctx = AuthContext::new(Principal::gateway_key("k"));
        for action in [
            Action::AdminRead,
            Action::AdminReload,
            Action::AdminValidate,
        ] {
            let r = guard(&ctx, action, &Resource::config(), || {
                json_response(200, json!({}))
            });
            assert_eq!(r.status(), 403, "action {action:?} must be denied");
        }
    }

    #[test]
    fn status_json_maps_failure_to_rejected() {
        let status = ReloadStatus {
            active_version: 3,
            last_attempt_version: Some(4),
            outcome: ReloadOutcome::Failure,
            message: "boom".to_string(),
            timestamp: "1970-01-01T00:00:00Z".to_string(),
            attempts: 1,
        };
        let v = status_json(&status);
        assert_eq!(v["last_result"], "rejected");
        assert_eq!(v["active_version"], 3);
        assert_eq!(v["rollback_target"], 2);
        assert_eq!(v["errors"][0]["message"], "boom");
    }
}
