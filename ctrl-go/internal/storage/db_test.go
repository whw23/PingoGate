// Package storage - tests for Open + migrate (T17). Opens an in-memory SQLite
// DB and verifies both migrations apply cleanly and the expected tables exist.
package storage

import "testing"

// TestOpenAndMigrate opens an in-memory SQLite DB via Open and verifies that
// the users table (migration 001) is created. Sub-tests cover the
// user_provider_keys table (migration 002), the index, and migration
// idempotency so a failure clearly identifies which migration broke.
func TestOpenAndMigrate(t *testing.T) {
	db, err := Open(":memory:")
	if err != nil {
		t.Fatalf("Open: %v", err)
	}
	defer db.Close()

	t.Run("users_table_exists", func(t *testing.T) {
		var name string
		err := db.Get(&name,
			"SELECT name FROM sqlite_master WHERE type='table' AND name='users'")
		if err != nil {
			t.Fatalf("users table not created: %v", err)
		}
		if name != "users" {
			t.Fatalf("got name=%q, want %q", name, "users")
		}
	})

	t.Run("user_provider_keys_table_exists", func(t *testing.T) {
		var name string
		err := db.Get(&name,
			"SELECT name FROM sqlite_master WHERE type='table' AND name='user_provider_keys'")
		if err != nil {
			t.Fatalf("user_provider_keys table not created: %v", err)
		}
		if name != "user_provider_keys" {
			t.Fatalf("got name=%q, want %q", name, "user_provider_keys")
		}
	})

	t.Run("migrations_are_idempotent", func(t *testing.T) {
		// Re-running migrate on the same DB must be a no-op: no error and no
		// duplicate rows in schema_migrations.
		if err := migrate(db.DB); err != nil {
			t.Fatalf("re-run migrate: %v", err)
		}
		var count int
		if err := db.Get(&count, "SELECT COUNT(*) FROM schema_migrations"); err != nil {
			t.Fatalf("count schema_migrations: %v", err)
		}
		if want := 2; count != want {
			t.Fatalf("schema_migrations count = %d, want %d", count, want)
		}
	})

	t.Run("user_provider_keys_index_exists", func(t *testing.T) {
		var name string
		err := db.Get(&name,
			"SELECT name FROM sqlite_master WHERE type='index' AND name='idx_provider_keys_owner'")
		if err != nil {
			t.Fatalf("idx_provider_keys_owner not created: %v", err)
		}
		if name != "idx_provider_keys_owner" {
			t.Fatalf("got index name=%q, want %q", name, "idx_provider_keys_owner")
		}
	})
}
