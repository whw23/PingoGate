// Package snapshot - builder test for virtual_keys (T29). Verifies the
// builder includes enabled virtual keys in the proto Snapshot, with the
// correct token_hash format (hex(sha256(token))) matching the Rust T26
// VirtualKeyAuth, and that disabled keys are excluded.
package snapshot

import (
	"context"
	"crypto/sha256"
	"encoding/hex"
	"testing"

	"github.com/whw23/pingogate/ctrl-go/internal/vkey"
)

// TestBuildIncludesVirtualKeys verifies the builder reads enabled
// virtual_keys from the DB and emits VirtualKeyEntry messages with the
// correct token_hash (hex(sha256(plaintext))).
func TestBuildIncludesVirtualKeys(t *testing.T) {
	db, adminID, _ := newTestDB(t)
	ctx := context.Background()
	vs := vkey.NewStore(db)

	plaintext, vk, err := vs.Issue(ctx, adminID, vkey.Scope{
		AllowedModels:    []string{"gpt-4"},
		AllowedProviders: []string{"openai"},
		MaxConcurrency:   5,
	})
	if err != nil {
		t.Fatalf("issue vkey: %v", err)
	}

	b := NewBuilder(db)
	snap, err := b.Build(ctx, 7)
	if err != nil {
		t.Fatalf("build: %v", err)
	}
	if len(snap.VirtualKeys) != 1 {
		t.Fatalf("virtual_keys = %d, want 1", len(snap.VirtualKeys))
	}

	entry := snap.VirtualKeys[0]
	if entry.Id != vk.ID {
		t.Fatalf("id = %q, want %q", entry.Id, vk.ID)
	}
	sum := sha256.Sum256([]byte(plaintext))
	wantHash := hex.EncodeToString(sum[:])
	if entry.TokenHash != wantHash {
		t.Fatalf("token_hash = %q, want %q", entry.TokenHash, wantHash)
	}
	if entry.OwnerUserId != adminID {
		t.Fatalf("owner_user_id = %q, want %q", entry.OwnerUserId, adminID)
	}
	if len(entry.AllowedModels) != 1 || entry.AllowedModels[0] != "gpt-4" {
		t.Fatalf("allowed_models = %v, want [gpt-4]", entry.AllowedModels)
	}
	if len(entry.AllowedProviders) != 1 || entry.AllowedProviders[0] != "openai" {
		t.Fatalf("allowed_providers = %v, want [openai]", entry.AllowedProviders)
	}
	if entry.MaxConcurrency != 5 {
		t.Fatalf("max_concurrency = %d, want 5", entry.MaxConcurrency)
	}
	if !entry.Enabled {
		t.Fatal("enabled = false, want true")
	}
}

// TestBuildSkipsDisabledVirtualKeys verifies that revoked (enabled=0)
// virtual keys are excluded from the snapshot.
func TestBuildSkipsDisabledVirtualKeys(t *testing.T) {
	db, adminID, _ := newTestDB(t)
	ctx := context.Background()
	vs := vkey.NewStore(db)

	_, vkOn, err := vs.Issue(ctx, adminID, vkey.Scope{})
	if err != nil {
		t.Fatalf("issue 1: %v", err)
	}
	_, vkOff, err := vs.Issue(ctx, adminID, vkey.Scope{})
	if err != nil {
		t.Fatalf("issue 2: %v", err)
	}
	if err := vs.Revoke(ctx, vkOff.ID); err != nil {
		t.Fatalf("revoke: %v", err)
	}

	b := NewBuilder(db)
	snap, err := b.Build(ctx, 1)
	if err != nil {
		t.Fatalf("build: %v", err)
	}
	if len(snap.VirtualKeys) != 1 {
		t.Fatalf("virtual_keys = %d, want 1 (only enabled)",
			len(snap.VirtualKeys))
	}
	if snap.VirtualKeys[0].Id != vkOn.ID {
		t.Fatalf("virtual_key id = %q, want %q (only enabled)",
			snap.VirtualKeys[0].Id, vkOn.ID)
	}
}
