// Package identity - HTTP middleware: AuthMiddleware (Bearer token -> User) and
// Authorize (RBAC boundary). Together they implement the constitution XX
// "Principal / authorize" boundary for all control-plane admin endpoints: no
// request reaches a handler without an authenticated User, and no privileged
// action happens without an authorize check. There is no global-admin-token
// shortcut (constitution XX: "不存在全局 admin token 等值判断硬编码鉴权").
package identity

import (
	"context"
	"encoding/json"
	"net/http"
	"strings"
)

// contextKey is an unexported type so no other package can collide on the
// context key namespace (idiomatic Go pattern; context keys must be
// package-private to avoid clashes).
type contextKey int

const userContextKey contextKey = iota

// UserFromContext returns the User injected by AuthMiddleware, or nil if no
// user is present (e.g., the route is unauthenticated). Authorize treats nil
// as "unauthenticated" and returns 401, not 403.
func UserFromContext(ctx context.Context) *User {
	v, _ := ctx.Value(userContextKey).(*User)
	return v
}

// withUser returns a new context carrying the given User. Exposed for tests
// that want to inject a User without running AuthMiddleware.
func withUser(ctx context.Context, u *User) context.Context {
	return context.WithValue(ctx, userContextKey, u)
}

// AuthMiddleware extracts an Authorization: Bearer <token> header, verifies the
// token against the stored bcrypt hashes via Store.VerifyToken, and injects the
// matched User into the request context. Requests without a valid Bearer token
// get 401 with a JSON error body (constitution IX: L0-L2 standard REST errors).
//
// On success the wrapped handler sees the User via UserFromContext. Failure
// modes: missing header (401), wrong scheme (401), empty token (401), no match
// (401). We deliberately return 401 (not 403) for "wrong token" because the
// caller is unauthenticated, not authenticated-but-unauthorized.
func AuthMiddleware(store *Store) func(http.Handler) http.Handler {
	return func(next http.Handler) http.Handler {
		return http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
			token, ok := extractBearer(r)
			if !ok {
				writeError(w, http.StatusUnauthorized, "unauthorized", "missing or malformed Authorization header")
				return
			}
			user, err := store.VerifyToken(r.Context(), token)
			if err != nil {
				// ErrNotFound or DB error: both are 401 from the caller's POV.
				// We don't leak whether the DB is up; just "unauthorized".
				writeError(w, http.StatusUnauthorized, "unauthorized", "invalid or revoked token")
				return
			}
			next.ServeHTTP(w, r.WithContext(withUser(r.Context(), user)))
		})
	}
}

// Authorize returns a middleware that checks the authenticated User can perform
// the given action on the given resource. It must run after AuthMiddleware
// (it reads the User from context). If no User is in context, it returns 401
// (caller didn't chain AuthMiddleware); if the User is present but lacks the
// permission, it returns 403.
//
// L0 has a flat permission model: admins can do everything, non-admins can do
// nothing privileged. The (action, resource) pair is accepted but currently
// only the admin flag matters; L4 multi-tenant RBAC will give the pair meaning.
// The signature is fixed now so that handlers don't need to change when L4
// lands (constitution II: reserve the position, don't build the framework).
func Authorize(action, resource string) func(http.Handler) http.Handler {
	return func(next http.Handler) http.Handler {
		return http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
			user := UserFromContext(r.Context())
			if user == nil {
				writeError(w, http.StatusUnauthorized, "unauthorized", "authentication required")
				return
			}
			if !canPerform(user, action, resource) {
				writeError(w, http.StatusForbidden, "forbidden",
					"user lacks permission for "+action+" on "+resource)
				return
			}
			next.ServeHTTP(w, r)
		})
	}
}

// canPerform is the single source of truth for the authorize boundary. L0
// rule: admin can do anything; non-admin can do nothing. L4 will replace this
// body with an RBAC lookup; the function signature is the stable contract.
//
// The action/resource parameters are intentionally unused at L0 (lint: the
// unused parameters document the future contract). L4 will consult them.
func canPerform(user *User, action, resource string) bool {
	_ = action
	_ = resource
	return user.IsAdmin
}

// extractBearer pulls the token out of "Authorization: Bearer <token>". Returns
// (token, true) on success, ("", false) when the header is missing, uses a
// non-Bearer scheme, or has an empty token. Scheme match is case-insensitive
// per RFC 7235; the token itself is case-sensitive.
func extractBearer(r *http.Request) (string, bool) {
	h := r.Header.Get("Authorization")
	if h == "" {
		return "", false
	}
	// Split on the first space: scheme + token.
	idx := strings.IndexByte(h, ' ')
	if idx <= 0 {
		return "", false
	}
	scheme := h[:idx]
	if !strings.EqualFold(scheme, "Bearer") {
		return "", false
	}
	token := strings.TrimSpace(h[idx+1:])
	if token == "" {
		return "", false
	}
	return token, true
}

// errorBody is the standard REST error envelope (constitution IX: L0-L2 control
// plane returns error code + message + optional detail).
type errorBody struct {
	Code   string `json:"code"`
	Message string `json:"message"`
	Detail string `json:"detail,omitempty"`
}

// writeError emits a JSON error response. Body shape matches constitution IX
// (L0-L2 control plane: error code + message + optional detail).
func writeError(w http.ResponseWriter, status int, code, msg string) {
	w.Header().Set("Content-Type", "application/json")
	w.WriteHeader(status)
	_ = json.NewEncoder(w).Encode(errorBody{Code: code, Message: msg})
}
