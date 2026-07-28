// Command pingogate-ctrl is the Go non-kernel binary (constitution: "Go 非内核").
//
// S1 scope (T11): prove gRPC connectivity to the Rust kernel. Loads the shared
// internal token, ensures mTLS certs exist (R10), dials the Rust gRPC server
// with mTLS + token interceptors, and pushes an empty snapshot.
//
// S2 scope (T18 brief Step 2): open the SQLite DB (T17 storage.Open) and run
// identity.BootstrapAdmin at startup so the first start creates an initial
// admin from PINGO_BOOTSTRAP_ADMIN_TOKEN (spec §12B).
//
// T20 scope: construct a snapshot.Builder + Pusher, perform the first full
// snapshot sync to the Rust kernel at startup (spec §12B: "Rust 启动后内存快照空,
// Go 检测到 Rust 连接后立即推全量快照"), and mount the keymgmt CRUD routes
// with the Pusher wired in so Create/Delete triggers a gRPC push (spec §12C).
// An HTTP server hosts the CRUD routes; the pusher is best-effort so a Rust
// outage does not block key management.
package main

import (
	"context"
	"log"
	"log/slog"
	"net"
	"net/http"
	"os"
	"time"

	"github.com/go-chi/chi/v5"
	"google.golang.org/grpc"

	"github.com/whw23/pingogate/ctrl-go/internal/grpcmtls"
	"github.com/whw23/pingogate/ctrl-go/internal/identity"
	"github.com/whw23/pingogate/ctrl-go/internal/keymgmt"
	pb "github.com/whw23/pingogate/ctrl-go/internal/proto"
	"github.com/whw23/pingogate/ctrl-go/internal/snapshot"
	"github.com/whw23/pingogate/ctrl-go/internal/storage"
	"github.com/whw23/pingogate/ctrl-go/internal/usage"
	"github.com/whw23/pingogate/ctrl-go/internal/vkey"
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

// Env var holding the HTTP listen address for the control-plane API. Defaults
// to :8080; production overrides via env. The listener hosts User CRUD +
// provider-key CRUD routes (T18 + T20).
const httpAddrEnv = "PINGO_HTTP_ADDR"

// defaultHTTPAddr is the default HTTP listen address when PINGO_HTTP_ADDR is
// unset. S2 is single-node; L4+ multi-tenant puts this behind a load balancer.
const defaultHTTPAddr = ":8080"

// Default gRPC server address for the UsageService that Go serves (T31). The
// Rust kernel dials this address to push UsageEvents. Loopback-only (spec
// §12A). Separate from the Rust-dial port 9091 because Go now serves in
// addition to being a client.
const defaultUsageGrpcAddr = "127.0.0.1:9092"

// Env var for the UsageService gRPC listen address. Defaults to 9092.
const usageGrpcAddrEnv = "PINGO_USAGE_GRPC_ADDR"

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
	idStore := identity.NewStore(db)
	created, err := identity.BootstrapAdmin(bootstrapCtx, idStore, bootstrapToken)
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

	// Construct the KeyVault client (T19) and snapshot Builder + Pusher (T20).
	// The Pusher owns the monotonic version counter and serializes pushes
	// (spec §12C). The Builder reads the DB and assembles a proto Snapshot;
	// the Pusher sends it over gRPC and advances synced_version only on a
	// matching Ack.
	kvClient := keymgmt.NewKeyVaultClient(pb.NewKeyVaultServiceClient(conn))
	builder := snapshot.NewBuilder(db)
	pusher := snapshot.NewPusher(pb.NewSnapshotServiceClient(conn), builder)

	// First full snapshot sync (spec §12B: "Rust 启动后内存快照空, Go 检测到
	// Rust 连接后立即推全量快照"). A failure here is fatal: without the
	// initial sync the Rust kernel stays not-ready and no traffic can flow.
	syncCtx, syncCancel := context.WithTimeout(context.Background(), 15*time.Second)
	defer syncCancel()
	if err := pusher.Push(syncCtx); err != nil {
		log.Fatalf("initial snapshot sync: %v", err)
	}
	log.Printf("snapshot: initial sync complete (version=%d)", pusher.SyncedVersion())

	// Mount the control-plane HTTP routes. AuthMiddleware runs first; each
	// RegisterRoutes adds its own Authorize check on top. The Pusher is
	// injected into keymgmt so Create/Delete triggers a best-effort push.
	r := chi.NewRouter()
	r.Use(identity.AuthMiddleware(idStore))
	identity.RegisterRoutes(r, idStore)
	keyStore := keymgmt.NewStore(db)
	keymgmt.RegisterRoutes(r, keyStore, kvClient, pusher)
	vkeyStore := vkey.NewStore(db)
	vkey.RegisterRoutes(r, vkeyStore, pusher)

	// T31: start the UsageService gRPC server (Rust -> Go). The Rust kernel
	// dials this address to push UsageEvents; Go persists them to the DB
	// (and runs the tiktoken-go estimator when needs_estimate=true). The
	// server runs on its own goroutine; the main goroutine continues to
	// serve HTTP. A failure to start the gRPC server is fatal: without it
	// no usage is recorded and billing is blind.
	usageAddr := usageGrpcAddrFromEnv()
	usageStore := usage.NewStore(db)
	usageEstimator := usage.NewEstimator()
	usageServer := usage.NewServer(usageStore, usageEstimator, slog.Default())
	if err := startUsageGrpcServer(usageAddr, certsDir, internalToken, usageServer); err != nil {
		log.Fatalf("start UsageService gRPC server: %v", err)
	}
	log.Printf("UsageService gRPC listening on %s", usageAddr)

	httpAddr := httpAddrFromEnv()
	log.Printf("control-plane HTTP listening on %s", httpAddr)
	// Constitution XXI: async, non-blocking. Timeouts prevent slow-client
	// exhaustion (review Important #2). Pusher mutex is bounded by HTTP context.
	srv := &http.Server{
		Addr:              httpAddr,
		Handler:           r,
		ReadHeaderTimeout: 10 * time.Second,
		ReadTimeout:       30 * time.Second,
		WriteTimeout:      30 * time.Second,
		IdleTimeout:       120 * time.Second,
	}
	if err := srv.ListenAndServe(); err != nil && err != http.ErrServerClosed {
		log.Fatalf("HTTP server: %v", err)
	}
}

// startUsageGrpcServer binds the UsageService gRPC server on addr with mTLS
// (ctrl cert as server identity, CA-verifies the Rust client cert) + the
// shared-token interceptor (spec §12A). Runs in a background goroutine; the
// caller does not block. Returns an error if the listener cannot bind.
func startUsageGrpcServer(addr, certsDir, internalToken string, srv *usage.Server) error {
	creds, err := grpcmtls.ServerCredentials(certsDir)
	if err != nil {
		return err
	}
	lis, err := net.Listen("tcp", addr)
	if err != nil {
		return err
	}
	gs := grpc.NewServer(
		grpc.Creds(creds),
		grpc.UnaryInterceptor(grpcmtls.TokenServerInterceptor(internalToken)),
		grpc.StreamInterceptor(grpcmtls.TokenStreamServerInterceptor(internalToken)),
	)
	pb.RegisterUsageServiceServer(gs, srv)
	go func() {
		if err := gs.Serve(lis); err != nil {
			log.Fatalf("UsageService gRPC server: %v", err)
		}
	}()
	return nil
}

// usageGrpcAddrFromEnv reads the UsageService gRPC listen address from
// PINGO_USAGE_GRPC_ADDR, defaulting to 127.0.0.1:9092 (spec §12A: loopback).
func usageGrpcAddrFromEnv() string {
	if addr := os.Getenv(usageGrpcAddrEnv); addr != "" {
		return addr
	}
	return defaultUsageGrpcAddr
}

// grpcAddrFromEnv reads the gRPC target from PINGO_GRPC_ADDR, defaulting to
// 127.0.0.1:9091 (spec §12A: loopback only).
func grpcAddrFromEnv() string {
	if addr := os.Getenv("PINGO_GRPC_ADDR"); addr != "" {
		return addr
	}
	return defaultGrpcAddr
}

// httpAddrFromEnv reads the HTTP listen address from PINGO_HTTP_ADDR, defaulting
// to :8080.
func httpAddrFromEnv() string {
	if addr := os.Getenv(httpAddrEnv); addr != "" {
		return addr
	}
	return defaultHTTPAddr
}
