//! S1 contract: these tests MUST pass before S2 starts.
//!
//! They verify the trait signatures, six-interface detection, and KeyVault
//! stub behavior that S2 relies on. S2's entry check (T13) runs this file
//! plus the Go connectivity contract to confirm S1's output is consumable.
//!
//! These are compile-time + unit assertions (no running server needed) so
//! they run in the standard `cargo test` suite. The gRPC connectivity
//! contract (Go client <-> Rust server) lives in
//! `ctrl-go/internal/snapshot/client_test.go` and requires a running Rust
//! gRPC server, so it is a separate, opt-in check.

#![cfg(test)]

use pingogate_core_types::ProtocolKind;

// --- Trait existence (compile-time: if these compile, the traits exist with
// the expected associated types / methods). ---

#[test]
fn snapshot_source_trait_exists() {
    fn _assert_trait<T: pingogate_storage::SnapshotSource>() {}
    _assert_trait::<pingogate_storage::FileSnapshotSource>();
}

#[test]
fn key_auth_trait_exists() {
    fn _assert_trait<T: pingogate_pipeline::KeyAuth>() {}
    _assert_trait::<pingogate_pipeline::StaticKeyAuth>();
}

#[test]
fn keyvault_trait_exists_and_stub_not_implemented() {
    use pingogate_storage::KeyVault;
    let kv = pingogate_storage::StubKeyVault;
    assert!(kv.encrypt(b"x").is_err(), "StubKeyVault::encrypt must error in S1");
    assert!(kv.decrypt(b"x").is_err(), "StubKeyVault::decrypt must error in S1");
}

// --- Six-interface protocol detection (the S1 falsification core). ---

#[test]
fn six_interfaces_detected() {
    use pingogate_pipeline::protocol::detect;
    // All six interfaces must be detected (Supported), not Unidentified.
    assert!(
        matches!(detect("POST", "/v1/chat/completions"), pingogate_pipeline::protocol::Detection::Supported(_)),
        "OpenAI Chat Completions must be detected"
    );
    assert!(
        matches!(detect("POST", "/v1/responses"), pingogate_pipeline::protocol::Detection::Supported(_)),
        "OpenAI Responses must be detected"
    );
    assert!(
        matches!(detect("POST", "/v1/messages"), pingogate_pipeline::protocol::Detection::Supported(_)),
        "Anthropic Messages must be detected"
    );
    assert!(
        matches!(detect("POST", "/v1beta/interactions"), pingogate_pipeline::protocol::Detection::Supported(_)),
        "Gemini Interactions must be detected"
    );
}

// --- Version-prefix wildcard (research finding: {ver} is provider-specific
// and variable, detection must not hardcode /v1). ---

#[test]
fn version_prefix_is_wildcarded() {
    use pingogate_pipeline::protocol::detect;
    // Same suffix under different version prefixes must all be detected.
    assert!(matches!(detect("POST", "/v1/chat/completions"), pingogate_pipeline::protocol::Detection::Supported(_)));
    assert!(matches!(detect("POST", "/v2/chat/completions"), pingogate_pipeline::protocol::Detection::Supported(_)));
    assert!(matches!(detect("POST", "/chat/completions"), pingogate_pipeline::protocol::Detection::Supported(_)));
    assert!(matches!(detect("POST", "/v1beta/interactions"), pingogate_pipeline::protocol::Detection::Supported(_)));
    assert!(matches!(detect("POST", "/v1/interactions"), pingogate_pipeline::protocol::Detection::Supported(_)));
    // Gemini generateContent surfaces under different version prefixes.
    assert!(matches!(detect("POST", "/v1beta/models/m:generateContent"), pingogate_pipeline::protocol::Detection::Supported(_)));
    assert!(matches!(detect("POST", "/v1/models/m:generateContent"), pingogate_pipeline::protocol::Detection::Supported(_)));
}

// --- ProtocolKind (T3: Responses + Gemini + Interactions variants exist). ---

#[test]
fn protocol_kind_has_five_variants() {
    let _openai_chat = ProtocolKind::OpenAiCompatible;
    let _openai_responses = ProtocolKind::OpenAiResponses;
    let _anthropic = ProtocolKind::Anthropic;
    let _gemini = ProtocolKind::Gemini;
    let _gemini_interactions = ProtocolKind::GeminiInteractions;
    // as_str coverage for the variants (T3 contract).
    assert_eq!(ProtocolKind::OpenAiResponses.as_str(), "openai-responses");
    assert_eq!(ProtocolKind::Gemini.as_str(), "gemini");
    assert_eq!(ProtocolKind::GeminiInteractions.as_str(), "gemini-interactions");
}
