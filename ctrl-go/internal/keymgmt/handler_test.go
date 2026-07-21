// Package keymgmt - tests for UserProviderKey CRUD HTTP handlers and the
// KeyVault client (constitution XIII: TDD, observe failure first). Uses an
// in-memory SQLite DB (T17 storage.Open) and a fake KeyVaultClient so tests
// are hermetic and run without the Rust binary.
package keymgmt

import (
	"context"
	"encoding/json"
	"net/http"
	"net/http/httptest"
	"strings"
	"testing"

	"github.com/go-chi/chi/v5"

	"github.com/whw23/pingogate/ctrl-go/internal/identity"
	"github.com/whw23/pingogate/ctrl-go/internal/storage"
)

// newTestRouter builds a chi router with AuthMiddleware at the root and the
// provider-key CRUD routes mounted via RegisterRoutes. The identity.Store and
// keymgmt.Store share the same DB so the FK from user_provider_keys to users
// resolves. Returns the router and a Bearer token for the bootstrap admin.
func newTestRouter(t *testing.T, kv KeyVaultClient) (http.Handler, *Store, *identity.Store, string) {
	t.Helper()
	db, err := storage.Open(":memory:")
	if err != nil {
		t.Fatalf("open in-memory DB: %v", err)
	}
	t.Cleanup(func() { _ = db.Close() })
	idStore := identity.NewStore(db)
	keyStore := NewStore(db)

	ctx := context.Background()
	if _, err := identity.BootstrapAdmin(ctx, idStore, "bootstrap-test-token"); err != nil {
		t.Fatalf("bootstrap admin: %v", err)
	}

	r := chi.NewRouter()
	r.Use(identity.AuthMiddleware(idStore))
	RegisterRoutes(r, keyStore, kv)
	return r, keyStore, idStore, "bootstrap-test-token"
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

// TestCreateAndGetProviderKey covers the happy path: POST a plaintext key,
// verify the response contains no plaintext and no ciphertext, then GET the
// key by ID and verify key_last4 matches the last 4 chars of the plaintext.
func TestCreateAndGetProviderKey(t *testing.T) {
	kv := newFakeKeyVaultClient()
	r, _, _, adminToken := newTestRouter(t, kv)

	plaintext := "sk-test-1234567890abcdef"
	body := "{\"provider_type\":\"openai\",\"plaintext_key\":\"" + plaintext + "\"}"
	rec := doRequest(t, r, http.MethodPost, "/api/provider-keys",
		"Bearer "+adminToken, body)
	if rec.Code != http.StatusCreated {
		t.Fatalf("create: status = %d, want %d; body = %s",
			rec.Code, http.StatusCreated, rec.Body.String())
	}

	var resp createResponse
	if err := json.Unmarshal(rec.Body.Bytes(), &resp); err != nil {
		t.Fatalf("unmarshal: %v; body=%s", err, rec.Body.String())
	}
	if resp.ID == "" {
		t.Fatal("create response id is empty")
	}
	if resp.ProviderType != "openai" {
		t.Fatalf("provider_type = %q, want %q", resp.ProviderType, "openai")
	}
	if resp.KeyLast4 != "cdef" {
		t.Fatalf("key_last4 = %q, want %q", resp.KeyLast4, "cdef")
	}
	if resp.CreatedBy != resp.OwnerUserID {
		t.Fatalf("created_by != owner_user_id (BYOK): %q vs %q",
			resp.CreatedBy, resp.OwnerUserID)
	}
	if !resp.Enabled {
		t.Fatal("new key should be enabled")
	}

	if strings.Contains(rec.Body.String(), plaintext) {
		t.Fatalf("response leaks plaintext: %s", rec.Body.String())
	}
	if strings.Contains(rec.Body.String(), "ENC:") {
		t.Fatalf("response leaks ciphertext marker: %s", rec.Body.String())
	}
	if strings.Contains(rec.Body.String(), "encrypted_key") {
		t.Fatalf("response leaks encrypted_key field: %s", rec.Body.String())
	}

	rec = doRequest(t, r, http.MethodGet, "/api/provider-keys/"+resp.ID,
		"Bearer "+adminToken, "")
	if rec.Code != http.StatusOK {
		t.Fatalf("get: status = %d, want %d; body = %s",
			rec.Code, http.StatusOK, rec.Body.String())
	}
	var got createResponse
	if err := json.Unmarshal(rec.Body.Bytes(), &got); err != nil {
		t.Fatalf("unmarshal get: %v; body=%s", err, rec.Body.String())
	}
	if got.ID != resp.ID {
		t.Fatalf("get id = %q, want %q", got.ID, resp.ID)
	}
	if got.KeyLast4 != "cdef" {
		t.Fatalf("get key_last4 = %q, want %q", got.KeyLast4, "cdef")
	}
}

// TestListProviderKeys verifies that List returns a JSON array with key_last4
// for each entry and NEVER includes ciphertext or plaintext.
func TestListProviderKeys(t *testing.T) {
	kv := newFakeKeyVaultClient()
	r, keyStore, idStore, adminToken := newTestRouter(t, kv)
	ctx := context.Background()

	admin, err := idStore.VerifyToken(ctx, adminToken)
	if err != nil {
		t.Fatalf("verify admin token: %v", err)
	}
	enc1, _ := kv.Encrypt(ctx, []byte("sk-first-AAAAAAAA"))
	enc2, _ := kv.Encrypt(ctx, []byte("sk-second-BBBBBBBB"))
	if _, err := keyStore.Create(ctx, admin.ID, "openai", enc1, "AAAA", "", admin.ID); err != nil {
		t.Fatalf("create key 1: %v", err)
	}
	if _, err := keyStore.Create(ctx, admin.ID, "anthropic", enc2, "BBBB", "", admin.ID); err != nil {
		t.Fatalf("create key 2: %v", err)
	}

	rec := doRequest(t, r, http.MethodGet, "/api/provider-keys", "Bearer "+adminToken, "")
	if rec.Code != http.StatusOK {
		t.Fatalf("list: status = %d, want %d; body = %s", rec.Code, http.StatusOK, rec.Body.String())
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
	last4set := map[string]bool{keys[0].KeyLast4: true, keys[1].KeyLast4: true}
	if !last4set["AAAA"] || !last4set["BBBB"] {
		t.Fatalf("list missing expected last4 values; got %v", last4set)
	}
	if strings.Contains(body, "ENC:") {
		t.Fatalf("list response leaks ciphertext marker: %s", body)
	}
	if strings.Contains(body, "sk-first") || strings.Contains(body, "sk-second") {
		t.Fatalf("list response leaks plaintext: %s", body)
	}
	if strings.Contains(body, "encrypted_key") {
		t.Fatalf("list response leaks encrypted_key field: %s", body)
	}
}

// TestCreate_EncryptFailure verifies that a KeyVault Encrypt failure maps to
// 502 (bad gateway) and that no row is inserted into the DB.
func TestCreate_EncryptFailure(t *testing.T) {
	kv := &failingKeyVaultClient{}
	r, keyStore, _, adminToken := newTestRouter(t, kv)

	body := "{\"provider_type\":\"openai\",\"plaintext_key\":\"sk-whatever\"}"
	rec := doRequest(t, r, http.MethodPost, "/api/provider-keys", "Bearer "+adminToken, body)
	if rec.Code != http.StatusBadGateway {
		t.Fatalf("create with encrypt failure: status = %d, want %d; body = %s",
			rec.Code, http.StatusBadGateway, rec.Body.String())
	}
	ctx := context.Background()
	keys, err := keyStore.ListByOwner(ctx, "")
	if err != nil {
		t.Fatalf("list after failed create: %v", err)
	}
	if len(keys) != 0 {
		t.Fatalf("expected 0 keys after failed create, got %d", len(keys))
	}
}

// TestCreate_Validation covers the 400 paths.
func TestCreate_Validation(t *testing.T) {
	kv := newFakeKeyVaultClient()
	r, _, _, adminToken := newTestRouter(t, kv)

	tests := []struct {
		name string
		body string
	}{
		{"missing provider_type", "{\"plaintext_key\":\"sk-test\"}"},
		{"missing plaintext_key", "{\"provider_type\":\"openai\"}"},
		{"empty provider_type", "{\"provider_type\":\"\",\"plaintext_key\":\"sk-test\"}"},
		{"empty plaintext_key", "{\"provider_type\":\"openai\",\"plaintext_key\":\"\"}"},
		{"malformed JSON", "{not json"},
	}
	for _, tc := range tests {
		t.Run(tc.name, func(t *testing.T) {
			rec := doRequest(t, r, http.MethodPost, "/api/provider-keys", "Bearer "+adminToken, tc.body)
			if rec.Code != http.StatusBadRequest {
				t.Fatalf("status = %d, want %d; body = %s", rec.Code, http.StatusBadRequest, rec.Body.String())
			}
		})
	}
}

// TestDeleteProviderKey covers the delete happy path and 404 for unknown ID.
func TestDeleteProviderKey(t *testing.T) {
	kv := newFakeKeyVaultClient()
	r, keyStore, idStore, adminToken := newTestRouter(t, kv)
	ctx := context.Background()

	admin, err := idStore.VerifyToken(ctx, adminToken)
	if err != nil {
		t.Fatalf("verify admin: %v", err)
	}
	enc, _ := kv.Encrypt(ctx, []byte("sk-to-delete-1234"))
	pk, err := keyStore.Create(ctx, admin.ID, "openai", enc, "1234", "", admin.ID)
	if err != nil {
		t.Fatalf("create key: %v", err)
	}

	rec := doRequest(t, r, http.MethodDelete, "/api/provider-keys/"+pk.ID, "Bearer "+adminToken, "")
	if rec.Code != http.StatusNoContent {
		t.Fatalf("delete existing: status = %d, want %d; body = %s", rec.Code, http.StatusNoContent, rec.Body.String())
	}
	rec = doRequest(t, r, http.MethodDelete, "/api/provider-keys/"+pk.ID, "Bearer "+adminToken, "")
	if rec.Code != http.StatusNotFound {
		t.Fatalf("delete missing: status = %d, want %d", rec.Code, http.StatusNotFound)
	}
}

// TestAuthAndAuthorize covers the 401/403 paths.
func TestAuthAndAuthorize(t *testing.T) {
	kv := newFakeKeyVaultClient()
	r, _, idStore, adminToken := newTestRouter(t, kv)
	ctx := context.Background()

	_, nonAdminToken, err := idStore.Create(ctx, "user@example.com", "user-token-123")
	if err != nil {
		t.Fatalf("create non-admin: %v", err)
	}

	rec := doRequest(t, r, http.MethodGet, "/api/provider-keys", "Bearer "+nonAdminToken, "")
	if rec.Code != http.StatusForbidden {
		t.Fatalf("non-admin GET: status = %d, want %d; body = %s", rec.Code, http.StatusForbidden, rec.Body.String())
	}
	rec = doRequest(t, r, http.MethodGet, "/api/provider-keys", "", "")
	if rec.Code != http.StatusUnauthorized {
		t.Fatalf("no-token GET: status = %d, want %d", rec.Code, http.StatusUnauthorized)
	}
	rec = doRequest(t, r, http.MethodGet, "/api/provider-keys", "Bearer pgt_wrong_token", "")
	if rec.Code != http.StatusUnauthorized {
		t.Fatalf("wrong-token GET: status = %d, want %d", rec.Code, http.StatusUnauthorized)
	}
	rec = doRequest(t, r, http.MethodGet, "/api/provider-keys", "Bearer "+adminToken, "")
	if rec.Code != http.StatusOK {
		t.Fatalf("admin GET: status = %d, want %d; body = %s", rec.Code, http.StatusOK, rec.Body.String())
	}
}
