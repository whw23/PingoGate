// Package keymgmt - tests for the Reveal endpoint (S3 dual-defense, Go L1
// side; constitution XX "1Password model"). Extracted from handler_test.go to
// keep that file under constitution V's 300-line limit. Same package, compiles
// as part of the same test binary.
//
// These tests exercise only the Go L1 check (created_by == requester). The
// Rust L2 check (requester == snapshot.key_owners[key_id]) is exercised in
// core-rs/keyvault/src/grpc_service.rs - the Go tests use a fake KeyVaultClient
// that always returns the plaintext, simulating a successful L2 check. This is
// the correct separation: Go tests verify Go's responsibility, Rust tests
// verify Rust's responsibility.
//
// Test scenarios:
//   - TestRevealAllowedForCreator: creator reveals plaintext, gets 200 + plaintext
//   - TestRevealDeniedForNonCreator: non-creator reveals, gets 404 (not 403, to
//     avoid leaking key existence)
//   - TestRevealUnauthenticated: no token, gets 401
//   - TestRevealKeyNotFound: unknown id, gets 404
//   - TestRevealKeyVaultFailure: Rust L2 rejects (simulated via failingKV),
//     gets 403 or 502 depending on error type
//   - TestRevealDoesNotLeakPlaintextInErrorBody: the response body never
//     contains the plaintext (constitution XX)
package keymgmt

import (
	"context"
	"encoding/json"
	"errors"
	"net/http"
	"strings"
	"testing"
)

// TestRevealAllowedForCreator verifies the happy path: the creator of a key
// calls Reveal and receives the plaintext once. The fake KeyVault simulates
// Rust L2 success (always returns the plaintext).
func TestRevealAllowedForCreator(t *testing.T) {
	kv := newFakeKeyVaultClient()
	r, keyStore, idStore, adminToken := newTestRouter(t, kv)
	ctx := context.Background()

	admin, err := idStore.VerifyToken(ctx, adminToken)
	if err != nil {
		t.Fatalf("verify admin: %v", err)
	}
	plaintext := "sk-reveal-test-1234567890"
	enc, _ := kv.Encrypt(ctx, []byte(plaintext))
	pk, err := keyStore.Create(ctx, admin.ID, "openai", enc, "7890", "", admin.ID)
	if err != nil {
		t.Fatalf("create key: %v", err)
	}

	rec := doRequest(t, r, http.MethodGet, "/api/provider-keys/"+pk.ID+"/reveal",
		"Bearer "+adminToken, "")
	if rec.Code != http.StatusOK {
		t.Fatalf("reveal: status = %d, want %d; body = %s",
			rec.Code, http.StatusOK, rec.Body.String())
	}
	var resp revealResponse
	if err := json.Unmarshal(rec.Body.Bytes(), &resp); err != nil {
		t.Fatalf("unmarshal: %v; body=%s", err, rec.Body.String())
	}
	if resp.PlaintextKey != plaintext {
		t.Fatalf("plaintext = %q, want %q", resp.PlaintextKey, plaintext)
	}
	if resp.KeyID != pk.ID {
		t.Fatalf("key_id = %q, want %q", resp.KeyID, pk.ID)
	}
}

// TestRevealDeniedForNonCreator verifies the Go L1 check: a user who did not
// create the key gets 404 (not 403, to avoid leaking the key's existence).
// The fake KeyVault is NOT consulted (Go short-circuits before the gRPC call).
func TestRevealDeniedForNonCreator(t *testing.T) {
	kv := newFakeKeyVaultClient()
	r, keyStore, idStore, adminToken := newTestRouter(t, kv)
	ctx := context.Background()

	admin, err := idStore.VerifyToken(ctx, adminToken)
	if err != nil {
		t.Fatalf("verify admin: %v", err)
	}
	// Create a second user (non-admin in L0, but they can still try the
	// endpoint - they'll get 403 from Authorize before reaching the handler).
	// To test the L1 created_by check properly, we need a second ADMIN user,
	// because Authorize("manage", "provider_keys") in L0 requires admin.
	u2, u2Token, err := idStore.Create(ctx, "revealer@example.com", "u2-token-xyz")
	if err != nil {
		t.Fatalf("create u2: %v", err)
	}
	// Promote u2 to admin so they pass Authorize and reach the handler.
	if _, err := idStore.SetAdmin(ctx, u2.ID, true); err != nil {
		t.Fatalf("promote u2 to admin: %v", err)
	}

	// admin creates a key.
	plaintext := "sk-reveal-non-creator-12"
	enc, _ := kv.Encrypt(ctx, []byte(plaintext))
	pk, err := keyStore.Create(ctx, admin.ID, "openai", enc, "r12", "", admin.ID)
	if err != nil {
		t.Fatalf("create key: %v", err)
	}

	// u2 (admin, but not the creator) tries to reveal admin's key.
	rec := doRequest(t, r, http.MethodGet, "/api/provider-keys/"+pk.ID+"/reveal",
		"Bearer "+u2Token, "")
	if rec.Code != http.StatusNotFound {
		t.Fatalf("non-creator reveal: status = %d, want %d (404 to avoid leaking existence); body = %s",
			rec.Code, http.StatusNotFound, rec.Body.String())
	}
	// The response body must NOT contain the plaintext.
	if strings.Contains(rec.Body.String(), plaintext) {
		t.Fatalf("non-creator reveal response leaked plaintext: %s", rec.Body.String())
	}
}

// TestRevealKeyNotFound verifies that revealing an unknown id returns 404.
func TestRevealKeyNotFound(t *testing.T) {
	kv := newFakeKeyVaultClient()
	r, _, _, adminToken := newTestRouter(t, kv)

	rec := doRequest(t, r, http.MethodGet, "/api/provider-keys/pk_nonexistent/reveal",
		"Bearer "+adminToken, "")
	if rec.Code != http.StatusNotFound {
		t.Fatalf("reveal unknown: status = %d, want %d; body = %s",
			rec.Code, http.StatusNotFound, rec.Body.String())
	}
}

// TestRevealUnauthenticated verifies that no token -> 401.
func TestRevealUnauthenticated(t *testing.T) {
	kv := newFakeKeyVaultClient()
	r, _, _, _ := newTestRouter(t, kv)

	rec := doRequest(t, r, http.MethodGet, "/api/provider-keys/pk_anything/reveal",
		"", "")
	if rec.Code != http.StatusUnauthorized {
		t.Fatalf("unauthenticated reveal: status = %d, want %d",
			rec.Code, http.StatusUnauthorized)
	}
}

// TestRevealKeyVaultFailure verifies that a Rust L2 rejection (simulated by a
// KeyVaultClient that returns a permission_denied-like error) maps to 403.
// In normal operation L1 already rejected non-creators, so L2 should only
// reject if Rust's owner map is stale or out of sync with Go's DB.
func TestRevealKeyVaultFailure(t *testing.T) {
	// fakeKeyVaultClient always succeeds; use a dedicated failing fake that
	// returns a permission_denied-shaped error to simulate Rust L2 rejection.
	kv := &denyKeyVaultClient{err: errors.New("rpc error: code = PermissionDenied desc = not key owner (Rust L2 dual-defense)")}
	r, keyStore, idStore, adminToken := newTestRouter(t, kv)
	ctx := context.Background()

	admin, err := idStore.VerifyToken(ctx, adminToken)
	if err != nil {
		t.Fatalf("verify admin: %v", err)
	}
	enc, _ := kv.Encrypt(ctx, []byte("sk-l2-reject-1234567"))
	pk, err := keyStore.Create(ctx, admin.ID, "openai", enc, "4567", "", admin.ID)
	if err != nil {
		t.Fatalf("create key: %v", err)
	}

	rec := doRequest(t, r, http.MethodGet, "/api/provider-keys/"+pk.ID+"/reveal",
		"Bearer "+adminToken, "")
	if rec.Code != http.StatusForbidden {
		t.Fatalf("L2 reject: status = %d, want %d; body = %s",
			rec.Code, http.StatusForbidden, rec.Body.String())
	}
	// The error response must not leak the plaintext.
	if strings.Contains(rec.Body.String(), "sk-l2-reject") {
		t.Fatalf("L2 reject response leaked plaintext: %s", rec.Body.String())
	}
}

// TestRevealKeyVaultUnreachable verifies that a non-permission error from the
// KeyVault (e.g., Rust down) maps to 502 Bad Gateway, not 403.
func TestRevealKeyVaultUnreachable(t *testing.T) {
	kv := &denyKeyVaultClient{err: errors.New("rpc error: code = Unavailable desc = connection refused")}
	r, keyStore, idStore, adminToken := newTestRouter(t, kv)
	ctx := context.Background()

	admin, err := idStore.VerifyToken(ctx, adminToken)
	if err != nil {
		t.Fatalf("verify admin: %v", err)
	}
	enc, _ := kv.Encrypt(ctx, []byte("sk-unreachable-1234567"))
	pk, err := keyStore.Create(ctx, admin.ID, "openai", enc, "4567", "", admin.ID)
	if err != nil {
		t.Fatalf("create key: %v", err)
	}

	rec := doRequest(t, r, http.MethodGet, "/api/provider-keys/"+pk.ID+"/reveal",
		"Bearer "+adminToken, "")
	if rec.Code != http.StatusBadGateway {
		t.Fatalf("L2 unreachable: status = %d, want %d; body = %s",
			rec.Code, http.StatusBadGateway, rec.Body.String())
	}
}

// TestRevealResponseShape verifies the successful response has exactly the
// expected fields (plaintext_key + key_id) and no extras (no ciphertext, no
// metadata leaks).
func TestRevealResponseShape(t *testing.T) {
	kv := newFakeKeyVaultClient()
	r, keyStore, idStore, adminToken := newTestRouter(t, kv)
	ctx := context.Background()

	admin, err := idStore.VerifyToken(ctx, adminToken)
	if err != nil {
		t.Fatalf("verify admin: %v", err)
	}
	plaintext := "sk-shape-test-1234567890"
	enc, _ := kv.Encrypt(ctx, []byte(plaintext))
	pk, err := keyStore.Create(ctx, admin.ID, "openai", enc, "7890", "", admin.ID)
	if err != nil {
		t.Fatalf("create key: %v", err)
	}

	rec := doRequest(t, r, http.MethodGet, "/api/provider-keys/"+pk.ID+"/reveal",
		"Bearer "+adminToken, "")
	if rec.Code != http.StatusOK {
		t.Fatalf("reveal: status = %d, want %d; body = %s",
			rec.Code, http.StatusOK, rec.Body.String())
	}

	// Parse as a generic map to verify the exact field set.
	var m map[string]any
	if err := json.Unmarshal(rec.Body.Bytes(), &m); err != nil {
		t.Fatalf("unmarshal: %v; body=%s", err, rec.Body.String())
	}
	wantFields := map[string]bool{
		"plaintext_key": true,
		"key_id":        true,
	}
	for k := range m {
		if !wantFields[k] {
			t.Errorf("unexpected field in reveal response: %q (value=%v)", k, m[k])
		}
	}
	for k := range wantFields {
		if _, ok := m[k]; !ok {
			t.Errorf("missing field in reveal response: %q", k)
		}
	}
}

// denyKeyVaultClient is a test-only KeyVaultClient whose Encrypt succeeds
// (using the same marker-prefix transform as fakeKeyVaultClient) but whose
// Decrypt returns a configurable error. Used to simulate Rust L2 rejection
// (permission_denied) or Rust unreachable (Unavailable) for the Reveal
// handler's error-mapping logic.
type denyKeyVaultClient struct {
	err error
}

func (d *denyKeyVaultClient) Encrypt(_ context.Context, plaintext []byte) ([]byte, error) {
	// Reuse the fake marker so Create still works.
	marker := []byte("ENC:")
	out := make([]byte, 0, len(marker)+len(plaintext))
	out = append(out, marker...)
	out = append(out, plaintext...)
	return out, nil
}

func (d *denyKeyVaultClient) Decrypt(_ context.Context, _ []byte, _, _, _ string) ([]byte, error) {
	return nil, d.err
}
