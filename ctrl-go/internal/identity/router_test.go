// Package identity - shared test router builder. Kept in a separate _test.go
// file so the handler_test.go stays focused on scenarios. newTestRouter mounts
// AuthMiddleware + RegisterRoutes the same way the real control plane will.
package identity

import (
	"net/http"

	"github.com/go-chi/chi/v5"
)

// newTestRouter builds a chi router with AuthMiddleware at the root and the
// User CRUD routes mounted via RegisterRoutes. This mirrors the wiring the
// real control plane will use; tests exercise the full middleware stack so
// that 401/403/404/409 paths are covered exactly as production would see them.
func newTestRouter(store *Store) http.Handler {
	r := chi.NewRouter()
	r.Use(AuthMiddleware(store))
	RegisterRoutes(r, store)
	return r
}
