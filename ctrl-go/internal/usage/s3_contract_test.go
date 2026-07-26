// Package usage - S3 cross-validation contract (spec §11 S3 falsification):
// Go side. Paired with `core-rs/pingogate-core/tests/s3_contract.rs`.
//
// TestS3Contract is the S3 entry check for the Go control plane. It exercises
// the three S3 falsification criteria observable from Go, all sharing one
// in-memory SQLite DB so the cross-package data flow is verified:
//
//  1. 1Password dual-defense (T30): Go L1 created_by check. Creator reveals
//     plaintext -> 200; a DIFFERENT admin (passes RBAC but fails created_by)
//     -> 404 (existence-hidden). The Rust L2 check (owner == requester) is
//     covered by the Rust-side s3_contract.rs::keyvault_owner_intercept_denies_non_owner.
//  2. vkey issue (T29): POST returns plaintext token once; DB stores
//     SHA-256 hex digest (not plaintext); response does not leak token_hash.
//  3. usage persist (T31): Server.handleEvent persists a provider-usage
//     event (estimated=0) and an estimate-needed event (estimated=1, tokens
//     filled in by the offline estimator); both rows land in the usage table.
//
// Hermetic: in-memory SQLite DB, fake KeyVaultClient (marker prefix, not real
// crypto), offline tiktoken estimator (no CDN). Runs without the Rust binary.
package usage

import (
	"context"
	"crypto/sha256"
	"encoding/hex"
	"net/http"
	"net/http/httptest"
	"strings"
	"testing"

	"github.com/go-chi/chi/v5"

	pb "github.com/whw23/pingogate/ctrl-go/internal/proto"
	"github.com/whw23/pingogate/ctrl-go/internal/identity"
	"github.com/whw23/pingogate/ctrl-go/internal/keymgmt"
	"github.com/whw23/pingogate/ctrl-go/internal/storage"
	"github.com/whw23/pingogate/ctrl-go/internal/vkey"
)

// fakeKeyVault for the S3 contract test. Produces a reversible marker-prefixed
// ciphertext (NOT real crypto; test only). The real AES-GCM roundtrip is
// covered by the Rust-side s3_contract.rs.
type s3FakeKeyVault struct{ marker []byte }

func newS3FakeKeyVault() *s3FakeKeyVault { return &s3FakeKeyVault{marker: []byte("ENC:")} }

func (f *s3FakeKeyVault) Encrypt(_ context.Context, pt []byte) ([]byte, error) {
	return append(append([]byte{}, f.marker...), pt...), nil
}

func (f *s3FakeKeyVault) Decrypt(_ context.Context, ct []byte, _, _, _ string) ([]byte, error) {
	if len(ct) < len(f.marker) {
		return ct, nil
	}
	return ct[len(f.marker):], nil
}

// s3TestEnv wires an in-memory DB with identity + keymgmt + vkey routes
// mounted on a chi router. Returns the router, the three stores, and the
// bootstrap admin token. The usage store is constructed but not routed
// (usage has no HTTP routes; it is a gRPC-only server).
type s3TestEnv struct {
	router     http.Handler
	idStore    *identity.Store
	keyStore   *keymgmt.Store
	vkeyStore  *vkey.Store
	usageStore *Store
	usageSrv   *Server
	adminToken string
}

func newS3TestEnv(t *testing.T) *s3TestEnv {
	t.Helper()
	db, err := storage.Open(":memory:")
	if err != nil {
		t.Fatalf("open in-memory DB: %v", err)
	}
	t.Cleanup(func() { _ = db.Close() })

	idStore := identity.NewStore(db)
	keyStore := keymgmt.NewStore(db)
	vkeyStore := vkey.NewStore(db)
	usageStore := NewStore(db)
	est := NewEstimator()
	est.SetOfflineFallback()
	usageSrv := NewServer(usageStore, est, nil)

	ctx := context.Background()
	if _, err := identity.BootstrapAdmin(ctx, idStore, "s3-admin-token"); err != nil {
		t.Fatalf("bootstrap admin: %v", err)
	}

	r := chi.NewRouter()
	r.Use(identity.AuthMiddleware(idStore))
	identity.RegisterRoutes(r, idStore)
	keymgmt.RegisterRoutes(r, keyStore, newS3FakeKeyVault(), nil)
	vkey.RegisterRoutes(r, vkeyStore, nil)

	return &s3TestEnv{
		router:     r,
		idStore:    idStore,
		keyStore:   keyStore,
		vkeyStore:  vkeyStore,
		usageStore: usageStore,
		usageSrv:   usageSrv,
		adminToken: "s3-admin-token",
	}
}

func (e *s3TestEnv) do(t *testing.T, method, path, token, body string) *httptest.ResponseRecorder {
	t.Helper()
	req := httptest.NewRequest(method, path, strings.NewReader(body))
	if token != "" {
		req.Header.Set("Authorization", "Bearer "+token)
	}
	if body != "" {
		req.Header.Set("Content-Type", "application/json")
	}
	rec := httptest.NewRecorder()
	e.router.ServeHTTP(rec, req)
	return rec
}

// TestS3Contract is the S3 entry check for the Go control plane. Sub-tests
// match spec §11 S3 falsification. S3 runs this first; if it fails, S3 output
// is broken and M0+M1 closure cannot be claimed.
func TestS3Contract(t *testing.T) {
	ctx := context.Background()
	env := newS3TestEnv(t)

	// 1. 1Password dual-defense Go L1 (T30).
	t.Run("T30_dual_defense_creator_reveals_non_creator_blocked", func(t *testing.T) {
		// Bootstrap admin creates a provider key (created_by = admin).
		plaintext := "sk-s3-dual-defense-LEAKMARKER-12345"
		rec := env.do(t, http.MethodPost, "/api/provider-keys",
			env.adminToken, `{"provider_type":"openai","plaintext_key":"`+plaintext+`"}`)
		if rec.Code != http.StatusCreated {
			t.Fatalf("create provider key: status=%d body=%s", rec.Code, rec.Body.String())
		}
		body := rec.Body.String()
		if strings.Contains(body, plaintext) {
			t.Fatalf("create response leaks plaintext: %s", body)
		}
		if strings.Contains(body, "encrypted_key") {
			t.Fatalf("create response leaks ciphertext field: %s", body)
		}
		keyID := extractJSONField(t, body, "id")

		// Creator (bootstrap admin) reveals -> 200 + plaintext.
		rec = env.do(t, http.MethodGet, "/api/provider-keys/"+keyID+"/reveal",
			env.adminToken, "")
		if rec.Code != http.StatusOK {
			t.Fatalf("creator reveal: status=%d body=%s", rec.Code, rec.Body.String())
		}
		if !strings.Contains(rec.Body.String(), plaintext) {
			t.Fatalf("creator reveal must return plaintext: %s", rec.Body.String())
		}

		// Create a SECOND admin (passes RBAC, but is NOT the creator).
		// This is the 1Password dual-defense key scenario: even an admin
		// cannot reveal another admin's BYOK key (created_by != requester).
		adminB, adminBToken, err := env.idStore.Create(ctx, "admin-b@s3.test", "admin-b-token")
		if err != nil {
			t.Fatalf("create admin-B: %v", err)
		}
		if _, err := env.idStore.SetAdmin(ctx, adminB.ID, true); err != nil {
			t.Fatalf("promote admin-B: %v", err)
		}

		// Admin-B reveals admin's key -> 404 (L1: created_by != admin-B).
		rec = env.do(t, http.MethodGet, "/api/provider-keys/"+keyID+"/reveal",
			adminBToken, "")
		if rec.Code != http.StatusNotFound {
			t.Fatalf("non-creator admin reveal: status=%d, want 404 (existence-hidden): %s",
				rec.Code, rec.Body.String())
		}
		if strings.Contains(rec.Body.String(), plaintext) {
			t.Fatalf("non-creator reveal must not leak plaintext: %s", rec.Body.String())
		}
	})

	// 2. vkey issue (T29): hash stored, plaintext returned once.
	t.Run("T29_vkey_issue_stores_hash_returns_plaintext_once", func(t *testing.T) {
		rec := env.do(t, http.MethodPost, "/api/virtual-keys",
			env.adminToken, "{}")
		if rec.Code != http.StatusCreated {
			t.Fatalf("issue vkey: status=%d body=%s", rec.Code, rec.Body.String())
		}
		body := rec.Body.String()
		token := extractJSONField(t, body, "token")
		if token == "" {
			t.Fatalf("issue response token is empty: %s", body)
		}
		if strings.Contains(body, "token_hash") {
			t.Fatalf("issue response leaks token_hash: %s", body)
		}

		// DB stores SHA-256 hex digest, not plaintext.
		vkeys, err := env.vkeyStore.ListByOwner(ctx, extractJSONField(t, body, "owner_user_id"))
		if err != nil {
			t.Fatalf("list vkeys: %v", err)
		}
		if len(vkeys) != 1 {
			t.Fatalf("expected 1 vkey in DB, got %d", len(vkeys))
		}
		storedHash := vkeys[0].TokenHash
		if storedHash == "" || storedHash == token {
			t.Fatalf("DB stores empty or plaintext as token_hash: %q", storedHash)
		}
		sum := sha256.Sum256([]byte(token))
		wantHash := hex.EncodeToString(sum[:])
		if storedHash != wantHash {
			t.Fatalf("token_hash = %q, want %q (hex(sha256(token)))", storedHash, wantHash)
		}
	})

	// 3. usage persist (T31): provider-usage + estimated events both land in DB.
	t.Run("T31_usage_persist_provider_and_estimated", func(t *testing.T) {
		// Provider-supplied usage (no estimation needed).
		eventWithUsage := &pb.UsageEvent{
			OwnerUserId:  "user-s3-usage-A",
			Provider:     "openai",
			Model:        "gpt-4o",
			InputTokens:  100,
			OutputTokens: 50,
			Success:      true,
			LatencyMs:    42,
		}
		if err := env.usageSrv.handleEvent(ctx, eventWithUsage); err != nil {
			t.Fatalf("handleEvent with usage: %v", err)
		}
		count, err := env.usageStore.CountByOwner(ctx, "user-s3-usage-A")
		if err != nil {
			t.Fatalf("count: %v", err)
		}
		if count != 1 {
			t.Fatalf("count = %d, want 1 (provider-usage event)", count)
		}

		// Needs-estimate event (no provider usage; Go fills in via tiktoken).
		eventNeedsEstimate := &pb.UsageEvent{
			OwnerUserId:   "user-s3-usage-B",
			Provider:      "openai",
			Model:         "gpt-4o",
			NeedsEstimate: true,
			BodyRef:       []byte(`{"request":{"messages":["hello"]},"response":{"choices":["hi"]}}`),
			Success:       true,
		}
		if err := env.usageSrv.handleEvent(ctx, eventNeedsEstimate); err != nil {
			t.Fatalf("handleEvent needs_estimate: %v", err)
		}
		if eventNeedsEstimate.InputTokens == 0 {
			t.Fatal("input tokens not estimated")
		}
		if eventNeedsEstimate.BodyRef != nil {
			t.Fatal("body_ref should be cleared after estimation")
		}
		count, err = env.usageStore.CountByOwner(ctx, "user-s3-usage-B")
		if err != nil {
			t.Fatalf("count: %v", err)
		}
		if count != 1 {
			t.Fatalf("count = %d, want 1 (estimated event)", count)
		}
	})
}

// extractJSONField pulls a string field from a JSON body. Test-only; panics
// on parse failure. Avoids pulling encoding/json into every assertion.
func extractJSONField(t *testing.T, body, field string) string {
	t.Helper()
	needle := `"` + field + `":"`
	idx := strings.Index(body, needle)
	if idx < 0 {
		t.Fatalf("field %q not found in body: %s", field, body)
	}
	start := idx + len(needle)
	end := strings.IndexByte(body[start:], '"')
	if end < 0 {
		t.Fatalf("unterminated string for field %q: %s", field, body)
	}
	return body[start : start+end]
}
