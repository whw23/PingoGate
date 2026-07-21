// Command pingogate-ctrl is the Go non-kernel binary (constitution: "Go 非内核").
//
// S1 scope (T11): prove gRPC connectivity to the Rust kernel. Loads the shared
// internal token, ensures mTLS certs exist (R10), dials the Rust gRPC server
// with mTLS + token interceptors, and pushes an empty snapshot.
//
// S2 scope (T18 brief Step 2): open the SQLite DB (T17 storage.Open) and run
// identity.BootstrapAdmin at startup so the first start creates an initial
// admin from PINGO_BOOTSTRAP_ADMIN_TOKEN (spec §12B). The DB and bootstrap
// wiring are additive to the S1 gRPC flow; HTTP handlers for User CRUD are
// deferred to T20/T21.
package main

import (
	"context"
	"log"
	"os"
	"time"

	"google.golang.org/grpc"

	"github.com/whw23/pingogate/ctrl-go/internal/grpcmtls"
	"github.com/whw23/pingogate/ctrl-go/internal/identity"
	pb "github.com/whw23/pingogate/ctrl-go/internal/proto"
	"github.com/whw23/pingogate/ctrl-go/internal/snapshot"
	"github.com/whw23/pingogate/ctrl-go/internal/storage"
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

// Env var holding the bootstrap admin token (spec §12B). Missing/empty =
// fatal; without it the first start cannot create an initial admin and the
// operator would have no way to administer the system. On subsequent starts
// (an admin already exists) the env var is a no-op but still required here
// to keep the contract simple (S2 is always platform mode).
const bootstrapTokenEnv = "PINGO_BOOTSTRAP_ADMIN_TOKEN"

// Env var holding the SQLite DSN. Defaults to a local file so a fresh
// checkout works out of the box; production overrides via env. Use
// ":memory:" for tests.
const dbDSNEnv = "PINGO_DB_DSN"

// defaultDBDSN is the default SQLite file path when PINGO_DB_DSN is unset.
const defaultDBDSN = "pingogate.db"

func main() {
	internalToken := os.Getenv(internalTokenEnv)
	if internalToken == "" {
		log.Fatalf("%s is required (spec §12B)", internalTokenEnv)
	}

	bootstrapToken := os.Getenv(bootstrapTokenEnv)
	if bootstrapToken == "" {
		log.Fatalf("%s is required (spec §12B: no degraded path)", bootstrapTokenEnv)
	}

	dsn := os.Getenv(dbDSNEnv)
	if dsn == "" {
		dsn = defaultDBDSN
	}

	// Open the SQLite DB and apply migrations (T17). The DB lives only in
	// the Go binary; the Rust kernel never touches it (constitution X).
	db, err := storage.Open(dsn)
	if err != nil {
		log.Fatalf("open DB %q: %v", dsn, err)
	}
	defer db.Close()

	// Bootstrap the initial admin from the env token (T18 spec §12B). On
	// the first start with an empty DB this creates the admin; on
	// subsequent starts it is a no-op (returns false). Any error is fatal:
	// without an admin the system is unadministerable.
	bootstrapCtx, bootstrapCancel := context.WithTimeout(context.Background(), 10*time.Second)
	defer bootstrapCancel()
	store := identity.NewStore(db)
	created, err := identity.BootstrapAdmin(bootstrapCtx, store, bootstrapToken)
	if err != nil {
		log.Fatalf("bootstrap admin: %v", err)
	}
	if created {
		log.Println("identity: bootstrap admin created")
	} else {
		log.Println("identity: admin already present, bootstrap no-op")
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

	log.Println("S2: DB opened, bootstrap admin ensured, gRPC connected (mTLS + token), empty snapshot pushed")
}

// grpcAddrFromEnv reads the gRPC target from PINGO_GRPC_ADDR, defaulting to
// 127.0.0.1:9091 (spec §12A: loopback only).
func grpcAddrFromEnv() string {
	if addr := os.Getenv("PINGO_GRPC_ADDR"); addr != "" {
		return addr
	}
	return defaultGrpcAddr
}
