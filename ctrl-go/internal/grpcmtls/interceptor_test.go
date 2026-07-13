package grpcmtls

import (
	"context"
	"testing"

	"google.golang.org/grpc"
	"google.golang.org/grpc/metadata"
)

// TestTokenUnaryInterceptor_AttachesToken verifies that the unary interceptor
// attaches the x-internal-token header to the outgoing context metadata before
// calling the invoker.
func TestTokenUnaryInterceptor_AttachesToken(t *testing.T) {
	token := "shared-internal-token"
	interceptor := TokenUnaryInterceptor(token)

	var capturedMD metadata.MD
	invoker := func(ctx context.Context, method string, req, reply any, cc *grpc.ClientConn, opts ...grpc.CallOption) error {
		capturedMD, _ = metadata.FromOutgoingContext(ctx)
		return nil
	}

	_ = interceptor(context.Background(), "/pingogate.SnapshotService/Heartbeat", nil, nil, nil, invoker)

	vals := capturedMD.Get(internalTokenHeader)
	if len(vals) != 1 {
		t.Fatalf("expected 1 token header value, got %d", len(vals))
	}
	if vals[0] != token {
		t.Errorf("expected token %q, got %q", token, vals[0])
	}
}

// TestTokenStreamInterceptor_AttachesToken verifies that the stream interceptor
// attaches the x-internal-token header to the outgoing context metadata.
func TestTokenStreamInterceptor_AttachesToken(t *testing.T) {
	token := "shared-internal-token"
	interceptor := TokenStreamInterceptor(token)

	var capturedMD metadata.MD
	streamer := func(ctx context.Context, desc *grpc.StreamDesc, cc *grpc.ClientConn, method string, opts ...grpc.CallOption) (grpc.ClientStream, error) {
		capturedMD, _ = metadata.FromOutgoingContext(ctx)
		return nil, nil
	}

	_, _ = interceptor(context.Background(), &grpc.StreamDesc{}, nil, "/pingogate.SnapshotService/PushSnapshot", streamer)

	vals := capturedMD.Get(internalTokenHeader)
	if len(vals) != 1 {
		t.Fatalf("expected 1 token header value, got %d", len(vals))
	}
	if vals[0] != token {
		t.Errorf("expected token %q, got %q", token, vals[0])
	}
}
