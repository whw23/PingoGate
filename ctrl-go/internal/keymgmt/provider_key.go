// Package keymgmt implements the L1 BYOK provider-key CRUD layer (constitution:
// Go non-kernel = all state). It owns the UserProviderKey entity, its SQLite
// store, a gRPC client to the Rust KeyVault (Encrypt/Decrypt), and the HTTP
// handlers that surface provider-key operations over the control-plane API.
//
// Visibility model (constitution XX, "1Password model"): Go never holds the
// plaintext provider key outside of the single Encrypt call at Create time.
// The ciphertext is stored in user_provider_keys.encrypted_key; the last 4
// characters of the original plaintext (key_last4) are captured at Create
// time and persisted so List/Get can show a masked preview without decrypting.
// Decrypt is only invoked by the Rust kernel on the hot path (T15), never by
// Go list/get handlers.
package keymgmt

// UserProviderKey is the L1 BYOK entity (spec S3). The EncryptedKey field is
// the AES-GCM ciphertext produced by the Rust KeyVault via gRPC; it is tagged
// json:"-" so it is never serialized to any HTTP response (constitution XX).
// KeyLast4 holds the last 4 characters of the original plaintext, captured
// once at Create time for display; it is non-sensitive (4 chars is not enough
// to recover the key).
//
// The CreatedBy field records the user who imported the key (BYOK: created_by
// = owner_user_id); per the 1Password model, only created_by may view the
// plaintext (S3 adds the Decrypt-based reveal endpoint).
type UserProviderKey struct {
	ID           string  `db:"id" json:"id"`
	OwnerUserID  string  `db:"owner_user_id" json:"owner_user_id"`
	ProviderType string  `db:"provider_type" json:"provider_type"`
	EncryptedKey []byte  `db:"encrypted_key" json:"-"` // ciphertext, never serialized
	KeyLast4     string  `db:"key_last4" json:"key_last4"`
	BaseURL      string  `db:"base_url" json:"base_url,omitempty"`
	CreatedBy    string  `db:"created_by" json:"created_by"` // = owner (BYOK)
	CreatedAt    string  `db:"created_at" json:"created_at"`
	LastUsedAt   *string `db:"last_used_at" json:"last_used_at,omitempty"`
	Enabled      bool    `db:"enabled" json:"enabled"`
}
