//! `pingo-config` — Runtime Config layer.
//!
//! Parses `pingogate.yaml`, validates it (schema + semantics), resolves secret
//! references, and builds the immutable [`RuntimeSnapshot`] that the data path
//! reads through an `ArcSwap` for lock-free hot reload (constitution XII).

pub mod model;
pub mod snapshot;
pub mod validate;

pub use model::GatewayConfig;
pub use snapshot::{RuntimeSnapshot, SnapshotHolder};
pub use validate::{ConfigError, ValidationError};
