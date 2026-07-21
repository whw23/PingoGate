// Package keymgmt - HTTP handlers for UserProviderKey CRUD. All routes are
// mounted under /api/provider-keys and require AuthMiddleware (T18) +
// Authorize("manage", "provider_keys"). The handlers are thin: they parse
// the request, call Store / KeyVaultClient, and map errors to HTTP status
// codes (constitution IX: HTTP semantics only at the handler boundary).
//
// Create flow (constitution XX "1Password model"):
//  1. User authenticated via AuthMiddleware (T18); user in context.
//  2. Handler reads plaintext_key from the request body (one-time view).
//  3. KeyVaultClient.Encrypt(plaintext) -> ciphertext (gRPC to Rust, T15).
//  4. Store.Create(encrypted_key=ciphertext, key_last4=last4(plaintext),
//     created_by=user.ID, owner_user_id=user.ID).
//  5. Return key metadata (NO plaintext, NO ciphertext).
//
// List/Get flow: return key_last4 (last 4 chars) + metadata, never
// ciphertext, never plaintext. The plaintext is never decrypted on the
// read path (S3 adds a reveal endpoint restricted to created_by).
package keymgmt

import (
	"encoding/json"
	"errors"
	"net/http"
	"strings"

	"github.com/go-chi/chi/v5"

	"github.com/whw23/pingogate/ctrl-go/internal/identity"
)

// RegisterRoutes mounts the UserProviderKey CRUD routes on the given chi
// router. The caller is expected to have already mounted AuthMiddleware at
// a parent level; RegisterRoutes adds the Authorize check on top, so only
// admins reach the handlers in L0 (L4 will relax this to per-owner).
func RegisterRoutes(r chi.Router, store *Store, kv KeyVaultClient) {
	r.Route("/api/provider-keys", func(r chi.Router) {
		r.Use(identity.Authorize("manage", "provider_keys"))
		r.Post("/", createHandler(store, kv))
		r.Get("/", listHandler(store))
		r.Get("/{id}", getHandler(store))
		r.Delete("/{id}", deleteHandler(store))
	})
}

// createRequest is the body accepted by POST /api/provider-keys.
// PlaintextKey is the raw provider key; it is consumed once at Create time
// and never persisted. ProviderType is one of openai/anthropic/gemini/
// openai-compatible. BaseURL is optional.
type createRequest struct {
	ProviderType string `json:"provider_type"`
	PlaintextKey string `json:"plaintext_key"`
	BaseURL      string `json:"base_url,omitempty"`
}

// createResponse is the body returned by POST /api/provider-keys. It
// contains NO plaintext and NO ciphertext (EncryptedKey is json:"-").
type createResponse struct {
	UserProviderKey
}

// createHandler handles POST /api/provider-keys. Reads plaintext_key from
// the body, calls KeyVaultClient.Encrypt, stores the ciphertext + key_last4,
// and returns the key metadata. The plaintext is consumed in this function
// scope only; it is not logged and not persisted (constitution XX).
func createHandler(store *Store, kv KeyVaultClient) http.HandlerFunc {
	return func(w http.ResponseWriter, r *http.Request) {
		user := identity.UserFromContext(r.Context())
		if user == nil {
			writeError(w, http.StatusUnauthorized, "unauthorized", "authentication required")
			return
		}

		var req createRequest
		if err := json.NewDecoder(r.Body).Decode(&req); err != nil {
			writeError(w, http.StatusBadRequest, "bad_request", "invalid JSON body")
			return
		}
		req.ProviderType = strings.TrimSpace(req.ProviderType)
		req.PlaintextKey = strings.TrimSpace(req.PlaintextKey)
		if req.ProviderType == "" {
			writeError(w, http.StatusBadRequest, "bad_request", "provider_type is required")
			return
		}
		if req.PlaintextKey == "" {
			writeError(w, http.StatusBadRequest, "bad_request", "plaintext_key is required")
			return
		}

		// Encrypt the plaintext via the Rust KeyVault (gRPC). The plaintext
		// lives only in this function stack; the returned ciphertext is
		// what gets persisted.
		ciphertext, err := kv.Encrypt(r.Context(), []byte(req.PlaintextKey))
		if err != nil {
			writeError(w, http.StatusBadGateway, "keyvault_error", "failed to encrypt provider key")
			return
		}

		// Capture last4 ONCE, here, where the plaintext is in memory.
		last4 := computeLast4(req.PlaintextKey)

		pk, err := store.Create(r.Context(),
			user.ID, req.ProviderType, ciphertext, last4, req.BaseURL, user.ID)
		if err != nil {
			mapStoreError(w, err)
			return
		}

		writeJSON(w, http.StatusCreated, createResponse{UserProviderKey: *pk})
	}
}

// listResponse is the body returned by GET /api/provider-keys. It is a
// JSON array of key metadata; each entry includes key_last4 but NEVER
// encrypted_key (EncryptedKey is json:"-").
type listResponse []UserProviderKey

// listHandler handles GET /api/provider-keys. Returns the caller keys
// with key_last4 for display; never returns ciphertext or plaintext.
func listHandler(store *Store) http.HandlerFunc {
	return func(w http.ResponseWriter, r *http.Request) {
		user := identity.UserFromContext(r.Context())
		if user == nil {
			writeError(w, http.StatusUnauthorized, "unauthorized", "authentication required")
			return
		}
		keys, err := store.ListByOwner(r.Context(), user.ID)
		if err != nil {
			writeError(w, http.StatusInternalServerError, "internal", "list provider keys failed")
			return
		}
		if keys == nil {
			keys = []UserProviderKey{}
		}
		writeJSON(w, http.StatusOK, listResponse(keys))
	}
}

// getHandler handles GET /api/provider-keys/{id}. Returns key_last4 +
// metadata; never returns ciphertext or plaintext. 404 when not found.
func getHandler(store *Store) http.HandlerFunc {
	return func(w http.ResponseWriter, r *http.Request) {
		user := identity.UserFromContext(r.Context())
		if user == nil {
			writeError(w, http.StatusUnauthorized, "unauthorized", "authentication required")
			return
		}
		id := chi.URLParam(r, "id")
		if id == "" {
			writeError(w, http.StatusBadRequest, "bad_request", "id is required")
			return
		}
		pk, err := store.GetByID(r.Context(), id)
		if err != nil {
			mapStoreError(w, err)
			return
		}
		// Enforce ownership: a user can only fetch their own keys.
		if pk.OwnerUserID != user.ID {
			writeError(w, http.StatusNotFound, "not_found", "provider key not found")
			return
		}
		writeJSON(w, http.StatusOK, createResponse{UserProviderKey: *pk})
	}
}

// deleteHandler handles DELETE /api/provider-keys/{id}. 404 when not
// found, 204 on success. Enforces ownership (owner or admin).
func deleteHandler(store *Store) http.HandlerFunc {
	return func(w http.ResponseWriter, r *http.Request) {
		user := identity.UserFromContext(r.Context())
		if user == nil {
			writeError(w, http.StatusUnauthorized, "unauthorized", "authentication required")
			return
		}
		id := chi.URLParam(r, "id")
		if id == "" {
			writeError(w, http.StatusBadRequest, "bad_request", "id is required")
			return
		}
		pk, err := store.GetByID(r.Context(), id)
		if err != nil {
			mapStoreError(w, err)
			return
		}
		if pk.OwnerUserID != user.ID {
			writeError(w, http.StatusNotFound, "not_found", "provider key not found")
			return
		}
		if err := store.Delete(r.Context(), id); err != nil {
			mapStoreError(w, err)
			return
		}
		w.WriteHeader(http.StatusNoContent)
	}
}

// computeLast4 returns the last 4 characters of s. For keys shorter than
// 4 characters, returns the whole string. Used at Create time to capture
// a non-sensitive preview for List/Get display (constitution XX).
func computeLast4(s string) string {
	if len(s) <= 4 {
		return s
	}
	return s[len(s)-4:]
}

// errorBody is the standard REST error envelope (constitution IX).
type errorBody struct {
	Code    string `json:"code"`
	Message string `json:"message"`
	Detail  string `json:"detail,omitempty"`
}

// writeError emits a JSON error response.
func writeError(w http.ResponseWriter, status int, code, msg string) {
	w.Header().Set("Content-Type", "application/json")
	w.WriteHeader(status)
	_ = json.NewEncoder(w).Encode(errorBody{Code: code, Message: msg})
}

// writeJSON encodes v as JSON and writes it with the given status.
func writeJSON(w http.ResponseWriter, status int, v any) {
	w.Header().Set("Content-Type", "application/json")
	w.WriteHeader(status)
	if err := json.NewEncoder(w).Encode(v); err != nil {
		_ = err
	}
}

// mapStoreError translates a Store error into the standard REST error
// body (constitution IX). Known sentinels get specific status codes.
func mapStoreError(w http.ResponseWriter, err error) {
	switch {
	case errors.Is(err, ErrNotFound):
		writeError(w, http.StatusNotFound, "not_found", "provider key not found")
	default:
		writeError(w, http.StatusInternalServerError, "internal", "store operation failed")
	}
}
