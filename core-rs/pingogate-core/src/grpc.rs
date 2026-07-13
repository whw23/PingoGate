//! PingoGate gRPC server (S1 stub).
//!
//! Platform-mode Rust kernel listens on 127.0.0.1 for the Go control plane.
//! Transport is mTLS (mutual cert verification) and every call must carry the
//! shared `x-internal-token` metadata (spec §12A), enforced by
//! [`grpc_auth::InternalTokenInterceptor`].
//!
//! S1 returns empty `Ack`/`HeartbeatResponse`/`HealthResponse` stubs; S2
//! implements real snapshot application, S3 adds KeyVault AES-GCM and usage
//! reporting. The stubs are sufficient to prove Go -> Rust connectivity with
//! an empty snapshot push (T11).

use std::net::SocketAddr;
use std::path::PathBuf;

use tonic::service::interceptor as interceptor_layer;
use tonic::transport::{Identity, Server, ServerTlsConfig};
use tonic::{Request, Response, Status, Streaming};

use crate::grpc_auth::InternalTokenInterceptor;

/// Generated protobuf types + service traits.
pub mod pingogate {
    include!(concat!(env!("OUT_DIR"), "/pingogate.rs"));
}

use pingogate::{
    health_service_server::{HealthService, HealthServiceServer},
    key_vault_service_server::{KeyVaultService, KeyVaultServiceServer},
    snapshot_service_server::{SnapshotService, SnapshotServiceServer},
    usage_service_server::{UsageService, UsageServiceServer},
    Ack, DecryptRequest, DecryptResponse, EncryptRequest, EncryptResponse, HealthRequest,
    HealthResponse, HeartbeatRequest, HeartbeatResponse, PushDeltaRequest, Snapshot, UsageEvent,
};

/// S1 stub: accepts snapshot streams, returns Ack without applying.
pub struct SnapshotServiceImpl;

#[tonic::async_trait]
impl SnapshotService for SnapshotServiceImpl {
    /// Client-streaming: drain the incoming `Snapshot` messages and return a
    /// single `Ack`. S1 only logs the first version it sees; S2 will validate
    /// and apply via `ArcSwap`.
    async fn push_snapshot(
        &self,
        request: Request<Streaming<Snapshot>>,
    ) -> Result<Response<Ack>, Status> {
        let mut stream = request.into_inner();
        let mut count = 0u32;
        let mut first_version = 0u64;
        while let Some(msg) = stream.message().await? {
            if count == 0 {
                first_version = msg.version;
                tracing::info!(
                    version = msg.version,
                    providers = msg.providers.len(),
                    routes = msg.routes.len(),
                    encrypted_keys = msg.encrypted_keys.len(),
                    "gRPC push_snapshot: first snapshot received (S1 stub, not applied)"
                );
            }
            count += 1;
        }
        tracing::info!(count, first_version, "gRPC push_snapshot: stream closed");
        Ok(Response::new(Ack {
            version: first_version,
            ok: true,
            error: String::new(),
        }))
    }

    async fn heartbeat(
        &self,
        _req: Request<HeartbeatRequest>,
    ) -> Result<Response<HeartbeatResponse>, Status> {
        Ok(Response::new(HeartbeatResponse {
            acknowledged: true,
        }))
    }

    async fn push_delta(
        &self,
        request: Request<Streaming<PushDeltaRequest>>,
    ) -> Result<Response<Ack>, Status> {
        let mut stream = request.into_inner();
        while stream.message().await?.is_some() {}
        Ok(Response::new(Ack {
            version: 0,
            ok: true,
            error: String::new(),
        }))
    }
}

/// S1 stub: KeyVault decrypt/encrypt return NotImplemented.
pub struct KeyVaultServiceImpl;

#[tonic::async_trait]
impl KeyVaultService for KeyVaultServiceImpl {
    async fn encrypt(
        &self,
        _req: Request<EncryptRequest>,
    ) -> Result<Response<EncryptResponse>, Status> {
        Ok(Response::new(EncryptResponse {
            ciphertext: Vec::new(),
            error: "KeyVault not implemented in S1".to_string(),
        }))
    }

    async fn decrypt(
        &self,
        _req: Request<DecryptRequest>,
    ) -> Result<Response<DecryptResponse>, Status> {
        Ok(Response::new(DecryptResponse {
            plaintext: Vec::new(),
            error: "KeyVault not implemented in S1".to_string(),
        }))
    }
}

/// S1 stub: usage report drains the stream and returns Ack.
pub struct UsageServiceImpl;

#[tonic::async_trait]
impl UsageService for UsageServiceImpl {
    async fn report_usage(
        &self,
        request: Request<Streaming<UsageEvent>>,
    ) -> Result<Response<Ack>, Status> {
        let mut stream = request.into_inner();
        while stream.message().await?.is_some() {}
        Ok(Response::new(Ack {
            version: 0,
            ok: true,
            error: String::new(),
        }))
    }
}

/// S1 stub: always reports ready with version 0 (no snapshot applied yet).
pub struct HealthServiceImpl;

#[tonic::async_trait]
impl HealthService for HealthServiceImpl {
    async fn check(
        &self,
        _req: Request<HealthRequest>,
    ) -> Result<Response<HealthResponse>, Status> {
        Ok(Response::new(HealthResponse {
            ready: false,
            version: 0,
        }))
    }
}

/// Load the server identity (cert+key) and client CA from disk, bind mTLS +
/// token interceptor, and serve all four gRPC services on `addr`.
///
/// Blocks until the server is shut down. The caller is expected to run this on
/// a dedicated Tokio runtime (platform mode).
pub async fn serve_grpc(
    addr: SocketAddr,
    server_cert_path: PathBuf,
    server_key_path: PathBuf,
    client_ca_path: PathBuf,
    internal_token: String,
) -> Result<(), Box<dyn std::error::Error>> {
    let cert_pem = std::fs::read(&server_cert_path).map_err(|e| {
        format!(
            "failed to read server cert '{}': {e}",
            server_cert_path.display()
        )
    })?;
    let key_pem = std::fs::read(&server_key_path).map_err(|e| {
        format!(
            "failed to read server key '{}': {e}",
            server_key_path.display()
        )
    })?;
    let client_ca_pem = std::fs::read(&client_ca_path).map_err(|e| {
        format!(
            "failed to read client CA '{}': {e}",
            client_ca_path.display()
        )
    })?;

    let identity = Identity::from_pem(cert_pem, key_pem);
    let client_ca_root = tonic::transport::Certificate::from_pem(client_ca_pem);

    let tls = ServerTlsConfig::new()
        .identity(identity)
        .client_ca_root(client_ca_root);

    let interceptor = InternalTokenInterceptor::new(internal_token);

    tracing::info!(%addr, "gRPC server starting (mTLS + internal token)");

    Server::builder()
        .tls_config(tls)?
        .layer(interceptor_layer(interceptor))
        .add_service(SnapshotServiceServer::new(SnapshotServiceImpl))
        .add_service(KeyVaultServiceServer::new(KeyVaultServiceImpl))
        .add_service(UsageServiceServer::new(UsageServiceImpl))
        .add_service(HealthServiceServer::new(HealthServiceImpl))
        .serve(addr)
        .await?;

    Ok(())
}
