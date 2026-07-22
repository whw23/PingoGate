-- 004_virtual_keys.up.sql: L2 virtual keys (BYOK gateway credentials).
-- A virtual key is an opaque token presented by the caller on the hot path;
-- PingoGate never sees the upstream provider key directly (constitution XX).
-- The token is stored ONLY as a SHA-256 hash (token_hash, lowercase hex; 64
-- chars). SHA-256 (not bcrypt) because the presented token has ~128 bits of
-- entropy (PingoGate-generated ), so a fast hash is sufficient
-- and brute-force is infeasible; bcrypt's ~72-byte input cap and per-call
-- cost would hurt hot-path latency without adding meaningful security. The
-- Rust kernel's VirtualKeyAuth (T26) compares the SHA-256 hex digest in
-- constant time (subtle::ct_eq).
--
-- token_hash format: hex(sha256(plaintext_token)) -- 64 lowercase hex chars,
-- matching core-rs/pipeline/src/virtual_key_auth.rs::hex_sha256.
--
-- allowed_models / allowed_providers are JSON-encoded arrays of string
-- allowlist entries (empty array or NULL = unrestricted within the owner's
-- scope). The Rust kernel reads them from the snapshot and enforces them at
-- route time (T26+); Go does not interpret the contents.
--
-- expires_at is a Unix timestamp (seconds, UTC); 0 means "never expires".
-- max_concurrency is the in-flight request quota; 0 means unlimited (Rust
-- tracks the counter atomically in memory; T26).
CREATE TABLE virtual_keys (
    id TEXT PRIMARY KEY,
    token_hash TEXT NOT NULL,
    owner_user_id TEXT NOT NULL REFERENCES users(id),
    provider_key_id TEXT REFERENCES user_provider_keys(id),
    allowed_models TEXT,     -- JSON array of strings; NULL/empty = unrestricted
    allowed_providers TEXT,  -- JSON array of strings; NULL/empty = unrestricted
    expires_at INTEGER NOT NULL DEFAULT 0,  -- unix ts; 0 = never
    max_concurrency INTEGER NOT NULL DEFAULT 0,  -- 0 = unlimited
    enabled INTEGER NOT NULL DEFAULT 1,
    created_at TEXT NOT NULL
);
CREATE INDEX idx_virtual_keys_owner ON virtual_keys(owner_user_id);
CREATE INDEX idx_virtual_keys_token_hash ON virtual_keys(token_hash);
