// Package keymgmt - push-trigger tests (extracted from handler_test.go to
// keep that file under constitution V's 300-line limit). Verifies that CRUD
// mutations trigger snapshot pushes via the SnapshotPusher hook, and that push
// failures are best-effort (do not fail the HTTP response). Same package,
// compiles as part of the test binary.
package keymgmt

import (
	"context"
	"errors"
	"net/http"
	"testing"
)

// recordingPusher is a test-only SnapshotPusher that records every Push call
// and optionally returns an error. Used to verify the handler triggers a push
// after Create/Delete and that push failures do not fail the HTTP response.
type recordingPusher struct {
	calls    int
	failWith error
}

func (r *recordingPusher) Push(_ context.Context) error {
	r.calls++
	return r.failWith
}

// errFakePush is the sentinel returned by recordingPusher.failWith.
var errFakePush = errors.New("fake pusher: push failed")

// TestCreateTriggersSnapshotPush verifies that a successful POST triggers
// pusher.Push exactly once (spec §12C: DB change -> push).
func TestCreateTriggersSnapshotPush(t *testing.T) {
	kv := newFakeKeyVaultClient()
	pusher := &recordingPusher{}
	r, _, _, adminToken := newTestRouterWithPusher(t, kv, pusher)

	body := `{"provider_type":"openai","plaintext_key":"sk-push-test-12345678"}`
	rec := doRequest(t, r, http.MethodPost, "/api/provider-keys", "Bearer "+adminToken, body)
	if rec.Code != http.StatusCreated {
		t.Fatalf("create: status = %d, want %d; body = %s",
			rec.Code, http.StatusCreated, rec.Body.String())
	}
	if pusher.calls != 1 {
		t.Fatalf("pusher calls after create = %d, want 1", pusher.calls)
	}
}

// TestCreatePushFailureDoesNotFailHTTP verifies that a push failure is logged
// but does NOT fail the HTTP response (best-effort push; DB is authoritative).
func TestCreatePushFailureDoesNotFailHTTP(t *testing.T) {
	kv := newFakeKeyVaultClient()
	pusher := &recordingPusher{failWith: errFakePush}
	r, _, _, adminToken := newTestRouterWithPusher(t, kv, pusher)

	body := `{"provider_type":"openai","plaintext_key":"sk-push-fail-12345678"}`
	rec := doRequest(t, r, http.MethodPost, "/api/provider-keys", "Bearer "+adminToken, body)
	if rec.Code != http.StatusCreated {
		t.Fatalf("create with push failure: status = %d, want %d (best-effort)",
			rec.Code, http.StatusCreated)
	}
	if pusher.calls != 1 {
		t.Fatalf("pusher calls = %d, want 1 (push was attempted despite failure)",
			pusher.calls)
	}
}

// TestDeleteTriggersSnapshotPush verifies that a successful DELETE triggers
// pusher.Push exactly once.
func TestDeleteTriggersSnapshotPush(t *testing.T) {
	kv := newFakeKeyVaultClient()
	pusher := &recordingPusher{}
	r, keyStore, idStore, adminToken := newTestRouterWithPusher(t, kv, pusher)
	ctx := context.Background()

	admin, err := idStore.VerifyToken(ctx, adminToken)
	if err != nil {
		t.Fatalf("verify admin: %v", err)
	}
	enc, _ := kv.Encrypt(ctx, []byte("sk-delete-push-1234567"))
	pk, err := keyStore.Create(ctx, admin.ID, "openai", enc, "4567", "", admin.ID)
	if err != nil {
		t.Fatalf("create key: %v", err)
	}
	// Create already triggered a push (via the handler in newTestRouter would
	// have, but we created directly via Store here, so calls=0 so far).
	pusher.calls = 0

	rec := doRequest(t, r, http.MethodDelete, "/api/provider-keys/"+pk.ID,
		"Bearer "+adminToken, "")
	if rec.Code != http.StatusNoContent {
		t.Fatalf("delete: status = %d, want %d; body = %s",
			rec.Code, http.StatusNoContent, rec.Body.String())
	}
	if pusher.calls != 1 {
		t.Fatalf("pusher calls after delete = %d, want 1", pusher.calls)
	}
}
