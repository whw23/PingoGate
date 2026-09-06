//! `pingogate-provider` - Provider namespace layer (constitution XI).
//!
//! Defines the [`ProviderAdapter`] enum and the provider-native error-body
//! renderers (`openai`/`anthropic`/`gemini`). Adapters declare their capability
//! family (`generation.stateless` this phase) and how the gateway injects
//! upstream credentials; they never touch plaintext provider keys (the pipeline
//! injects auth in `upstream_request_filter`).
//!
//! S1 is homomorphic passthrough: adapters declare auth method + error body
//! shape per protocol, they do NOT transform request bodies (that is the
//! transform crate's job). The six inbound interfaces collapse to three
//! adapters because auth + error shape are identical within each provider
//! family (see [`adapter`] module docs and constitution II/YAGNI):
//! - OpenAI Chat Completions + OpenAI Responses -> [`ProviderAdapter::OpenAi`]
//! - Anthropic Messages -> [`ProviderAdapter::Anthropic`]
//! - Gemini generateContent / streamGenerateContent / Interactions ->
//!   [`ProviderAdapter::Gemini`]

pub mod adapter;
pub mod anthropic;
mod error_shape;
pub mod gemini;
pub mod interactions;
pub mod openai;
pub mod responses;
pub mod token_source;
pub mod usage;

pub use adapter::ProviderAdapter;
pub use error_shape::ErrorClass;
pub use token_source::{CommandTokenSource, TokenError, TokenSource};
pub use usage::{parse_usage, TokenUsage};
