// gRPC UsageService client (T31): pushes UsageEvents from the Rust kernel to
// the Go control plane UsageService server. The Go server (T31) listens on
// 127.0.0.1:9092 with mTLS + the shared internal token, symmetric to the
// Rust gRPC server on 9091. This file implements the pipeline UsageReporter
// trait in terms of tonic::transport::Channel + the generated
// UsageServiceClient.
//
// The reporter is fire-and-forget: each report() call opens a
// client-streaming ReportUsage RPC, sends one event, and closes the stream.
// Failures are logged at WARN level and dropped (constitution XIX: usage is
// best-effort; a Go outage must not block the hot path). A bounded timeout
// prevents the gRPC call from hanging if Go is slow.

use std::time::Duration;

use async_trait::async_trait;
use tokio::sync::Mutex;
use tonic::transport::{Channel, ClientTlsConfig, Uri};
use tonic::service::interceptor::InterceptedService;
use tonic::{Request, Status};

use pingogate_pipeline::{UsageEvent, UsageReporter};

use crate::grpc::pingogate::usage_service_client::UsageServiceClient;
use crate::grpc::pingogate::UsageEvent as ProtoUsageEvent;

// gRPC metadata header for the shared internal token (spec sec 12A). Must
// match the Go server interceptor (ctrl-go/internal/grpcmtls/server_interceptor.go).
const INTERNAL_TOKEN_HEADER: &str = "x-internal-token";

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
// Constructed once at platform-mode startup; shared across all Pingora
// worker threads via Arc<dyn UsageReporter>.
//
// The underlying tonic Channel is internally multiplexed (HTTP/2) so a
// single channel handles concurrent report() calls. A Mutex guards the
// client only because tonic's client-streaming API borrows the client
// mutably; the lock is held for the duration of one send+close (bounded
// by the RPC timeout).
pub struct GrpcUsageReporter {
    client: Mutex<UsageServiceClient<InterceptedService<Channel, TokenInterceptor>>>,
    timeout: Duration,
}

impl GrpcUsageReporter {
    // Connect to the Go UsageService server at  (e.g. "127.0.0.1:9092")
    // using mTLS with the core cert (Rust presents as client) and the shared
    // internal token. Returns an error if the connection cannot be
    // established; the caller (main.rs) treats this as fatal in platform
    // mode (no usage recording = blind billing).
    //
    // Cert paths reuse PINGO_GRPC_CERT / PINGO_GRPC_KEY / PINGO_GRPC_CA: the
    // same CA signs both the core cert (Rust server identity, now also Rust
    // client identity) and the ctrl cert (Go client identity, now also Go
    // server identity). Both certs are dual-usage after T31.
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
            client: Mutex::new(client),
            timeout: Duration::from_secs(5),
        })
    }
}

#[async_trait]
impl UsageReporter for GrpcUsageReporter {
    async fn report(&self, event: UsageEvent) {
        // Convert the pipeline UsageEvent to the proto UsageEvent. Field
        // names mirror pingogate.UsageEvent (proto/pingogate.proto).
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

        // Wrap the single event in a stream. tokio_stream::iter with a
        // one-element Vec is the idiomatic way to call a client-streaming
        // RPC with a single message.
        let stream = tokio_stream::iter(vec![proto_event]);

        let timeout_future = async {
            let mut client = self.client.lock().await;
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
}
