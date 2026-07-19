// Package identity - HTTP handlers for User CRUD. All routes are mounted under
// /api/users and require AuthMiddleware + Authorize("manage", "users"). The
// handlers are thin: they parse the request, call Store, and map errors to HTTP
// status codes (constitution IX: HTTP semantics only at the handler boundary).
package identity

import (
	"encoding/json"
	"errors"
	"net/http"
	"strings"

	"github.com/go-chi/chi/v5"
)

// RegisterRoutes mounts the User CRUD routes on the given chi router. The
// caller is expected to have already mounted AuthMiddleware at a parent level
// (so every /api/users/* request is authenticated); RegisterRoutes adds the
// Authorize("manage", "users") check on top, so only admins reach the handlers.
//
// Routes:
//
//	POST   /api/users      - create a user (returns plaintext token once)
//	GET    /api/users      - list users (no token hashes in response)
//	GET    /api/users/{id} - get a single user
//	DELETE /api/users/{id} - delete a user
//
// The plaintext token is returned ONLY on POST, in the response body, exactly
// once. There is no way to retrieve it later (constitution XX).
func RegisterRoutes(r chi.Router, store *Store) {
	r.Route("/api/users", func(r chi.Router) {
		r.Use(Authorize("manage", "users"))
		r.Post("/", createHandler(store))
		r.Get("/", listHandler(store))
		r.Get("/{id}", getHandler(store))
		r.Delete("/{id}", deleteHandler(store))
	})
}

// createResponse is the body returned by POST /api/users. The Token field is
// the plaintext API token; it is documented as "returned once" and there is no
// GET endpoint that returns it.
type createResponse struct {
	User  User   `json:"user"`
	Token string `json:"token"`
}

// createRequest is the body accepted by POST /api/users. Email is required;
// the token is generated server-side (never client-supplied) so the operator
// can't be tricked into reusing a token they already used elsewhere.
type createRequest struct {
	Email string `json:"email"`
}

// createHandler handles POST /api/users. Generates a random plaintext token,
// stores its bcrypt hash, and returns the plaintext once. Email must be
// non-empty and unique; duplicate email returns 409, empty email returns 400.
func createHandler(store *Store) http.HandlerFunc {
	return func(w http.ResponseWriter, r *http.Request) {
		var req createRequest
		if err := json.NewDecoder(r.Body).Decode(&req); err != nil {
			writeError(w, http.StatusBadRequest, "bad_request", "invalid JSON body")
			return
		}
		req.Email = strings.TrimSpace(req.Email)
		if req.Email == "" {
			writeError(w, http.StatusBadRequest, "bad_request", "email is required")
			return
		}

		token, err := generateToken()
		if err != nil {
			writeError(w, http.StatusInternalServerError, "internal", "token generation failed")
			return
		}
		user, _, err := store.Create(r.Context(), req.Email, token)
		if err != nil {
			mapStoreError(w, err)
			return
		}
		writeJSON(w, http.StatusCreated, createResponse{User: *user, Token: token})
	}
}

// listHandler handles GET /api/users. Returns all users as a JSON array. No
// token hashes are included (User.APITokenHash is json:"-").
func listHandler(store *Store) http.HandlerFunc {
	return func(w http.ResponseWriter, r *http.Request) {
		users, err := store.List(r.Context())
		if err != nil {
			writeError(w, http.StatusInternalServerError, "internal", "list users failed")
			return
		}
		// Always emit a JSON array, even when empty (frontend-friendly).
		if users == nil {
			users = []User{}
		}
		writeJSON(w, http.StatusOK, users)
	}
}

// getHandler handles GET /api/users/{id}. 404 when the ID doesn't exist.
func getHandler(store *Store) http.HandlerFunc {
	return func(w http.ResponseWriter, r *http.Request) {
		id := chi.URLParam(r, "id")
		if id == "" {
			writeError(w, http.StatusBadRequest, "bad_request", "id is required")
			return
		}
		user, err := store.GetByID(r.Context(), id)
		if err != nil {
			mapStoreError(w, err)
			return
		}
		writeJSON(w, http.StatusOK, user)
	}
}

// deleteHandler handles DELETE /api/users/{id}. 404 when the ID doesn't exist,
// 204 on success. FOREIGN KEY violations (user owns provider keys) surface as
// 409 Conflict.
func deleteHandler(store *Store) http.HandlerFunc {
	return func(w http.ResponseWriter, r *http.Request) {
		id := chi.URLParam(r, "id")
		if id == "" {
			writeError(w, http.StatusBadRequest, "bad_request", "id is required")
			return
		}
		if err := store.Delete(r.Context(), id); err != nil {
			mapStoreError(w, err)
			return
		}
		w.WriteHeader(http.StatusNoContent)
	}
}

// mapStoreError translates a Store error into the standard REST error body
// (constitution IX). Known sentinels get specific status codes; unknown errors
// get 500. FOREIGN KEY violations (e.g., deleting a user who owns provider
// keys) are detected by substring match on the SQLite error text.
func mapStoreError(w http.ResponseWriter, err error) {
	switch {
	case errors.Is(err, ErrNotFound):
		writeError(w, http.StatusNotFound, "not_found", "user not found")
	case errors.Is(err, ErrEmailTaken):
		writeError(w, http.StatusConflict, "conflict", "email already taken")
	case strings.Contains(err.Error(), "FOREIGN KEY constraint failed"):
		writeError(w, http.StatusConflict, "conflict", "user owns resources; remove them first")
	default:
		writeError(w, http.StatusInternalServerError, "internal", "store operation failed")
	}
}

// writeJSON encodes v as JSON and writes it with the given status. JSON
// encoding failures are extremely unlikely (we control all the types); on
// failure we log via the error body so the client sees something coherent.
func writeJSON(w http.ResponseWriter, status int, v any) {
	w.Header().Set("Content-Type", "application/json")
	w.WriteHeader(status)
	if err := json.NewEncoder(w).Encode(v); err != nil {
		// Best-effort; the status and Content-Type are already sent.
		_ = err
	}
}
