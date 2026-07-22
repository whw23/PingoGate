//! `pingogate-snapshot` - Runtime Config layer.
//!
//! Parses `pingogate.yaml`, validates it (schema + semantics), resolves secret
//! references, and builds the immutable [`RuntimeSnapshot`] that the data path
//! reads through an `ArcSwap` for lock-free hot reload (constitution XII).
//!
//! Depends only on `pingogate-core-types` (cycle-breaker, constitution VIII):
//! the `SecretResolver` trait lives there so this crate does not depend on
//! `pingogate-storage`.

pub mod model;
pub mod snapshot;
pub mod validate;

pub use model::GatewayConfig;
pub use snapshot::{
    GatewayKey, ResolvedProvider, Route, RuntimeSnapshot, SnapshotHolder, UpstreamConfig,
    VirtualKeyEntry,
};
pub use validate::{ConfigError, ValidationError};
