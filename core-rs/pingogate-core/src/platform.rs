//! Platform-mode bootstrap (constitution XII: platform = standalone data plane
//! + Go control plane over gRPC).
//!
//! Constructs three subsystems and wires them together:
//! 1. **gRPC server** (mTLS + internal token, spec §12A): receives snapshot
//!    pushes + KeyVault Encrypt/Decrypt calls from Go. Runs on a dedicated
//!    background thread with its own multi-threaded tokio runtime.
//! 2. **Pingora data plane** (public + admin listeners): the request pipeline
//!    (VirtualKeyAuth -> KeyVault decrypt -> upstream inject -> passthrough ->
//!    usage extract). Pingora owns the main thread and creates its own runtime.
//! 3. **Usage reporter**: a tonic Channel to the Go UsageService, built on the
//!    gRPC runtime. The data plane calls `report()` from Pingora's runtime;
//!    tonic Channels are `Send + Sync` and dispatch RPCs across runtimes (the
//!    background connection driver stays on the gRPC runtime).
//!
//! KeyVault sharing: the gRPC `KeyVaultGrpcService` holds an
//! `Arc<AesGcmKeyVault>` for Encrypt/Decrypt; the hot path holds a second
//! `AesGcmKeyVault` constructed from the same `PINGO_MKEK`. `AesGcmKeyVault` is
//! stateless after construction (constitution XX: AES-GCM cipher is `Sync` and
//! deterministic from the master key), so two instances with the same mkek are
//! functionally identical - ciphertext from one decrypts with the other.

use std::net::SocketAddr;
use std::path::PathBuf;
use std::str::FromStr;
use std::sync::Arc;

use pingogate_listener::{
    build_admin_service, build_public_service, AdminServiceConfig, PublicServiceConfig,
    Reloader, ReloadStatusStore,
};
use pingogate_pipeline::{KeyAuth, Metrics, VirtualKeyAuth};
use pingogate_snapshot::{RuntimeSnapshot, SnapshotHolder};
use pingogate_storage::{AesGcmKeyVault as StorageKv, GrpcSnapshotSource, KeyVault as KeyVaultTrait};
use pingora::server::Server;

use crate::grpc;
use crate::usage_client;

/// Env var: shared gRPC internal token (spec §12A). Required in platform mode.
const INTERNAL_TOKEN_ENV: &str = "PINGO_INTERNAL_TOKEN";
/// Env var: path to the server (Rust) TLS certificate PEM (platform mode).
const GRPC_CERT_ENV: &str = "PINGO_GRPC_CERT";
/// Env var: path to the server (Rust) TLS private key PEM (platform mode).
const GRPC_KEY_ENV: &str = "PINGO_GRPC_KEY";
/// Env var: path to the CA certificate PEM that signed the Go client cert.
const GRPC_CA_ENV: &str = "PINGO_GRPC_CA";
/// Env var: 32-byte AES-GCM master key (constitution XX KeyVault). Required
/// in platform mode; missing or not exactly 32 bytes is fatal.
const MKEK_ENV: &str = "PINGO_MKEK";
/// Default gRPC listen address when `--grpc-addr` is not given (platform mode).
const DEFAULT_GRPC_ADDR: &str = "127.0.0.1:9091";
/// Env var: Go UsageService gRPC address (T31). Defaults to 127.0.0.1:9092.
const USAGE_GRPC_ADDR_ENV: &str = "PINGO_USAGE_GRPC_ADDR";
/// Default Go UsageService gRPC address (T31).
const DEFAULT_USAGE_GRPC_ADDR: &str = "127.0.0.1:9092";
/// Env var: public data-plane listen address (platform mode). Standalone reads
/// this from the YAML config; platform mode has no YAML so it comes from env.
const PUBLIC_ADDR_ENV: &str = "PINGO_PUBLIC_ADDR";
/// Default public data-plane address.
const DEFAULT_PUBLIC_ADDR: &str = "0.0.0.0:8080";
/// Env var: admin API listen address (platform mode).
const ADMIN_ADDR_ENV: &str = "PINGO_ADMIN_ADDR";
/// Default admin API address.
const DEFAULT_ADMIN_ADDR: &str = "127.0.0.1:9090";

/// Platform mode: start the gRPC server (mTLS + internal token) on a background
/// thread, then start the Pingora data plane (public + admin) on the main
/// thread. The two share configuration via the `SnapshotHolder` (ArcSwap) and
/// the master key (`PINGO_MKEK`), but run on separate tokio runtimes (Pingora
/// owns the main runtime; gRPC owns a background runtime). Constitution XII:
/// platform mode = standalone data plane + Go control plane.
pub(crate) fn run_platform() -> Result<(), Box<dyn std::error::Error>> {
    // rustls 0.23 requires a process-wide CryptoProvider. Install the `ring`
    // provider before tonic's TLS stack touches rustls (idempotent).
    let _ = rustls::crypto::ring::default_provider().install_default();

    let internal_token = std::env::var(INTERNAL_TOKEN_ENV)
        .map_err(|_| format!("{INTERNAL_TOKEN_ENV} is required in platform mode (spec §12B)"))?;
    if internal_token.is_empty() {
        return Err(format!("{INTERNAL_TOKEN_ENV} must not be empty").into());
    }

    let server_cert = PathBuf::from(
        std::env::var(GRPC_CERT_ENV)
            .map_err(|_| format!("{GRPC_CERT_ENV} is required in platform mode"))?,
    );
    let server_key = PathBuf::from(
        std::env::var(GRPC_KEY_ENV)
            .map_err(|_| format!("{GRPC_KEY_ENV} is required in platform mode"))?,
    );
    let client_ca = PathBuf::from(
        std::env::var(GRPC_CA_ENV)
            .map_err(|_| format!("{GRPC_CA_ENV} is required in platform mode"))?,
    );

    // Load the 32-byte master key (PINGO_MKEK). Missing/wrong length is fatal.
    let mkek = load_master_key()?;

    // Platform-mode snapshot holder (version 0 sentinel). Constructed BEFORE
    // the KeyVault service: the S3 dual-defense (constitution XX) requires
    // the KeyVault to read `key_owners` from the live snapshot on each
    // `view_plaintext` Decrypt (AWS KMS pattern: Rust holds the owner
    // mapping independently of Go).
    let holder = Arc::new(SnapshotHolder::empty());
    let snapshot_source = Arc::new(GrpcSnapshotSource::new(holder.clone()));

    // AES-GCM KeyVault + gRPC service. `SnapshotOwnerLookup` gives the
    // KeyVault independent read access to `key_owners` (dual-defense).
    let keyvault = Arc::new(pingogate_keyvault::AesGcmKeyVault::from_master_key(&mkek));
    let owner_lookup: Arc<dyn pingogate_keyvault::OwnerLookup> =
        Arc::new(grpc::SnapshotOwnerLookup::new(holder.clone()));
    let keyvault_service = pingogate_keyvault::KeyVaultGrpcService::new(keyvault, owner_lookup);

    let addr_str = grpc_addr_from_args();
    let addr = SocketAddr::from_str(&addr_str)
        .map_err(|e| format!("invalid gRPC address '{addr_str}': {e}"))?;

    // T31: build the UsageService gRPC client (Rust -> Go) on the gRPC runtime.
    // The tonic Channel is created here (on this runtime) but called from
    // Pingora's runtime - Channel is Send + Sync and dispatches RPCs across
    // runtimes (background connection driver stays on this runtime).
    let usage_addr = std::env::var(USAGE_GRPC_ADDR_ENV)
        .unwrap_or_else(|_| DEFAULT_USAGE_GRPC_ADDR.to_string());
    let usage_cert = server_cert.to_string_lossy().to_string();
    let usage_key = server_key.to_string_lossy().to_string();
    let usage_ca = client_ca.to_string_lossy().to_string();
    let usage_token = internal_token.clone();

    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;

    let usage_reporter = runtime.block_on(async move {
        usage_client::GrpcUsageReporter::build_reporter(
            &usage_addr,
            &usage_cert,
            &usage_key,
            &usage_ca,
            &usage_token,
        )
        .await
    });

    // Spawn the gRPC server as a background task on this runtime, then move
    // the runtime to a dedicated thread so it keeps polling the task. The
    // data plane (Pingora) runs on the main thread with its own runtime.
    // Clone `snapshot_source` for the Reloader before moving the original
    // into the gRPC task (the GrpcSnapshotSource is shared via Arc).
    let reloader_source: Arc<dyn pingogate_storage::SnapshotSource<Snapshot = RuntimeSnapshot>> =
        snapshot_source.clone();
    let grpc_handle = runtime.spawn(async move {
        if let Err(e) = grpc::serve_grpc(
            addr,
            server_cert,
            server_key,
            client_ca,
            internal_token,
            keyvault_service,
            snapshot_source,
        )
        .await
        {
            tracing::error!(error = %e, "gRPC server failed");
        }
    });

    let grpc_thread = std::thread::Builder::new()
        .name("pingogate-grpc".to_string())
        .spawn(move || {
            // block_on the JoinHandle so the runtime keeps polling until the
            // gRPC server exits. The runtime (and its tonic background tasks)
            // stays alive for the lifetime of the gRPC server.
            let _ = runtime.block_on(grpc_handle);
        })
        .map_err(|e| format!("failed to spawn gRPC thread: {e}"))?;
    tracing::info!(thread = ?grpc_thread.thread().name(), "gRPC server thread spawned");

    // --- Data plane (Pingora, main thread) ---

    // Hot-path KeyVault: second AesGcmKeyVault from the same mkek (stateless
    // after construction, so functionally identical to the gRPC instance).
    let hotpath_keyvault: Arc<dyn KeyVaultTrait> =
        Arc::new(StorageKv(pingogate_keyvault::AesGcmKeyVault::from_master_key(&mkek)));

    // VirtualKeyAuth: SHA-256 hash + virtual-key lookup + concurrency quota.
    let auth: Arc<dyn KeyAuth> = Arc::new(VirtualKeyAuth::new());

    // R1: admin token (shared with standalone). Missing = authed endpoints reject.
    let admin_token = crate::read_admin_token();
    if admin_token.is_none() {
        tracing::warn!(
            env = crate::ADMIN_TOKEN_ENV,
            "admin token not set; authed admin endpoints will reject (platform mode R1)"
        );
    }

    // Reloader for platform mode: the GrpcSnapshotSource reads from the holder
    // (already updated by gRPC push). SIGHUP in platform mode rebuilds from
    // the current holder (a no-op when no new push has arrived, but harmless
    // and keeps the standalone/platform signal path uniform). Admin /reload
    // API works the same way.
    let status = Arc::new(ReloadStatusStore::new(1));
    let reloader = Arc::new(Reloader::new(holder.clone(), status, reloader_source));
    crate::signal::spawn_sighup_handler(reloader.clone());

    let public_addr = std::env::var(PUBLIC_ADDR_ENV)
        .unwrap_or_else(|_| DEFAULT_PUBLIC_ADDR.to_string());
    let admin_addr = std::env::var(ADMIN_ADDR_ENV)
        .unwrap_or_else(|_| DEFAULT_ADMIN_ADDR.to_string());

    let mut server = Server::new(None)?;
    server.bootstrap();

    let metrics = Arc::new(Metrics::new());

    let public = build_public_service(PublicServiceConfig {
        conf: &server.configuration,
        holder: holder.clone(),
        metrics: metrics.clone(),
        auth,
        keyvault: Some(hotpath_keyvault),
        usage: Some(usage_reporter),
        address: &public_addr,
    });
    let admin = build_admin_service(AdminServiceConfig {
        admin_token,
        holder,
        reloader,
        metrics,
        address: admin_addr.clone(),
    });
    server.add_service(public);
    server.add_service(admin);

    tracing::info!(
        public = %public_addr,
        admin = %admin_addr,
        "pingogate-core starting (platform mode: gRPC + Pingora data plane)"
    );
    server.run_forever()
}

/// Load the 32-byte AES-GCM master key (`PINGO_MKEK`) from the environment
/// (platform mode). Missing, empty, or not exactly 32 bytes is fatal. No
/// secret material in the error value (constitution XX).
fn load_master_key() -> Result<[u8; 32], Box<dyn std::error::Error>> {
    let raw = std::env::var(MKEK_ENV).map_err(|_| {
        format!("{MKEK_ENV} is required in platform mode (32-byte AES-GCM master key)")
    })?;
    let bytes = raw.into_bytes();
    if bytes.len() != 32 {
        return Err(format!(
            "{MKEK_ENV} must be exactly 32 bytes, got {} bytes",
            bytes.len()
        )
        .into());
    }
    let mut mkek = [0u8; 32];
    mkek.copy_from_slice(&bytes);
    Ok(mkek)
}

/// Resolve `--grpc-addr <addr>` / `--grpc-addr=<addr>`, defaulting to
/// `127.0.0.1:9091` (spec §12A: loopback only). Used in platform mode.
fn grpc_addr_from_args() -> String {
    crate::arg_value("--grpc-addr").unwrap_or_else(|| DEFAULT_GRPC_ADDR.to_string())
}
