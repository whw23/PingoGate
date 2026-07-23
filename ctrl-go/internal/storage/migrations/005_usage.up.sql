-- 005_usage.up.sql: L5 usage records (constitution XIX; spec §10/§12D SC-10).
--
-- Each row is one completed (or failed) request through the Rust kernel's
-- data plane. The Rust kernel extracts whatever usage the provider returned
-- (OpenAI include_usage / Anthropic message_delta / Gemini usageMetadata);
-- when the provider returned no usage, the Go control plane fills in the
-- numbers via tiktoken-go estimation (estimated=1). The Rust kernel never
-- runs a tokenizer (constitution XIX: "Rust 不带 tokenizer").
--
-- Columns mirror pingogate.UsageEvent (proto/pingogate.proto). owner_user_id
-- is always present (every request has an authenticated virtual-key owner);
-- virtual_key_id is nullable for standalone mode where no vkey is presented.
-- success=0 covers upstream failures AND partial/interrupted streaming
-- (constitution XIX: "失败 / 中断的部分计量归 Go 估算").
--
-- Indexes: owner_user_id for the per-user usage view (L6 console),
-- created_at for time-window aggregations (L5/L6 billing queries).
CREATE TABLE usage (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    virtual_key_id TEXT,
    owner_user_id TEXT NOT NULL,
    provider TEXT NOT NULL,
    model TEXT NOT NULL,
    input_tokens INTEGER NOT NULL DEFAULT 0,
    output_tokens INTEGER NOT NULL DEFAULT 0,
    reasoning_tokens INTEGER NOT NULL DEFAULT 0,
    cache_read_tokens INTEGER NOT NULL DEFAULT 0,
    cache_write_tokens INTEGER NOT NULL DEFAULT 0,
    success INTEGER NOT NULL,
    latency_ms INTEGER,
    estimated INTEGER NOT NULL DEFAULT 0,  -- 1 = Go tiktoken estimate (no provider usage)
    created_at TEXT NOT NULL
);
CREATE INDEX idx_usage_owner ON usage(owner_user_id);
CREATE INDEX idx_usage_created ON usage(created_at);
