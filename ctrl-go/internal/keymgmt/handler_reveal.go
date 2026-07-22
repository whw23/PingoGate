// Package keymgmt - Reveal handler for the 1Password "view plaintext once"
// endpoint (constitution XX). Extracted from handler.go to keep that file
// under constitution V's 300-line limit.
//
// Dual-defense (spec §12A):
//
//  1. Go L1 (this handler): `pk.CreatedBy == user.ID`. Rejects with 404 if the
//     requester is not the key's creator. This uses the DB's latest created_by
//     value (the authoritative source for "who imported this key").
//  2. Rust L2 (KeyVault.Decrypt): `requester == snapshot.key_owners[key_id]`.
//     Rust independently holds the owner mapping (pushed via gRPC snapshot) and
//     REJECTS (not just audit) if the requester is not the owner. Go cannot
//     bypass this by crafting the request (AWS KMS pattern).
//
// Only if BOTH checks pass is the plaintext returned. The plaintext lives only
// in the HTTP response body; it is not logged and not persisted.
package keymgmt

import (
	"log"
	"net/http"
	"strings"

	"github.com/go-chi/chi/v5"

	"github.com/whw23/pingogate/ctrl-go/internal/identity"
)

// revealResponse is the body returned by GET /api/provider-keys/{id}/reveal.
// Plaintext is returned ONCE; the caller is expected to discard it after use.
// The response carries no ciphertext, no metadata beyond what's needed for the
// caller to confirm which key was revealed.
type revealResponse struct {
	PlaintextKey string `json:"plaintext_key"`
	KeyID        string `json:"key_id"`
}

// revealHandler handles GET /api/provider-keys/{id}/reveal.
//
// Error mapping:
//   - 401: no authenticated user (AuthMiddleware didn't run)
//   - 404: key not found in DB (non-creators get 404, not 403, to avoid
//     leaking the existence of keys they don't own - matches getHandler)
//   - 403: Rust KeyVault returned permission_denied (L2 check failed; this
//     should not happen in normal operation because L1 already checked, but
//     the dual-defense is belt-and-suspenders: if Rust's owner map is stale
//     or out of sync with Go's DB, Rust may still reject)
//   - 502: Rust KeyVault unreachable or returned a non-permission error
func revealHandler(store *Store, kv KeyVaultClient) http.HandlerFunc {
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
			// 404 for not-found (don't leak key existence to non-creators).
			mapStoreError(w, err)
			return
		}

		// L1 dual-defense: created_by == requester (constitution XX). This is
		// the Go-side check using the DB's authoritative created_by value.
		// Even an admin cannot reveal another user's key (BYOK visibility is
		// bound to created_by, not RBAC).
		if pk.CreatedBy != user.ID {
			// 404 (not 403) to avoid leaking the key's existence to users who
			// don't own it - matches getHandler's behavior.
			writeError(w, http.StatusNotFound, "not_found", "provider key not found")
			return
		}

		// L2 dual-defense: call Rust KeyVault.Decrypt with intent=view_plaintext.
		// Rust independently checks requester == snapshot.key_owners[key_id]
		// and rejects with permission_denied if mismatch. We map that to 403.
		plaintext, err := kv.Decrypt(r.Context(), pk.EncryptedKey, user.ID, pk.ID, "view_plaintext")
		if err != nil {
			// Distinguish Rust's permission_denied (L2 rejection) from other
			// errors (Rust down, MKEK not loaded, etc.). The KeyVaultClient
			// surfaces both as errors; we don't have structured codes here, so
			// we check the error message for the L2 rejection sentinel.
			//
			// This is belt-and-suspenders: in normal operation L1 already
			// rejected non-creators, so L2 should only reject if Rust's owner
			// map is stale or out of sync with Go's DB. A 403 here signals a
			// desync that operators should investigate.
			msg := err.Error()
			if strings.Contains(msg, "permission_denied") || strings.Contains(msg, "not key owner") {
				log.Printf("keymgmt: L2 dual-defense rejected Reveal for key %q (requester %q): %v",
					pk.ID, user.ID, err)
				writeError(w, http.StatusForbidden, "forbidden",
					"key owner check failed (Rust L2 dual-defense)")
				return
			}
			// Other errors (Rust unreachable, MKEK not loaded, decrypt failure)
			// map to 502 Bad Gateway: the control plane cannot reach its
			// security core.
			log.Printf("keymgmt: KeyVault Decrypt failed for Reveal key %q: %v", pk.ID, err)
			writeError(w, http.StatusBadGateway, "keyvault_error",
				"failed to decrypt provider key")
			return
		}

		// Return the plaintext ONCE. The caller is expected to discard it
		// after use; it is not persisted, not logged (constitution XX). The
		// response body is the only place the plaintext lives in the Go
		// process.
		writeJSON(w, http.StatusOK, revealResponse{
			PlaintextKey: string(plaintext),
			KeyID:        pk.ID,
		})
	}
}
