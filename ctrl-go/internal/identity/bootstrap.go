// Package identity - bootstrap admin flow (spec §12B). The first time the Go
// binary starts with an empty DB, there is no admin to administer the system.
// BootstrapAdmin creates one from the PINGO_BOOTSTRAP_ADMIN_TOKEN env var so
// the operator knows the initial admin token out-of-band (not over HTTP). On
// subsequent starts the env var is a no-op: if an admin already exists,
// BootstrapAdmin returns nil without touching the DB.
package identity

import (
	"context"
	"errors"
	"fmt"
)

// BootstrapAdminEmail is the email used for the bootstrap admin. It is a fixed
// placeholder; the operator is expected to replace it via the User CRUD API
// after first login if a real email is needed (L0 has no email-verify flow;
// OAuth/profile editing is L6).
const BootstrapAdminEmail = "admin@bootstrap.local"

// BootstrapAdmin creates the initial admin user if and only if no admin exists.
// The token argument becomes the admin's API token (bcrypt-hashed at rest); it
// must come from an env var or secret store, not from user input. The env var
// PINGO_BOOTSTRAP_ADMIN_TOKEN is the intended source (spec §12B).
//
// Returns nil (no-op) when an admin already exists. Returns an error if the DB
// query fails or if Create/SetAdmin fail; callers MUST treat a non-nil error as
// fatal (spec §12B: missing bootstrap is fatal, no degraded path).
//
// The returned bool reports whether a new admin was created (true) or the
// call was a no-op (false), so callers can log the distinction.
func BootstrapAdmin(ctx context.Context, store *Store, token string) (bool, error) {
	if token == "" {
		return false, fmt.Errorf("identity: bootstrap token is empty (spec §12B requires PINGO_BOOTSTRAP_ADMIN_TOKEN)")
	}

	count, err := store.CountAdmins(ctx)
	if err != nil {
		return false, fmt.Errorf("identity: bootstrap count admins: %w", err)
	}
	if count > 0 {
		// An admin already exists; skip. This is the steady-state path after
		// the first bootstrap. The env var can be unset on subsequent starts.
		return false, nil
	}

	// Create the bootstrap user (non-admin initially), then promote. Two-step
	// because Create always inserts is_admin=0 and SetAdmin is the only path
	// that flips the flag. Doing it in one INSERT would bypass SetAdmin and
	// split the admin-promotion logic across two code paths.
	u, _, err := store.Create(ctx, BootstrapAdminEmail, token)
	if err != nil {
		// If the bootstrap email is already taken (e.g., a previous partial
		// bootstrap failed before SetAdmin), Create returns ErrEmailTaken. In
		// that case there is no admin but the placeholder email is occupied;
		// surface as a fatal error so the operator resolves it manually.
		if errors.Is(err, ErrEmailTaken) {
			return false, fmt.Errorf("identity: bootstrap email %q already exists but no admin present; resolve manually (delete the placeholder user or promote it)", BootstrapAdminEmail)
		}
		return false, fmt.Errorf("identity: bootstrap create: %w", err)
	}
	if _, err := store.SetAdmin(ctx, u.ID, true); err != nil {
		return false, fmt.Errorf("identity: bootstrap set admin: %w", err)
	}
	return true, nil
}
