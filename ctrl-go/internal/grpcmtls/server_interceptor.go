// Package grpcmtls - server-side interceptors that validate the
// x-internal-token metadata on incoming RPCs (T31). The Go UsageService gRPC
// server uses these to enforce the same shared-secret check the Rust kernel
// uses (spec §12A), so a process with the wrong token cannot push usage
// events.
package grpcmtls

import (
	"context"
	"crypto/subtle"

	"google.golang.org/grpc"
	"google.golang.org/grpc/codes"
	"google.golang.org/grpc/metadata"
	"google.golang.org/grpc/status"
)

// TokenServerInterceptor returns a unary server interceptor that validates
// the x-internal-token metadata on every incoming RPC. Symmetric to
// TokenUnaryInterceptor (client-side). Used by the Go UsageService gRPC
// server (T31) so the Rust kernel must present the shared token.
func TokenServerInterceptor(expected string) grpc.UnaryServerInterceptor {
	want := []byte(expected)
	return func(ctx context.Context, req any, info *grpc.UnaryServerInfo, handler grpc.UnaryHandler) (any, error) {
		if err := verifyToken(ctx, want); err != nil {
			return nil, err
		}
		return handler(ctx, req)
	}
}

// TokenStreamServerInterceptor returns a stream server interceptor that
// validates the x-internal-token metadata on every incoming stream RPC.
// Used by the Go UsageService gRPC server for the ReportUsage
// client-streaming RPC (T31).
func TokenStreamServerInterceptor(expected string) grpc.StreamServerInterceptor {
	want := []byte(expected)
	return func(srv any, ss grpc.ServerStream, info *grpc.StreamServerInfo, handler grpc.StreamHandler) error {
		if err := verifyToken(ss.Context(), want); err != nil {
			return err
		}
		return handler(srv, ss)
	}
}

// verifyToken checks the x-internal-token metadata against expected. Uses
// subtle.ConstantTimeEq (re-exported here for server use) to avoid timing
// oracles on the shared secret (constitution XX).
func verifyToken(ctx context.Context, want []byte) error {
	md, ok := metadata.FromIncomingContext(ctx)
	if !ok {
		return status.Error(codes.Unauthenticated, "missing metadata")
	}
	vals := md.Get(internalTokenHeader)
	if len(vals) == 0 {
		return status.Error(codes.Unauthenticated, "missing internal token")
	}
	got := []byte(vals[0])
	if len(got) != len(want) || subtle.ConstantTimeCompare(got, want) != 1 {
		return status.Error(codes.Unauthenticated, "invalid internal token")
	}
	return nil
}
