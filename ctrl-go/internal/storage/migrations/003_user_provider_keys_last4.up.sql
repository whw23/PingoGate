-- 003_user_provider_keys_last4.up.sql: L1 BYOK - add key_last4 column.
-- The 1Password visibility model (constitution XX) requires that List endpoints
-- return only the last 4 characters of the original plaintext key, never the
-- ciphertext and never the full plaintext. Decrypting on every List would
-- defeat the 1Password model (Go would hold plaintext on every list call) and
-- is expensive. Instead, we capture last4 ONCE at Create time, when the
-- plaintext is already in Go memory for the Encrypt call, and persist it here.
-- key_last4 is therefore non-sensitive (4 chars, not enough to recover the
-- key) and safe to surface in List responses.
ALTER TABLE user_provider_keys ADD COLUMN key_last4 TEXT;
