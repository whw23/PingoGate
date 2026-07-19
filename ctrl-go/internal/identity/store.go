// Package identity - Store provides User CRUD against the SQLite DB. The Store
// owns all bcrypt hashing; callers never handle hashes directly. Create returns
// the plaintext API token once (constitution XX: plaintext never persisted);
// callers must surface it to the user immediately and not log it.
package identity

import (
	"context"
	"database/sql"
	"errors"
	"fmt"
	"strings"
	"time"

	"golang.org/x/crypto/bcrypt"

	"github.com/whw23/pingogate/ctrl-go/internal/storage"
)

// ErrNotFound is returned by Get/Delete/VerifyToken when no user matches.
// Sentinels keep handler-layer HTTP mapping clean (constitution IX).
var ErrNotFound = errors.New("identity: user not found")

// ErrEmailTaken is returned by Create when a user with the email already
// exists. SQLite UNIQUE(email) is the source of truth; we map the constraint
// violation to this sentinel so callers don't inspect driver-specific errors.
var ErrEmailTaken = errors.New("identity: email already taken")

// Store implements User CRUD against the wrapped *storage.DB. It is the only
// component that touches the users table; handlers and middleware depend on
// Store, not on raw SQL (constitution VII: focused interface, injected dep).
type Store struct {
	db *storage.DB
}

// NewStore wraps a *storage.DB. The DB must already have migrations applied
// (T17 storage.Open does this); Store does not run migrations itself.
func NewStore(db *storage.DB) *Store {
	return &Store{db: db}
}

// Create inserts a new non-admin user with the given plaintext API token. The
// token is bcrypt-hashed before storage; the plaintext is returned as the
// second result so the caller can surface it to the user once and never persist
// it (constitution XX). Callers that don't have a predetermined token (e.g.,
// the POST /api/users handler) should call generateToken() first and pass the
// result in; bootstrap passes the env-var token directly so the first admin
// knows its token without it being surfaced over HTTP.
//
// The user is created with is_admin=0; SetAdmin promotes an existing user.
// Keeping Create and SetAdmin separate avoids a confusing "create-and-promote"
// call site and matches the brief's bootstrap flow (Create then SetAdmin).
func (s *Store) Create(ctx context.Context, email, apiToken string) (*User, string, error) {
	if email == "" {
		return nil, "", fmt.Errorf("identity: email is required")
	}
	if apiToken == "" {
		return nil, "", fmt.Errorf("identity: api token is required")
	}
	hash, err := bcrypt.GenerateFromPassword([]byte(apiToken), bcrypt.DefaultCost)
	if err != nil {
		return nil, "", fmt.Errorf("identity: bcrypt hash: %w", err)
	}

	now := time.Now().UTC().Format(time.RFC3339)
	u := &User{
		ID:           newID(),
		Email:        email,
		APITokenHash: string(hash),
		IsAdmin:      false,
		CreatedAt:    now,
		UpdatedAt:    now,
	}

	const q = `INSERT INTO users (id, email, api_token_hash, is_admin, created_at, updated_at)
	           VALUES (?, ?, ?, 0, ?, ?)`
	_, err = s.db.ExecContext(ctx, q, u.ID, u.Email, u.APITokenHash, u.CreatedAt, u.UpdatedAt)
	if err != nil {
		// SQLite error text is "UNIQUE constraint failed: users.email" on
		// duplicate email. Match on the stable substring rather than importing
		// the sqlite3 driver type (keeps the dep surface minimal).
		if strings.Contains(err.Error(), "UNIQUE constraint failed") {
			return nil, "", ErrEmailTaken
		}
		return nil, "", fmt.Errorf("identity: insert user: %w", err)
	}
	return u, apiToken, nil
}

// GetByID loads a single user by primary key. Returns ErrNotFound when no row
// matches, so handlers can map cleanly to 404 (constitution IX).
func (s *Store) GetByID(ctx context.Context, id string) (*User, error) {
	var u User
	const q = `SELECT id, email, api_token_hash, is_admin, created_at, updated_at
	           FROM users WHERE id = ?`
	err := s.db.GetContext(ctx, &u, q, id)
	if errors.Is(err, sql.ErrNoRows) {
		return nil, ErrNotFound
	}
	if err != nil {
		return nil, fmt.Errorf("identity: get user %q: %w", id, err)
	}
	return &u, nil
}

// List returns all users ordered by created_at (stable order for tests and UI).
// L0 scale is small (single admin org); pagination is deferred to L4.
func (s *Store) List(ctx context.Context) ([]User, error) {
	var users []User
	const q = `SELECT id, email, api_token_hash, is_admin, created_at, updated_at
	           FROM users ORDER BY created_at ASC`
	if err := s.db.SelectContext(ctx, &users, q); err != nil {
		return nil, fmt.Errorf("identity: list users: %w", err)
	}
	return users, nil
}

// Delete removes a user by ID. Returns ErrNotFound when the ID does not exist
// so handlers can distinguish 404 from 204. FOREIGN KEY ON (user_provider_keys)
// will block deletion if the user owns provider keys; that surfaces as a plain
// error (callers can map to 409 Conflict at the handler layer).
func (s *Store) Delete(ctx context.Context, id string) error {
	res, err := s.db.ExecContext(ctx, `DELETE FROM users WHERE id = ?`, id)
	if err != nil {
		return fmt.Errorf("identity: delete user %q: %w", id, err)
	}
	n, err := res.RowsAffected()
	if err != nil {
		return fmt.Errorf("identity: rows affected: %w", err)
	}
	if n == 0 {
		return ErrNotFound
	}
	return nil
}

// CountAdmins returns the number of users with is_admin=1. Used by
// BootstrapAdmin to decide whether to create the initial admin.
func (s *Store) CountAdmins(ctx context.Context) (int, error) {
	var count int
	const q = `SELECT COUNT(*) FROM users WHERE is_admin = 1`
	if err := s.db.GetContext(ctx, &count, q); err != nil {
		return 0, fmt.Errorf("identity: count admins: %w", err)
	}
	return count, nil
}

// SetAdmin sets the is_admin flag on an existing user and returns the updated
// user. SetAdmin(false) is allowed (demotion); the caller is responsible for
// ensuring at least one admin remains (not enforced here to keep the method
// side-effect-free beyond the flag flip).
func (s *Store) SetAdmin(ctx context.Context, id string, isAdmin bool) (*User, error) {
	flag := 0
	if isAdmin {
		flag = 1
	}
	now := time.Now().UTC().Format(time.RFC3339)
	res, err := s.db.ExecContext(ctx,
		`UPDATE users SET is_admin = ?, updated_at = ? WHERE id = ?`,
		flag, now, id)
	if err != nil {
		return nil, fmt.Errorf("identity: set admin %q: %w", id, err)
	}
	n, err := res.RowsAffected()
	if err != nil {
		return nil, fmt.Errorf("identity: rows affected: %w", err)
	}
	if n == 0 {
		return nil, ErrNotFound
	}
	return s.GetByID(ctx, id)
}

// VerifyToken returns the User whose bcrypt-hashed API token matches the given
// plaintext. Returns ErrNotFound when no user matches. Used by AuthMiddleware.
//
// Implementation note: there is no indexed lookup by token (we store only the
// hash, so lookup is necessarily linear). L0 scale is small (single-admin-org,
// dozens of users at most); L4+ would add a token-id prefix index. bcrypt's
// CompareHashAndPassword is constant-time per comparison.
func (s *Store) VerifyToken(ctx context.Context, plaintext string) (*User, error) {
	users, err := s.List(ctx)
	if err != nil {
		return nil, err
	}
	for i := range users {
		if bcrypt.CompareHashAndPassword([]byte(users[i].APITokenHash), []byte(plaintext)) == nil {
			return &users[i], nil
		}
	}
	return nil, ErrNotFound
}
