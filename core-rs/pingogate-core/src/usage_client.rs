// gRPC UsageService client (T31): pushes UsageEvents from the Rust kernel to
// the Go control plane. Fire-and-forget: failures are logged and dropped
// (constitution XIX: usage is best-effort). A bounded timeout prevents hangs.
//
// Concurrency cap (T31 fix #4): a `Semaphore` bounds in-flight pushes. When
// Go is slow/dead, over-capacity events are dropped immediately (constitution
// XIX/XXI). The tonic client is `Clone` (shares the HTTP/2 channel), so no
// Mutex is needed; each `report()` clones the client for its `&mut self` call.

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use tokio::sync::Semaphore;
use tonic::transport::{Channel, ClientTlsConfig, Uri};
use tonic::service::interceptor::InterceptedService;
use tonic::{Request, Status};

use pingogate_pipeline::{NoopUsageReporter, UsageEvent, UsageReporter};

use crate::grpc::pingogate::usage_service_client::UsageServiceClient;
use crate::grpc::pingogate::UsageEvent as ProtoUsageEvent;

// gRPC metadata header for the shared internal token (spec sec 12A).
const INTERNAL_TOKEN_HEADER: &str = "x-internal-token";

// Maximum concurrent in-flight usage pushes (T31 fix #4). When at capacity,
// additional events are logged and dropped (constitution XIX: usage is best-
// effort). 16 is generous for a single-binary kernel pushing to a local Go
// process; the cap exists to prevent unbounded task pile-up when Go is slow.
const MAX_CONCURRENT_PUSHES: usize = 16;

// Token interceptor attaching the shared secret to every outgoing RPC.
// Tonic does not provide a convenient per-call metadata helper for
// client-streaming RPCs, so we use an InterceptedService.
#[derive(Clone)]
struct TokenInterceptor {
    token: String,
}

impl tonic::service::Interceptor for TokenInterceptor {
    fn call(&mut self, mut req: Request<()>) -> Result<Request<()>, Status> {
        req.metadata_mut().insert(
            INTERNAL_TOKEN_HEADER,
            self.token
                .parse()
                .map_err(|_| Status::invalid_argument("bad token"))?,
        );
        Ok(req)
    }
}

// GrpcUsageReporter pushes UsageEvents to the Go UsageService server.
// Constructed once at platform-mode startup; shared via Arc<dyn UsageReporter>.
// Each report() clones the client (cheap - shares the HTTP/2 channel) for the
// `&mut self` RPC API; no Mutex needed. A `Semaphore` caps concurrent in-flight
// pushes (T31 fix #4) preventing unbounded task pile-up when Go is slow/dead.
pub struct GrpcUsageReporter {
    client: UsageServiceClient<InterceptedService<Channel, TokenInterceptor>>,
    timeout: Duration,
    semaphore: Arc<Semaphore>,
}

impl GrpcUsageReporter {
    // Connect to the Go UsageService server at `addr` using mTLS with the
    // core cert and the shared internal token. Cert paths reuse
    // PINGO_GRPC_CERT / PINGO_GRPC_KEY / PINGO_GRPC_CA (same CA signs both
    // core and ctrl certs; dual-usage after T31).
    pub async fn connect(
        addr: &str,
        server_cert_path: &str,
        server_key_path: &str,
        client_ca_path: &str,
        internal_token: &str,
    ) -> Result<Self, Box<dyn std::error::Error>> {
        let cert = std::fs::read(server_cert_path)?;
        let key = std::fs::read(server_key_path)?;
        let ca = std::fs::read(client_ca_path)?;

        let identity = tonic::transport::Identity::from_pem(cert, key);
        let ca_cert = tonic::transport::Certificate::from_pem(ca);

        let tls = ClientTlsConfig::new()
            .identity(identity)
            .ca_certificate(ca_cert)
            .domain_name("localhost");

        let uri: Uri = format!("https://{}", addr).parse()?;
        let channel = Channel::builder(uri)
            .tls_config(tls)?
            .connect_timeout(Duration::from_secs(5))
            .timeout(Duration::from_secs(10))
            .connect()
            .await?;

        let interceptor = TokenInterceptor {
            token: internal_token.to_string(),
        };
        // with_interceptor wraps the channel so every outgoing RPC carries
        // the internal token. Equivalent to the Go client interceptors.
        let client = UsageServiceClient::with_interceptor(channel, interceptor);

        Ok(Self {
            client,
            timeout: Duration::from_secs(5),
            semaphore: Arc::new(Semaphore::new(MAX_CONCURRENT_PUSHES)),
        })
    }

    // Build the usage reporter for platform mode (T31 fix #2). Tries to
    // connect; on failure falls back to `NoopUsageReporter` (constitution XXI).
    // Extracted from main.rs to keep that file ≤300 lines (constitution V).
    pub async fn build_reporter(
        addr: &str,
        server_cert_path: &str,
        server_key_path: &str,
        client_ca_path: &str,
        internal_token: &str,
    ) -> Arc<dyn UsageReporter> {
        match Self::connect(addr, server_cert_path, server_key_path, client_ca_path, internal_token)
            .await
        {
            Ok(reporter) => {
                tracing::info!(usage_addr = %addr, "UsageService gRPC client connected");
                Arc::new(reporter)
            }
            Err(e) => {
                tracing::warn!(
                    error = %e,
                    usage_addr = %addr,
                    "UsageService gRPC client connect failed; falling back to no-op reporter"
                );
                Arc::new(NoopUsageReporter)
            }
        }
    }
}

#[async_trait]
impl UsageReporter for GrpcUsageReporter {
    async fn report(&self, event: UsageEvent) {
        // T31 fix #4: try_acquire a push permit. If at capacity, drop the
        // event immediately (constitution XIX/XXI). Prevents unbounded task
        // pile-up when Go is slow/dead.
        let permit = match self.semaphore.clone().try_acquire_owned() {
            Ok(permit) => permit,
            Err(_) => {
                tracing::warn!(
                    concurrent_cap = MAX_CONCURRENT_PUSHES,
                    "usage: push dropped (concurrent push cap reached; Go slow/dead?)"
                );
                return;
            }
        };

        let proto_event = ProtoUsageEvent {
            version: 0,
            virtual_key_id: event.virtual_key_id,
            owner_user_id: event.owner_user_id,
            provider: event.provider,
            model: event.model,
            input_tokens: event.input_tokens as i64,
            output_tokens: event.output_tokens as i64,
            reasoning_tokens: event.reasoning_tokens as i64,
            cache_read_tokens: event.cache_read_tokens as i64,
            cache_write_tokens: event.cache_write_tokens as i64,
            success: event.success,
            latency_ms: event.latency_ms as i64,
            needs_estimate: event.needs_estimate,
            body_ref: event.body_ref,
        };

        let stream = tokio_stream::iter(vec![proto_event]);

        // Clone the client (cheap - shares the HTTP/2 channel) for the
        // `&mut self` RPC API. Permit held for the RPC duration.
        let timeout_future = async {
            let mut client = self.client.clone();
            match client.report_usage(stream).await {
                Ok(_response) => {}
                Err(e) => {
                    tracing::warn!(error = %e, "usage: report_usage RPC failed");
                }
            }
        };

        match tokio::time::timeout(self.timeout, timeout_future).await {
            Ok(()) => {}
            Err(_) => {
                tracing::warn!("usage: report timed out after {:?}", self.timeout);
            }
        }
        // Release the permit (drop) - allows the next waiting push to proceed.
        drop(permit);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tonic::service::Interceptor;

    #[test]
    fn token_interceptor_attaches_header() {
        let mut interceptor = TokenInterceptor {
            token: "secret".to_string(),
        };
        let req = Request::new(());
        let result = interceptor.call(req).unwrap();
        assert_eq!(
            result.metadata().get(INTERNAL_TOKEN_HEADER).unwrap(),
            "secret"
        );
    }

    // T31 fix #3: report() returns within bounded time when Go unreachable.
    // Proves the timeout fires (not hangs) and fire-and-forget is intact.
    // Uses a lazy channel at 127.0.0.1:1 (connection refused immediately).
    #[tokio::test]
    async fn report_returns_within_bound_when_go_unreachable() {
        let reporter = GrpcUsageReporter::new_unreachable_for_test().await;
        let event = UsageEvent {
            virtual_key_id: "vk-test".to_string(),
            owner_user_id: "user-test".to_string(),
            provider: "openai".to_string(),
            model: "gpt-4o".to_string(),
            input_tokens: 10,
            output_tokens: 5,
            ..Default::default()
        };

        // Outer 10s bound; inner timeout is 1s. Hanging here = invariant broken.
        let result = tokio::time::timeout(Duration::from_secs(10), reporter.report(event)).await;
        assert!(
            result.is_ok(),
            "report() did not return within 10s (fire-and-forget invariant broken)"
        );
    }

    // T31 fix #4: when the semaphore is exhausted, report() drops the event
    // immediately (no gRPC call). Proves the cap prevents unbounded pile-up.
    #[tokio::test]
    async fn report_drops_event_when_semaphore_full() {
        let reporter = GrpcUsageReporter::new_unreachable_for_test().await;

        // Exhaust all permits so the next report() must drop.
        let mut held_permits = Vec::new();
        for _ in 0..MAX_CONCURRENT_PUSHES {
            held_permits.push(reporter.semaphore.clone().try_acquire_owned().unwrap());
        }

        let event = UsageEvent {
            provider: "openai".to_string(),
            ..Default::default()
        };
        // Should return immediately (event dropped) - well under 1s.
        let result = tokio::time::timeout(
            Duration::from_millis(500),
            reporter.report(event),
        )
        .await;
        assert!(
            result.is_ok(),
            "report() did not return immediately when semaphore was full"
        );

        drop(held_permits);
    }

    #[cfg(test)]
    impl GrpcUsageReporter {
        // Test-only constructor: lazy channel at 127.0.0.1:1 (unreachable).
        // No TLS, no connection until first RPC. Short timeouts for fast tests.
        async fn new_unreachable_for_test() -> Self {
            let uri: Uri = "http://127.0.0.1:1".parse().unwrap();
            let channel = Channel::builder(uri)
                .connect_timeout(Duration::from_millis(200))
                .timeout(Duration::from_millis(500))
                .connect_lazy();
            let interceptor = TokenInterceptor {
                token: "test".to_string(),
            };
            let client = UsageServiceClient::with_interceptor(channel, interceptor);
            Self {
                client,
                timeout: Duration::from_secs(1),
                semaphore: Arc::new(Semaphore::new(MAX_CONCURRENT_PUSHES)),
            }
        }
    }
}
