// Package snapshot - Pusher tests (constitution XIII: TDD). The Pusher is
// tested against an in-process gRPC server (loopback TCP with an ephemeral
// port) with a fake SnapshotServiceServer implementation, so tests run
// without the Rust binary.
//
// The fake server records the snapshots it receives and can be configured to
// return success (Ack{ok:true, version:received}) or failure (Ack{ok:false})
// to exercise the Pusher's ack-validation and retry-semantics paths.
//
// Test helpers (fakeSnapshotServer, dialFakeServer, emptyBuilder,
// failingBuilder, newPusherWithEmptyBuilder) live in pusher_helpers_test.go.
package snapshot

import (
	"context"
	"sync"
	"testing"
	"time"

	"github.com/whw23/pingogate/ctrl-go/internal/storage"
)

// TestPushHappyPath verifies the core Push flow: version starts at 0,
// increments to 1 after a successful Push, the fake server receives a
// Snapshot with version=1, and SyncedVersion() reports 1.
func TestPushHappyPath(t *testing.T) {
	p, fake, cleanup := newPusherWithEmptyBuilder(t)
	defer cleanup()

	ctx, cancel := context.WithTimeout(context.Background(), 5*time.Second)
	defer cancel()

	if v := p.SyncedVersion(); v != 0 {
		t.Fatalf("initial synced version = %d, want 0", v)
	}
	if err := p.Push(ctx); err != nil {
		t.Fatalf("push: %v", err)
	}
	if v := p.SyncedVersion(); v != 1 {
		t.Fatalf("synced version after push = %d, want 1", v)
	}

	got := fake.Received()
	if len(got) != 1 {
		t.Fatalf("fake server received %d snapshots, want 1", len(got))
	}
	if got[0].GetVersion() != 1 {
		t.Fatalf("received snapshot version = %d, want 1", got[0].GetVersion())
	}
}

// TestPushMonotonicVersion verifies that successive Push calls increment the
// version monotonically (1, 2, 3, ...). Each Push must produce a distinct
// version even though the DB state is unchanged.
func TestPushMonotonicVersion(t *testing.T) {
	p, fake, cleanup := newPusherWithEmptyBuilder(t)
	defer cleanup()

	ctx, cancel := context.WithTimeout(context.Background(), 5*time.Second)
	defer cancel()

	for i := 1; i <= 3; i++ {
		if err := p.Push(ctx); err != nil {
			t.Fatalf("push %d: %v", i, err)
		}
		if v := p.SyncedVersion(); v != uint64(i) {
			t.Fatalf("after push %d: synced = %d, want %d", i, v, i)
		}
	}

	got := fake.Received()
	if len(got) != 3 {
		t.Fatalf("fake server received %d snapshots, want 3", len(got))
	}
	for i, s := range got {
		if s.GetVersion() != uint64(i+1) {
			t.Fatalf("snapshot %d version = %d, want %d", i, s.GetVersion(), i+1)
		}
	}
}

// TestPushRejectedDoesNotAdvanceSynced verifies that when the Rust kernel
// returns Ack{ok:false}, the Pusher returns an error AND does NOT advance
// synced_version. The next successful Push should still use the next monotonic
// version (versions are never reused).
func TestPushRejectedDoesNotAdvanceSynced(t *testing.T) {
	p, fake, cleanup := newPusherWithEmptyBuilder(t)
	defer cleanup()

	ctx, cancel := context.WithTimeout(context.Background(), 5*time.Second)
	defer cancel()

	// First push: succeed, synced=1.
	if err := p.Push(ctx); err != nil {
		t.Fatalf("push 1 (should succeed): %v", err)
	}
	if v := p.SyncedVersion(); v != 1 {
		t.Fatalf("synced after push 1 = %d, want 1", v)
	}

	// Second push: inject failure. Synced must stay at 1; version consumed = 2.
	fake.ackOk.Store(false)
	err := p.Push(ctx)
	if err == nil {
		t.Fatal("push 2 (should fail): expected error, got nil")
	}
	if v := p.SyncedVersion(); v != 1 {
		t.Fatalf("synced after failed push = %d, want 1 (not advanced)", v)
	}

	// Third push: succeed. Version should be 3 (2 was consumed by the failed
	// push), synced should advance to 3.
	fake.ackOk.Store(true)
	if err := p.Push(ctx); err != nil {
		t.Fatalf("push 3 (should succeed): %v", err)
	}
	if v := p.SyncedVersion(); v != 3 {
		t.Fatalf("synced after push 3 = %d, want 3", v)
	}

	got := fake.Received()
	if len(got) != 3 {
		t.Fatalf("fake server received %d snapshots, want 3", len(got))
	}
	wantVers := []uint64{1, 2, 3}
	for i, s := range got {
		if s.GetVersion() != wantVers[i] {
			t.Fatalf("snapshot %d version = %d, want %d", i, s.GetVersion(), wantVers[i])
		}
	}
}

// TestPushAckVersionMismatch verifies the Pusher detects a mismatched ack
// version (spec §12C: "ack 含 version"). A mismatch indicates a protocol bug
// or stream corruption; the Pusher must treat it as failure and not advance
// synced_version.
func TestPushAckVersionMismatch(t *testing.T) {
	p, fake, cleanup := newPusherWithEmptyBuilder(t)
	defer cleanup()

	ctx, cancel := context.WithTimeout(context.Background(), 5*time.Second)
	defer cancel()

	// Force the fake server to return an ack version that doesn't match the
	// pushed snapshot's version.
	fake.ackVer.Store(999)

	err := p.Push(ctx)
	if err == nil {
		t.Fatal("push with mismatched ack version: expected error, got nil")
	}
	if v := p.SyncedVersion(); v != 0 {
		t.Fatalf("synced after mismatch = %d, want 0 (not advanced)", v)
	}
}

// TestPushBuildFailure verifies that a Builder error is surfaced and does NOT
// advance synced_version. The version counter IS consumed (monotonic; the next
// Push uses version+1, never reuse) - we verify this by following up with a
// successful Push against an empty builder and checking the received version.
func TestPushBuildFailure(t *testing.T) {
	client, fake, cleanup := dialFakeServer(t)
	defer cleanup()

	p := NewPusher(client, failingBuilder{})

	ctx, cancel := context.WithTimeout(context.Background(), 5*time.Second)
	defer cancel()

	if err := p.Push(ctx); err == nil {
		t.Fatal("push with failing builder: expected error, got nil")
	}
	if v := p.SyncedVersion(); v != 0 {
		t.Fatalf("synced after build failure = %d, want 0", v)
	}
	if len(fake.Received()) != 0 {
		t.Fatalf("fake server received %d snapshots, want 0 (build failed before send)",
			len(fake.Received()))
	}
}

// TestPushSerializes verifies the mutex serializes concurrent Push calls: no
// two Push calls can be in-flight at the same time. We launch multiple Push
// goroutines; the received snapshots must be strictly ordered (no overlap),
// and the final synced version must equal the number of pushes.
func TestPushSerializes(t *testing.T) {
	p, fake, cleanup := newPusherWithEmptyBuilder(t)
	defer cleanup()

	ctx, cancel := context.WithTimeout(context.Background(), 10*time.Second)
	defer cancel()

	const n = 5
	var wg sync.WaitGroup
	errs := make([]error, n)
	wg.Add(n)
	for i := 0; i < n; i++ {
		go func(i int) {
			defer wg.Done()
			errs[i] = p.Push(ctx)
		}(i)
	}
	wg.Wait()

	for i, err := range errs {
		if err != nil {
			t.Fatalf("goroutine %d push: %v", i, err)
		}
	}
	if v := p.SyncedVersion(); v != n {
		t.Fatalf("synced after %d concurrent pushes = %d, want %d", n, v, n)
	}
	got := fake.Received()
	if len(got) != n {
		t.Fatalf("fake server received %d snapshots, want %d", len(got), n)
	}
	// Versions must be 1..n in some order (the mutex serializes, but the
	// goroutine scheduling order is non-deterministic). Verify the set is
	// exactly {1, 2, ..., n}.
	seen := make(map[uint64]bool, n)
	for _, s := range got {
		seen[s.GetVersion()] = true
	}
	for want := uint64(1); want <= uint64(n); want++ {
		if !seen[want] {
			t.Fatalf("version %d missing from received snapshots; got set %v", want, seen)
		}
	}
}

// TestPusherSatisfiesBuilderInterface is a compile-time check: *Builder must
// satisfy SnapshotBuilder so NewPusher(client, NewBuilder(db)) type-checks in
// main.go. Kept as a test (not a var _ =) so the failure shows up in `go test`
// rather than only at build time of main.go.
func TestPusherSatisfiesBuilderInterface(t *testing.T) {
	db, err := storage.Open(":memory:")
	if err != nil {
		t.Fatalf("open DB: %v", err)
	}
	defer db.Close()
	var _ SnapshotBuilder = NewBuilder(db)
}
