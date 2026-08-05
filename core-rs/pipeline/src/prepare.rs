// Request preparation: protocol detection, authentication, routing, and
// context commit (constitution VI).
//
// Extracted from `proxy.rs` to keep that file focused on the `ProxyHttp` stage
// wiring (constitution V: file <= 300 lines). The `prepare` function is called
// from `request_filter` to short-circuit with a mirrored error or return `None`
// to proceed with upstream dispatch. `commit_route` records the resolved route,
// streaming flag, usage extractor, and stashed request body on the per-request
// context.

use bytes::Bytes;
use pingogate_core_types::{AppError, CapabilityFamily, ProtocolKind};
use pingora::proxy::Session;
use pingora::Result;

use crate::ctx::GatewayCtx;
use crate::protocol::{self, Detected, Detection};
use crate::streaming;
use crate::upstream_peer::{expand_upstream_path, resolve_route};
use crate::usage_extractor::UsageExtractor;
use crate::wire;

// Identify, authenticate, and route the request. Returns `Some` to short-circuit
// with a mirrored error, or `None` to proceed.
pub(crate) async fn prepare(
    session: &mut Session,
    ctx: &mut GatewayCtx,
) -> Result<Option<(Option<ProtocolKind>, AppError)>> {
    let req = session.req_header();
    let method = req.method.as_str().to_string();
    let path = req
        .uri
        .path_and_query()
        .map(|pq| pq.as_str().to_string())
        .unwrap_or_default();
    let authz = wire::header_str(req, "authorization");
    let x_api_key = wire::header_str(req, "x-api-key");

    let detection = protocol::detect(&method, &path);
    let protocol = wire::detection_protocol(&detection);
    let detected: Detected = match detection {
        Detection::Unidentified => return Ok(Some((None, AppError::UnknownProtocol))),
        Detection::UnsupportedCapability { family, .. } => {
            if let Some(err) = ctx.authenticate(authz.as_deref(), x_api_key.as_deref()) {
                return Ok(Some((protocol, err)));
            }
            return Ok(Some((protocol, AppError::UnsupportedCapability { family })));
        }
        Detection::Supported(detected) => detected,
    };

    if let Some(err) = ctx.authenticate(authz.as_deref(), x_api_key.as_deref()) {
        return Ok(Some((protocol, err)));
    }

    let (model, body) = match wire::extract_model_and_body(session, detected.protocol, &path).await
    {
        Ok(pair) => pair,
        Err(err) => return Ok(Some((protocol, err))),
    };
    let Some(alias) = model else {
        let err = AppError::Validation {
            message: "request is missing the 'model' field".to_string(),
        };
        return Ok(Some((protocol, err)));
    };

    match resolve_route(&ctx.snapshot, &alias) {
        Ok(resolution) => {
            commit_route(ctx, detected, resolution, body);
            Ok(None)
        }
        Err(err) => Ok(Some((protocol, err))),
    }
}

// Record the resolved route, streaming flag, and usage extractor on the
// request context. For OpenAI Chat streaming, pre-computes the injected body
// (T28: `stream_options.include_usage`) so `upstream_request_filter` can set
// `Content-Length` before headers ship (Pingora sends headers before body).
pub(crate) fn commit_route(
    ctx: &mut GatewayCtx,
    detected: Detected,
    resolution: crate::upstream_peer::RouteResolution,
    body: Option<Vec<u8>>,
) {
    ctx.streaming =
        streaming::request_is_streaming(detected.streaming_by_path, body.as_deref().unwrap_or(b""));
    ctx.protocol = Some(detected.protocol);
    ctx.request.protocol = Some(detected.protocol);
    ctx.request.provider = Some(resolution.provider.clone());
    ctx.request.capability_family = Some(CapabilityFamily::GenerationStateless);
    ctx.route_provider = Some(resolution.provider);
    ctx.route_model = Some(resolution.upstream_model.clone());
    ctx.upstream = Some(resolution.target);
    ctx.upstream_proxy = resolution.upstream_proxy;
    ctx.upstream_timeout_ms = resolution.timeout_ms;
    // Expand the upstream_path template if configured (issue 2).
    ctx.upstream_path_rewrite = resolution
        .upstream_path
        .as_deref()
        .map(|tmpl| expand_upstream_path(tmpl, &resolution.upstream_model));
    ctx.route_auth_override = resolution.auth_override;
    ctx.route_kind = Some(resolution.kind);
    ctx.route_anthropic_version = resolution.anthropic_version;
    ctx.usage_extractor = Some(UsageExtractor::new(detected.protocol));
    if ctx.streaming && detected.protocol == ProtocolKind::OpenAiCompatible {
        if let Some(b) = body.as_deref() {
            let injected = pingogate_transform::inject_include_usage(
                b,
                ProtocolKind::OpenAiCompatible,
                true,
            );
            if injected != b {
                ctx.injected_request_body = Some(Bytes::from(injected));
            }
        }
    }
    // Stash the request body for usage estimation. Only the bytes needed
    // for estimation are retained; the response body is never buffered
    // (O(1) memory invariant, constitution XXI). Cleared after the push.
    ctx.request_body_for_usage = body;
}
