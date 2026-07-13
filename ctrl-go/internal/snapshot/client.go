// Package snapshot contains the Go control plane's gRPC client for pushing
// runtime snapshots to the Rust kernel (spec §12C). S1 only pushes an empty
// placeholder snapshot to prove connectivity; S2 builds and pushes the real
// RuntimeSnapshot payload.
package snapshot

import (
	"context"
	"fmt"

	pb "github.com/whw23/pingogate/ctrl-go/internal/proto"
)

// PushEmpty sends a single Snapshot{version:1, payload:"S1-empty"} over the
// PushSnapshot client-streaming RPC and returns the Ack from the Rust kernel.
// This is the S1 connectivity probe (T11).
func PushEmpty(ctx context.Context, client pb.SnapshotServiceClient) error {
	stream, err := client.PushSnapshot(ctx)
	if err != nil {
		return fmt.Errorf("open push_snapshot stream: %w", err)
	}

	if err := stream.Send(&pb.Snapshot{
		Version: 1,
		Payload: []byte("S1-empty"),
	}); err != nil {
		return fmt.Errorf("send snapshot: %w", err)
	}

	if err := stream.CloseSend(); err != nil {
		return fmt.Errorf("close send: %w", err)
	}

	ack, err := stream.CloseAndRecv()
	if err != nil {
		return fmt.Errorf("recv ack: %w", err)
	}

	if !ack.GetOk() {
		return fmt.Errorf("rust kernel rejected snapshot: %s", ack.GetError())
	}

	return nil
}
