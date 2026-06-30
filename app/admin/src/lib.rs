//! `pingo-admin` — control plane layer.
//!
//! Implements the machine-readable Admin API (health / readiness /
//! config-validate / reload / reload-status). Every request crosses the
//! `Principal` / `AuthContext` / `authorize(action, resource)` boundary —
//! including bootstrap credentials (constitution XX). Authentication
//! ([`BootstrapAuth`]), routing/endpoints ([`AdminHandler`]), and the reload
//! orchestrator ([`Reloader`]) are separate, independently testable units.

pub mod app;
pub mod bootstrap;
pub mod handler;
pub mod metrics_endpoint;
pub mod reload;
pub mod status;

pub use app::AdminApp;
pub use bootstrap::BootstrapAuth;
pub use handler::AdminHandler;
pub use reload::Reloader;
pub use status::{ReloadOutcome, ReloadStatus, ReloadStatusStore};
