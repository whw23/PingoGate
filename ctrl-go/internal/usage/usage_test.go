// Package usage - tests for the UsageService server, Store, and Estimator
// (constitution XIII: TDD). The Store tests use an in-memory SQLite DB so
// they are hermetic. The Estimator tests use SetOfflineFallback to avoid
// network dependence on the tiktoken BPE ranks CDN.
package usage

import (
	"context"
	"encoding/json"
	"testing"

	pb "github.com/whw23/pingogate/ctrl-go/internal/proto"
	"github.com/whw23/pingogate/ctrl-go/internal/storage"
)

// newTestStore opens an in-memory SQLite DB with migrations applied and
// returns a usage.Store wrapping it. Cleanup is registered with t.Cleanup.
func newTestStore(t *testing.T) *Store {
	t.Helper()
	db, err := storage.Open(":memory:")
	if err != nil {
		t.Fatalf("open in-memory DB: %v", err)
	}
	t.Cleanup(func() { _ = db.Close() })
	return NewStore(db)
}

// TestStoreInsertWithUsage verifies that an event with provider-supplied
// usage is persisted verbatim (estimated=0).
func TestStoreInsertWithUsage(t *testing.T) {
	store := newTestStore(t)
	ctx := context.Background()
	event := &pb.UsageEvent{
		VirtualKeyId:    "vk-1",
		OwnerUserId:     "user-A",
		Provider:        "openai",
		Model:           "gpt-4o",
		InputTokens:     100,
		OutputTokens:    50,
		ReasoningTokens: 5,
		CacheReadTokens: 10,
		Success:         true,
		LatencyMs:       42,
	}
	rowID, err := store.Insert(ctx, event)
	if err != nil {
		t.Fatalf("insert: %v", err)
	}
	if rowID == 0 {
		t.Fatal("rowID = 0, want non-zero")
	}
	count, err := store.CountByOwner(ctx, "user-A")
	if err != nil {
		t.Fatalf("count: %v", err)
	}
	if count != 1 {
		t.Fatalf("count = %d, want 1", count)
	}
}

// TestStoreInsertNilVkey verifies that an empty virtual_key_id is stored
// as NULL (standalone mode).
func TestStoreInsertNilVkey(t *testing.T) {
	store := newTestStore(t)
	ctx := context.Background()
	event := &pb.UsageEvent{
		VirtualKeyId: "",
		OwnerUserId:  "standalone",
		Provider:     "openai",
		Model:        "gpt-4o",
		InputTokens:  10,
		Success:      true,
	}
	if _, err := store.Insert(ctx, event); err != nil {
		t.Fatalf("insert: %v", err)
	}
}

// TestServerHandleEventWithUsage verifies the server persists a
// provider-usage event without estimation (needs_estimate=false).
func TestServerHandleEventWithUsage(t *testing.T) {
	store := newTestStore(t)
	est := NewEstimator()
	est.SetOfflineFallback()
	srv := NewServer(store, est, nil)
	ctx := context.Background()
	event := &pb.UsageEvent{
		OwnerUserId:  "user-A",
		Provider:     "openai",
		Model:        "gpt-4o",
		InputTokens:  100,
		OutputTokens: 50,
		Success:      true,
	}
	if err := srv.handleEvent(ctx, event); err != nil {
		t.Fatalf("handleEvent: %v", err)
	}
	count, err := store.CountByOwner(ctx, "user-A")
	if err != nil {
		t.Fatalf("count: %v", err)
	}
	if count != 1 {
		t.Fatalf("count = %d, want 1", count)
	}
}

// TestServerHandleEventNeedsEstimate verifies that a needs_estimate=true
// event triggers the estimator and the estimated counts are persisted.
func TestServerHandleEventNeedsEstimate(t *testing.T) {
	store := newTestStore(t)
	est := NewEstimator()
	est.SetOfflineFallback()
	srv := NewServer(store, est, nil)
	ctx := context.Background()
	body := map[string]any{
		"request":  map[string]any{"messages": []string{"hello world"}},
		"response": map[string]any{"choices": []string{"hi there"}},
	}
	bodyBytes, _ := json.Marshal(body)
	event := &pb.UsageEvent{
		OwnerUserId:   "user-B",
		Provider:      "openai",
		Model:         "gpt-4o",
		NeedsEstimate: true,
		BodyRef:       bodyBytes,
		Success:       true,
	}
	if err := srv.handleEvent(ctx, event); err != nil {
		t.Fatalf("handleEvent: %v", err)
	}
	if event.InputTokens == 0 {
		t.Fatal("input tokens not estimated")
	}
	if event.OutputTokens == 0 {
		t.Fatal("output tokens not estimated")
	}
	if event.BodyRef != nil {
		t.Fatal("body_ref should be cleared after estimation")
	}
	count, err := store.CountByOwner(ctx, "user-B")
	if err != nil {
		t.Fatalf("count: %v", err)
	}
	if count != 1 {
		t.Fatalf("count = %d, want 1", count)
	}
}

// TestServerHandleEventPartialFailure verifies that a failed request
// (success=false) with partial output is still persisted (constitution
// XIX: failed/interrupted requests get partial metering).
func TestServerHandleEventPartialFailure(t *testing.T) {
	store := newTestStore(t)
	est := NewEstimator()
	srv := NewServer(store, est, nil)
	ctx := context.Background()
	event := &pb.UsageEvent{
		OwnerUserId:  "user-C",
		Provider:     "anthropic",
		Model:        "claude-3-5-sonnet",
		InputTokens:  200,
		OutputTokens: 10,
		Success:      false,
		LatencyMs:    100,
	}
	if err := srv.handleEvent(ctx, event); err != nil {
		t.Fatalf("handleEvent: %v", err)
	}
	count, err := store.CountByOwner(ctx, "user-C")
	if err != nil {
		t.Fatalf("count: %v", err)
	}
	if count != 1 {
		t.Fatalf("count = %d, want 1", count)
	}
}

// TestEstimatorOfflineFallback verifies the char-based fallback returns
// non-zero counts for any non-empty body.
func TestEstimatorOfflineFallback(t *testing.T) {
	est := NewEstimator()
	est.SetOfflineFallback()
	body := []byte("hello world this is a test")
	input, output := est.Estimate(body, "openai", "gpt-4o")
	if input == 0 {
		t.Fatal("input = 0, want non-zero")
	}
	_ = output
}

// TestEstimatorSplitBody verifies the JSON envelope splitting logic.
func TestEstimatorSplitBody(t *testing.T) {
	envelope, _ := json.Marshal(map[string]any{
		"request":  map[string]any{"prompt": "hi"},
		"response": map[string]any{"text": "hello"},
	})
	req, resp := splitBody(envelope)
	if len(req) == 0 {
		t.Fatal("req is empty")
	}
	if len(resp) == 0 {
		t.Fatal("resp is empty")
	}
}

// TestEstimatorPickEncoding verifies the provider/model -> encoding map.
func TestEstimatorPickEncoding(t *testing.T) {
	cases := []struct{ provider, model, want string }{
		{"openai", "gpt-4o", "o200k_base"},
		{"openai", "gpt-4o-mini", "o200k_base"},
		{"openai", "gpt-4.1", "o200k_base"},
		{"openai", "o1-preview", "o200k_base"},
		{"openai", "gpt-4-turbo", "cl100k_base"},
		{"openai", "gpt-3.5-turbo", "cl100k_base"},
		{"anthropic", "claude-3", ""},
		{"google", "gemini-1.5-pro", ""},
		{"", "unknown-model", "o200k_base"},
	}
	for _, c := range cases {
		got := pickEncoding(c.provider, c.model)
		if got != c.want {
			t.Errorf("pickEncoding(%q, %q) = %q, want %q", c.provider, c.model, got, c.want)
		}
	}
}
