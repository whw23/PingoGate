//! Gemini Interactions API surface (`ProtocolKind::GeminiInteractions`).
//!
//! S1 is homomorphic passthrough: the Interactions API uses the same
//! `x-goog-api-key` / `?key=` auth and the same
//! `{"error": {"code", "message", "status"}}` error envelope as
//! `generateContent` / `streamGenerateContent`. Per constitution II (YAGNI - do
//! not duplicate when auth + error shape are identical), the Interactions
//! surface shares [`ProviderAdapter::Gemini`] rather than getting its own
//! adapter variant. This module re-exports the shared error renderer so callers
//! that dispatch by protocol can name a module per surface.
//!
//! Usage extraction assumes the same `usageMetadata` shape as Gemini
//! `generateContent`; [`crate::usage::parse_usage`] routes
//! `GeminiInteractions` through the same `parse_gemini` branch. If the
//! Interactions API diverges upstream, the parser returns `None` (graceful
//! degradation) and a dedicated branch can be added then.

pub use crate::gemini::render_error;
