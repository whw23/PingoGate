// Command pingogate-ctrl is the Go non-kernel binary (constitution: "Go 非内核").
//
// S1 scope (T11): prove gRPC connectivity to the Rust kernel. Loads the shared
// internal token, ensures mTLS certs exist (R10), dials the Rust gRPC server
// with mTLS + token interceptors, and pushes an empty snapshot. S2+ adds the
// full control plane (identity, key management, virtual keys, billing).
package main

import (
	"context"
	"log"
	"os"
	"time"

	"google.golang.org/grpc"

	pb "github.com/whw23/pingogate/ctrl-go/internal/proto"
	"github.com/whw23/pingogate/ctrl-go/internal/grpcmtls"
	"github.com/whw23/pingogate/ctrl-go/internal/snapshot"
)

// Env var holding the shared gRPC internal token (spec §12A/§12B). Missing =
// fatal; the Rust kernel would reject every call without it.
const internalTokenEnv = "PINGO_INTERNAL_TOKEN"

// Env var: directory for the mTLS certs (R10). Defaults to
// internal/grpcmtls/certs relative to the module root.
const certsDirEnv = "PINGO_GRPC_CERTS_DIR"

// Default cert directory if PINGO_GRPC_CERTS_DIR is unset.
const defaultCertsDir = "internal/grpcmtls/certs"

// Default gRPC server address (spec §12A: loopback only).
const defaultGrpcAddr = "127.0.0.1:9091"

func main() {
	internalToken := os.Getenv(internalTokenEnv)
	if internalToken == "" {
		log.Fatalf("%s is required (spec §12B)", internalTokenEnv)
	}

	certsDir := os.Getenv(certsDirEnv)
	if certsDir == "" {
		certsDir = defaultCertsDir
	}

	if err := grpcmtls.EnsureCerts(certsDir); err != nil {
		log.Fatalf("ensure mTLS certs: %v", err)
	}

	creds, err := grpcmtls.ClientCredentials(certsDir)
	if err != nil {
		log.Fatalf("load mTLS credentials: %v", err)
	}

	addr := grpcAddrFromEnv()
	conn, err := grpc.NewClient(addr,
		grpc.WithTransportCredentials(creds),
		grpc.WithUnaryInterceptor(grpcmtls.TokenUnaryInterceptor(internalToken)),
		grpc.WithStreamInterceptor(grpcmtls.TokenStreamInterceptor(internalToken)),
	)
	if err != nil {
		log.Fatalf("dial %s: %v", addr, err)
	}
	defer conn.Close()

	ctx, cancel := context.WithTimeout(context.Background(), 10*time.Second)
	defer cancel()

	client := pb.NewSnapshotServiceClient(conn)
	if err := snapshot.PushEmpty(ctx, client); err != nil {
		log.Fatalf("push empty snapshot: %v", err)
	}

	log.Println("S1: gRPC connected (mTLS + token), empty snapshot pushed")
}

// grpcAddrFromEnv reads the gRPC target from PINGO_GRPC_ADDR, defaulting to
// 127.0.0.1:9091 (spec §12A: loopback only).
func grpcAddrFromEnv() string {
	if addr := os.Getenv("PINGO_GRPC_ADDR"); addr != "" {
		return addr
	}
	return defaultGrpcAddr
}
