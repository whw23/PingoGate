// Package vkey - auth + push tests (extracted to keep handler_test.go under
// constitution V 300-line limit). Same package, compiles as part of the test binary.
package vkey

import (
	"context"
	"errors"
	"net/http"
	"testing"
)

// recordingPusher is a test-only SnapshotPusher that records every Push
// call and optionally returns an error.
type recordingPusher struct {
	calls    int
	failWith error
}

func (r *recordingPusher) Push(_ context.Context) error {
	r.calls++
	return r.failWith
}

var errFakePush = errors.New("fake pusher: push failed")

// TestAuthAndAuthorize covers the 401/403 paths.
func TestAuthAndAuthorize(t *testing.T) {
	r, _, idStore, adminToken := newTestRouter(t)
	ctx := context.Background()

	_, nonAdminToken, err := idStore.Create(ctx, "user@example.com", "user-token-123")
	if err != nil {
		t.Fatalf("create non-admin: %v", err)
	}

	rec := doRequest(t, r, http.MethodGet, "/api/virtual-keys",
		"Bearer "+nonAdminToken, "")
	if rec.Code != http.StatusForbidden {
		t.Fatalf("non-admin GET: status = %d, want %d",
			rec.Code, http.StatusForbidden)
	}
	rec = doRequest(t, r, http.MethodGet, "/api/virtual-keys", "", "")
	if rec.Code != http.StatusUnauthorized {
		t.Fatalf("no-token GET: status = %d, want %d",
			rec.Code, http.StatusUnauthorized)
	}
	rec = doRequest(t, r, http.MethodGet, "/api/virtual-keys",
		"Bearer pgt_wrong_token", "")
	if rec.Code != http.StatusUnauthorized {
		t.Fatalf("wrong-token GET: status = %d, want %d",
			rec.Code, http.StatusUnauthorized)
	}
	rec = doRequest(t, r, http.MethodGet, "/api/virtual-keys",
		"Bearer "+adminToken, "")
	if rec.Code != http.StatusOK {
		t.Fatalf("admin GET: status = %d, want %d; body = %s",
			rec.Code, http.StatusOK, rec.Body.String())
	}
}

// TestIssueTriggersSnapshotPush verifies that POST triggers pusher.Push once.
func TestIssueTriggersSnapshotPush(t *testing.T) {
	pusher := &recordingPusher{}
	r, _, _, adminToken := newTestRouterWithPusher(t, pusher)

	rec := doRequest(t, r, http.MethodPost, "/api/virtual-keys",
		"Bearer "+adminToken, "{}")
	if rec.Code != http.StatusCreated {
		t.Fatalf("issue: status = %d, want %d; body = %s",
			rec.Code, http.StatusCreated, rec.Body.String())
	}
	if pusher.calls != 1 {
		t.Fatalf("pusher calls after issue = %d, want 1", pusher.calls)
	}
}

// TestIssuePushFailureDoesNotFailHTTP verifies push failure is best-effort.
func TestIssuePushFailureDoesNotFailHTTP(t *testing.T) {
	pusher := &recordingPusher{failWith: errFakePush}
	r, _, _, adminToken := newTestRouterWithPusher(t, pusher)

	rec := doRequest(t, r, http.MethodPost, "/api/virtual-keys",
		"Bearer "+adminToken, "{}")
	if rec.Code != http.StatusCreated {
		t.Fatalf("issue with push failure: status = %d, want %d",
			rec.Code, http.StatusCreated)
	}
	if pusher.calls != 1 {
		t.Fatalf("pusher calls = %d, want 1", pusher.calls)
	}
}

// TestRevokeTriggersSnapshotPush verifies DELETE triggers pusher.Push once.
func TestRevokeTriggersSnapshotPush(t *testing.T) {
	pusher := &recordingPusher{}
	r, vkeyStore, _, adminToken := newTestRouterWithPusher(t, pusher)
	adminID := getAdminID(t, vkeyStore, adminToken)

	_, vk, err := vkeyStore.Issue(context.Background(), adminID, Scope{})
	if err != nil {
		t.Fatalf("issue: %v", err)
	}
	pusher.calls = 0 // Issue was via Store, not handler; reset.

	rec := doRequest(t, r, http.MethodDelete, "/api/virtual-keys/"+vk.ID,
		"Bearer "+adminToken, "")
	if rec.Code != http.StatusNoContent {
		t.Fatalf("revoke: status = %d, want %d; body = %s",
			rec.Code, http.StatusNoContent, rec.Body.String())
	}
	if pusher.calls != 1 {
		t.Fatalf("pusher calls after revoke = %d, want 1", pusher.calls)
	}
}
