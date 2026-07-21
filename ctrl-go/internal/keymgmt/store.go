// Package keymgmt - Store provides UserProviderKey CRUD against the SQLite DB.
// The Store only ever sees ciphertext (encrypted by Rust KeyVault via gRPC);
// it never handles plaintext. Sentinels (ErrNotFound) keep the handler layer's
// HTTP mapping clean (constitution IX).
package keymgmt

import (
	"context"
	"crypto/rand"
	"database/sql"
	"encoding/hex"
	"errors"
	"fmt"
	"time"

	"github.com/whw23/pingogate/ctrl-go/internal/storage"
)

// ErrNotFound is returned by Get/Delete when no provider key matches the ID.
// Sentinels keep handler-layer HTTP mapping clean (constitution IX).
var ErrNotFound = errors.New("keymgmt: provider key not found")

// Store implements UserProviderKey CRUD against the wrapped *storage.DB. It is
// the only component that touches the user_provider_keys table; handlers depend
// on Store, not on raw SQL (constitution VII: focused interface, injected dep).
type Store struct {
	db *storage.DB
}

// NewStore wraps a *storage.DB. The DB must already have migrations applied
// (T17 storage.Open does this, including 003_user_provider_keys_last4); Store
// does not run migrations itself.
func NewStore(db *storage.DB) *Store {
	return &Store{db: db}
}

// Create inserts a new provider key with the given ciphertext. The caller
// (handler) is responsible for producing ciphertext via KeyVaultClient.Encrypt
// and for computing keyLast4 from the plaintext before calling Create; Store
// never sees the plaintext. ownerUserID and createdBy are equal under BYOK
// (the user imports their own key); both are accepted so the schema is
// forward-compatible with admin-imported keys (L4 multi-tenant).
func (s *Store) Create(ctx context.Context, ownerUserID, providerType string, encryptedKey []byte, keyLast4, baseURL, createdBy string) (*UserProviderKey, error) {
	if ownerUserID == "" {
		return nil, fmt.Errorf("keymgmt: owner_user_id is required")
	}
	if providerType == "" {
		return nil, fmt.Errorf("keymgmt: provider_type is required")
	}
	if len(encryptedKey) == 0 {
		return nil, fmt.Errorf("keymgmt: encrypted_key is required")
	}
	if createdBy == "" {
		return nil, fmt.Errorf("keymgmt: created_by is required")
	}

	now := time.Now().UTC().Format(time.RFC3339)
	pk := &UserProviderKey{
		ID:           newKeyID(),
		OwnerUserID:  ownerUserID,
		ProviderType: providerType,
		EncryptedKey: encryptedKey,
		KeyLast4:     keyLast4,
		BaseURL:      baseURL,
		CreatedBy:    createdBy,
		CreatedAt:    now,
		Enabled:      true,
	}

	q := "INSERT INTO user_provider_keys (id, owner_user_id, provider_type, encrypted_key, key_last4, base_url, created_by, created_at, enabled) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)"
	_, err := s.db.ExecContext(ctx, q,
		pk.ID, pk.OwnerUserID, pk.ProviderType, pk.EncryptedKey, pk.KeyLast4,
		pk.BaseURL, pk.CreatedBy, pk.CreatedAt, boolToInt(pk.Enabled))
	if err != nil {
		return nil, fmt.Errorf("keymgmt: insert provider key: %w", err)
	}
	return pk, nil
}

// GetByID loads a single provider key by primary key. Returns ErrNotFound when
// no row matches, so handlers can map cleanly to 404 (constitution IX).
func (s *Store) GetByID(ctx context.Context, id string) (*UserProviderKey, error) {
	var pk UserProviderKey
	q := "SELECT id, owner_user_id, provider_type, encrypted_key, key_last4, base_url, created_by, created_at, last_used_at, enabled FROM user_provider_keys WHERE id = ?"
	err := s.db.GetContext(ctx, &pk, q, id)
	if errors.Is(err, sql.ErrNoRows) {
		return nil, ErrNotFound
	}
	if err != nil {
		return nil, fmt.Errorf("keymgmt: get provider key %q: %w", id, err)
	}
	return &pk, nil
}

// ListByOwner returns all provider keys owned by the given user, ordered by
// created_at (stable order for tests and UI). L0 scale is small (single
// user's keys); pagination is deferred to L4.
func (s *Store) ListByOwner(ctx context.Context, ownerUserID string) ([]UserProviderKey, error) {
	var keys []UserProviderKey
	q := "SELECT id, owner_user_id, provider_type, encrypted_key, key_last4, base_url, created_by, created_at, last_used_at, enabled FROM user_provider_keys WHERE owner_user_id = ? ORDER BY created_at ASC"
	if err := s.db.SelectContext(ctx, &keys, q, ownerUserID); err != nil {
		return nil, fmt.Errorf("keymgmt: list provider keys for %q: %w", ownerUserID, err)
	}
	return keys, nil
}

// Delete removes a provider key by ID. Returns ErrNotFound when the ID does
// not exist so handlers can distinguish 404 from 204.
func (s *Store) Delete(ctx context.Context, id string) error {
	res, err := s.db.ExecContext(ctx, "DELETE FROM user_provider_keys WHERE id = ?", id)
	if err != nil {
		return fmt.Errorf("keymgmt: delete provider key %q: %w", id, err)
	}
	n, err := res.RowsAffected()
	if err != nil {
		return fmt.Errorf("keymgmt: rows affected: %w", err)
	}
	if n == 0 {
		return ErrNotFound
	}
	return nil
}

// newKeyID returns a random 128-bit provider-key ID as a hex string prefixed
// with "pk_" (3 + 32 = 35 chars). Collision-resistant for L0/L1 scale; a
// collision would cause an INSERT failure (PRIMARY KEY) which the caller
// surfaces as an error.
func newKeyID() string {
	b := make([]byte, 16)
	if _, err := rand.Read(b); err != nil {
		return fmt.Sprintf("pk_fallback_%d", time.Now().UnixNano())
	}
	return "pk_" + hex.EncodeToString(b)
}

// boolToInt maps a Go bool to the SQLite INTEGER representation used by the
// enabled column (1/0). SQLite has no native BOOLEAN type.
func boolToInt(b bool) int {
	if b {
		return 1
	}
	return 0
}
