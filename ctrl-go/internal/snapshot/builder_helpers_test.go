// Package snapshot - helpers for Builder tests (extracted from builder_test.go
// to keep that file under constitution V's 300-line limit). Same package,
// compiles as part of the test binary.
package snapshot

import (
	"context"
	"testing"

	"github.com/whw23/pingogate/ctrl-go/internal/identity"
	"github.com/whw23/pingogate/ctrl-go/internal/keymgmt"
	"github.com/whw23/pingogate/ctrl-go/internal/storage"
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
