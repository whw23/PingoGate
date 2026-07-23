// Package usage - DB persistence for usage events (constitution XIX; spec
// SC-10). The Store inserts UsageEvent rows into the `usage` table
// (migration 005). It is the only writer for that table; the Rust kernel
// never touches the DB (constitution X: Rust zero DB).
//
// All columns are stored verbatim from the proto UsageEvent. `estimated`
// mirrors the event needs_estimate flag: 1 = Go filled in tokens via
// tiktoken-go, 0 = provider-supplied usage. The Store does not interpret or
// recompute tokens; it persists what the caller hands it.
package usage

import (
	"context"
	"fmt"
	"time"

	pb "github.com/whw23/pingogate/ctrl-go/internal/proto"
	"github.com/whw23/pingogate/ctrl-go/internal/storage"
)

// Store wraps a *storage.DB for usage inserts. It is safe for concurrent
// use (sqlx/sqlite3 handle their own locking). Construction injects the
// *storage.DB (constitution VII); the Store has no knowledge of gRPC or
// the Rust kernel.
type Store struct {
	db *storage.DB
}

// NewStore wraps a *storage.DB. The DB must already have migrations
// applied (migration 005 creates the usage table).
func NewStore(db *storage.DB) *Store {
	return &Store{db: db}
}

// Insert persists one UsageEvent. The event fields are mapped 1:1 to the
// usage table columns; estimated is set to 1 when needs_estimate was true
// (tokens were filled in by the Go estimator), 0 otherwise. created_at is
// set server-side to UTC RFC3339 so events are always timestamped even if
// the Rust kernel clock drifts.
//
// Returns the rowid of the inserted row on success.
func (s *Store) Insert(ctx context.Context, event *pb.UsageEvent) (int64, error) {
	if event == nil {
		return 0, fmt.Errorf("usage: insert nil event")
	}
	estimated := 0
	if event.NeedsEstimate {
		estimated = 1
	}
	success := 0
	if event.Success {
		success = 1
	}
	createdAt := time.Now().UTC().Format(time.RFC3339)

	q := `INSERT INTO usage
      (virtual_key_id, owner_user_id, provider, model,
       input_tokens, output_tokens, reasoning_tokens,
       cache_read_tokens, cache_write_tokens,
       success, latency_ms, estimated, created_at)
      VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)`

	res, err := s.db.ExecContext(ctx, q,
		nullableString(event.VirtualKeyId),
		event.OwnerUserId,
		event.Provider,
		event.Model,
		event.InputTokens,
		event.OutputTokens,
		event.ReasoningTokens,
		event.CacheReadTokens,
		event.CacheWriteTokens,
		success,
		event.LatencyMs,
		estimated,
		createdAt,
	)
	if err != nil {
		return 0, fmt.Errorf("usage: insert: %w", err)
	}
	rowID, err := res.LastInsertId()
	if err != nil {
		return 0, fmt.Errorf("usage: last insert id: %w", err)
	}
	return rowID, nil
}

// CountByOwner returns the number of usage rows for the given owner. Used
// by tests and the L6 console (per-user usage view). Not on the hot path.
func (s *Store) CountByOwner(ctx context.Context, ownerUserID string) (int64, error) {
	var count int64
	err := s.db.GetContext(ctx, &count,
		"SELECT COUNT(*) FROM usage WHERE owner_user_id = ?", ownerUserID)
	if err != nil {
		return 0, fmt.Errorf("usage: count by owner: %w", err)
	}
	return count, nil
}

// nullableString returns "" as NULL so the virtual_key_id column is NULL
// for standalone-mode events (no virtual key presented). SQLite treats an
// empty string as a real value, which would pollute per-vkey aggregations.
func nullableString(s string) any {
	if s == "" {
		return nil
	}
	return s
}
