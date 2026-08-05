-- 006_routes.up.sql
-- Routes table for model-alias routing. A route is a MODEL nested under a
-- provider (config hierarchy: model > provider > global). The provider column
-- references user_provider_keys.id (the key ID, prefixed with "imp_" for
-- imported keys or "pk_" for API-created keys).
--
-- Each column except alias/provider/upstream_model is an optional model-level
-- override; NULL = inherit from the provider's default (kind/auth_method/
-- timeout_ms/upstream_path from user_provider_keys config, anthropic_version
-- from the provider default).
--
-- kind: optional per-route upstream protocol kind override (for future
--   protocol conversion). NULL = provider kind.
-- auth_method: optional per-route auth override. NULL = provider auth method.
-- upstream_path: optional path template with {model} placeholder for
--   non-standard upstream endpoints. NULL = forward inbound path.
-- anthropic_version: optional per-route anthropic-version header override.
-- timeout_ms: optional per-route timeout override.
CREATE TABLE IF NOT EXISTS routes (
    alias            TEXT    NOT NULL,
    provider         TEXT    NOT NULL,
    upstream_model   TEXT    NOT NULL,
    upstream_path    TEXT,
    auth_method      TEXT,
    kind             TEXT,
    anthropic_version TEXT,
    timeout_ms       INTEGER,
    created_at       TEXT    NOT NULL DEFAULT (datetime('now')),
    PRIMARY KEY (alias)
);
