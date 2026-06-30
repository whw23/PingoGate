//! `pingo-storage` — storage abstraction layer.
//!
//! This phase provides only secret-reference resolution (`env:VAR`); file and
//! in-memory control-plane state live alongside it in later work. Resolved
//! secret material must never be logged in plaintext (constitution XX).

pub mod secret;

pub use secret::{EnvSecretResolver, SecretError, SecretResolver};
