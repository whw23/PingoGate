//! OpenAI Responses API surface (`/v1/responses`, `ProtocolKind::OpenAiResponses`).
//!
//! S1 is homomorphic passthrough: the Responses API uses the same
//! `Authorization: Bearer <key>` auth and the same `{"error": {"message",
//! "type", "code"}}` error envelope as Chat Completions. Per constitution II
//! (YAGNI - do not duplicate when auth + error shape are identical), the
//! Responses surface shares [`ProviderAdapter::OpenAi`] rather than getting its
//! own adapter variant. This module re-exports the shared error renderer so
//! callers that dispatch by protocol can name a module per surface.
//!
//! What IS protocol-specific - and therefore lives in [`crate::usage`] under a
//! dedicated `parse_openai_responses` branch - is the usage block shape:
//! Responses reports `input_tokens` / `output_tokens` (with
//! `input_tokens_details` / `output_tokens_details`), whereas Chat Completions
//! reports `prompt_tokens` / `completion_tokens`.

pub use crate::openai::render_error;
