// Package vkey - Store implements VirtualKey Issue/Revoke/List against the
// SQLite DB. The Store owns all SHA-256 hashing; callers never handle hashes
// directly. Issue generates a random 128-bit token, hashes it with SHA-256,
// stores the hex digest, and returns the plaintext token ONCE (constitution
// XX: plaintext never persisted).
//
// The Store depends only on *storage.DB (constitution VII: focused interface,
// injected dep); snapshot-push triggering lives in the handler layer so the
// Store stays free of gRPC concerns.
package vkey

import (
	"context"
	"crypto/rand"
	"crypto/sha256"
	"database/sql"
	"encoding/hex"
	"encoding/json"
	"errors"
	"fmt"
	"time"

	"github.com/whw23/pingogate/ctrl-go/internal/storage"
)

// ErrNotFound is returned by Revoke when no virtual key matches the ID.
var ErrNotFound = errors.New("vkey: virtual key not found")

// Store implements VirtualKey Issue/Revoke/List against the wrapped *storage.DB.
type Store struct {
	db *storage.DB
}

// NewStore wraps a *storage.DB. The DB must already have migrations applied.
func NewStore(db *storage.DB) *Store {
	return &Store{db: db}
}

// Issue generates a new plaintext token (32 random bytes, ~128 bits of
// entropy, hex-encoded and prefixed with pg_vkey_), hashes it with
// SHA-256, stores the hex digest, and returns the plaintext token ONCE.
// ownerUserID is required. scope.ProviderKeyID is optional.
func (s *Store) Issue(ctx context.Context, ownerID string, scope Scope) (string, *VirtualKey, error) {
	if ownerID == "" {
		return "", nil, fmt.Errorf("vkey: owner_user_id is required")
	}
	plaintext, err := generateToken()
	if err != nil {
		return "", nil, fmt.Errorf("vkey: generate token: %w", err)
	}
	tokenHash := hashToken(plaintext)
	allowedModels, err := encodeJSONList(scope.AllowedModels)
	if err != nil {
		return "", nil, fmt.Errorf("vkey: encode allowed_models: %w", err)
	}
	allowedProviders, err := encodeJSONList(scope.AllowedProviders)
	if err != nil {
		return "", nil, fmt.Errorf("vkey: encode allowed_providers: %w", err)
	}
	now := time.Now().UTC().Format(time.RFC3339)
	vk := &VirtualKey{
		ID:               newKeyID(),
		TokenHash:        tokenHash,
		OwnerUserID:      ownerID,
		ProviderKeyID:    scope.ProviderKeyID,
		AllowedModels:    scope.AllowedModels,
		AllowedProviders: scope.AllowedProviders,
		ExpiresAt:        scope.ExpiresAt,
		MaxConcurrency:   scope.MaxConcurrency,
		Enabled:          true,
		CreatedAt:        now,
	}
	q := `INSERT INTO virtual_keys
	      (id, token_hash, owner_user_id, provider_key_id, allowed_models, allowed_providers, expires_at, max_concurrency, enabled, created_at)
	      VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)`
	_, err = s.db.ExecContext(ctx, q,
		vk.ID, vk.TokenHash, vk.OwnerUserID, nullableString(vk.ProviderKeyID),
		allowedModels, allowedProviders, vk.ExpiresAt, vk.MaxConcurrency,
		boolToInt(vk.Enabled), vk.CreatedAt)
	if err != nil {
		return "", nil, fmt.Errorf("vkey: insert virtual key: %w", err)
	}
	return plaintext, vk, nil
}

// Revoke disables a virtual key by setting enabled=0. Returns ErrNotFound
// when the ID does not exist.
// GetByID returns a single virtual key by its ID. Returns ErrNotFound if the
// key does not exist.
func (s *Store) GetByID(ctx context.Context, id string) (*VirtualKey, error) {
	keys, err := s.selectKeys(ctx, "WHERE id = ? LIMIT 1", id)
	if err != nil {
		return nil, fmt.Errorf("vkey: get by id %q: %w", id, err)
	}
	if len(keys) == 0 {
		return nil, ErrNotFound
	}
	return &keys[0], nil
}

func (s *Store) Revoke(ctx context.Context, id string) error {
	res, err := s.db.ExecContext(ctx,
		"UPDATE virtual_keys SET enabled = 0 WHERE id = ? AND enabled = 1", id)
	if err != nil {
		return fmt.Errorf("vkey: revoke %q: %w", id, err)
	}
	n, err := res.RowsAffected()
	if err != nil {
		return fmt.Errorf("vkey: rows affected: %w", err)
	}
	if n == 0 {
		return ErrNotFound
	}
	return nil
}

// ListByOwner returns all virtual keys owned by the given user, ordered by
// created_at. TokenHash is loaded but tagged json:- so never serialized.
func (s *Store) ListByOwner(ctx context.Context, ownerID string) ([]VirtualKey, error) {
	rows, err := s.selectKeys(ctx,
		"WHERE owner_user_id = ? ORDER BY created_at ASC", ownerID)
	if err != nil {
		return nil, fmt.Errorf("vkey: list virtual keys for %q: %w", ownerID, err)
	}
	return rows, nil
}

// ListAllEnabled returns all enabled virtual keys (used by the snapshot
// builder). Only enabled keys are included (constitution X).
func (s *Store) ListAllEnabled(ctx context.Context) ([]VirtualKey, error) {
	rows, err := s.selectKeys(ctx, "WHERE enabled = 1 ORDER BY id ASC")
	if err != nil {
		return nil, fmt.Errorf("vkey: list all enabled virtual keys: %w", err)
	}
	return rows, nil
}

// vkeyRow is the private DB row model.
type vkeyRow struct {
	ID                string           `db:"id"`
	TokenHash         string           `db:"token_hash"`
	OwnerUserID       string           `db:"owner_user_id"`
	ProviderKeyID     sql.NullString   `db:"provider_key_id"`
	AllowedModels     sql.NullString   `db:"allowed_models"`
	AllowedProviders  sql.NullString   `db:"allowed_providers"`
	ExpiresAt         int64            `db:"expires_at"`
	MaxConcurrency    int32            `db:"max_concurrency"`
	Enabled           int              `db:"enabled"`
	CreatedAt         string           `db:"created_at"`
}

// selectKeys runs the canonical SELECT with a caller-supplied WHERE/ORDER.
func (s *Store) selectKeys(ctx context.Context, whereSuffix string, args ...any) ([]VirtualKey, error) {
	q := "SELECT id, token_hash, owner_user_id, provider_key_id, allowed_models, " +
		"allowed_providers, expires_at, max_concurrency, enabled, created_at " +
		"FROM virtual_keys " + whereSuffix
	var rows []vkeyRow
	if err := s.db.SelectContext(ctx, &rows, q, args...); err != nil {
		return nil, err
	}
	keys := make([]VirtualKey, 0, len(rows))
	for _, r := range rows {
		vk := VirtualKey{
			ID:             r.ID,
			TokenHash:      r.TokenHash,
			OwnerUserID:    r.OwnerUserID,
			ProviderKeyID:  r.ProviderKeyID.String,
			ExpiresAt:      r.ExpiresAt,
			MaxConcurrency: r.MaxConcurrency,
			Enabled:        r.Enabled == 1,
			CreatedAt:      r.CreatedAt,
		}
		vk.AllowedModels = decodeJSONList(r.AllowedModels)
		vk.AllowedProviders = decodeJSONList(r.AllowedProviders)
		keys = append(keys, vk)
	}
	return keys, nil
}

// generateToken returns a random 128-bit plaintext token prefixed with
// pg_vkey_ (8 + 64 = 72 chars). Uses crypto/rand.
func generateToken() (string, error) {
	b := make([]byte, 32)
	if _, err := rand.Read(b); err != nil {
		return "", fmt.Errorf("crypto/rand: %w", err)
	}
	return "pg_vkey_" + hex.EncodeToString(b), nil
}

// hashToken returns SHA-256(plaintext) as lowercase hex (64 chars). Matches
// the Rust kernel hex_sha256 format (core-rs/pipeline/src/virtual_key_auth.rs).
// SHA-256 (not bcrypt) is intentional: the token has ~128 bits of entropy.
func hashToken(plaintext string) string {
	sum := sha256.Sum256([]byte(plaintext))
	return hex.EncodeToString(sum[:])
}

// newKeyID returns a random 128-bit ID prefixed with vk_.
func newKeyID() string {
	b := make([]byte, 16)
	if _, err := rand.Read(b); err != nil {
		return fmt.Sprintf("vk_fallback_%d", time.Now().UnixNano())
	}
	return "vk_" + hex.EncodeToString(b)
}

// encodeJSONList serializes a string slice into a JSON array string.
// nil returns empty string (stored as NULL); non-nil returns the JSON array.
func encodeJSONList(list []string) (string, error) {
	if list == nil {
		return "", nil
	}
	b, err := json.Marshal(list)
	if err != nil {
		return "", err
	}
	return string(b), nil
}

// decodeJSONList parses a JSON array string. Empty/NULL returns nil.
func decodeJSONList(s sql.NullString) []string {
	if !s.Valid || s.String == "" {
		return nil
	}
	var list []string
	if err := json.Unmarshal([]byte(s.String), &list); err != nil {
		return nil
	}
	return list
}

// nullableString returns nil for empty string (stored as NULL).
func nullableString(s string) any {
	if s == "" {
		return nil
	}
	return s
}

// boolToInt maps bool to SQLite INTEGER (1/0).
func boolToInt(b bool) int {
	if b {
		return 1
	}
	return 0
}
