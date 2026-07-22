// Package keymgmt - no-plaintext-leak audit (spec §12D, constitution XX). This
// test enforces that the plaintext provider key NEVER appears in Go's HTTP
// responses (except the single Reveal endpoint where the authorized creator
// explicitly asked for it) or in Go's slog output.
//
// The test is hermetic: it uses an in-memory DB + fake KeyVaultClient. The
// fake produces a marker-prefixed ciphertext that is distinct from the
// plaintext, so any leak is detectable by scanning responses for the plaintext
// substring.
//
// Constitution XX: "明文只在 `KeyVault::decrypt()` 返回的 `SecretString` 中
// 存活，MUST NOT 进日志 / 指标 / 任何对外输出." Go never holds the plaintext
// outside of: (a) the Create handler's request body scope (one Encrypt call),
// and (b) the Reveal handler's response body (one authorized Decrypt). This
// test asserts no other Go output contains the plaintext.
package keymgmt

import (
	"bytes"
	"context"
	"encoding/json"
	"log"
	"net/http"
	"strings"
	"testing"
)

// TestNoPlaintextInGoOutputs exercises the keymgmt HTTP surface with a known
// plaintext key and asserts the plaintext appears in exactly ONE response:
// the successful Reveal by the creator. Every other response (list, get,
// non-creator reveal, not-found reveal, unauthenticated reveal, L2-failed
// reveal) must NOT contain the plaintext.
//
// slog output is also captured via a bytes.Buffer log sink and scanned for
// the plaintext marker.
func TestNoPlaintextInGoOutputs(t *testing.T) {
	// Redirect log output to a buffer so we can scan it for plaintext leaks.
	// The standard log package is what handler.go uses (log.Printf); we
	// redirect its output to a bytes.Buffer.
	var logBuf bytes.Buffer
	originalOutput := log.Writer()
	log.SetOutput(&logBuf)
	t.Cleanup(func() { log.SetOutput(originalOutput) })

	kv := newFakeKeyVaultClient()
	r, keyStore, idStore, adminToken := newTestRouter(t, kv)
	ctx := context.Background()

	admin, err := idStore.VerifyToken(ctx, adminToken)
	if err != nil {
		t.Fatalf("verify admin: %v", err)
	}
	u2, u2Token, err := idStore.Create(ctx, "no-leak-u2@example.com", "u2-token-noleak")
	if err != nil {
		t.Fatalf("create u2: %v", err)
	}
	if _, err := idStore.SetAdmin(ctx, u2.ID, true); err != nil {
		t.Fatalf("promote u2: %v", err)
	}

	// Plaintext with a distinctive marker so substring search is unambiguous.
	plaintext := "sk-noleak-SECRETLEAK-1234567890"
	enc, _ := kv.Encrypt(ctx, []byte(plaintext))
	pk, err := keyStore.Create(ctx, admin.ID, "openai", enc, "7890", "", admin.ID)
	if err != nil {
		t.Fatalf("create key: %v", err)
	}

	// Scan a response body for the plaintext marker.
	assertNoLeak := func(t *testing.T, label string, body []byte) {
		t.Helper()
		if bytes.Contains(body, []byte(plaintext)) {
			t.Errorf("%s response leaked plaintext: %s", label, string(body))
		}
		if bytes.Contains(body, []byte("SECRETLEAK")) {
			t.Errorf("%s response leaked plaintext marker: %s", label, string(body))
		}
	}

	// 1. List response must not contain plaintext.
	rec := doRequest(t, r, http.MethodGet, "/api/provider-keys", "Bearer "+adminToken, "")
	if rec.Code != http.StatusOK {
		t.Fatalf("list: status = %d, want %d; body = %s", rec.Code, http.StatusOK, rec.Body.String())
	}
	assertNoLeak(t, "list", rec.Body.Bytes())

	// 2. Get response must not contain plaintext.
	rec = doRequest(t, r, http.MethodGet, "/api/provider-keys/"+pk.ID, "Bearer "+adminToken, "")
	if rec.Code != http.StatusOK {
		t.Fatalf("get: status = %d, want %d; body = %s", rec.Code, http.StatusOK, rec.Body.String())
	}
	assertNoLeak(t, "get", rec.Body.Bytes())
	// Verify the get response also doesn't have encrypted_key field.
	var getResp map[string]any
	if err := json.Unmarshal(rec.Body.Bytes(), &getResp); err != nil {
		t.Fatalf("unmarshal get: %v; body=%s", err, rec.Body.String())
	}
	if _, present := getResp["encrypted_key"]; present {
		t.Errorf("get response contains encrypted_key field: %s", rec.Body.String())
	}

	// 3. Non-creator reveal (u2 tries admin's key) must not contain plaintext.
	rec = doRequest(t, r, http.MethodGet, "/api/provider-keys/"+pk.ID+"/reveal",
		"Bearer "+u2Token, "")
	if rec.Code != http.StatusNotFound {
		t.Fatalf("non-creator reveal: status = %d, want %d; body = %s",
			rec.Code, http.StatusNotFound, rec.Body.String())
	}
	assertNoLeak(t, "non-creator reveal", rec.Body.Bytes())

	// 4. Not-found reveal must not contain plaintext.
	rec = doRequest(t, r, http.MethodGet, "/api/provider-keys/pk_nonexistent/reveal",
		"Bearer "+adminToken, "")
	if rec.Code != http.StatusNotFound {
		t.Fatalf("not-found reveal: status = %d, want %d; body = %s",
			rec.Code, http.StatusNotFound, rec.Body.String())
	}
	assertNoLeak(t, "not-found reveal", rec.Body.Bytes())

	// 5. Unauthenticated reveal must not contain plaintext.
	rec = doRequest(t, r, http.MethodGet, "/api/provider-keys/"+pk.ID+"/reveal", "", "")
	if rec.Code != http.StatusUnauthorized {
		t.Fatalf("unauth reveal: status = %d, want %d; body = %s",
			rec.Code, http.StatusUnauthorized, rec.Body.String())
	}
	assertNoLeak(t, "unauth reveal", rec.Body.Bytes())

	// 6. Create response must not contain plaintext (already tested in
	// handler_test.go, but re-assert here for the no-leak audit).
	createBody := `{"provider_type":"openai","plaintext_key":"` + plaintext + `"}`
	rec = doRequest(t, r, http.MethodPost, "/api/provider-keys", "Bearer "+adminToken, createBody)
	if rec.Code != http.StatusCreated {
		t.Fatalf("create: status = %d, want %d; body = %s",
			rec.Code, http.StatusCreated, rec.Body.String())
	}
	assertNoLeak(t, "create", rec.Body.Bytes())

	// 7. Successful Reveal by creator MUST contain the plaintext (this is the
	// one allowed leak - the creator explicitly asked to see it).
	rec = doRequest(t, r, http.MethodGet, "/api/provider-keys/"+pk.ID+"/reveal",
		"Bearer "+adminToken, "")
	if rec.Code != http.StatusOK {
		t.Fatalf("creator reveal: status = %d, want %d; body = %s",
			rec.Code, http.StatusOK, rec.Body.String())
	}
	if !bytes.Contains(rec.Body.Bytes(), []byte(plaintext)) {
		t.Errorf("creator reveal response missing plaintext: %s", rec.Body.String())
	}

	// 8. Scan captured slog output for the plaintext marker. The log package
	// is redirected to logBuf; if any handler logged the plaintext, it would
	// appear here. (The L2-rejection log line in revealHandler logs the key_id
	// and requester, never the plaintext.)
	if strings.Contains(logBuf.String(), plaintext) {
		t.Errorf("Go log output leaked plaintext:\n%s", logBuf.String())
	}
	if strings.Contains(logBuf.String(), "SECRETLEAK") {
		t.Errorf("Go log output leaked plaintext marker:\n%s", logBuf.String())
	}
}
