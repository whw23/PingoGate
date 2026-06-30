//! Pingora wire helpers for the proxy pipeline.
//!
//! Thin adapters between our pure pipeline logic and Pingora's `Session` /
//! header types: reading headers, draining the request body, injecting upstream
//! credentials, and writing mirrored error responses. Kept separate so
//! `proxy.rs` stays focused on the `ProxyHttp` stage wiring.

use bytes::Bytes;
use pingo_core::{AppError, ProtocolKind};
use pingora::http::{RequestHeader, ResponseHeader};
use pingora::proxy::Session;
use pingora::Result;

use crate::protocol::Detection;
use crate::upstream_auth::{self, UpstreamAuth};
use crate::{error_response, router};

/// Reject request bodies larger than this before buffering them for routing.
pub const MAX_BODY_BYTES: usize = 4 * 1024 * 1024;

/// Read a request header value as an owned `String`.
pub fn header_str(req: &RequestHeader, name: &str) -> Option<String> {
    req.headers
        .get(name)
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string())
}

/// The protocol to render an error in, if the request was identified at all.
pub fn detection_protocol(detection: &Detection) -> Option<ProtocolKind> {
    match detection {
        Detection::Supported(d) => Some(d.protocol),
        Detection::UnsupportedCapability { protocol, .. } => Some(*protocol),
        Detection::Unidentified => None,
    }
}

/// Stable lowercase label for a protocol, for structured logs.
pub fn protocol_label(p: ProtocolKind) -> &'static str {
    match p {
        ProtocolKind::OpenAiCompatible => "openai",
        ProtocolKind::Anthropic => "anthropic",
        ProtocolKind::Gemini => "gemini",
    }
}

/// Extract the model alias presented by the client, draining the body when the
/// protocol carries the model there (OpenAI/Anthropic). Retry buffering is
/// enabled first so Pingora captures the drained body and replays it upstream
/// verbatim — the H1 proxy never re-invokes `request_body_filter` for a body
/// already drained here, so the native retry-buffer replay is what forwards it.
/// The returned body is used only for streaming detection (`stream: true`).
pub async fn extract_model_and_body(
    session: &mut Session,
    protocol: ProtocolKind,
    path: &str,
) -> std::result::Result<(Option<String>, Option<Vec<u8>>), AppError> {
    match protocol {
        ProtocolKind::Gemini => Ok((router::model_from_gemini_path(path), None)),
        _ => {
            session.enable_retry_buffering();
            let body = read_full_body(session).await?;
            Ok((router::model_from_body(&body), Some(body)))
        }
    }
}

/// Drain the whole request body for model routing. Rejects bodies past the
/// retry-buffer limit (those cannot be replayed upstream, so forwarding them
/// would hang) and past [`MAX_BODY_BYTES`] as a hard memory backstop.
async fn read_full_body(session: &mut Session) -> std::result::Result<Vec<u8>, AppError> {
    let mut buf = Vec::new();
    while let Some(chunk) = session
        .read_request_body()
        .await
        .map_err(|_| AppError::Internal {
            message: "failed to read request body".to_string(),
        })?
    {
        if session.retry_buffer_truncated() {
            return Err(AppError::Validation {
                message: "request body exceeds the 64KiB model-routing limit".to_string(),
            });
        }
        if buf.len() + chunk.len() > MAX_BODY_BYTES {
            return Err(AppError::Validation {
                message: "request body exceeds the maximum size".to_string(),
            });
        }
        buf.extend_from_slice(&chunk);
    }
    Ok(buf)
}

/// Apply the upstream credential to the request header (or path, for query keys).
pub fn apply_upstream_auth(
    req: &mut RequestHeader,
    auth: UpstreamAuth,
    path: &mut String,
) -> Result<()> {
    match auth {
        UpstreamAuth::BearerHeader(value) => {
            req.insert_header("authorization", value.as_str())?;
        }
        UpstreamAuth::ApiKeyHeader { key, version } => {
            req.insert_header("x-api-key", key.as_str())?;
            if let Some(version) = version {
                req.insert_header("anthropic-version", version.as_str())?;
            }
        }
        UpstreamAuth::QueryKey(key) => {
            *path = upstream_auth::append_query_key(path, &key);
        }
    }
    Ok(())
}

/// Write a mirrored, protocol-shaped error response to the downstream (IX).
pub async fn respond_rendered(
    session: &mut Session,
    protocol: Option<ProtocolKind>,
    err: &AppError,
) -> Result<()> {
    let (status, body) = error_response::render(protocol, err);
    let mut header = ResponseHeader::build(status, Some(2))?;
    header.insert_header("content-type", "application/json")?;
    header.insert_header("content-length", body.len().to_string())?;
    session
        .write_response_header(Box::new(header), false)
        .await?;
    session
        .write_response_body(Some(Bytes::from(body)), true)
        .await?;
    Ok(())
}
