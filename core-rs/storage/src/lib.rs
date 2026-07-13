//! `pingogate-storage` - storage abstraction layer.
//!
//! This crate owns three concerns for the Rust kernel:
//! - [`secret::SecretResolver`] / [`secret::EnvSecretResolver`]: resolves opaque
//!   secret references (e.g. `env:VAR`) to [`SecretString`] material at
//!   snapshot-build time. Resolved material must never be logged in plaintext
//!   (constitution XX).
//! - [`snapshot_source::SnapshotSource`]: abstracts how a `RuntimeSnapshot` is
//!   built - standalone file source (S1) vs platform gRPC source (S2).
//! - [`keyvault::KeyVault`]: provider-key encryption/decryption boundary. S1
//!   ships [`keyvault::StubKeyVault`]; S2 replaces it with an AES-GCM impl.
//!
//! The full [`snapshot_source::FileSnapshotSource`] implementation is deferred
//! to T5, which introduces the `RuntimeSnapshot` type in `pingogate-snapshot`.

pub mod keyvault;
pub mod secret;
pub mod snapshot_source;

pub use keyvault::{KeyError, KeyVault, StubKeyVault};
pub use secret::{EnvSecretResolver, SecretError, SecretResolver};
pub use snapshot_source::{FileSnapshotSource, SnapshotError, SnapshotSource};
