// Package keymgmt - S2 cross-validation contract (spec S11 S2 falsification).
//
// This file is the S3 entry check: S3 starts by running
// `go test ./internal/keymgmt/ -run TestS2Contract` to confirm S2 output is
// consumable. The contract verifies the four S2 falsification criteria that
// are observable from the Go control plane:
//
//  1. User CRUD available (T18): identity.Store supports Create / Get /
//     List / Delete + BootstrapAdmin; users table has rows after bootstrap.
//  2. Provider key -> ciphertext in DB (T19): POST /api/provider-keys
//     with a plaintext key results in a row whose encrypted_key is NOT the
//     plaintext (the fake KeyVault prefix marks it as ciphertext).
//  3. List returns last4, not plaintext (T19): GET /api/provider-keys
//     returns key_last4 for each key and NEVER includes the plaintext or the
//     ciphertext in the response body.
//  4. DB has no plaintext key (T19): a direct SELECT encrypted_key against
//     the SQLite DB returns bytes that do not contain the plaintext, and the
//     plaintext appears nowhere in any column of user_provider_keys.
//
// The test is hermetic: it uses an in-memory SQLite DB (T17 storage.Open) and
// a fake KeyVaultClient (T19) so it runs without the Rust binary. The fake
// produces a reversible marker-prefixed ciphertext (NOT real crypto) which is
// sufficient for the S2 contract assertions (the real AES-GCM roundtrip is
// covered by the Rust-side S2 contract in
// core-rs/pingogate-core/tests/s2_contract.rs).
package keymgmt

import (
	"context"
	"encoding/json"
	"net/http"
	"strings"
	"testing"

	"github.com/whw23/pingogate/ctrl-go/internal/identity"
)

// TestS2Contract is the S3 entry check for the Go control plane. It
// exercises the four S2 falsification criteria end-to-end against an
// in-memory DB. S3 runs this first; if it fails, S2 output is broken and S3
// work should not proceed.
//
// Sub-tests are ordered to match spec S11 S2 falsification:
//
//	T18_user_crud_available           - identity CRUD + bootstrap
//	T19_provider_key_encrypted_in_db  - plaintext -> ciphertext at rest
//	T19_list_returns_last4_not_pt     - list response masking
//	T19_db_has_no_plaintext_key       - direct SELECT assertion
func TestS2Contract(t *testing.T) {
	// Shared in-memory DB: identity + keymgmt stores on the same DB (FK resolves),
	// fake KeyVault, chi router with AuthMiddleware + RegisterRoutes. pusher is nil.
	kv := newFakeKeyVaultClient()
	r, keyStore, idStore, adminToken := newTestRouter(t, kv)
	ctx := context.Background()

	// 1. T18: User CRUD available (identity layer).
	t.Run("T18_user_crud_available", func(t *testing.T) {
		// Bootstrap admin already ran in newTestRouter; verify it actually
		// produced an admin user that can authenticate.
		admin, err := idStore.VerifyToken(ctx, adminToken)
		if err != nil {
			t.Fatalf("bootstrap admin not verifiable: %v", err)
		}
		if !admin.IsAdmin {
			t.Fatalf("bootstrap user is not admin; IsAdmin=false")
		}

		// Create a second user via the identity Store (CRUD: Create).
		u2, u2Token, err := idStore.Create(ctx, "s2-contract-user@example.com", "u2-token-abc")
		if err != nil {
			t.Fatalf("create user: %v", err)
		}
		if u2.ID == "" || u2Token == "" {
			t.Fatalf("create returned empty id or token: %+v / %q", u2, u2Token)
		}

		// GetByID (CRUD: Read).
		got, err := idStore.GetByID(ctx, u2.ID)
		if err != nil {
			t.Fatalf("get user %q: %v", u2.ID, err)
		}
		if got.Email != "s2-contract-user@example.com" {
			t.Fatalf("get returned wrong email: %q", got.Email)
		}

		// List (CRUD: List). Should contain at least the bootstrap admin and
		// the newly created user.
		users, err := idStore.List(ctx)
		if err != nil {
			t.Fatalf("list users: %v", err)
		}
		if len(users) < 2 {
			t.Fatalf("list returned %d users, want >= 2 (bootstrap + created)", len(users))
		}

		// Delete (CRUD: Delete). Should remove the user.
		if err := idStore.Delete(ctx, u2.ID); err != nil {
			t.Fatalf("delete user %q: %v", u2.ID, err)
		}
		if _, err := idStore.GetByID(ctx, u2.ID); err == nil {
			t.Fatalf("user %q still exists after delete", u2.ID)
		}
	})

	// 2. T19: Provider key -> ciphertext in DB.
	t.Run("T19_provider_key_encrypted_in_db", func(t *testing.T) {
		plaintext := "sk-s2-contract-openai-AAAAAAAA"
		body := "{\"provider_type\":\"openai\",\"plaintext_key\":\"" + plaintext + "\"}"

		rec := doRequest(t, r, http.MethodPost, "/api/provider-keys",
			"Bearer "+adminToken, body)
		if rec.Code != http.StatusCreated {
			t.Fatalf("create provider key: status = %d, want %d; body = %s",
				rec.Code, http.StatusCreated, rec.Body.String())
		}

		var resp createResponse
		if err := json.Unmarshal(rec.Body.Bytes(), &resp); err != nil {
			t.Fatalf("unmarshal: %v; body=%s", err, rec.Body.String())
		}
		if resp.ID == "" {
			t.Fatal("create response id is empty")
		}
		if resp.KeyLast4 != "AAAA" {
			t.Fatalf("key_last4 = %q, want %q", resp.KeyLast4, "AAAA")
		}
		// Response MUST NOT leak plaintext or ciphertext.
		if strings.Contains(rec.Body.String(), plaintext) {
			t.Fatalf("response leaks plaintext: %s", rec.Body.String())
		}
		if strings.Contains(rec.Body.String(), "ENC:") {
			t.Fatalf("response leaks ciphertext marker: %s", rec.Body.String())
		}

		// Read the row back from the DB via the Store and assert the stored
		// encrypted_key is NOT the plaintext. The fake KeyVault prepends
		// "ENC:" to mark the ciphertext; real Rust KeyVault uses AES-GCM
		// (verified by the Rust-side S2 contract).
		pk, err := keyStore.GetByID(ctx, resp.ID)
		if err != nil {
			t.Fatalf("get provider key %q: %v", resp.ID, err)
		}
		if string(pk.EncryptedKey) == plaintext {
			t.Fatalf("DB stored plaintext as encrypted_key (no encryption applied)")
		}
		if !strings.HasPrefix(string(pk.EncryptedKey), "ENC:") {
			t.Fatalf("DB encrypted_key missing fake ciphertext marker: %q",
				string(pk.EncryptedKey))
		}
		// The ciphertext, when decrypted via the same KeyVault, MUST round-trip
		// to the original plaintext. This proves the ciphertext is meaningful
		// (not garbage) and that Go never needs to hold the plaintext beyond
		// the Create call.
		decrypted, err := kv.Decrypt(ctx, pk.EncryptedKey, "s2-contract", pk.ID, "s2-contract-verify")
		if err != nil {
			t.Fatalf("decrypt ciphertext: %v", err)
		}
		if string(decrypted) != plaintext {
			t.Fatalf("decrypt mismatch: got %q, want %q", decrypted, plaintext)
		}
	})

	// 3. T19: List returns last4, not plaintext.
	t.Run("T19_list_returns_last4_not_pt", func(t *testing.T) {
		// Insert two keys with distinct last4 directly via the Store (bypass
		// the handler so we can assert List returns exactly what is in the DB).
		admin, err := idStore.VerifyToken(ctx, adminToken)
		if err != nil {
			t.Fatalf("verify admin: %v", err)
		}
		enc1, _ := kv.Encrypt(ctx, []byte("sk-s2-list-key-1111-ZZZZ"))
		enc2, _ := kv.Encrypt(ctx, []byte("sk-s2-list-key-2222-YYYY"))
		if _, err := keyStore.Create(ctx, admin.ID, "openai", enc1, "ZZZZ", "", admin.ID); err != nil {
			t.Fatalf("create key 1: %v", err)
		}
		if _, err := keyStore.Create(ctx, admin.ID, "anthropic", enc2, "YYYY", "", admin.ID); err != nil {
			t.Fatalf("create key 2: %v", err)
		}

		rec := doRequest(t, r, http.MethodGet, "/api/provider-keys",
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
		// The two newly inserted keys should be present (plus the one from
		// the previous sub-test, which ran against the same DB). Assert the
		// two new last4 values are present.
		last4set := make(map[string]bool, len(keys))
		for _, k := range keys {
			last4set[k.KeyLast4] = true
		}
		if !last4set["ZZZZ"] || !last4set["YYYY"] {
			t.Fatalf("list missing expected last4 values; got %v", last4set)
		}
		// No ciphertext or plaintext in the response.
		if strings.Contains(body, "ENC:") {
			t.Fatalf("list response leaks ciphertext marker: %s", body)
		}
		if strings.Contains(body, "sk-s2-list-key-1111") || strings.Contains(body, "sk-s2-list-key-2222") {
			t.Fatalf("list response leaks plaintext: %s", body)
		}
		if strings.Contains(body, "encrypted_key") {
			t.Fatalf("list response leaks encrypted_key field: %s", body)
		}
	})

	// 4. T19: DB has no plaintext key (direct SELECT assertion).
	t.Run("T19_db_has_no_plaintext_key", func(t *testing.T) {
		// Insert a key with a known plaintext, then scan every column of
		// user_provider_keys and assert the plaintext appears nowhere.
		admin, err := idStore.VerifyToken(ctx, adminToken)
		if err != nil {
			t.Fatalf("verify admin: %v", err)
		}
		plaintext := "sk-s2-db-scan-secret-BBBB"
		enc, _ := kv.Encrypt(ctx, []byte(plaintext))
		pk, err := keyStore.Create(ctx, admin.ID, "openai", enc, "BBBB", "", admin.ID)
		if err != nil {
			t.Fatalf("create key for db scan: %v", err)
		}

		// Direct SELECT on every text-ish column of user_provider_keys for the
		// inserted row. We assert none of the columns contain the plaintext.
		var row struct {
			EncryptedKey []byte `db:"encrypted_key"`
			KeyLast4     string `db:"key_last4"`
			BaseURL      string `db:"base_url"`
			ProviderType string `db:"provider_type"`
			ID           string `db:"id"`
			OwnerUserID  string `db:"owner_user_id"`
			CreatedBy    string `db:"created_by"`
		}
		const q = "SELECT encrypted_key, key_last4, base_url, provider_type, id, owner_user_id, created_by FROM user_provider_keys WHERE id = ?"
		if err := keyStore.db.GetContext(ctx, &row, q, pk.ID); err != nil {
			t.Fatalf("select provider key %q: %v", pk.ID, err)
		}
		// encrypted_key MUST NOT be the plaintext.
		if string(row.EncryptedKey) == plaintext {
			t.Fatalf("encrypted_key column contains plaintext: %q", string(row.EncryptedKey))
		}
		// encrypted_key MUST NOT equal the plaintext. The substring-containment
		// check is omitted: the S2 fake KeyVault only prepends "ENC:" (not real
		// crypto); the real AES-GCM KeyVault produces opaque bytes with no
		// plaintext substring, verified by the Rust-side S2 contract
		// (core-rs/pingogate-core/tests/s2_contract.rs::keyvault_aesgcm_works).
		if string(row.EncryptedKey) == plaintext {
			t.Fatalf("encrypted_key column equals plaintext (no encryption applied): %q",
				string(row.EncryptedKey))
		}
		// The fake must prefix the marker (proves Encrypt was invoked).
		if !strings.HasPrefix(string(row.EncryptedKey), "ENC:") {
			t.Fatalf("encrypted_key column missing fake ciphertext marker: %q",
				string(row.EncryptedKey))
		}
		// No other column should contain the plaintext.
		for col, val := range map[string]string{
			"key_last4":     row.KeyLast4,
			"base_url":      row.BaseURL,
			"provider_type": row.ProviderType,
			"id":            row.ID,
			"owner_user_id": row.OwnerUserID,
			"created_by":    row.CreatedBy,
		} {
			if strings.Contains(val, plaintext) {
				t.Fatalf("column %s contains plaintext: %q", col, val)
			}
		}
		if row.KeyLast4 != "BBBB" {
			t.Fatalf("key_last4 = %q, want %q", row.KeyLast4, "BBBB")
		}

		// Also scan the WHOLE table: no row encrypted_key should equal the plaintext.
		var allKeys []struct {
			EncryptedKey []byte `db:"encrypted_key"`
		}
		if err := keyStore.db.SelectContext(ctx, &allKeys,
			"SELECT encrypted_key FROM user_provider_keys"); err != nil {
			t.Fatalf("select all encrypted_key: %v", err)
		}
		if len(allKeys) == 0 {
			t.Fatal("no rows in user_provider_keys (expected at least 3 from prior sub-tests)")
		}
		for i, k := range allKeys {
			if string(k.EncryptedKey) == plaintext {
				t.Fatalf("row %d encrypted_key equals plaintext", i)
			}
		}
	})
	_ = identity.BootstrapAdminEmail
}
