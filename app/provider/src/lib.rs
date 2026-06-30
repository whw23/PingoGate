//! `pingo-provider` — Provider namespace layer.
//!
//! Defines the [`ProviderAdapter`] trait, the [`AuthMethod`] descriptor, and the
//! provider-native error-body renderers (`openai`/`anthropic`/`gemini`). Adapters
//! declare their capability family (`generation.stateless` this phase) and never
//! touch plaintext provider keys (the core injects upstream auth in the pipeline).

pub mod adapter;
pub mod anthropic;
mod error_shape;
pub mod gemini;
pub mod openai;
pub mod usage;

pub use adapter::{AuthMethod, ProviderAdapter};
pub use error_shape::ErrorClass;
pub use usage::{parse_usage, TokenUsage};
