// Package identity implements the L0 control-plane identity layer (constitution:
// Go non-kernel = all state). It provides the User entity, CRUD storage, a
// bootstrap admin flow, and the Principal/authorize boundary used by all
// control-plane HTTP endpoints. Plaintext API tokens are returned once at
// creation time and never stored; only bcrypt hashes persist (constitution XX).
package identity

import (
	"crypto/rand"
	"encoding/hex"
	"fmt"
	"time"
)

// User is the L0 identity entity (spec §3: flat, no tenant/project; multi-tenant
// deferred to L4). The API token is stored only as a bcrypt hash; the plaintext
// is returned once at creation and never persisted (constitution XX: no plaintext
// tokens at rest). APITokenHash is tagged json:"-" so it is never serialized.
type User struct {
	ID           string `db:"id" json:"id"`
	Email        string `db:"email" json:"email"`
	APITokenHash string `db:"api_token_hash" json:"-"`
	IsAdmin      bool   `db:"is_admin" json:"is_admin"`
	CreatedAt    string `db:"created_at" json:"created_at"`
	UpdatedAt    string `db:"updated_at" json:"updated_at"`
}

// generateToken returns a random 256-bit plaintext API token as a hex string
// prefixed with "pgt_" (4 + 64 = 68 chars). Uses crypto/rand; failure is
// extremely unlikely and surfaces as an error so callers can handle it.
func generateToken() (string, error) {
	b := make([]byte, 32)
	if _, err := rand.Read(b); err != nil {
		return "", fmt.Errorf("identity: crypto/rand: %w", err)
	}
	return "pgt_" + hex.EncodeToString(b), nil
}

// newID returns a random 128-bit user ID as a hex string prefixed with "u_"
// (2 + 32 = 34 chars). Not a UUID, but collision-resistant for L0 scale; a
// collision would cause an INSERT failure (PRIMARY KEY) which the caller
// handles. On crypto/rand failure (should never happen), falls back to a
// timestamp-based ID so the INSERT can proceed and surface any real issue.
func newID() string {
	b := make([]byte, 16)
	if _, err := rand.Read(b); err != nil {
		return fmt.Sprintf("u_fallback_%d", time.Now().UnixNano())
	}
	return "u_" + hex.EncodeToString(b)
}
