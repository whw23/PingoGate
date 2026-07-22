// Package vkey - tests for VirtualKey Issue/Revoke/List handlers and Store
// (constitution XIII: TDD). Uses an in-memory SQLite DB (T17 storage.Open) so
// tests are hermetic and run without the Rust binary or gRPC server.
package vkey

import (
	"context"
	"crypto/sha256"
	"encoding/hex"
	"encoding/json"
	"net/http"
	"net/http/httptest"
	"strings"
	"testing"

	"github.com/go-chi/chi/v5"

	"github.com/whw23/pingogate/ctrl-go/internal/identity"
	"github.com/whw23/pingogate/ctrl-go/internal/storage"
)

// newTestRouter builds a chi router with AuthMiddleware + vkey routes.
// Returns the router, the vkey Store, the identity Store, and the bootstrap
// admin token for authenticated requests. The pusher is nil (no push).
func newTestRouter(t *testing.T) (http.Handler, *Store, *identity.Store, string) {
	return newTestRouterWithPusher(t, nil)
}

// newTestRouterWithPusher is like newTestRouter but injects a SnapshotPusher.
func newTestRouterWithPusher(t *testing.T, pusher SnapshotPusher) (http.Handler, *Store, *identity.Store, string) {
	t.Helper()
	db, err := storage.Open(":memory:")
	if err != nil {
		t.Fatalf("open in-memory DB: %v", err)
	}
	t.Cleanup(func() { _ = db.Close() })
	idStore := identity.NewStore(db)
	vkeyStore := NewStore(db)

	ctx := context.Background()
	if _, err := identity.BootstrapAdmin(ctx, idStore, "bootstrap-test-token"); err != nil {
		t.Fatalf("bootstrap admin: %v", err)
	}

	r := chi.NewRouter()
	r.Use(identity.AuthMiddleware(idStore))
	RegisterRoutes(r, vkeyStore, pusher)
	return r, vkeyStore, idStore, "bootstrap-test-token"
}

// doRequest issues a request with an optional Authorization header.
func doRequest(t *testing.T, h http.Handler, method, path, authHeader, body string) *httptest.ResponseRecorder {
	t.Helper()
	req := httptest.NewRequest(method, path, strings.NewReader(body))
	if authHeader != "" {
		req.Header.Set("Authorization", authHeader)
	}
	if body != "" {
		req.Header.Set("Content-Type", "application/json")
	}
	rec := httptest.NewRecorder()
	h.ServeHTTP(rec, req)
	return rec
}

// TestIssueReturnsPlaintextAndHashesInDB verifies the Issue happy path:
// POST returns a plaintext token once, the DB stores the SHA-256 hash (not
// the plaintext), and the response does not leak the hash. The hash format
// matches the Rust kernel hex_sha256 (T26): hex(sha256(plaintext)).
func TestIssueReturnsPlaintextAndHashesInDB(t *testing.T) {
	r, vkeyStore, _, adminToken := newTestRouter(t)

	rec := doRequest(t, r, http.MethodPost, "/api/virtual-keys",
		"Bearer "+adminToken, "{}")
	if rec.Code != http.StatusCreated {
		t.Fatalf("issue: status = %d, want %d; body = %s",
			rec.Code, http.StatusCreated, rec.Body.String())
	}

	var resp issueResponse
	if err := json.Unmarshal(rec.Body.Bytes(), &resp); err != nil {
		t.Fatalf("unmarshal: %v; body=%s", err, rec.Body.String())
	}
	if resp.Token == "" {
		t.Fatal("issue response token is empty")
	}
	if resp.ID == "" {
		t.Fatal("issue response id is empty")
	}
	if !resp.Enabled {
		t.Fatal("new vkey should be enabled")
	}

	// The response MUST NOT leak token_hash (TokenHash is json:-).
	if strings.Contains(rec.Body.String(), "token_hash") {
		t.Fatalf("response leaks token_hash: %s", rec.Body.String())
	}

	// The DB MUST store the SHA-256 hash, not the plaintext.
	keys, err := vkeyStore.ListByOwner(context.Background(), resp.OwnerUserID)
	if err != nil {
		t.Fatalf("list vkeys: %v", err)
	}
	if len(keys) != 1 {
		t.Fatalf("expected 1 vkey in DB, got %d", len(keys))
	}
	storedHash := keys[0].TokenHash
	if storedHash == "" {
		t.Fatal("DB token_hash is empty")
	}
	if storedHash == resp.Token {
		t.Fatal("DB stores plaintext as token_hash")
	}
	if strings.Contains(resp.Token, storedHash) {
		t.Fatal("plaintext contains hash (hash is substring of token)")
	}

	// The stored hash MUST equal hex(sha256(plaintext)) -- matches Rust T26.
	sum := sha256.Sum256([]byte(resp.Token))
	wantHash := hex.EncodeToString(sum[:])
	if storedHash != wantHash {
		t.Fatalf("token_hash = %q, want %q (hex(sha256(token)))",
			storedHash, wantHash)
	}
}

// TestRevokeSetsEnabledZero verifies that DELETE sets enabled=0 (soft
// delete for audit) and returns 204. A second DELETE returns 404.
func TestRevokeSetsEnabledZero(t *testing.T) {
	r, vkeyStore, _, adminToken := newTestRouter(t)

	// Issue a vkey via the Store directly (no pusher needed for this test).
	adminID := getAdminID(t, vkeyStore, adminToken)
	plaintext, vk, err := vkeyStore.Issue(context.Background(), adminID, Scope{})
	if err != nil {
		t.Fatalf("issue: %v", err)
	}
	_ = plaintext

	rec := doRequest(t, r, http.MethodDelete, "/api/virtual-keys/"+vk.ID,
		"Bearer "+adminToken, "")
	if rec.Code != http.StatusNoContent {
		t.Fatalf("revoke: status = %d, want %d; body = %s",
			rec.Code, http.StatusNoContent, rec.Body.String())
	}

	// Verify enabled=0 in the DB.
	keys, err := vkeyStore.ListByOwner(context.Background(), adminID)
	if err != nil {
		t.Fatalf("list: %v", err)
	}
	var found bool
	for _, k := range keys {
		if k.ID == vk.ID {
			found = true
			if k.Enabled {
				t.Fatal("revoked vkey is still enabled")
			}
		}
	}
	if !found {
		t.Fatal("revoked vkey not found in DB (should be kept for audit)")
	}

	// Second DELETE returns 404 (Revoke returns ErrNotFound when n==0,
	// because the UPDATE WHERE id=? matches no enabled row).
	rec = doRequest(t, r, http.MethodDelete, "/api/virtual-keys/"+vk.ID,
		"Bearer "+adminToken, "")
	if rec.Code != http.StatusNotFound {
		t.Fatalf("second revoke: status = %d, want %d",
			rec.Code, http.StatusNotFound)
	}
}

// getAdminID resolves the bootstrap admin user ID from their token.
func getAdminID(t *testing.T, store *Store, token string) string {
	t.Helper()
	// We need the identity store to verify the token, but we only have the
	// vkey store. Instead, we use a direct DB query via the vkey store DB.
	// This is a test-only shortcut; production code uses identity.Store.
	var id string
	q := "SELECT id FROM users WHERE is_admin = 1 LIMIT 1"
	if err := store.db.GetContext(context.Background(), &id, q); err != nil {
		t.Fatalf("get admin id: %v", err)
	}
	_ = token
	return id
}

// TestListReturnsNoPlaintext verifies that GET returns key metadata
// (id, owner, scope) but NEVER token_hash or plaintext.
func TestListReturnsNoPlaintext(t *testing.T) {
	r, vkeyStore, _, adminToken := newTestRouter(t)
	adminID := getAdminID(t, vkeyStore, adminToken)

	// Issue two vkeys.
	pt1, _, err := vkeyStore.Issue(context.Background(), adminID, Scope{})
	if err != nil {
		t.Fatalf("issue 1: %v", err)
	}
	pt2, _, err := vkeyStore.Issue(context.Background(), adminID, Scope{
		AllowedModels: []string{"gpt-4"},
	})
	if err != nil {
		t.Fatalf("issue 2: %v", err)
	}

	rec := doRequest(t, r, http.MethodGet, "/api/virtual-keys",
		"Bearer "+adminToken, "")
	if rec.Code != http.StatusOK {
		t.Fatalf("list: status = %d, want %d; body = %s",
			rec.Code, http.StatusOK, rec.Body.String())
	}
	body := strings.TrimSpace(rec.Body.String())
	if !strings.HasPrefix(body, "[") {
		t.Fatalf("list response is not a JSON array: %s", body)
	}

	var keys listResponse
	if err := json.Unmarshal([]byte(body), &keys); err != nil {
		t.Fatalf("unmarshal list: %v; body=%s", err, body)
	}
	if len(keys) != 2 {
		t.Fatalf("list returned %d keys, want 2", len(keys))
	}

	// No plaintext, no hash.
	if strings.Contains(body, pt1) || strings.Contains(body, pt2) {
		t.Fatalf("list response leaks plaintext: %s", body)
	}
	if strings.Contains(body, "token_hash") {
		t.Fatalf("list response leaks token_hash field: %s", body)
	}
}
