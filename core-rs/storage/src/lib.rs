//! `pingogate-storage` - storage abstraction layer.
//!
//! This crate owns three concerns for the Rust kernel:
//! - [`secret::SecretResolver`] / [`secret::EnvSecretResolver`]: resolves opaque
//!   secret references (e.g. `env:VAR`) to [`SecretString`] material at
//!   snapshot-build time. The `SecretResolver` trait lives in
//!   `pingogate-core-types` (cycle-breaker); this crate ships the env impl.
//!   Resolved material must never be logged in plaintext (constitution XX).
//! - [`snapshot_source::SnapshotSource`]: abstracts how a `RuntimeSnapshot` is
//!   built - standalone file source (S1, [`FileSnapshotSource`]) vs platform
//!   gRPC source (S2).
//! - [`keyvault::KeyVault`]: provider-key encryption/decryption boundary. S1
//!   ships [`keyvault::StubKeyVault`]; S2 replaces it with an AES-GCM impl.

pub mod keyvault;
pub mod secret;
pub mod snapshot_source;

pub use keyvault::{AesGcmKeyVault, KeyError, KeyVault, StubKeyVault};
pub use secret::{EnvSecretResolver, SecretError, SecretResolver};
pub use snapshot_source::{FileSnapshotSource, GrpcSnapshotSource, SnapshotError, SnapshotSource};
