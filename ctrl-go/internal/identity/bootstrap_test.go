// Package identity - tests for the bootstrap admin flow (spec §12B). Verifies
// the first call creates an admin, the second call is a no-op, and an empty
// bootstrap token is a fatal error (no degraded path).
package identity

import (
	"context"
	"testing"
)

// TestBootstrapAdmin verifies the brief's bootstrap contract: the first call
// creates an admin (returns true), the second call is a no-op (returns false)
// because an admin already exists. Also verifies the created user can
// authenticate with the bootstrap token.
func TestBootstrapAdmin(t *testing.T) {
	store := newTestStore(t)
	ctx := context.Background()

	// First call: no admin exists, so create one.
	created, err := BootstrapAdmin(ctx, store, "bootstrap-token-abc")
	if err != nil {
		t.Fatalf("first BootstrapAdmin: %v", err)
	}
	if !created {
		t.Fatal("first BootstrapAdmin: created = false, want true")
	}

	// Verify the admin exists and can authenticate with the bootstrap token.
	user, err := store.VerifyToken(ctx, "bootstrap-token-abc")
	if err != nil {
		t.Fatalf("VerifyToken after bootstrap: %v", err)
	}
	if !user.IsAdmin {
		t.Fatal("bootstrap user is not admin")
	}
	if user.Email != BootstrapAdminEmail {
		t.Fatalf("bootstrap email = %q, want %q", user.Email, BootstrapAdminEmail)
	}

	// Count admins must be exactly 1.
	count, err := store.CountAdmins(ctx)
	if err != nil {
		t.Fatalf("CountAdmins: %v", err)
	}
	if count != 1 {
		t.Fatalf("admin count after bootstrap = %d, want 1", count)
	}

	// Second call: an admin exists, so no-op. Returns false, no error.
	created, err = BootstrapAdmin(ctx, store, "different-token-xyz")
	if err != nil {
		t.Fatalf("second BootstrapAdmin: %v", err)
	}
	if created {
		t.Fatal("second BootstrapAdmin: created = true, want false (no-op)")
	}

	// The second bootstrap must NOT have changed the admin count or created a
	// second user. The "different-token-xyz" must not authenticate (the
	// original bootstrap token is still the only valid one).
	count, err = store.CountAdmins(ctx)
	if err != nil {
		t.Fatalf("CountAdmins after second bootstrap: %v", err)
	}
	if count != 1 {
		t.Fatalf("admin count after second bootstrap = %d, want 1", count)
	}
	if _, err := store.VerifyToken(ctx, "different-token-xyz"); err == nil {
		t.Fatal("second bootstrap token unexpectedly authenticates; bootstrap should have been a no-op")
	}
}

// TestBootstrapAdmin_EmptyTokenFatal verifies spec §12B: an empty bootstrap
// token is a fatal error, not a degraded path.
func TestBootstrapAdmin_EmptyTokenFatal(t *testing.T) {
	store := newTestStore(t)
	_, err := BootstrapAdmin(context.Background(), store, "")
	if err == nil {
		t.Fatal("BootstrapAdmin with empty token: want error, got nil")
	}
}
