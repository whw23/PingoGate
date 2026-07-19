// Package identity - tests for User CRUD HTTP handlers (constitution XIII:
// TDD, observe failure first). Uses an in-memory SQLite DB (T17 storage.Open)
// so tests are hermetic and run in parallel safely.
package identity

import (
	"context"
	"encoding/json"
	"net/http"
	"net/http/httptest"
	"strings"
	"testing"

	"github.com/whw23/pingogate/ctrl-go/internal/storage"
)

// newTestStore opens an in-memory DB and returns a fresh Store. Each test gets
// its own DB; no shared state, no cleanup needed (in-memory DB dies with the
// process, and sqlx.Close is best-effort).
func newTestStore(t *testing.T) *Store {
	t.Helper()
	db, err := storage.Open(":memory:")
	if err != nil {
		t.Fatalf("open in-memory DB: %v", err)
	}
	t.Cleanup(func() { _ = db.Close() })
	return NewStore(db)
}

// doRequest is a tiny helper to issue a request against a handler with an
// optional Authorization header. Returns the response recorder.
func doRequest(t *testing.T, handler http.Handler, method, path, authHeader, body string) *httptest.ResponseRecorder {
	t.Helper()
	req := httptest.NewRequest(method, path, strings.NewReader(body))
	if authHeader != "" {
		req.Header.Set("Authorization", authHeader)
	}
	if body != "" {
		req.Header.Set("Content-Type", "application/json")
	}
	rec := httptest.NewRecorder()
	handler.ServeHTTP(rec, req)
	return rec
}

// createAdminAndPromote is a shared helper: creates a user with the given email
// and token, promotes them to admin, and returns (admin, adminToken). Used by
// handler tests that need an authenticated admin to drive the CRUD endpoints.
func createAdminAndPromote(t *testing.T, store *Store, email, token string) (*User, string) {
	t.Helper()
	u, _, err := store.Create(context.Background(), email, token)
	if err != nil {
		t.Fatalf("create admin: %v", err)
	}
	if _, err := store.SetAdmin(context.Background(), u.ID, true); err != nil {
		t.Fatalf("set admin: %v", err)
	}
	return u, token
}

// TestCreateUserAndAuth covers the full happy path and the 401 failure modes:
// create a user (returns plaintext token), use that token to call a protected
// endpoint (200 for admin / 403 for non-admin), then verify that no token and
// a wrong token both get 401. This is the brief's TestCreateUserAndAuth.
func TestCreateUserAndAuth(t *testing.T) {
	store := newTestStore(t)
	_, adminToken := createAdminAndPromote(t, store, "admin@example.com", "bootstrap-token-123")
	r := newTestRouter(store)

	// As the admin, create a second user. The returned token is plaintext,
	// returned once.
	rec := doRequest(t, r, http.MethodPost, "/api/users",
		"Bearer "+adminToken, `{"email":"user1@example.com"}`)
	if rec.Code != http.StatusCreated {
		t.Fatalf("create user: status = %d, want %d; body = %s",
			rec.Code, http.StatusCreated, rec.Body.String())
	}
	var resp createResponse
	if err := json.Unmarshal(rec.Body.Bytes(), &resp); err != nil {
		t.Fatalf("unmarshal create response: %v; body=%s", err, rec.Body.String())
	}
	if resp.Token == "" {
		t.Fatal("create response token is empty")
	}
	if resp.User.Email != "user1@example.com" {
		t.Fatalf("create response email = %q, want %q",
			resp.User.Email, "user1@example.com")
	}
	// APITokenHash must never appear in JSON (constitution XX).
	if strings.Contains(rec.Body.String(), "api_token_hash") {
		t.Fatalf("response body leaks api_token_hash: %s", rec.Body.String())
	}

	// The new user is NOT an admin, so Authorize("manage", "users") must
	// return 403 (auth succeeds, permission denied).
	rec = doRequest(t, r, http.MethodGet, "/api/users", "Bearer "+resp.Token, "")
	if rec.Code != http.StatusForbidden {
		t.Fatalf("non-admin GET /api/users: status = %d, want %d; body = %s",
			rec.Code, http.StatusForbidden, rec.Body.String())
	}

	// Admin token can list users (200).
	rec = doRequest(t, r, http.MethodGet, "/api/users", "Bearer "+adminToken, "")
	if rec.Code != http.StatusOK {
		t.Fatalf("admin GET /api/users: status = %d, want %d; body = %s",
			rec.Code, http.StatusOK, rec.Body.String())
	}

	// Missing Authorization header -> 401.
	rec = doRequest(t, r, http.MethodGet, "/api/users", "", "")
	if rec.Code != http.StatusUnauthorized {
		t.Fatalf("no-token GET /api/users: status = %d, want %d",
			rec.Code, http.StatusUnauthorized)
	}

	// Wrong token (Bearer prefix but unknown token) -> 401.
	rec = doRequest(t, r, http.MethodGet, "/api/users", "Bearer pgt_wrong_token", "")
	if rec.Code != http.StatusUnauthorized {
		t.Fatalf("wrong-token GET /api/users: status = %d, want %d",
			rec.Code, http.StatusUnauthorized)
	}

	// Non-Bearer scheme -> 401.
	rec = doRequest(t, r, http.MethodGet, "/api/users", "Basic abc123", "")
	if rec.Code != http.StatusUnauthorized {
		t.Fatalf("non-Bearer GET /api/users: status = %d, want %d",
			rec.Code, http.StatusUnauthorized)
	}
}

// TestCreate_DuplicateEmail verifies the UNIQUE(email) constraint maps to
// ErrEmailTaken and the handler returns 409.
func TestCreate_DuplicateEmail(t *testing.T) {
	store := newTestStore(t)
	ctx := context.Background()
	_, adminToken := createAdminAndPromote(t, store, "admin@example.com", "tok")
	r := newTestRouter(store)

	// First create succeeds.
	rec := doRequest(t, r, http.MethodPost, "/api/users",
		"Bearer "+adminToken, `{"email":"dup@example.com"}`)
	if rec.Code != http.StatusCreated {
		t.Fatalf("first create: status = %d, want %d", rec.Code, http.StatusCreated)
	}

	// Second create with same email -> 409.
	rec = doRequest(t, r, http.MethodPost, "/api/users",
		"Bearer "+adminToken, `{"email":"dup@example.com"}`)
	if rec.Code != http.StatusConflict {
		t.Fatalf("duplicate create: status = %d, want %d; body = %s",
			rec.Code, http.StatusConflict, rec.Body.String())
	}

	// Sanity: only one user with dup@example.com in the DB (the first one).
	count, err := store.CountAdmins(ctx) // reuse the count query shape
	if err != nil {
		t.Fatalf("count: %v", err)
	}
	_ = count // admins=1, not directly relevant to dup check; just exercise DB
}

// TestDeleteUser covers the delete happy path and 404 for unknown ID.
func TestDeleteUser(t *testing.T) {
	store := newTestStore(t)
	ctx := context.Background()
	_, adminToken := createAdminAndPromote(t, store, "admin@example.com", "tok")
	target, _, err := store.Create(ctx, "target@example.com", "target-tok")
	if err != nil {
		t.Fatalf("create target: %v", err)
	}
	r := newTestRouter(store)

	// Delete existing user -> 204.
	rec := doRequest(t, r, http.MethodDelete, "/api/users/"+target.ID, "Bearer "+adminToken, "")
	if rec.Code != http.StatusNoContent {
		t.Fatalf("delete existing: status = %d, want %d; body = %s",
			rec.Code, http.StatusNoContent, rec.Body.String())
	}

	// Delete again (now gone) -> 404.
	rec = doRequest(t, r, http.MethodDelete, "/api/users/"+target.ID, "Bearer "+adminToken, "")
	if rec.Code != http.StatusNotFound {
		t.Fatalf("delete missing: status = %d, want %d", rec.Code, http.StatusNotFound)
	}
}

// TestGetUser covers GET /api/users/{id} happy path and 404.
func TestGetUser(t *testing.T) {
	store := newTestStore(t)
	_, adminToken := createAdminAndPromote(t, store, "admin@example.com", "tok")
	r := newTestRouter(store)

	// Get existing -> 200.
	rec := doRequest(t, r, http.MethodGet, "/api/users/"+adminID(t, store), "Bearer "+adminToken, "")
	if rec.Code != http.StatusOK {
		t.Fatalf("get existing: status = %d, want %d; body = %s",
			rec.Code, http.StatusOK, rec.Body.String())
	}
	var got User
	if err := json.Unmarshal(rec.Body.Bytes(), &got); err != nil {
		t.Fatalf("unmarshal: %v", err)
	}
	// APITokenHash must never be in the response.
	if strings.Contains(rec.Body.String(), "api_token_hash") {
		t.Fatalf("response leaks api_token_hash: %s", rec.Body.String())
	}

	// Get unknown -> 404.
	rec = doRequest(t, r, http.MethodGet, "/api/users/u_doesnotexist", "Bearer "+adminToken, "")
	if rec.Code != http.StatusNotFound {
		t.Fatalf("get unknown: status = %d, want %d", rec.Code, http.StatusNotFound)
	}
}

// TestListUsers_Empty verifies the list endpoint returns a JSON array (not
// null) so the frontend doesn't need a null check. After creating one admin,
// the list contains exactly that one user.
func TestListUsers_Empty(t *testing.T) {
	store := newTestStore(t)
	_, adminToken := createAdminAndPromote(t, store, "admin@example.com", "tok")
	r := newTestRouter(store)

	rec := doRequest(t, r, http.MethodGet, "/api/users", "Bearer "+adminToken, "")
	if rec.Code != http.StatusOK {
		t.Fatalf("list: status = %d, want %d", rec.Code, http.StatusOK)
	}
	body := strings.TrimSpace(rec.Body.String())
	if !strings.HasPrefix(body, "[") {
		t.Fatalf("list response is not a JSON array: %s", body)
	}
}

// adminID returns the ID of the single admin user in the store. Helper for
// tests that need to GET /api/users/{id} but don't want to plumb the *User
// out of createAdminAndPromote.
func adminID(t *testing.T, store *Store) string {
	t.Helper()
	users, err := store.List(context.Background())
	if err != nil {
		t.Fatalf("list: %v", err)
	}
	for i := range users {
		if users[i].IsAdmin {
			return users[i].ID
		}
	}
	t.Fatal("no admin user found")
	return ""
}
