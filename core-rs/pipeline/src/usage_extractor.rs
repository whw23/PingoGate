//! Inline event-driven usage extraction (FR-032; constitution XIX/XX/XXI).
//!
//! Replaces the previous bounded-buffer capture with an **O(1)-memory**
//! streaming extractor: a small [`LineBuffer`] reassembles SSE `data:` lines
//! across chunks, a cheap byte-scan marker check skips most lines, and only
//! usage-bearing lines are JSON-parsed. Works for streaming SSE and non-
//! streaming single-body responses. Merges Anthropic's split `message_start`
//! (input) + `message_delta` (output) into one [`TokenUsage`].
//!
//! Memory budget (constitution XXI): residual capped at 64 KiB; over-long
//! lines drop the residual and degrade gracefully. Only integer token counts
//! are read - never prompt/response content (constitution XX).

use pingogate_core_types::ProtocolKind;
use pingogate_provider::TokenUsage;
use serde_json::Value;

/// Residual cap. A single `data:` line larger than this without a newline is
/// pathological for usage extraction - we drop the residual and stop
/// extracting from that line.
const MAX_RESIDUAL: usize = 64 * 1024;

/// Reassembles newline-terminated lines across `push_chunk` calls. Returns
/// each complete line (without the trailing `\n`); keeps the final partial
/// line in `residual`. If `residual` exceeds [`MAX_RESIDUAL`] it is dropped
/// (usage extraction degrades, memory stays bounded).
#[derive(Debug, Default)]
pub struct LineBuffer {
    residual: Vec<u8>,
}

impl LineBuffer {
    pub fn new() -> Self {
        Self::default()
    }

    /// Append `chunk` and split out every complete line (no trailing `\n`).
    /// The final partial line is retained in `residual` for the next call.
    pub fn push_chunk(&mut self, chunk: &[u8]) -> Vec<Vec<u8>> {
        let mut out = Vec::new();
        let mut start = 0usize;
        self.residual.extend_from_slice(chunk);

        while start < self.residual.len() {
            match self.residual[start..].iter().position(|&b| b == b'\n') {
                Some(rel) => {
                    let end = start + rel;
                    let line = self.residual[start..end].to_vec();
                    out.push(line);
                    start = end + 1;
                }
                None => break,
            }
        }

        if start > 0 {
            self.residual.drain(0..start);
        }

        if self.residual.len() > MAX_RESIDUAL {
            self.residual.clear();
        }

        out
    }

    /// Current residual length (for tests / health metrics).
    pub fn residual_len(&self) -> usize {
        self.residual.len()
    }

    /// Drain the residual partial line (if any). Used at `end_of_stream` to
    /// flush a final non-newline-terminated line (e.g. a non-streaming JSON
    /// body). Returns `None` if the residual is empty.
    pub fn take_residual(&mut self) -> Option<Vec<u8>> {
        if self.residual.is_empty() {
            None
        } else {
            Some(std::mem::take(&mut self.residual))
        }
    }
}

/// Streaming usage extractor. Feed it every body chunk via
/// [`on_body_chunk`]; call [`finalize`] at `end_of_stream` to retrieve the
/// merged [`TokenUsage`] (if any).
#[derive(Debug)]
pub struct UsageExtractor {
    line_buf: LineBuffer,
    protocol: ProtocolKind,
    tokens: Option<TokenUsage>,
}

impl UsageExtractor {
    pub fn new(protocol: ProtocolKind) -> Self {
        Self {
            line_buf: LineBuffer::new(),
            protocol,
            tokens: None,
        }
    }

    /// Feed one body chunk. Each complete line is checked for a usage marker;
    /// matching lines are JSON-parsed and merged into `self.tokens`. When
    /// `end_of_stream` is true, any remaining residual is treated as a final
    /// line (non-streaming single-body responses have no trailing newline).
    pub fn on_body_chunk(&mut self, body: &[u8], end_of_stream: bool) {
        let mut lines = self.line_buf.push_chunk(body);
        if end_of_stream {
            // Drain any final partial line - a non-streaming JSON body has no
            // trailing newline, so the last bytes live in the residual.
            if let Some(tail) = self.line_buf.take_residual() {
                lines.push(tail);
            }
        }
        for line in lines {
            if line_has_usage_marker(&line, self.protocol) {
                if let Some(parsed) = extract_from_line(&line, self.protocol) {
                    merge_tokens(&mut self.tokens, parsed);
                }
            }
        }
    }

    /// Drain and return the merged token usage, if any was observed.
    pub fn finalize(&mut self) -> Option<TokenUsage> {
        self.tokens.take()
    }

    /// Current residual buffer length (for tests / health checks).
    pub fn residual_len(&self) -> usize {
        self.line_buf.residual_len()
    }
}

/// Merge `incoming` into `accumulated`: input/output/reasoning/cache fields are
/// each overwritten when `incoming` carries a non-zero / Some value. This is
/// what lets Anthropic's `message_start` (input only) combine with a later
/// `message_delta` (output only) into a single record.
fn merge_tokens(accumulated: &mut Option<TokenUsage>, incoming: TokenUsage) {
    let acc = accumulated.get_or_insert(TokenUsage::default());
    if incoming.input > 0 {
        acc.input = incoming.input;
    }
    if incoming.output > 0 {
        acc.output = incoming.output;
    }
    if incoming.reasoning.is_some() {
        acc.reasoning = incoming.reasoning;
    }
    if incoming.cache_read.is_some() {
        acc.cache_read = incoming.cache_read;
    }
    if incoming.cache_write.is_some() {
        acc.cache_write = incoming.cache_write;
    }
}

/// Cheap byte-scan to decide whether a line is worth JSON-parsing. Matches the
/// minimal stable substring each protocol puts on a usage-bearing event.
pub fn line_has_usage_marker(line: &[u8], protocol: ProtocolKind) -> bool {
    match protocol {
        ProtocolKind::OpenAiCompatible => contains_subseq(line, b"\"usage\""),
        ProtocolKind::OpenAiResponses => {
            contains_subseq(line, b"response.completed")
                || contains_subseq(line, b"\"usage\"")
        }
        ProtocolKind::Anthropic => {
            contains_subseq(line, b"message_start")
                || contains_subseq(line, b"message_delta")
        }
        ProtocolKind::Gemini | ProtocolKind::GeminiInteractions => {
            contains_subseq(line, b"usageMetadata")
        }
    }
}

/// `memmem`-style substring search over bytes. We avoid pulling a `memchr`
/// dependency for this - the hot path is dominated by the fast negative case
/// (most SSE lines are token deltas without usage), and a linear scan over a
/// few-KiB line is well within budget.
fn contains_subseq(haystack: &[u8], needle: &[u8]) -> bool {
    if needle.is_empty() {
        return true;
    }
    if needle.len() > haystack.len() {
        return false;
    }
    haystack.windows(needle.len()).any(|w| w == needle)
}

/// Strip a leading `data: ` (SSE) prefix if present, then JSON-parse and
/// extract token usage per protocol. Returns `None` for unparseable lines or
/// lines that match the marker but carry no usage block (graceful degradation).
pub fn extract_from_line(line: &[u8], protocol: ProtocolKind) -> Option<TokenUsage> {
    let payload = strip_sse_prefix(line);
    let value: Value = serde_json::from_slice(payload).ok()?;
    match protocol {
        ProtocolKind::OpenAiCompatible => parse_openai_chat(&value),
        ProtocolKind::OpenAiResponses => parse_openai_responses(&value),
        ProtocolKind::Anthropic => parse_anthropic_event(&value),
        ProtocolKind::Gemini | ProtocolKind::GeminiInteractions => parse_gemini(&value),
    }
}

/// Strip the SSE `data: ` prefix. Tolerates the spec-mandated single space and
/// the common no-space variant. Returns the original slice when no prefix
/// matches (non-streaming single-body responses).
fn strip_sse_prefix(line: &[u8]) -> &[u8] {
    if line.starts_with(b"data: ") {
        &line[6..]
    } else if line.starts_with(b"data:") {
        &line[5..]
    } else {
        line
    }
}

fn u64_at(value: &Value, key: &str) -> Option<u64> {
    value.get(key).and_then(Value::as_u64)
}

/// OpenAI Chat Completions: `usage.prompt_tokens` / `completion_tokens`.
fn parse_openai_chat(value: &Value) -> Option<TokenUsage> {
    let usage = value.get("usage")?;
    Some(TokenUsage {
        input: u64_at(usage, "prompt_tokens").unwrap_or(0),
        output: u64_at(usage, "completion_tokens").unwrap_or(0),
        reasoning: usage
            .get("completion_tokens_details")
            .and_then(|d| u64_at(d, "reasoning_tokens")),
        cache_read: usage
            .get("prompt_tokens_details")
            .and_then(|d| u64_at(d, "cached_tokens")),
        cache_write: None,
    })
}

/// OpenAI Responses API: `usage.input_tokens` / `output_tokens`. The final
/// `response.completed` event carries the full usage; incremental events also
/// carry `usage` and are idempotent under [`merge_tokens`].
fn parse_openai_responses(value: &Value) -> Option<TokenUsage> {
    let usage = value.get("usage")?;
    Some(TokenUsage {
        input: u64_at(usage, "input_tokens").unwrap_or(0),
        output: u64_at(usage, "output_tokens").unwrap_or(0),
        reasoning: usage
            .get("output_tokens_details")
            .and_then(|d| u64_at(d, "reasoning_tokens")),
        cache_read: usage
            .get("input_tokens_details")
            .and_then(|d| u64_at(d, "cached_tokens")),
        cache_write: None,
    })
}

/// Anthropic streaming: `message_start` carries `message.usage.input_tokens`
/// (output is zero at that point); `message_delta` carries `usage.output_tokens`
/// (final). Both shapes are accepted here - the `message.usage` path handles
/// `message_start`, the top-level `usage` path handles `message_delta`.
fn parse_anthropic_event(value: &Value) -> Option<TokenUsage> {
    // message_start: { "type":"message_start", "message":{ "usage":{...} } }
    if let Some(usage) = value.get("message").and_then(|m| m.get("usage")) {
        return Some(TokenUsage {
            input: u64_at(usage, "input_tokens").unwrap_or(0),
            output: u64_at(usage, "output_tokens").unwrap_or(0),
            reasoning: None,
            cache_read: u64_at(usage, "cache_read_input_tokens"),
            cache_write: u64_at(usage, "cache_creation_input_tokens"),
        });
    }
    // message_delta: { "type":"message_delta", "usage":{ "output_tokens":N } }
    if let Some(usage) = value.get("usage") {
        return Some(TokenUsage {
            input: u64_at(usage, "input_tokens").unwrap_or(0),
            output: u64_at(usage, "output_tokens").unwrap_or(0),
            reasoning: None,
            cache_read: u64_at(usage, "cache_read_input_tokens"),
            cache_write: u64_at(usage, "cache_creation_input_tokens"),
        });
    }
    None
}

/// Gemini / GeminiInteractions: `usageMetadata.promptTokenCount` /
/// `candidatesTokenCount` / `thoughtsTokenCount` / `cachedContentTokenCount`.
fn parse_gemini(value: &Value) -> Option<TokenUsage> {
    let usage = value.get("usageMetadata")?;
    Some(TokenUsage {
        input: u64_at(usage, "promptTokenCount").unwrap_or(0),
        output: u64_at(usage, "candidatesTokenCount").unwrap_or(0),
        reasoning: u64_at(usage, "thoughtsTokenCount"),
        cache_read: u64_at(usage, "cachedContentTokenCount"),
        cache_write: None,
    })
}

#[cfg(test)]
#[path = "usage_extractor_tests.rs"]
mod tests;
