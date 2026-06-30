//! `pingo-core` — shared domain types, error taxonomy, and the authorization
//! boundary used across every PingoGate layer.
//!
//! This crate depends on no other internal crate; it breaks dependency cycles
//! (constitution VIII) and is the single home for cross-cutting domain types.

pub mod auth;
pub mod context;
pub mod error;
pub mod redact;
pub mod secret;
pub mod time;
pub mod trace;
pub mod types;

pub use auth::{Action, AuthContext, Principal, PrincipalKind, Resource, ResourceKind};
pub use context::{RequestContext, TraceId, TRACE_HEADER};
pub use error::{AppError, ErrorLayer};
pub use redact::{redact_header_value, redact_query_key, redact_secret};
pub use secret::SecretString;
pub use time::{format_rfc3339, now_rfc3339, now_unix_secs};
pub use trace::{record_facets, request_span};
pub use types::{AuthMethod, CapabilityFamily, ProtocolKind, ProviderKind};
