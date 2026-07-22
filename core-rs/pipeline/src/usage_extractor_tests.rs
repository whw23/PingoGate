//! Tests for [`UsageExtractor`] / [`LineBuffer`] (constitution XX/XXI; task 27).
//!
//! Extracted from `usage_extractor.rs` to keep that module under the
//! constitution V file-size limit (≤300 lines).

use super::*;

// --- LineBuffer unit tests -------------------------------------------

#[test]
fn line_buffer_splits_complete_lines_keeps_residual() {
    let mut buf = LineBuffer::new();
    let lines = buf.push_chunk(b"alpha\nbeta\ngamma");
    assert_eq!(lines, vec![b"alpha".to_vec(), b"beta".to_vec()]);
    assert_eq!(buf.residual_len(), b"gamma".len());

    let lines = buf.push_chunk(b"_more\n");
    assert_eq!(lines, vec![b"gamma_more".to_vec()]);
    assert_eq!(buf.residual_len(), 0);
}

#[test]
fn line_buffer_handles_split_newline() {
    let mut buf = LineBuffer::new();
    let lines = buf.push_chunk(b"foo\r");
    assert!(lines.is_empty());
    let lines = buf.push_chunk(b"\nbar\n");
    assert_eq!(lines, vec![b"foo\r".to_vec(), b"bar".to_vec()]);
}

#[test]
fn line_buffer_caps_runaway_residual() {
    let mut buf = LineBuffer::new();
    // One huge partial line (no newline) - should be dropped at cap.
    let big = vec![b'x'; MAX_RESIDUAL + 1];
    buf.push_chunk(&big);
    assert_eq!(buf.residual_len(), 0);
}

// --- UsageExtractor: OpenAI Chat -------------------------------------

#[test]
fn extracts_openai_chat_usage_from_last_chunk() {
    let mut ex = UsageExtractor::new(ProtocolKind::OpenAiCompatible);
    // First two chunks carry content but no usage.
    ex.on_body_chunk(
        b"data: {\"choices\":[{\"delta\":{\"content\":\"hi\"}}]}\n",
        false,
    );
    ex.on_body_chunk(
        b"data: {\"choices\":[{\"delta\":{\"content\":\"!\"}}]}\n",
        false,
    );
    // Final chunk carries usage.
    ex.on_body_chunk(
        b"data: {\"choices\":[],\"usage\":{\"prompt_tokens\":10,\"completion_tokens\":20}}\n",
        true,
    );
    let tokens = ex.finalize().expect("usage should be extracted");
    assert_eq!(tokens.input, 10);
    assert_eq!(tokens.output, 20);
}

// --- UsageExtractor: Anthropic (two events, merged) ------------------

#[test]
fn extracts_anthropic_usage_from_two_events() {
    let mut ex = UsageExtractor::new(ProtocolKind::Anthropic);
    ex.on_body_chunk(
        b"data: {\"type\":\"message_start\",\"message\":{\"usage\":{\"input_tokens\":5}}}\n",
        false,
    );
    ex.on_body_chunk(
        b"data: {\"type\":\"message_delta\",\"usage\":{\"output_tokens\":8}}\n",
        true,
    );
    let tokens = ex.finalize().expect("merged usage should be extracted");
    assert_eq!(tokens.input, 5);
    assert_eq!(tokens.output, 8);
}

#[test]
fn extracts_anthropic_cache_fields_from_message_start() {
    let mut ex = UsageExtractor::new(ProtocolKind::Anthropic);
    ex.on_body_chunk(
        b"data: {\"type\":\"message_start\",\"message\":{\"usage\":{\
         \"input_tokens\":5,\
         \"cache_read_input_tokens\":2,\
         \"cache_creation_input_tokens\":4}}}\n",
        false,
    );
    ex.on_body_chunk(
        b"data: {\"type\":\"message_delta\",\"usage\":{\"output_tokens\":9}}\n",
        true,
    );
    let tokens = ex.finalize().expect("usage should be extracted");
    assert_eq!(tokens.input, 5);
    assert_eq!(tokens.output, 9);
    assert_eq!(tokens.cache_read, Some(2));
    assert_eq!(tokens.cache_write, Some(4));
}

// --- UsageExtractor: Gemini ------------------------------------------

#[test]
fn extracts_gemini_usage_metadata() {
    let mut ex = UsageExtractor::new(ProtocolKind::Gemini);
    ex.on_body_chunk(
        b"data: {\"usageMetadata\":{\"promptTokenCount\":3,\"candidatesTokenCount\":7}}\n",
        true,
    );
    let tokens = ex.finalize().expect("usage should be extracted");
    assert_eq!(tokens.input, 3);
    assert_eq!(tokens.output, 7);
}

#[test]
fn extracts_gemini_interactions_usage_metadata() {
    let mut ex = UsageExtractor::new(ProtocolKind::GeminiInteractions);
    ex.on_body_chunk(
        b"data: {\"usageMetadata\":{\"promptTokenCount\":2,\"candidatesTokenCount\":4,\
         \"thoughtsTokenCount\":1,\"cachedContentTokenCount\":6}}\n",
        true,
    );
    let tokens = ex.finalize().expect("usage should be extracted");
    assert_eq!(tokens.input, 2);
    assert_eq!(tokens.output, 4);
    assert_eq!(tokens.reasoning, Some(1));
    assert_eq!(tokens.cache_read, Some(6));
}

// --- UsageExtractor: OpenAI Responses --------------------------------

#[test]
fn extracts_openai_responses_usage() {
    let mut ex = UsageExtractor::new(ProtocolKind::OpenAiResponses);
    ex.on_body_chunk(
        b"data: {\"type\":\"response.completed\",\"usage\":{\"input_tokens\":11,\"output_tokens\":22}}\n",
        true,
    );
    let tokens = ex.finalize().expect("usage should be extracted");
    assert_eq!(tokens.input, 11);
    assert_eq!(tokens.output, 22);
}

// --- Memory bound: 1000 non-marker chunks ----------------------------

#[test]
fn o1_memory_no_full_buffer() {
    let mut ex = UsageExtractor::new(ProtocolKind::OpenAiCompatible);
    // Feed 1000 chunks, each a complete SSE line without usage. The
    // extractor must NOT accumulate them - residual stays at zero after
    // each chunk (every chunk ends with `\n`).
    let chunk = b"data: {\"choices\":[{\"delta\":{\"content\":\"token\"}}]}\n";
    for _ in 0..1000 {
        ex.on_body_chunk(chunk, false);
    }
    assert_eq!(ex.residual_len(), 0);

    // Final chunk carries usage and is still extracted correctly.
    ex.on_body_chunk(
        b"data: {\"choices\":[],\"usage\":{\"prompt_tokens\":1,\"completion_tokens\":2}}\n",
        true,
    );
    let tokens = ex.finalize().expect("usage should be extracted");
    assert_eq!(tokens.input, 1);
    assert_eq!(tokens.output, 2);
}

#[test]
fn o1_memory_partial_line_does_not_accumulate_across_chunks() {
    // Even if no newline arrives for many chunks (pathological upstream),
    // residual is capped at MAX_RESIDUAL - not the full feed.
    let mut ex = UsageExtractor::new(ProtocolKind::OpenAiCompatible);
    let chunk = vec![b'x'; 1024];
    for _ in 0..1000 {
        ex.on_body_chunk(&chunk, false);
        if ex.residual_len() > MAX_RESIDUAL {
            // Cap enforces the bound at every push.
            panic!("residual exceeded cap: {}", ex.residual_len());
        }
    }
}

// --- Non-streaming single body ---------------------------------------

#[test]
fn non_streaming_single_body() {
    let mut ex = UsageExtractor::new(ProtocolKind::OpenAiCompatible);
    // No `data:` prefix, no trailing newline - a plain JSON body.
    ex.on_body_chunk(
        b"{\"id\":\"chatcmpl-1\",\"choices\":[],\"usage\":{\"prompt_tokens\":1,\"completion_tokens\":2}}",
        true,
    );
    let tokens = ex.finalize().expect("usage should be extracted");
    assert_eq!(tokens.input, 1);
    assert_eq!(tokens.output, 2);
}

#[test]
fn non_streaming_gemini_single_body() {
    let mut ex = UsageExtractor::new(ProtocolKind::Gemini);
    ex.on_body_chunk(
        b"{\"candidates\":[],\"usageMetadata\":{\"promptTokenCount\":4,\"candidatesTokenCount\":5}}",
        true,
    );
    let tokens = ex.finalize().expect("usage should be extracted");
    assert_eq!(tokens.input, 4);
    assert_eq!(tokens.output, 5);
}

// --- Negative cases ---------------------------------------------------

#[test]
fn no_usage_yields_none() {
    let mut ex = UsageExtractor::new(ProtocolKind::OpenAiCompatible);
    ex.on_body_chunk(
        b"data: {\"choices\":[{\"delta\":{\"content\":\"hi\"}}]}\n",
        true,
    );
    assert!(ex.finalize().is_none());
}

#[test]
fn malformed_usage_line_is_ignored() {
    let mut ex = UsageExtractor::new(ProtocolKind::OpenAiCompatible);
    // Marker substring is present but JSON is broken - must not panic.
    ex.on_body_chunk(b"data: {\"usage\": broken}\n", false);
    ex.on_body_chunk(
        b"data: {\"choices\":[],\"usage\":{\"prompt_tokens\":7,\"completion_tokens\":3}}\n",
        true,
    );
    let tokens = ex.finalize().expect("valid later line still extracts");
    assert_eq!(tokens.input, 7);
    assert_eq!(tokens.output, 3);
}

#[test]
fn empty_body_yields_none() {
    let mut ex = UsageExtractor::new(ProtocolKind::OpenAiCompatible);
    ex.on_body_chunk(b"", true);
    assert!(ex.finalize().is_none());
}

// --- Marker / helper unit tests --------------------------------------

#[test]
fn marker_matches_per_protocol() {
    assert!(line_has_usage_marker(
        b"data: {\"usage\":1}",
        ProtocolKind::OpenAiCompatible
    ));
    assert!(!line_has_usage_marker(
        b"data: {\"choices\":[]}",
        ProtocolKind::OpenAiCompatible
    ));

    assert!(line_has_usage_marker(
        b"data: {\"type\":\"response.completed\"}",
        ProtocolKind::OpenAiResponses
    ));
    assert!(line_has_usage_marker(
        b"data: {\"usage\":{}}",
        ProtocolKind::OpenAiResponses
    ));

    assert!(line_has_usage_marker(
        b"data: {\"type\":\"message_start\"}",
        ProtocolKind::Anthropic
    ));
    assert!(line_has_usage_marker(
        b"data: {\"type\":\"message_delta\"}",
        ProtocolKind::Anthropic
    ));

    assert!(line_has_usage_marker(
        b"data: {\"usageMetadata\":{}}",
        ProtocolKind::Gemini
    ));
    assert!(line_has_usage_marker(
        b"data: {\"usageMetadata\":{}}",
        ProtocolKind::GeminiInteractions
    ));
}

#[test]
fn strip_sse_prefix_variants() {
    assert_eq!(strip_sse_prefix(b"data: {\"a\":1}"), b"{\"a\":1}");
    assert_eq!(strip_sse_prefix(b"data:{\"a\":1}"), b"{\"a\":1}");
    assert_eq!(strip_sse_prefix(b"{\"a\":1}"), b"{\"a\":1}");
}
