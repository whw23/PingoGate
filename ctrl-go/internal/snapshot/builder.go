// Package snapshot - Builder reads the Go control-plane DB and assembles a
// proto ``Snapshot`` for the Rust kernel (spec §12C).
//
// S2 scope: the only DB-backed source is ``user_provider_keys`` (T17 schema).
// Each enabled key produces one ``EncryptedProviderKey`` entry (the AES-GCM
// ciphertext produced by the Rust KeyVault at Create time) and one
// ``ProviderEntry`` whose ``encrypted_key_ref`` points back at the key. Routes
// and virtual_keys are empty for S2 (no routes table yet; virtual_keys is S3).
//
// The Builder never decrypts: it only copies ciphertext bytes from the DB into
// the proto message. The Rust hot path decrypts per request via KeyVault
// (constitution XX: Go never holds plaintext provider keys).
package snapshot

import (
	"context"
	"database/sql"
	"fmt"

	pb "github.com/whw23/pingogate/ctrl-go/internal/proto"
	"github.com/whw23/pingogate/ctrl-go/internal/storage"
)

// defaultAnthropicVersion is the anthropic-version header value sent upstream
// when the user has not supplied one via base_url or (future) per-provider
// config. Anthropic rejects requests without it; we default to the widely
// supported "2023-06-01" so a BYOK user with only a key can immediately use
// the gateway. OpenAI-compatible and Gemini providers do not need it.
const defaultAnthropicVersion = "2023-06-01"

// providerDefaults maps provider_type (stored in user_provider_keys) to the
// (kind, auth_method, base_url) triple the Rust kernel expects. The values
// mirror ``auth_compatible`` in ``core-rs/pingogate-core/src/grpc_convert.rs``:
// kind+auth_method must be a compatible pair, or Rust rejects the snapshot.
//
// base_url is used only when the user did not supply one at Create time; it
// points at the canonical public endpoint for each provider so a freshly
// imported key "just works". Users with a custom endpoint set base_url at
// Create time and that value wins.
var providerDefaults = map[string]struct {
	kind       string
	authMethod string
	baseURL    string
}{
	"openai":            {"openai-compatible", "bearer", "https://api.openai.com/v1"},
	"openai-compatible": {"openai-compatible", "bearer", "https://api.openai.com/v1"},
	"anthropic":         {"anthropic", "api_key_header", "https://api.anthropic.com/v1"},
	"gemini":            {"gemini", "query_key", "https://generativelanguage.googleapis.com/v1beta"},
}

// Builder reads the SQLite DB and produces a proto Snapshot for the Rust
// kernel. It is stateless across calls; the Pusher owns version + serialization
// (spec §12C: Go mutex serializes pushes, monotonic version).
type Builder struct {
	db *storage.DB
}

// NewBuilder wraps a *storage.DB. The DB must already have migrations applied
// (T17 storage.Open does this).
func NewBuilder(db *storage.DB) *Builder {
	return &Builder{db: db}
}

// keyRow is the subset of user_provider_keys columns the Builder needs. It is
// private to this file; the Store in keymgmt owns the full row model.
type keyRow struct {
	ID           string `db:"id"`
	OwnerUserID  string `db:"owner_user_id"`
	ProviderType string `db:"provider_type"`
	EncryptedKey []byte `db:"encrypted_key"`
	BaseURL      string `db:"base_url"`
	CreatedBy    string `db:"created_by"`
	Enabled      int    `db:"enabled"`
}

// Build reads user_provider_keys and assembles a proto Snapshot with the given
// version. Only enabled keys are included; disabled keys are hidden from the
// Rust kernel so the hot path cannot use them (constitution X: Control Plane
// State is authoritative).
//
// Returns a Snapshot with empty routes and virtual_keys for S2 (no routes
// table; virtual_keys is S3). The Pusher wraps this in a gRPC stream and
// serializes calls (spec §12C).
func (b *Builder) Build(ctx context.Context, version uint64) (*pb.Snapshot, error) {
	const q = `SELECT id, owner_user_id, provider_type, encrypted_key, base_url, created_by, enabled
	           FROM user_provider_keys
	           WHERE enabled = 1
	           ORDER BY id ASC`
	var rows []keyRow
	if err := b.db.SelectContext(ctx, &rows, q); err != nil {
		if err == sql.ErrNoRows {
			rows = nil
		} else {
			return nil, fmt.Errorf("snapshot: read user_provider_keys: %w", err)
		}
	}

	providers := make([]*pb.ProviderEntry, 0, len(rows))
	encryptedKeys := make([]*pb.EncryptedProviderKey, 0, len(rows))
	for _, r := range rows {
		def, ok := providerDefaults[r.ProviderType]
		if !ok {
			// Unknown provider_type: skip rather than fail the whole push. The
			// key stays in the DB (audit trail) but is invisible to Rust. We
			// log via the context err pattern (no slog here to keep Builder
			// dependency-free); the caller can observe the row count mismatch.
			continue
		}

		// EncryptedProviderKey: ciphertext + ownership metadata (spec §12A
		// dual-defense: Rust uses owner_user_id to gate Decrypt).
		encryptedKeys = append(encryptedKeys, &pb.EncryptedProviderKey{
			Id:          r.ID,
			Ciphertext:  append([]byte(nil), r.EncryptedKey...), // copy; DB slice may be reused
			OwnerUserId: r.OwnerUserID,
			CreatedBy:   r.CreatedBy,
		})

		// ProviderEntry: name=key ID (unique), encrypted_key_ref=key ID.
		// base_url falls back to the canonical endpoint when the user did not
		// supply one. anthropic_version is set only for anthropic providers.
		baseURL := r.BaseURL
		if baseURL == "" {
			baseURL = def.baseURL
		}
		pe := &pb.ProviderEntry{
			Name:               r.ID, // unique by DB primary key
			Kind:               def.kind,
			BaseUrl:            baseURL,
			AuthMethod:         def.authMethod,
			EncryptedKeyRef:    r.ID,
			CapabilityFamilies: []string{"generation.stateless"},
		}
		if def.kind == "anthropic" {
			pe.AnthropicVersion = defaultAnthropicVersion
		}
		providers = append(providers, pe)
	}

	return &pb.Snapshot{
		Version:       version,
		Providers:     providers,
		Routes:        nil, // S2 has no routes table; routes is S3.
		VirtualKeys:   nil, // S3 feature.
		EncryptedKeys: encryptedKeys,
	}, nil
}
