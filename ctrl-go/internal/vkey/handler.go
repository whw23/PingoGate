// Package vkey - HTTP handlers for VirtualKey Issue/Revoke/List.
package vkey

import (
	"context"
	"encoding/json"
	"errors"
	"log"
	"net/http"
	"strings"

	"github.com/go-chi/chi/v5"

	"github.com/whw23/pingogate/ctrl-go/internal/identity"
)

// SnapshotPusher is the focused interface for pushing snapshots.
type SnapshotPusher interface {
	Push(ctx context.Context) error
}

// RegisterRoutes mounts the VirtualKey routes.
func RegisterRoutes(r chi.Router, store *Store, pusher SnapshotPusher) {
	r.Route("/api/virtual-keys", func(r chi.Router) {
		r.Use(identity.Authorize("manage", "virtual_keys"))
		r.Post("/", issueHandler(store, pusher))
		r.Get("/", listHandler(store))
		r.Delete("/{id}", revokeHandler(store, pusher))
	})
}

type issueRequest struct {
	ProviderKeyID    string   `json:"provider_key_id,omitempty"`
	AllowedModels    []string `json:"allowed_models,omitempty"`
	AllowedProviders []string `json:"allowed_providers,omitempty"`
	ExpiresAt        int64    `json:"expires_at,omitempty"`
	MaxConcurrency   int32    `json:"max_concurrency,omitempty"`
}

type issueResponse struct {
	VirtualKey
	Token string `json:"token"`
}

func issueHandler(store *Store, pusher SnapshotPusher) http.HandlerFunc {
	return func(w http.ResponseWriter, r *http.Request) {
		user := identity.UserFromContext(r.Context())
		if user == nil {
			writeError(w, http.StatusUnauthorized, "unauthorized", "authentication required")
			return
		}
		var req issueRequest
		if err := json.NewDecoder(r.Body).Decode(&req); err != nil {
			writeError(w, http.StatusBadRequest, "bad_request", "invalid JSON body")
			return
		}
		req.ProviderKeyID = strings.TrimSpace(req.ProviderKeyID)
		scope := Scope{
			ProviderKeyID:    req.ProviderKeyID,
			AllowedModels:    req.AllowedModels,
			AllowedProviders: req.AllowedProviders,
			ExpiresAt:        req.ExpiresAt,
			MaxConcurrency:   req.MaxConcurrency,
		}
		plaintext, vk, err := store.Issue(r.Context(), user.ID, scope)
		if err != nil {
			writeError(w, http.StatusInternalServerError, "internal", "issue virtual key failed")
			return
		}
		if pusher != nil {
			if err := pusher.Push(r.Context()); err != nil {
				log.Printf("vkey: snapshot push after Issue %q failed: %v", vk.ID, err)
			}
		}
		writeJSON(w, http.StatusCreated, issueResponse{VirtualKey: *vk, Token: plaintext})
	}
}

type listResponse []VirtualKey

func listHandler(store *Store) http.HandlerFunc {
	return func(w http.ResponseWriter, r *http.Request) {
		user := identity.UserFromContext(r.Context())
		if user == nil {
			writeError(w, http.StatusUnauthorized, "unauthorized", "authentication required")
			return
		}
		keys, err := store.ListByOwner(r.Context(), user.ID)
		if err != nil {
			writeError(w, http.StatusInternalServerError, "internal", "list virtual keys failed")
			return
		}
		if keys == nil {
			keys = []VirtualKey{}
		}
		writeJSON(w, http.StatusOK, listResponse(keys))
	}
}

func revokeHandler(store *Store, pusher SnapshotPusher) http.HandlerFunc {
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
		// Ownership check (review Important #3): only the owner can revoke.
		// Returns 404 (not 403) to avoid leaking key existence, consistent
		// with keymgmt's pattern.
		vk, err := store.GetByID(r.Context(), id)
		if err != nil {
			mapStoreError(w, err)
			return
		}
		if vk.OwnerUserID != user.ID {
			writeError(w, http.StatusNotFound, "not_found", "virtual key not found")
			return
		}
		if err := store.Revoke(r.Context(), id); err != nil {
			mapStoreError(w, err)
			return
		}
		if pusher != nil {
			if err := pusher.Push(r.Context()); err != nil {
				log.Printf("vkey: snapshot push after Revoke %q failed: %v", id, err)
			}
		}
		w.WriteHeader(http.StatusNoContent)
	}
}

type errorBody struct {
	Code    string `json:"code"`
	Message string `json:"message"`
	Detail  string `json:"detail,omitempty"`
}

func writeError(w http.ResponseWriter, status int, code, msg string) {
	w.Header().Set("Content-Type", "application/json")
	w.WriteHeader(status)
	_ = json.NewEncoder(w).Encode(errorBody{Code: code, Message: msg})
}

func writeJSON(w http.ResponseWriter, status int, v any) {
	w.Header().Set("Content-Type", "application/json")
	w.WriteHeader(status)
	if err := json.NewEncoder(w).Encode(v); err != nil {
		_ = err
	}
}

func mapStoreError(w http.ResponseWriter, err error) {
	switch {
	case errors.Is(err, ErrNotFound):
		writeError(w, http.StatusNotFound, "not_found", "virtual key not found")
	default:
		writeError(w, http.StatusInternalServerError, "internal", "store operation failed")
	}
}
