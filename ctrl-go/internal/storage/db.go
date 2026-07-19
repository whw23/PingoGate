// Package storage provides the Go control plane's SQLite database layer
// (constitution: Go non-kernel = all state, Rust kernel zero DB access). S2 adds the
// DB connection (sqlx + SQLite) and the first two schema migrations:
// users (L0 identity) and user_provider_keys (L1 BYOK). The DB lives only in
// the Go binary; the Rust kernel never touches it.
package storage

import (
	"database/sql"
	"embed"
	"errors"
	"fmt"
	"io/fs"
	"strings"

	"github.com/jmoiron/sqlx"
	_ "github.com/mattn/go-sqlite3" // register the "sqlite3" driver
)

// migrationsFS embeds the migrations/*.sql files at compile time so the binary
// is self-contained (no external migration directory needed at runtime).
//
//go:embed migrations/*.sql
var migrationsFS embed.FS

// DB wraps a sqlx.DB connection with applied schema migrations.
type DB struct {
	*sqlx.DB
}

// Open connects to the SQLite database at dsn, applies any pending migrations
// from the embedded migrations/ directory, and returns a wrapped *DB. Use
// ":memory:" for an ephemeral in-process DB (tests) or a file path for
// persistent storage. On migration failure the connection is closed.
func Open(dsn string) (*DB, error) {
	db, err := sqlx.Connect("sqlite3", dsn)
	if err != nil {
		return nil, fmt.Errorf("storage: connect sqlite: %w", err)
	}
	if err := migrate(db); err != nil {
		_ = db.Close()
		return nil, fmt.Errorf("storage: migrate: %w", err)
	}
	return &DB{db}, nil
}

// schemaMigrationsDDL is the DDL for the bookkeeping table that records which
// migration versions have been applied. Kept inline (not as a migration file)
// because it must exist before migrations are enumerated.
const schemaMigrationsDDL = `CREATE TABLE IF NOT EXISTS schema_migrations (
    version    TEXT PRIMARY KEY,
    applied_at TEXT NOT NULL DEFAULT (datetime('now'))
)`

// migrate applies all pending *.up.sql files from the embedded migrations/
// directory in lexical order (001_ before 002_). Each migration runs in its
// own transaction; applied versions are recorded in schema_migrations to make
// the operation idempotent. This is the 'sqlx migrate pattern' (R9 decision):
// run migrations through sqlx itself, without a separate migrate library.
func migrate(db *sqlx.DB) error {
	if _, err := db.Exec(schemaMigrationsDDL); err != nil {
		return fmt.Errorf("create schema_migrations: %w", err)
	}

	entries, err := fs.ReadDir(migrationsFS, "migrations")
	if err != nil {
		return fmt.Errorf("read migrations dir: %w", err)
	}

	for _, entry := range entries {
		name := entry.Name()
		if !strings.HasSuffix(name, ".up.sql") {
			continue
		}

		var applied string
		err := db.Get(&applied, "SELECT version FROM schema_migrations WHERE version = ?", name)
		if err == nil {
			continue // already applied
		}
		if !errors.Is(err, sql.ErrNoRows) {
			return fmt.Errorf("check migration %s: %w", name, err)
		}

		content, err := migrationsFS.ReadFile("migrations/" + name)
		if err != nil {
			return fmt.Errorf("read migration %s: %w", name, err)
		}

		tx, err := db.Beginx()
		if err != nil {
			return fmt.Errorf("begin tx for %s: %w", name, err)
		}
		if _, err := tx.Exec(string(content)); err != nil {
			_ = tx.Rollback()
			return fmt.Errorf("apply %s: %w", name, err)
		}
		if _, err := tx.Exec("INSERT INTO schema_migrations (version) VALUES (?)", name); err != nil {
			_ = tx.Rollback()
			return fmt.Errorf("record %s: %w", name, err)
		}
		if err := tx.Commit(); err != nil {
			return fmt.Errorf("commit %s: %w", name, err)
		}
	}
	return nil
}
