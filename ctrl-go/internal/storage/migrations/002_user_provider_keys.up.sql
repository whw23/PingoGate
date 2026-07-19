-- 002_user_provider_keys.up.sql: L1 BYOK - user provider keys table.
-- Stores provider keys as AES-GCM ciphertext (encrypted by the Rust KeyVault
-- via gRPC, constitution XX: Go never holds plaintext provider keys). The
-- created_by column records the user who imported the key (BYOK: created_by =
-- owner_user_id); per the 1Password model, only created_by may view the
-- plaintext, admins included.
CREATE TABLE user_provider_keys (
    id TEXT PRIMARY KEY,
    owner_user_id TEXT NOT NULL REFERENCES users(id),
    provider_type TEXT NOT NULL,  -- openai / anthropic / gemini / openai-compatible
    encrypted_key BLOB NOT NULL,  -- AES-GCM ciphertext (Rust KeyVault)
    base_url TEXT,
    created_by TEXT NOT NULL REFERENCES users(id),  -- = owner_user_id (BYOK)
    created_at TEXT NOT NULL,
    last_used_at TEXT,
    enabled INTEGER NOT NULL DEFAULT 1
);
CREATE INDEX idx_provider_keys_owner ON user_provider_keys(owner_user_id);
