//! Phase 7 polish (T063): end-to-end quickstart validation.
//!
//! Walks the quickstart.md verification script against a single running gateway,
//! exercising all four user stories in one narrative: data-plane passthrough and
//! its native error mirrors (US1), the authenticated admin plane (US3), a hot
//! reload that grows the route table (US2), and the metrics exposition (US4).
//! This is the executable form of the quickstart's acceptance-mapping table.
#![allow(clippy::unwrap_used, clippy::expect_used, dead_code)]

mod common;

use common::client::{self, Request};
use common::{Harness, DEFAULT_ROUTES, GATEWAY_KEY, OPENAI_UPSTREAM_KEY};

/// The bootstrap admin token the harness starts the gateway with.
const ADMIN_TOKEN: &str = "admin-secret";

/// GET `path` on the admin listener with the bootstrap admin credential.
fn admin_get(gw: &Harness, path: &str) -> client::Resp {
    client::send(
        gw.admin_port(),
        &Request::get(path).header("authorization", &format!("Bearer {ADMIN_TOKEN}")),
    )
}

/// Parse a response body as JSON.
fn json(resp: &client::Resp) -> serde_json::Value {
    serde_json::from_slice(&resp.body).expect("response body is JSON")
}

#[test]
fn quickstart_end_to_end() {
    let gw = Harness::start();
    verify_data_plane(&gw); // quickstart §4 (US1)
    verify_admin_plane(&gw); // quickstart §5 (US3)
    verify_hot_reload(&gw); // quickstart §6 (US2)
    verify_observability(&gw); // quickstart §7 (US4)
}

/// §4 — a gateway-key request is routed and forwarded with the upstream
/// credential injected, and the two error mirrors render in OpenAI shape.
fn verify_data_plane(gw: &Harness) {
    let auth = format!("Bearer {GATEWAY_KEY}");
    let ok = gw.send(
        &Request::post(
            "/v1/chat/completions",
            br#"{"model":"gpt-4o","messages":[{"role":"user","content":"hi"}]}"#,
        )
        .header("authorization", &auth),
    );
    assert_eq!(ok.status, 200, "body: {}", ok.body_str());
    // Gateway key stripped, upstream bearer injected (FR-010/FR-011).
    assert_eq!(
        ok.header("x-observed-authorization"),
        Some(format!("Bearer {OPENAI_UPSTREAM_KEY}").as_str())
    );

    // Bad gateway key → 401 in OpenAI shape (FR-013/FR-035).
    let unauth = gw.send(&Request::post(
        "/v1/chat/completions",
        br#"{"model":"gpt-4o"}"#,
    ));
    assert_eq!(unauth.status, 401);
    assert_eq!(json(&unauth)["error"]["code"], "invalid_api_key");

    // Unknown model → no-route mirror (FR-016).
    let no_route = gw.send(
        &Request::post("/v1/chat/completions", br#"{"model":"nope","messages":[]}"#)
            .header("authorization", &auth),
    );
    assert_eq!(no_route.status, 404);
    assert_eq!(json(&no_route)["error"]["code"], "model_not_found");
}

/// §5 — the admin plane answers health/readiness for an authenticated caller and
/// rejects an unauthenticated one (SC-006).
fn verify_admin_plane(gw: &Harness) {
    let health = admin_get(gw, "/healthz");
    assert_eq!(health.status, 200);
    assert_eq!(json(&health)["status"], "ok");

    let ready = admin_get(gw, "/readyz");
    assert_eq!(ready.status, 200);
    assert_eq!(json(&ready)["active_version"], 1, "initial version is 1");

    // No credential → rejected without revealing the endpoint exists.
    let anon = client::send(gw.admin_port(), &Request::get("/healthz"));
    assert_eq!(
        anon.status, 401,
        "unauthenticated admin access must be rejected"
    );
}

/// §6 — a SIGHUP reload grows the route table atomically; `/reload/status` then
/// reports success at the advanced version (SC-004).
fn verify_hot_reload(gw: &Harness) {
    let auth = format!("Bearer {GATEWAY_KEY}");
    let request = || {
        Request::post(
            "/v1/chat/completions",
            br#"{"model":"gpt-4o-mini","messages":[]}"#,
        )
        .header("authorization", &auth)
    };

    assert_eq!(
        gw.send(&request()).status,
        404,
        "alias unknown before reload"
    );

    let grown = format!(
        "{DEFAULT_ROUTES}  - alias: \"gpt-4o-mini\"\n    \
         provider: \"openai-up\"\n    upstream_model: \"gpt-4o-mini\"\n"
    );
    gw.reload_with_routes(&grown);

    assert!(
        gw.poll_until(|gw| gw.send(&request()).status == 200),
        "new route should be live after the reload"
    );
    let status = admin_get(gw, "/reload/status");
    assert_eq!(status.status, 200);
    assert_eq!(json(&status)["last_result"], "success");
    assert_eq!(
        json(&status)["active_version"],
        2,
        "reload advances the version"
    );
}

/// §7 — `/metrics` exposes labeled request counters and leaks no secret material
/// (SC-008). By now several requests have completed, so the series is present.
fn verify_observability(gw: &Harness) {
    let mut body = String::new();
    let present = gw.poll_until(|gw| {
        let resp = admin_get(gw, "/metrics");
        if resp.status != 200 {
            return false;
        }
        body = resp.body_str();
        body.contains("pingogate_requests_total{provider=\"openai-up\",capability_family=\"generation.stateless\"")
    });
    assert!(
        present,
        "request metrics never appeared; last body:\n{body}"
    );

    // No key, upstream credential, or admin token is ever exposed (FR-034).
    assert!(
        !body.contains(GATEWAY_KEY),
        "gateway key leaked into /metrics"
    );
    assert!(
        !body.contains(OPENAI_UPSTREAM_KEY),
        "upstream key leaked into /metrics"
    );
    assert!(
        !body.contains(ADMIN_TOKEN),
        "admin token leaked into /metrics"
    );
}
