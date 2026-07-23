// Usage event construction (T31; constitution XIX).
//
// Builds a pipeline-level [] from the per-request context at the
//  hook. Extracted from  to keep that file focused on the
//  stage wiring (constitution V: file <= 300 lines).
//
// The event is pushed to the Go control plane via the []
// trait.  is true when no provider usage was extracted
// (ctx.tokens is None); in that case the stashed request body is packed
// into  so the Go estimator can produce input token counts. The
// response body is never buffered (O(1) memory invariant, constitution XXI),
// so output estimation falls back to 0 (conservative).

use crate::ctx::GatewayCtx;
use crate::usage_reporter::UsageEvent;

// Build a UsageEvent from the per-request context at the logging hook.
// Returns None when no route resolved (nothing to bill). The owner_user_id
// comes from the authenticated principal (virtual key owner); in standalone
// mode it falls back to the "standalone" sentinel so Go persists.
//
// needs_estimate is true when no provider usage was extracted (ctx.tokens
// is None). In that case body_ref is populated with the stashed request
// body so the Go estimator can produce input token counts. The response
// body is not available (never buffered); the Go estimator falls back to
// 0 output tokens (conservative).
pub(crate) fn build_usage_event(ctx: &GatewayCtx, status: u16) -> Option<UsageEvent> {
    let provider = ctx.route_provider.clone()?;

    let virtual_key_id = ctx
        .authenticated_principal
        .as_ref()
        .map(|p| p.id.clone())
        .unwrap_or_default();
    let owner_user_id = ctx
        .authenticated_principal
        .as_ref()
        .and_then(|p| p.owner_user_id.clone())
        .unwrap_or_else(|| "standalone".to_string());

    let tokens = ctx.tokens.clone();
    let needs_estimate = tokens.is_none();
    let (input, output, reasoning, cache_read, cache_write) = match &tokens {
        Some(t) => (
            t.input,
            t.output,
            t.reasoning.unwrap_or(0),
            t.cache_read.unwrap_or(0),
            t.cache_write.unwrap_or(0),
        ),
        None => (0, 0, 0, 0, 0),
    };

    // body_ref: when estimating, pack the stashed request body as a JSON
    // envelope {"request": <body>} so the Go estimator can split it. The
    // response body is unavailable; the Go estimator handles a missing
    // "response" field by returning 0 output tokens (conservative).
    let body_ref = if needs_estimate {
        ctx.request_body_for_usage
            .as_deref()
            .map(|b| {
                let parsed = serde_json::from_slice::<serde_json::Value>(b)
                    .unwrap_or(serde_json::Value::Null);
                let wrapped = serde_json::json!({ "request": parsed });
                serde_json::to_vec(&wrapped).unwrap_or_default()
            })
            .unwrap_or_default()
    } else {
        Vec::new()
    };

    let latency_ms = ctx.started.elapsed().as_millis() as u64;
    let success = (200..300).contains(&status);

    Some(UsageEvent {
        virtual_key_id,
        owner_user_id,
        provider,
        model: String::new(),
        input_tokens: input,
        output_tokens: output,
        reasoning_tokens: reasoning,
        cache_read_tokens: cache_read,
        cache_write_tokens: cache_write,
        success,
        latency_ms,
        needs_estimate,
        body_ref,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ctx::GatewayCtx;
    use crate::StaticKeyAuth;
    use pingogate_core_types::{Principal, PrincipalKind};
    use pingogate_snapshot::SnapshotHolder;
    use std::sync::Arc;

    fn empty_ctx() -> GatewayCtx {
        let holder = SnapshotHolder::empty();
        let auth: Arc<dyn crate::KeyAuth> = Arc::new(StaticKeyAuth);
        GatewayCtx::new(holder.load_full(), auth)
    }

    #[test]
    fn build_usage_event_returns_none_without_route() {
        let mut ctx = empty_ctx();
        // No route_provider set; nothing to bill.
        assert!(build_usage_event(&ctx, 200).is_none());
        ctx.route_provider = Some("openai".to_string());
        let event = build_usage_event(&ctx, 200).unwrap();
        assert_eq!(event.provider, "openai");
        assert!(event.needs_estimate); // no tokens extracted
        assert_eq!(event.input_tokens, 0);
        assert_eq!(event.owner_user_id, "standalone");
    }

    #[test]
    fn build_usage_event_with_tokens_skips_estimate() {
        let mut ctx = empty_ctx();
        ctx.route_provider = Some("anthropic".to_string());
        ctx.tokens = Some(pingogate_provider::TokenUsage {
            input: 100,
            output: 50,
            reasoning: Some(5),
            cache_read: Some(10),
            cache_write: None,
        });
        let event = build_usage_event(&ctx, 200).unwrap();
        assert!(!event.needs_estimate);
        assert_eq!(event.input_tokens, 100);
        assert_eq!(event.output_tokens, 50);
        assert_eq!(event.reasoning_tokens, 5);
        assert_eq!(event.cache_read_tokens, 10);
        assert_eq!(event.cache_write_tokens, 0);
        assert!(event.body_ref.is_empty());
        assert!(event.success);
    }

    #[test]
    fn build_usage_event_uses_principal_owner() {
        let mut ctx = empty_ctx();
        ctx.route_provider = Some("openai".to_string());
        ctx.authenticated_principal = Some(Principal {
            id: "vk-123".to_string(),
            kind: PrincipalKind::VirtualKey,
            owner_user_id: Some("user-A".to_string()),
        });
        let event = build_usage_event(&ctx, 500).unwrap();
        assert_eq!(event.virtual_key_id, "vk-123");
        assert_eq!(event.owner_user_id, "user-A");
        assert!(!event.success); // status 500
    }

    #[test]
    fn build_usage_event_includes_body_ref_when_estimating() {
        let mut ctx = empty_ctx();
        ctx.route_provider = Some("openai".to_string());
        ctx.request_body_for_usage =
            Some(br#"{"messages":[{"role":"user","content":"hello"}]}"#.to_vec());
        let event = build_usage_event(&ctx, 200).unwrap();
        assert!(event.needs_estimate);
        assert!(!event.body_ref.is_empty());
        // The body should be wrapped in a JSON envelope.
        let parsed: serde_json::Value = serde_json::from_slice(&event.body_ref).unwrap();
        assert!(parsed.get("request").is_some());
    }
}
