//! `pingo-admin` — control plane layer.
//!
//! Implements the machine-readable Admin API (health / readiness /
//! config-validate / reload / reload-status). Every request crosses the
//! `Principal` / `AuthContext` / `authorize(action, resource)` boundary —
//! including bootstrap credentials (constitution XX). Endpoint handlers and
//! the reload orchestrator are added in User Story 2/3.
