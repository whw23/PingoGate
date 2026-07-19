-- 001_users.up.sql: L0 identity - users table.
-- Stores control-plane users with a bcrypt-hashed API token (constitution XX:
-- no plaintext tokens; BYOK visibility binds to created_by, orthogonal to
-- RBAC). is_admin is an INTEGER boolean (SQLite has no native BOOL type).
CREATE TABLE users (
    id TEXT PRIMARY KEY,
    email TEXT UNIQUE NOT NULL,
    api_token_hash TEXT NOT NULL,  -- bcrypt hash
    is_admin INTEGER NOT NULL DEFAULT 0,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);
