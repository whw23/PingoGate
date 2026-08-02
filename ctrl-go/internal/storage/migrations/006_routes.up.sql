-- 006_routes.up.sql
-- Routes table for model-alias routing (issue 1/2/3: per-route upstream_path +
-- auth_method overrides). A route maps a client-visible model alias to a
-- provider key + upstream model name. The provider column references
-- user_provider_keys.id (the key ID, prefixed with "imp_" for imported keys
-- or "pk_" for API-created keys).
--
-- upstream_path: optional path template with {model} placeholder for
--   non-standard upstream endpoints (issue 2). NULL = forward inbound path.
-- auth_method: optional per-route auth override (issue 1). NULL = use the
--   provider's default auth method.
CREATE TABLE IF NOT EXISTS routes (
    alias          TEXT    NOT NULL,
    provider       TEXT    NOT NULL,
    upstream_model TEXT    NOT NULL,
    upstream_path  TEXT,
    auth_method    TEXT,
    created_at     TEXT    NOT NULL DEFAULT (datetime('now')),
    PRIMARY KEY (alias)
);
