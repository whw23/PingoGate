// Package main (import-yaml) imports a standalone-mode pingogate-core.yaml into
// the Go control-plane DB (issue 5: platform mode should auto-add standalone
// config to Go).
//
// Usage:
//
//	go run ./cmd/import-yaml --yaml ../pingogate-core.yaml
//
// For each provider in the YAML, the command:
//  1. Resolves the key_ref (env:VAR or plain:value) to plaintext.
//  2. Calls the Rust KeyVault gRPC Encrypt to produce ciphertext.
//  3. Inserts a user_provider_keys row (owner = bootstrap admin).
//  4. Inserts routes rows for each route alias.
//
// The command is idempotent: re-running with the same YAML skips providers
// whose name (key ID) already exists in the DB. It does NOT delete providers
// that are absent from the YAML (safe by default; use --prune to remove).
//
// This is a one-time bootstrap tool: after import, the user manages keys via
// the control-plane API. The YAML is not watched for changes.
package main

import (
	"context"
	"flag"
	"fmt"
	"log"
	"os"
	"strings"
	"time"

	"gopkg.in/yaml.v3"
	"google.golang.org/grpc"

	"github.com/whw23/pingogate/ctrl-go/internal/grpcmtls"
	"github.com/whw23/pingogate/ctrl-go/internal/identity"
	pb "github.com/whw23/pingogate/ctrl-go/internal/proto"
	"github.com/whw23/pingogate/ctrl-go/internal/storage"
)

// standaloneConfig mirrors the pingogate-core.yaml schema (subset: providers
// with nested models). We intentionally do not import listeners/gateway_keys
// (platform mode gets those from env/Go config, not YAML). The old flat
// `routes:` section is gone; models are nested under each provider.
type standaloneConfig struct {
	Providers []providerCfg `yaml:"providers"`
}

type providerCfg struct {
	Name              string    `yaml:"name"`
	Kind              string    `yaml:"kind"`
	BaseURL           string    `yaml:"base_url"`
	AnthropicVersion  string    `yaml:"anthropic_version"`
	Auth              authCfg   `yaml:"auth"`
	CapabilityFamilies []string `yaml:"capability_families"`
	TimeoutMs         *uint64   `yaml:"timeout_ms"`
	UpstreamPath      string    `yaml:"upstream_path"`
	Models            []modelCfg `yaml:"models"`
}

type authCfg struct {
	Method string `yaml:"method"`
	KeyRef string `yaml:"key_ref"`
}

// modelCfg mirrors a model nested under a provider (config hierarchy:
// model > provider). All fields except alias/upstream_model are optional
// overrides.
type modelCfg struct {
	Alias            string  `yaml:"alias"`
	UpstreamModel    string  `yaml:"upstream_model"`
	Kind             string  `yaml:"kind"`
	AuthMethod       string  `yaml:"auth_method"`
	AnthropicVersion string  `yaml:"anthropic_version"`
	TimeoutMs        *uint64 `yaml:"timeout_ms"`
	UpstreamPath     string  `yaml:"upstream_path"`
}

// providerTypeFromKind maps the YAML kind to the DB provider_type used by
// keymgmt.providerDefaults. openai-compatible -> openai; others pass through.
func providerTypeFromKind(kind string) string {
	switch kind {
	case "openai-compatible":
		return "openai"
	default:
		return kind
	}
}

func main() {
	yamlPath := flag.String("yaml", "", "path to pingogate-core.yaml (required)")
	dsn := flag.String("db", "", "SQLite DSN (defaults to PINGO_DB_DSN or pingogate.db)")
	grpcAddr := flag.String("grpc-addr", "", "Rust gRPC address (defaults to PINGO_GRPC_ADDR or 127.0.0.1:9091)")
	adminToken := flag.String("admin-token", "", "bootstrap admin token (defaults to PINGO_BOOTSTRAP_ADMIN_TOKEN)")
	certsDir := flag.String("certs-dir", "", "mTLS certs directory (defaults to PINGO_GRPC_CERTS_DIR)")
	flag.Parse()

	if *yamlPath == "" {
		log.Fatal("--yaml is required")
	}

	if *dsn == "" {
		*dsn = os.Getenv("PINGO_DB_DSN")
		if *dsn == "" {
			*dsn = "pingogate.db"
		}
	}
	if *grpcAddr == "" {
		*grpcAddr = os.Getenv("PINGO_GRPC_ADDR")
		if *grpcAddr == "" {
			*grpcAddr = "127.0.0.1:9091"
		}
	}
	if *adminToken == "" {
		*adminToken = os.Getenv("PINGO_BOOTSTRAP_ADMIN_TOKEN")
	}
	if *adminToken == "" {
		log.Fatal("admin token is required (set --admin-token or PINGO_BOOTSTRAP_ADMIN_TOKEN)")
	}
	if *certsDir == "" {
		*certsDir = os.Getenv("PINGO_GRPC_CERTS_DIR")
		if *certsDir == "" {
			*certsDir = "internal/grpcmtls/certs"
		}
	}

	// Parse the YAML.
	data, err := os.ReadFile(*yamlPath)
	if err != nil {
		log.Fatalf("read %q: %v", *yamlPath, err)
	}
	var cfg standaloneConfig
	if err := yaml.Unmarshal(data, &cfg); err != nil {
		log.Fatalf("parse YAML: %v", err)
	}
	if len(cfg.Providers) == 0 {
		log.Fatal("no providers found in YAML")
	}

	// Open the DB and apply migrations.
	db, err := storage.Open(*dsn)
	if err != nil {
		log.Fatalf("open DB %q: %v", *dsn, err)
	}
	defer db.Close()

	// Ensure the bootstrap admin exists (so we have an owner for imported keys).
	ctx, cancel := context.WithTimeout(context.Background(), 30*time.Second)
	defer cancel()
	idStore := identity.NewStore(db)
	created, err := identity.BootstrapAdmin(ctx, idStore, *adminToken)
	if err != nil {
		log.Fatalf("bootstrap admin: %v", err)
	}
	if created {
		log.Println("identity: bootstrap admin created")
	}
	// Fetch the admin user ID (needed as owner_user_id for imported keys).
	adminUser, err := getFirstAdmin(ctx, idStore)
	if err != nil {
		log.Fatalf("get admin user: %v (ensure PINGO_BOOTSTRAP_ADMIN_TOKEN matches)", err)
	}

	// Ensure mTLS certs + dial the Rust gRPC.
	if err := grpcmtls.EnsureCerts(*certsDir); err != nil {
		log.Fatalf("ensure mTLS certs: %v", err)
	}
	creds, err := grpcmtls.ClientCredentials(*certsDir)
	if err != nil {
		log.Fatalf("load mTLS credentials: %v", err)
	}
	internalToken := os.Getenv("PINGO_INTERNAL_TOKEN")
	if internalToken == "" {
		log.Fatal("PINGO_INTERNAL_TOKEN is required (to call Rust KeyVault Encrypt)")
	}
	conn, err := grpc.NewClient(*grpcAddr,
		grpc.WithTransportCredentials(creds),
		grpc.WithUnaryInterceptor(grpcmtls.TokenUnaryInterceptor(internalToken)),
	)
	if err != nil {
		log.Fatalf("dial %s: %v", *grpcAddr, err)
	}
	defer conn.Close()
	kvClient := pb.NewKeyVaultServiceClient(conn)

	// Import each provider and its nested models as routes.
	imported, skipped := 0, 0
	routeImported := 0
	for _, p := range cfg.Providers {
		// Check if a provider key with this name already exists (idempotent).
		keyID := "imp_" + p.Name
		exists, err := providerKeyExists(ctx, db, keyID)
		if err != nil {
			log.Fatalf("check existing key %q: %v", keyID, err)
		}
		if exists {
			log.Printf("skip: provider %q already imported (key_id=%s)", p.Name, keyID)
			skipped++
		} else {
			// Resolve the key_ref to plaintext.
			plaintext, err := resolveKeyRef(p.Auth.KeyRef)
			if err != nil {
				log.Fatalf("resolve key_ref for provider %q: %v", p.Name, err)
			}

			// Encrypt via Rust KeyVault.
			encResp, err := kvClient.Encrypt(ctx, &pb.EncryptRequest{Plaintext: []byte(plaintext)})
			if err != nil {
				log.Fatalf("encrypt key for provider %q: %v", p.Name, err)
			}
			if encResp.Error != "" {
				log.Fatalf("encrypt key for provider %q: %s", p.Name, encResp.Error)
			}

			// Insert into DB.
			providerType := providerTypeFromKind(p.Kind)
			last4 := computeLast4(plaintext)
			_, err = db.ExecContext(ctx,
				`INSERT INTO user_provider_keys (id, owner_user_id, provider_type, encrypted_key, key_last4, base_url, created_by, created_at, enabled)
				 VALUES (?, ?, ?, ?, ?, ?, ?, ?, 1)`,
				keyID, adminUser.ID, providerType, encResp.Ciphertext, last4, p.BaseURL, adminUser.ID,
				time.Now().UTC().Format(time.RFC3339))
			if err != nil {
				log.Fatalf("insert provider key %q: %v", p.Name, err)
			}
			log.Printf("imported: provider %q -> key_id=%s (type=%s, last4=...%s)", p.Name, keyID, providerType, last4)
			imported++
		}

		// Import the provider's models as routes (model override ?? provider
		// default). The routes table is optional; if it does not exist yet,
		// the insert fails and we skip gracefully.
		for _, m := range p.Models {
			routeImported += importRoute(ctx, db, keyID, p, m)
		}
	}

	log.Printf("done: %d provider(s) imported, %d skipped, %d route(s) imported", imported, skipped, routeImported)
	if imported > 0 {
		log.Print("trigger a snapshot push (restart pingogate-ctrl or call POST /api/snapshot/push) to sync to the Rust kernel")
	}
}

// importRoute inserts one model as a routes row, applying the model > provider
// override chain so the stored row carries effective values. Returns 1 on
// success, 0 if the routes table does not exist yet (skipped gracefully).
func importRoute(ctx context.Context, db *storage.DB, keyID string, p providerCfg, m modelCfg) int {
	kind := m.Kind
	if kind == "" {
		kind = p.Kind
	}
	auth := m.AuthMethod
	if auth == "" {
		auth = p.Auth.Method
	}
	path := m.UpstreamPath
	if path == "" {
		path = p.UpstreamPath
	}
	version := m.AnthropicVersion
	if version == "" {
		version = p.AnthropicVersion
	}
	timeout := m.TimeoutMs
	if timeout == nil {
		timeout = p.TimeoutMs
	}

	// Store timeout as nullable int (nil -> NULL).
	var timeoutVal any
	if timeout != nil {
		timeoutVal = *timeout
	}

	_, err := db.ExecContext(ctx,
		`INSERT OR IGNORE INTO routes
		 (alias, provider, upstream_model, upstream_path, auth_method, kind, anthropic_version, timeout_ms)
		 VALUES (?, ?, ?, ?, ?, ?, ?, ?)`,
		m.Alias, keyID, m.UpstreamModel, path, auth, kind, version, timeoutVal)
	if err != nil {
		log.Printf("skip route %q: %v (routes table may not exist yet)", m.Alias, err)
		return 0
	}
	return 1
}

// resolveKeyRef resolves an env:VAR or plain:value reference to plaintext.
func resolveKeyRef(ref string) (string, error) {
	ref = strings.TrimSpace(ref)
	if ref == "" {
		return "", fmt.Errorf("empty key_ref")
	}
	parts := strings.SplitN(ref, ":", 2)
	if len(parts) != 2 {
		return "", fmt.Errorf("unsupported key_ref format: %q", ref)
	}
	scheme, rest := parts[0], strings.TrimSpace(parts[1])
	switch scheme {
	case "env":
		val := os.Getenv(rest)
		if val == "" {
			return "", fmt.Errorf("env var %s not set or empty", rest)
		}
		return val, nil
	case "plain":
		if rest == "" {
			return "", fmt.Errorf("plain: value is empty")
		}
		return rest, nil
	default:
		return "", fmt.Errorf("unsupported scheme %q", scheme)
	}
}

// computeLast4 returns the last 4 chars (same logic as keymgmt).
func computeLast4(s string) string {
	if len(s) < 4 {
		return "****"
	}
	return s[len(s)-4:]
}

// providerKeyExists checks if a key with the given ID already exists.
func providerKeyExists(ctx context.Context, db *storage.DB, id string) (bool, error) {
	var count int
	err := db.GetContext(ctx, &count, "SELECT COUNT(*) FROM user_provider_keys WHERE id = ?", id)
	if err != nil {
		return false, err
	}
	return count > 0, nil
}

// getFirstAdmin returns the first admin user from the DB.
func getFirstAdmin(ctx context.Context, store *identity.Store) (*identity.User, error) {
	users, err := store.List(ctx)
	if err != nil {
		return nil, err
	}
	for _, u := range users {
		if u.IsAdmin {
			return &u, nil
		}
	}
	return nil, fmt.Errorf("no admin user found")
}
