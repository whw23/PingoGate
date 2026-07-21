// Package snapshot - helpers for Pusher tests (extracted from pusher_test.go
// to keep that file under constitution V's 300-line limit). Same package,
// compiles as part of the test binary.
package snapshot

import (
	"context"
	"errors"
	"net"
	"sync"
	"sync/atomic"
	"testing"

	pb "github.com/whw23/pingogate/ctrl-go/internal/proto"
	"google.golang.org/grpc"
	"google.golang.org/grpc/credentials/insecure"
)

// fakeSnapshotServer is a minimal SnapshotServiceServer that records every
// Snapshot it receives over a PushSnapshot stream and returns a configurable
// Ack. It is the Rust kernel stand-in for Pusher tests.
type fakeSnapshotServer struct {
	pb.UnimplementedSnapshotServiceServer

	mu       sync.Mutex
	received []*pb.Snapshot
	ackOk    atomic.Bool   // if false, Ack{ok:false, error:"injected"}
	ackVer   atomic.Uint64 // if non-zero, override the ack version (for mismatch test)
}

func newFakeSnapshotServer() *fakeSnapshotServer {
	f := &fakeSnapshotServer{}
	f.ackOk.Store(true)
	return f
}

// PushSnapshot drains the client stream, records each Snapshot, and returns
// a single Ack. The Ack version echoes the last received snapshot's version
// (matching the Rust kernel behavior in core-rs/pingogate-core/src/grpc.rs)
// unless ackVer has been set to a non-zero override.
func (f *fakeSnapshotServer) PushSnapshot(stream pb.SnapshotService_PushSnapshotServer) error {
	var lastVer uint64
	for {
		msg, err := stream.Recv()
		if err != nil {
			// Recv returns io.EOF when the client closes the send side.
			break
		}
		f.mu.Lock()
		f.received = append(f.received, msg)
		f.mu.Unlock()
		lastVer = msg.GetVersion()
	}

	if !f.ackOk.Load() {
		return stream.SendAndClose(&pb.Ack{
			Version: lastVer,
			Ok:      false,
			Error:   "injected failure",
		})
	}
	ackVer := f.ackVer.Load()
	if ackVer == 0 {
		ackVer = lastVer
	}
	return stream.SendAndClose(&pb.Ack{
		Version: ackVer,
		Ok:      true,
	})
}

// Received returns a snapshot of the snapshots received so far (copy so callers
// can't mutate the internal slice).
func (f *fakeSnapshotServer) Received() []*pb.Snapshot {
	f.mu.Lock()
	defer f.mu.Unlock()
	out := make([]*pb.Snapshot, len(f.received))
	copy(out, f.received)
	return out
}

// dialFakeServer wires up a loopback gRPC server with the fakeSnapshotServer
// registered and returns a connected client + the fake + a cleanup func. Uses
// an ephemeral TCP port (port 0 -> kernel assigns) so parallel tests don't
// collide.
func dialFakeServer(t *testing.T) (pb.SnapshotServiceClient, *fakeSnapshotServer, func()) {
	t.Helper()
	fake := newFakeSnapshotServer()
	lis, err := net.Listen("tcp", "127.0.0.1:0")
	if err != nil {
		t.Fatalf("listen: %v", err)
	}
	srv := grpc.NewServer()
	pb.RegisterSnapshotServiceServer(srv, fake)

	go func() { _ = srv.Serve(lis) }()

	addr := lis.Addr().String()
	conn, err := grpc.NewClient(addr,
		grpc.WithTransportCredentials(insecure.NewCredentials()),
	)
	if err != nil {
		t.Fatalf("dial %s: %v", addr, err)
	}
	cleanup := func() {
		_ = conn.Close()
		srv.Stop()
		_ = lis.Close()
	}
	return pb.NewSnapshotServiceClient(conn), fake, cleanup
}

// emptyBuilder is a Builder stand-in for Pusher tests that don't care about
// DB contents - they only verify the Pusher's serialization / version / ack
// logic. It returns a Snapshot with the requested version and no entries.
type emptyBuilder struct{}

func (emptyBuilder) Build(_ context.Context, version uint64) (*pb.Snapshot, error) {
	return &pb.Snapshot{Version: version}, nil
}

// failingBuilder always returns an error from Build, to verify the Pusher
// surfaces build failures and does NOT advance synced_version.
type failingBuilder struct{}

func (failingBuilder) Build(_ context.Context, _ uint64) (*pb.Snapshot, error) {
	return nil, errors.New("injected build failure")
}

// newPusherWithEmptyBuilder wires a Pusher against a fake gRPC server and an
// emptyBuilder. Returns the Pusher, the fake server, and cleanup.
func newPusherWithEmptyBuilder(t *testing.T) (*Pusher, *fakeSnapshotServer, func()) {
	t.Helper()
	client, fake, cleanup := dialFakeServer(t)
	p := NewPusher(client, emptyBuilder{})
	return p, fake, cleanup
}
