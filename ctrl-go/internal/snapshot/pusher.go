// Package snapshot - Pusher drives the gRPC PushSnapshot client-streaming RPC
// (spec §12C). It enforces three invariants required by the spec:
//
//  1. **Monotonic version**: each Push call increments the Pusher's internal
//     version counter; the Rust kernel sees a strictly increasing sequence.
//  2. **Serialized pushes**: a sync.Mutex guarantees only one Push is in flight
//     at a time (spec §12C: "Go 侧 snapshot 推送经 mutex 串行"). Concurrent
//     callers block on the mutex; this is acceptable because pushes are
//     infrequent (key CRUD) and not on the hot path.
//  3. **Ack-before-synced**: the "synced version" is updated only after the
//     Rust kernel returns Ack{ok:true, version:pushed}. A rejected or
//     network-failed push does NOT advance synced_version, so the next Push
//     will re-attempt (with a new version; versions are monotonic, not
//     reused).
//
// S2 implements **full push only** (delta is S3+ per spec §12C trigger
// conditions). Each Push sends a single Snapshot message built by the Builder
// from the current DB state.
package snapshot

import (
	"context"
	"fmt"
	"sync"

	pb "github.com/whw23/pingogate/ctrl-go/internal/proto"
)

// SnapshotBuilder is the focused interface the Pusher depends on (constitution
// VII). *Builder satisfies it; tests substitute a fake without standing up a
// real DB. Keeping this in the snapshot package (not keymgmt) because the
// Pusher is the only consumer.
type SnapshotBuilder interface {
	Build(ctx context.Context, version uint64) (*pb.Snapshot, error)
}

// Pusher builds a Snapshot from the DB and pushes it to the Rust kernel over
// gRPC. It is safe for concurrent use: the mutex serializes pushes so that
// version ordering is preserved and only one gRPC stream is open at a time.
//
// Construction (main.go): NewPusher(client, builder). The gRPC client must
// already carry mTLS + internal-token interceptors (T11 grpcmtls); the Pusher
// does not add them itself.
type Pusher struct {
	client  pb.SnapshotServiceClient
	builder SnapshotBuilder

	mu            sync.Mutex // serializes Push calls (spec §12C)
	version       uint64     // last version sent (monotonic; incremented per Push)
	syncedVersion uint64     // last version ack'd by Rust (lags version on failure)
}

// NewPusher constructs a Pusher. The client and builder must be non-nil; main.go
// is responsible for constructing them with the right mTLS + DB.
func NewPusher(client pb.SnapshotServiceClient, builder SnapshotBuilder) *Pusher {
	return &Pusher{
		client:  client,
		builder: builder,
	}
}

// SyncedVersion returns the last version the Rust kernel acknowledged. Safe
// for concurrent access (read under the mutex). Returns 0 before the first
// successful push (spec §12B: Rust is not-ready until the first full snapshot).
func (p *Pusher) SyncedVersion() uint64 {
	p.mu.Lock()
	defer p.mu.Unlock()
	return p.syncedVersion
}

// Push builds a Snapshot from the DB and pushes it to the Rust kernel. The
// call is serialized: if another Push is in flight, this one blocks on the
// mutex. The version is incremented BEFORE building so that a build failure
// still consumes a version (keeps the sequence monotonic even on partial
// failures; the next Push will use version+1, never reuse).
//
// Returns nil on success (Ack{ok:true, version==pushed}). On failure the
// synced_version is NOT advanced; the caller (keymgmt handler) surfaces the
// error but the DB change that triggered the push remains in place - the next
// Push (next CRUD op, or main.go startup) will re-attempt with a fresh
// version.
func (p *Pusher) Push(ctx context.Context) error {
	p.mu.Lock()
	defer p.mu.Unlock()

	p.version++
	pushedVersion := p.version

	snap, err := p.builder.Build(ctx, pushedVersion)
	if err != nil {
		return fmt.Errorf("snapshot: build v%d: %w", pushedVersion, err)
	}

	stream, err := p.client.PushSnapshot(ctx)
	if err != nil {
		return fmt.Errorf("snapshot: open push stream: %w", err)
	}

	if err := stream.Send(snap); err != nil {
		return fmt.Errorf("snapshot: send v%d: %w", pushedVersion, err)
	}

	ack, err := stream.CloseAndRecv()
	if err != nil {
		return fmt.Errorf("snapshot: recv ack v%d: %w", pushedVersion, err)
	}

	if !ack.GetOk() {
		return fmt.Errorf("snapshot: rust rejected v%d: %s", pushedVersion, ack.GetError())
	}

	// Ack must carry the version we just pushed (spec §12C: "ack 含 version").
	// A mismatched version indicates a protocol bug or stream corruption;
	// treat as failure and do NOT advance synced_version.
	if ack.GetVersion() != pushedVersion {
		return fmt.Errorf("snapshot: ack version mismatch: pushed=%d ack=%d",
			pushedVersion, ack.GetVersion())
	}

	p.syncedVersion = pushedVersion
	return nil
}

// Ensure Pusher satisfies the keymgmt.SnapshotPusher interface at compile time.
// The interface is declared in keymgmt to keep the dependency direction
// (keymgmt -> snapshot, not the reverse); the closure is here so a signature
// drift in either package breaks the build immediately.
var _ interface {
	Push(context.Context) error
} = (*Pusher)(nil)
