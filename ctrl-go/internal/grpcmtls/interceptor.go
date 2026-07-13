// Package grpcmtls - gRPC interceptors that attach the x-internal-token metadata
// header to every outgoing unary and stream RPC (spec §12A).
package grpcmtls

import (
	"context"

	"google.golang.org/grpc"
	"google.golang.org/grpc/metadata"
)

// internalTokenHeader is the metadata key checked by the Rust interceptor.
const internalTokenHeader = "x-internal-token"

// TokenUnaryInterceptor returns a unary client interceptor that attaches the
// shared internal token to every outgoing RPC.
func TokenUnaryInterceptor(token string) grpc.UnaryClientInterceptor {
	return func(ctx context.Context, method string, req, reply any, cc *grpc.ClientConn, invoker grpc.UnaryInvoker, opts ...grpc.CallOption) error {
		ctx = metadata.AppendToOutgoingContext(ctx, internalTokenHeader, token)
		return invoker(ctx, method, req, reply, cc, opts...)
	}
}

// TokenStreamInterceptor returns a stream client interceptor that attaches the
// shared internal token to every outgoing stream RPC.
func TokenStreamInterceptor(token string) grpc.StreamClientInterceptor {
	return func(ctx context.Context, desc *grpc.StreamDesc, cc *grpc.ClientConn, method string, streamer grpc.Streamer, opts ...grpc.CallOption) (grpc.ClientStream, error) {
		ctx = metadata.AppendToOutgoingContext(ctx, internalTokenHeader, token)
		return streamer(ctx, desc, cc, method, opts...)
	}
}
