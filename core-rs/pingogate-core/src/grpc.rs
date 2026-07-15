//! PingoGate gRPC server (S2: KeyVault + Snapshot real; Usage stubbed).
//!
//! Platform-mode Rust kernel listens on 127.0.0.1 for the Go control plane.
//! Transport is mTLS (mutual cert verification) and every call must carry the
//! shared `x-internal-token` metadata (spec §12A), enforced by
//! [`grpc_auth::InternalTokenInterceptor`].
//!
//! S2 implements the real `KeyVaultService` (AES-GCM Encrypt/Decrypt via
//! [`KeyVaultGrpcService`], backed by `PINGO_MKEK`) and the real
//! `SnapshotService` (client-streaming `PushSnapshot` -> proto `Snapshot` ->
//! [`RuntimeSnapshot`] -> `ArcSwap`). `UsageService` remains a stub pending S3
//! usage reporting. `HealthService` reports the active snapshot version and
//! `ready: false` until the first snapshot is applied (spec §12B).

#[path = "grpc_convert.rs"]
mod grpc_convert;

use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;

use pingogate_snapshot::SnapshotHolder;
use pingogate_storage::GrpcSnapshotSource;
use tonic::service::interceptor as interceptor_layer;
use tonic::transport::{Identity, Server, ServerTlsConfig};
use tonic::{Request, Response, Status, Streaming};

use crate::grpc_auth::InternalTokenInterceptor;

/// Generated protobuf types + service traits.
pub mod pingogate {
    include!(concat!(env!("OUT_DIR"), "/pingogate.rs"));
}

use grpc_convert::build_runtime_snapshot;
use pingogate::{
    health_service_server::{HealthService, HealthServiceServer},
    snapshot_service_server::{SnapshotService, SnapshotServiceServer},
    usage_service_server::{UsageService, UsageServiceServer},
    Ack, HealthRequest, HealthResponse, HeartbeatRequest, HeartbeatResponse, PushDeltaRequest,
    Snapshot, UsageEvent,
};
use pingogate_keyvault::{
    proto::key_vault_service_server::KeyVaultServiceServer, KeyVaultGrpcService,
};

/// Real S2 `SnapshotService`: drains the client-streaming `PushSnapshot` flow,
/// converts each proto `Snapshot` into a [`RuntimeSnapshot`] (providers carry
/// AES-GCM ciphertext in `ResolvedProvider::encrypted_key`), and atomically
/// swaps it into the [`SnapshotHolder`] via [`GrpcSnapshotSource::apply`].
pub struct SnapshotServiceImpl {
    source: Arc<GrpcSnapshotSource>,
}

impl SnapshotServiceImpl {
    pub fn new(source: Arc<GrpcSnapshotSource>) -> Self {
        Self { source }
    }
}

#[tonic::async_trait]
impl SnapshotService for SnapshotServiceImpl {
    /// Client-streaming: drain the incoming `Snapshot` messages, apply each via
    /// `ArcSwap`, and return a single `Ack` with the last applied version. A
    /// stream with zero messages is a no-op and returns `ok: true, version: 0`.
    async fn push_snapshot(
        &self,
        request: Request<Streaming<Snapshot>>,
    ) -> Result<Response<Ack>, Status> {
        let mut stream = request.into_inner();
        let mut last_version = 0u64;
        while let Some(msg) = stream.message().await? {
            last_version = msg.version;
            let snapshot = build_runtime_snapshot(&msg)?;
            tracing::info!(
                version = msg.version,
                providers = msg.providers.len(),
                routes = msg.routes.len(),
                encrypted_keys = msg.encrypted_keys.len(),
                "gRPC push_snapshot: applying snapshot"
            );
            self.source.apply(snapshot);
        }
        tracing::info!(last_version, "gRPC push_snapshot: stream closed");
        Ok(Response::new(Ack {
            version: last_version,
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
        // Delta push is reserved (spec §12C); drain and ack. S2/S3 do not
        // implement incremental snapshots - the Go control plane re-pushes the
        // full snapshot when the delta trigger fires.
        let mut stream = request.into_inner();
        while stream.message().await?.is_some() {}
        Ok(Response::new(Ack {
            version: 0,
            ok: true,
            error: String::new(),
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

/// S2 `HealthService`: reports `ready: false` until the first snapshot is
/// applied (spec §12B). Readiness is derived from the active snapshot version:
/// `version == 0` means the holder still holds the empty sentinel from
/// `SnapshotHolder::empty`, i.e. no Go push has been applied yet.
pub struct HealthServiceImpl {
    holder: Arc<SnapshotHolder>,
}

impl HealthServiceImpl {
    pub fn new(holder: Arc<SnapshotHolder>) -> Self {
        Self { holder }
    }
}

#[tonic::async_trait]
impl HealthService for HealthServiceImpl {
    async fn check(
        &self,
        _req: Request<HealthRequest>,
    ) -> Result<Response<HealthResponse>, Status> {
        let snap = self.holder.load();
        let ready = snap.version > 0;
        Ok(Response::new(HealthResponse {
            ready,
            version: snap.version,
        }))
    }
}

/// Load the server identity (cert+key) and client CA from disk, bind mTLS +
/// token interceptor, and serve all four gRPC services on `addr`.
///
/// `keyvault_service` is the real S2 AES-GCM KeyVault (constructed in `main`
/// from `PINGO_MKEK`). `snapshot_source` carries the [`SnapshotHolder`] that
/// the `SnapshotService` swaps into and the `HealthService` reads from.
/// `UsageService` remains an S1 stub (S3).
///
/// Blocks until the server is shut down. The caller is expected to run this on
/// a dedicated Tokio runtime (platform mode).
pub async fn serve_grpc(
    addr: SocketAddr,
    server_cert_path: PathBuf,
    server_key_path: PathBuf,
    client_ca_path: PathBuf,
    internal_token: String,
    keyvault_service: KeyVaultGrpcService,
    snapshot_source: Arc<GrpcSnapshotSource>,
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

    let holder = snapshot_source.holder().clone();
    tracing::info!(%addr, "gRPC server starting (mTLS + internal token)");

    Server::builder()
        .tls_config(tls)?
        .layer(interceptor_layer(interceptor))
        .add_service(SnapshotServiceServer::new(SnapshotServiceImpl::new(
            snapshot_source,
        )))
        .add_service(KeyVaultServiceServer::new(keyvault_service))
        .add_service(UsageServiceServer::new(UsageServiceImpl))
        .add_service(HealthServiceServer::new(HealthServiceImpl::new(holder)))
        .serve(addr)
        .await?;

    Ok(())
}

#[cfg(test)]
#[path = "grpc_tests.rs"]
mod tests;
