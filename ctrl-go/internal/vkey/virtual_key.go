// Package vkey implements the L2 virtual-key layer (constitution: Go
// non-kernel = all state). A VirtualKey is an opaque PingoGate-issued token
// that the caller presents on the hot path; PingoGate never sees the upstream
// provider key directly (the Rust kernel decrypts it per request via KeyVault,
// T26). The token is stored ONLY as a SHA-256 hash: SHA-256 (not bcrypt)
// because the token has ~128 bits of entropy (PingoGate-generated), so a fast
// hash is sufficient and brute-force is infeasible. The Rust kernel's
// VirtualKeyAuth (T26) compares the SHA-256 hex digest in constant time
// (subtle::ct_eq on hex_sha256(credential)).
//
// token_hash format: hex(sha256(plaintext_token)) -- 64 lowercase hex chars,
// matching core-rs/pipeline/src/virtual_key_auth.rs::hex_sha256.
//
// The plaintext token is returned ONCE at Issue time and never persisted or
// logged (constitution XX: same visibility contract as identity API tokens).
package vkey

// VirtualKey is the L2 entity (spec S3). The TokenHash field is the SHA-256
// hex digest of the plaintext token; it is tagged json:"-" so it is never
// serialized to any HTTP response (constitution XX: no sensitive material in
// responses). The plaintext token is returned only by Store.Issue and only
// to the caller that created the key.
//
// AllowedModels and AllowedProviders are optional allowlists enforced by the
// Rust kernel at route time (T26+); Go does not interpret them. Empty/nil
// means "unrestricted within the owner's scope" (all of the owner's providers
// / all models the provider supports).
//
// ExpiresAt is a Unix timestamp (seconds, UTC); 0 means "never expires".
// MaxConcurrency is the in-flight request quota enforced atomically by the
// Rust kernel (T26); 0 means unlimited.
type VirtualKey struct {
	ID               string   `db:"id" json:"id"`
	TokenHash        string   `db:"token_hash" json:"-"`
	OwnerUserID      string   `db:"owner_user_id" json:"owner_user_id"`
	ProviderKeyID    string   `db:"provider_key_id" json:"provider_key_id,omitempty"`
	AllowedModels    []string `db:"allowed_models" json:"allowed_models,omitempty"`
	AllowedProviders []string `db:"allowed_providers" json:"allowed_providers,omitempty"`
	ExpiresAt        int64    `db:"expires_at" json:"expires_at"`
	MaxConcurrency   int32    `db:"max_concurrency" json:"max_concurrency"`
	Enabled          bool     `db:"enabled" json:"enabled"`
	CreatedAt        string   `db:"created_at" json:"created_at"`
}

// Scope is the Issue-time allowlist configuration. Empty slices mean
// "unrestricted" (nil serializes to NULL in the DB; an empty []string
// serializes to "[]"). Both are treated identically by the Rust kernel.
type Scope struct {
	ProviderKeyID    string
	AllowedModels    []string
	AllowedProviders []string
	ExpiresAt        int64
	MaxConcurrency   int32
}
