// Package snapshot - Builder tests (constitution XIII: TDD, observe failure
// first). Uses an in-memory SQLite DB (T17 storage.Open) so tests are hermetic
// and run without the Rust binary or a running gRPC server.
package snapshot

import (
	"context"
	"testing"

	"github.com/whw23/pingogate/ctrl-go/internal/identity"
	"github.com/whw23/pingogate/ctrl-go/internal/keymgmt"
	"github.com/whw23/pingogate/ctrl-go/internal/storage"
	pb "github.com/whw23/pingogate/ctrl-go/internal/proto"
)

// fakeKeyVault is a test-only KeyVaultClient that produces a reversible
// marker-prefixed ciphertext. NOT real crypto; test only. Defined here so the
// snapshot package tests don't depend on keymgmt's unexported fake.
type fakeKeyVault struct {
	marker []byte
}

func newFakeKeyVault() *fakeKeyVault {
	return &fakeKeyVault{marker: []byte("ENC:")}
}

func (f *fakeKeyVault) Encrypt(_ context.Context, plaintext []byte) ([]byte, error) {
	out := make([]byte, 0, len(f.marker)+len(plaintext))
	out = append(out, f.marker...)
	out = append(out, plaintext...)
	return out, nil
}

func (f *fakeKeyVault) Decrypt(_ context.Context, ciphertext []byte, _, _, _ string) ([]byte, error) {
	if len(ciphertext) < len(f.marker) {
		return ciphertext, nil
	}
	return ciphertext[len(f.marker):], nil
}

// newTestDB opens an in-memory SQLite DB with migrations applied and
// bootstraps an admin user so user_provider_keys FKs resolve. Returns the DB,
// the admin's user ID, and a fake KeyVaultClient.
func newTestDB(t *testing.T) (*storage.DB, string, keymgmt.KeyVaultClient) {
	t.Helper()
	db, err := storage.Open(":memory:")
	if err != nil {
		t.Fatalf("open in-memory DB: %v", err)
	}
	t.Cleanup(func() { _ = db.Close() })

	idStore := identity.NewStore(db)
	ctx := context.Background()
	if _, err := identity.BootstrapAdmin(ctx, idStore, "bootstrap-test-token"); err != nil {
		t.Fatalf("bootstrap admin: %v", err)
	}
	adminUser, err := idStore.VerifyToken(ctx, "bootstrap-test-token")
	if err != nil {
		t.Fatalf("verify bootstrap admin: %v", err)
	}
	return db, adminUser.ID, newFakeKeyVault()
}

// TestBuildEmptyDB verifies the Builder produces a valid (empty) Snapshot when
// no provider keys exist. Rust should accept this and become "ready" with
// version=1 but no providers.
func TestBuildEmptyDB(t *testing.T) {
	db, _, _ := newTestDB(t)
	b := NewBuilder(db)

	snap, err := b.Build(context.Background(), 1)
	if err != nil {
		t.Fatalf("build: %v", err)
	}
	if snap.Version != 1 {
		t.Fatalf("version = %d, want 1", snap.Version)
	}
	if len(snap.Providers) != 0 {
		t.Fatalf("providers = %d, want 0", len(snap.Providers))
	}
	if len(snap.EncryptedKeys) != 0 {
		t.Fatalf("encrypted_keys = %d, want 0", len(snap.EncryptedKeys))
	}
	if len(snap.Routes) != 0 {
		t.Fatalf("routes = %d, want 0 (S2)", len(snap.Routes))
	}
	if len(snap.VirtualKeys) != 0 {
		t.Fatalf("virtual_keys = %d, want 0 (S3)", len(snap.VirtualKeys))
	}
}

// TestBuildSingleKey verifies the Builder produces a correct Snapshot for a
// single enabled provider key: one ProviderEntry + one EncryptedProviderKey,
// with encrypted_key_ref pointing at the key ID and the ciphertext copied
// verbatim from the DB.
func TestBuildSingleKey(t *testing.T) {
	db, adminID, kv := newTestDB(t)
	ctx := context.Background()

	ks := keymgmt.NewStore(db)
	enc, err := kv.Encrypt(ctx, []byte("sk-test-1234567890"))
	if err != nil {
		t.Fatalf("encrypt: %v", err)
	}
	pk, err := ks.Create(ctx, adminID, "openai", enc, "7890", "", adminID)
	if err != nil {
		t.Fatalf("create key: %v", err)
	}

	b := NewBuilder(db)
	snap, err := b.Build(ctx, 5)
	if err != nil {
		t.Fatalf("build: %v", err)
	}
	if snap.Version != 5 {
		t.Fatalf("version = %d, want 5", snap.Version)
	}
	if len(snap.Providers) != 1 {
		t.Fatalf("providers = %d, want 1", len(snap.Providers))
	}
	if len(snap.EncryptedKeys) != 1 {
		t.Fatalf("encrypted_keys = %d, want 1", len(snap.EncryptedKeys))
	}

	pe := snap.Providers[0]
	if pe.Name != pk.ID {
		t.Fatalf("provider name = %q, want %q", pe.Name, pk.ID)
	}
	if pe.Kind != "openai-compatible" {
		t.Fatalf("kind = %q, want %q", pe.Kind, "openai-compatible")
	}
	if pe.AuthMethod != "bearer" {
		t.Fatalf("auth_method = %q, want %q", pe.AuthMethod, "bearer")
	}
	if pe.EncryptedKeyRef != pk.ID {
		t.Fatalf("encrypted_key_ref = %q, want %q", pe.EncryptedKeyRef, pk.ID)
	}
	if pe.BaseUrl == "" {
		t.Fatal("base_url is empty; should default to canonical endpoint")
	}
	if pe.AnthropicVersion != "" {
		t.Fatalf("anthropic_version = %q, want empty for openai", pe.AnthropicVersion)
	}
	if len(pe.CapabilityFamilies) != 1 || pe.CapabilityFamilies[0] != "generation.stateless" {
		t.Fatalf("capability_families = %v, want [generation.stateless]", pe.CapabilityFamilies)
	}

	ek := snap.EncryptedKeys[0]
	if ek.Id != pk.ID {
		t.Fatalf("encrypted_key id = %q, want %q", ek.Id, pk.ID)
	}
	if string(ek.Ciphertext) != string(enc) {
		t.Fatalf("ciphertext mismatch: got %q, want %q", ek.Ciphertext, enc)
	}
	if ek.OwnerUserId != adminID {
		t.Fatalf("owner_user_id = %q, want %q", ek.OwnerUserId, adminID)
	}
	if ek.CreatedBy != adminID {
		t.Fatalf("created_by = %q, want %q", ek.CreatedBy, adminID)
	}
}

// TestBuildMultipleProviderTypes verifies the provider_type -> (kind, auth,
// base_url) mapping for each supported type. Anthropic must get a non-empty
// anthropic_version; the others must not.
func TestBuildMultipleProviderTypes(t *testing.T) {
	db, adminID, kv := newTestDB(t)
	ctx := context.Background()
	ks := keymgmt.NewStore(db)

	tests := []struct {
		providerType     string
		wantKind         string
		wantAuth         string
		wantAnthropicVer string
	}{
		{"openai", "openai-compatible", "bearer", ""},
		{"openai-compatible", "openai-compatible", "bearer", ""},
		{"anthropic", "anthropic", "api_key_header", defaultAnthropicVersion},
		{"gemini", "gemini", "query_key", ""},
	}
	for _, tc := range tests {
		enc, _ := kv.Encrypt(ctx, []byte("sk-"+tc.providerType+"-1234567890"))
		if _, err := ks.Create(ctx, adminID, tc.providerType, enc, "7890", "", adminID); err != nil {
			t.Fatalf("create %s key: %v", tc.providerType, err)
		}
	}

	b := NewBuilder(db)
	snap, err := b.Build(ctx, 1)
	if err != nil {
		t.Fatalf("build: %v", err)
	}
	if len(snap.Providers) != len(tests) {
		t.Fatalf("providers = %d, want %d", len(snap.Providers), len(tests))
	}

	byKind := make(map[string]*pb.ProviderEntry, len(snap.Providers))
	for _, pe := range snap.Providers {
		byKind[pe.Kind] = pe
	}
	for _, tc := range tests {
		pe, ok := byKind[tc.wantKind]
		if !ok {
			t.Fatalf("missing provider with kind %q", tc.wantKind)
		}
		if pe.AuthMethod != tc.wantAuth {
			t.Fatalf("kind=%s auth_method = %q, want %q", tc.wantKind, pe.AuthMethod, tc.wantAuth)
		}
		if pe.AnthropicVersion != tc.wantAnthropicVer {
			t.Fatalf("kind=%s anthropic_version = %q, want %q",
				tc.wantKind, pe.AnthropicVersion, tc.wantAnthropicVer)
		}
	}
}

// TestBuildSkipsDisabledKeys verifies that disabled keys (enabled=0) are
// excluded from the snapshot: the Rust hot path must not see them.
func TestBuildSkipsDisabledKeys(t *testing.T) {
	db, adminID, kv := newTestDB(t)
	ctx := context.Background()
	ks := keymgmt.NewStore(db)

	enc, _ := kv.Encrypt(ctx, []byte("sk-enabled-12345678"))
	pkOn, err := ks.Create(ctx, adminID, "openai", enc, "5678", "", adminID)
	if err != nil {
		t.Fatalf("create enabled key: %v", err)
	}
	enc2, _ := kv.Encrypt(ctx, []byte("sk-disabled-1234567"))
	pkOff, err := ks.Create(ctx, adminID, "openai", enc2, "4567", "", adminID)
	if err != nil {
		t.Fatalf("create to-be-disabled key: %v", err)
	}
	// Flip the second key off directly; keymgmt.Store has no Update path yet.
	if _, err := db.ExecContext(ctx,
		"UPDATE user_provider_keys SET enabled = 0 WHERE id = ?", pkOff.ID); err != nil {
		t.Fatalf("disable key: %v", err)
	}

	b := NewBuilder(db)
	snap, err := b.Build(ctx, 1)
	if err != nil {
		t.Fatalf("build: %v", err)
	}
	if len(snap.Providers) != 1 {
		t.Fatalf("providers = %d, want 1 (only enabled)", len(snap.Providers))
	}
	if snap.Providers[0].Name != pkOn.ID {
		t.Fatalf("provider name = %q, want %q (only enabled)", snap.Providers[0].Name, pkOn.ID)
	}
	if len(snap.EncryptedKeys) != 1 {
		t.Fatalf("encrypted_keys = %d, want 1 (only enabled)", len(snap.EncryptedKeys))
	}
}

// TestBuildSkipsUnknownProviderType verifies that a row with an unrecognized
// provider_type is skipped (not fatal) so one bad row doesn't block the whole
// push. The row stays in the DB for audit; it just doesn't reach Rust.
func TestBuildSkipsUnknownProviderType(t *testing.T) {
	db, adminID, kv := newTestDB(t)
	ctx := context.Background()
	ks := keymgmt.NewStore(db)

	// Insert a valid openai key.
	enc, _ := kv.Encrypt(ctx, []byte("sk-openai-123456789"))
	if _, err := ks.Create(ctx, adminID, "openai", enc, "6789", "", adminID); err != nil {
		t.Fatalf("create openai key: %v", err)
	}
	// Insert a row with an unknown provider_type directly (Store.Create
	// doesn't validate provider_type against the map).
	enc2, _ := kv.Encrypt(ctx, []byte("sk-weird-1234567890"))
	if _, err := db.ExecContext(ctx,
		`INSERT INTO user_provider_keys (id, owner_user_id, provider_type, encrypted_key, key_last4, base_url, created_by, created_at, enabled)
		 VALUES (?, ?, ?, ?, ?, '', ?, ?, 1)`,
		"pk_unknown", adminID, "some-new-provider", enc2, "7890", adminID, "2026-01-01T00:00:00Z"); err != nil {
		t.Fatalf("insert unknown provider_type: %v", err)
	}

	b := NewBuilder(db)
	snap, err := b.Build(ctx, 1)
	if err != nil {
		t.Fatalf("build: %v", err)
	}
	if len(snap.Providers) != 1 {
		t.Fatalf("providers = %d, want 1 (unknown skipped)", len(snap.Providers))
	}
	if snap.Providers[0].Kind != "openai-compatible" {
		t.Fatalf("provider kind = %q, want openai-compatible", snap.Providers[0].Kind)
	}
}

// TestBuildRespectsCustomBaseURL verifies that a user-supplied base_url wins
// over the canonical default.
func TestBuildRespectsCustomBaseURL(t *testing.T) {
	db, adminID, kv := newTestDB(t)
	ctx := context.Background()
	ks := keymgmt.NewStore(db)

	enc, _ := kv.Encrypt(ctx, []byte("sk-custom-123456789"))
	customURL := "https://my-proxy.example.com/v1"
	if _, err := ks.Create(ctx, adminID, "openai", enc, "6789", customURL, adminID); err != nil {
		t.Fatalf("create key with custom base_url: %v", err)
	}

	b := NewBuilder(db)
	snap, err := b.Build(ctx, 1)
	if err != nil {
		t.Fatalf("build: %v", err)
	}
	if len(snap.Providers) != 1 {
		t.Fatalf("providers = %d, want 1", len(snap.Providers))
	}
	if snap.Providers[0].BaseUrl != customURL {
		t.Fatalf("base_url = %q, want %q", snap.Providers[0].BaseUrl, customURL)
	}
}
