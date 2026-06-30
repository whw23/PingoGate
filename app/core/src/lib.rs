//! `pingo-core` — shared domain types, error taxonomy, and the authorization
//! boundary used across every PingoGate layer.
//!
//! This crate depends on no other internal crate; it breaks dependency cycles
//! (constitution VIII) and is the single home for cross-cutting domain types.

pub mod auth;
pub mod context;
pub mod error;
pub mod secret;
pub mod types;

pub use auth::{Action, AuthContext, Principal, PrincipalKind, Resource, ResourceKind};
pub use context::{RequestContext, TraceId, TRACE_HEADER};
pub use error::{AppError, ErrorLayer};
pub use secret::SecretString;
pub use types::{AuthMethod, CapabilityFamily, ProtocolKind, ProviderKind};
