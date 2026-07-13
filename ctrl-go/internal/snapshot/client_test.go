// Package snapshot - S1 contract tests (T12).
//
// These verify the Go<->Rust gRPC connectivity contract that S2's entry
// check (T13) runs to confirm S1's output is consumable. They require a
// running Rust gRPC server (platform mode, 127.0.0.1:9091, mTLS + internal
// token); when the server is not running the tests skip rather than fail.
package snapshot

import (
	"context"
	"os"
	"testing"
	"time"

	pb "github.com/whw23/pingogate/ctrl-go/internal/proto"
	"github.com/whw23/pingogate/ctrl-go/internal/grpcmtls"

	"google.golang.org/grpc"
	"google.golang.org/grpc/codes"
	"google.golang.org/grpc/credentials/insecure"
	"google.golang.org/grpc/metadata"
	"google.golang.org/grpc/status"
)

// grpcAddr is the Rust kernel's gRPC listener (spec §12A: 127.0.0.1 only).
const grpcAddr = "127.0.0.1:9091"

// dialWithToken dials the Rust gRPC server with mTLS + internal token
// interceptors. Returns (conn, skipReason) where skipReason is non-empty when
// the server or certs are unavailable - callers skip in that case.
func dialWithToken(t *testing.T, token string) (*grpc.ClientConn, string) {
	t.Helper()
	creds, err := grpcmtls.ClientCredentials("internal/grpcmtls/certs")
	if err != nil {
		return nil, "certs not ready: " + err.Error()
	}
	conn, err := grpc.NewClient(grpcAddr,
		grpc.WithTransportCredentials(creds),
		grpc.WithUnaryInterceptor(grpcmtls.TokenUnaryInterceptor(token)),
		grpc.WithStreamInterceptor(grpcmtls.TokenStreamInterceptor(token)),
	)
	if err != nil {
		return nil, "dial failed: " + err.Error()
	}
	return conn, ""
}

// TestPushEmptyContract is the core S1 connectivity contract: Go pushes an
// empty snapshot over mTLS + internal token, Rust receives and returns Ack.
// S2's T13 runs this as its entry check.
func TestPushEmptyContract(t *testing.T) {
	conn, skip := dialWithToken(t, os.Getenv("PINGO_INTERNAL_TOKEN"))
	if skip != "" {
		t.Skipf("S1 Rust gRPC not available: %s", skip)
	}
	defer conn.Close()

	ctx, cancel := context.WithTimeout(context.Background(), 5*time.Second)
	defer cancel()
	if err := PushEmpty(ctx, pb.NewSnapshotServiceClient(conn)); err != nil {
		t.Fatalf("push empty snapshot: %v", err)
	}
}

// TestGrpcRejectsMissingOrWrongToken verifies spec §12A security: a call
// without the internal token (or with a wrong one) is rejected by the Rust
// interceptor with Unauthenticated, even when mTLS succeeds.
func TestGrpcRejectsMissingOrWrongToken(t *testing.T) {
	for _, token := range []string{"", "wrong-token"} {
		conn, skip := dialWithToken(t, token)
		if skip != "" {
			t.Skipf("S1 Rust gRPC not available: %s", skip)
		}
		ctx, cancel := context.WithTimeout(context.Background(), 5*time.Second)
		// Heartbeat is a unary RPC: simplest probe to trigger the interceptor.
		_, err := pb.NewSnapshotServiceClient(conn).Heartbeat(ctx, &pb.HeartbeatRequest{})
		cancel()
		conn.Close()
		if err == nil {
			t.Fatalf("token %q: expected Unauthenticated, got nil", token)
		}
		if got := status.Code(err); got != codes.Unauthenticated {
			t.Fatalf("token %q: expected Unauthenticated, got %v", token, got)
		}
	}
}

// TestTokenInterceptorAttachesHeader is a pure-Go unit test (no server) that
// verifies TokenUnaryInterceptor attaches the x-internal-token metadata. This
// part of the contract does NOT require a running Rust server, so it never
// skips.
func TestTokenInterceptorAttachesHeader(t *testing.T) {
	ctx := context.Background()
	captured := make(chan string, 1)
	invoker := func(ctx context.Context, method string, req, reply any, cc *grpc.ClientConn, opts ...grpc.CallOption) error {
		md, ok := metadata.FromOutgoingContext(ctx)
		if !ok {
			t.Fatal("no outgoing metadata attached")
		}
		vals := md.Get("x-internal-token")
		if len(vals) == 0 {
			t.Fatal("x-internal-token not attached")
		}
		captured <- vals[0]
		return nil
	}
	_ = grpc.NewClient // reference to keep import; not used here
	intc := grpcmtls.TokenUnaryInterceptor("s1-contract-token")
	if err := intc(ctx, "/pingogate.SnapshotService/Heartbeat", nil, nil, nil, invoker); err != nil {
		t.Fatalf("interceptor invoker: %v", err)
	}
	select {
	case got := <-captured:
		if got != "s1-contract-token" {
			t.Fatalf("attached token = %q, want %q", got, "s1-contract-token")
		}
	default:
		t.Fatal("interceptor did not invoke the caller")
	}
}

// silence unused import when insecure is not referenced in all build tags.
var _ = insecure.NewCredentials
