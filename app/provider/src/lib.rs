//! `pingo-provider` — Provider namespace layer.
//!
//! Defines the [`ProviderAdapter`] trait, the [`AuthMethod`] descriptor, and
//! provider-native error-body shapes. Adapters declare their capability family
//! (`generation.stateless` this phase) and never touch plaintext provider keys
//! (the core injects upstream auth in the pipeline).

pub mod adapter;

pub use adapter::{AuthMethod, ProviderAdapter};
