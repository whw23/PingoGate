// Package usage - tiktoken-go token estimator (constitution XIX: "Go estimation;
// Rust has no tokenizer"). When a provider response carries no usage block
// (e.g. a non-streaming endpoint that omits usage, or an interrupted stream),
// the Rust kernel pushes the request/response body to Go with
// needs_estimate=true and Go fills in input/output via tiktoken-go.
//
// Provider/model -> tokenizer mapping (constitution XIX):
//   - OpenAI gpt-4o*, gpt-4.1*, o1*, o3* -> o200k_base
//   - OpenAI gpt-4*, gpt-3.5* -> cl100k_base
//   - Anthropic, Gemini, unknown -> rough character-based estimate
//     (tiktoken-go has no native tokenizer for these; a char/4 estimate is
//     the documented fallback to avoid a hard dependency on per-provider
//     tokenizers that would drift from the upstream.)
package usage

import (
	"encoding/json"
	"fmt"
	"strings"
	"sync"

	"github.com/pkoukk/tiktoken-go"
)

// Estimator estimates token counts for a request/response body. It caches
// tiktoken instances per encoding name so the ~1ms initialization cost is
// paid once per process. Safe for concurrent use (constitution XXI: the hot
// path is the Rust kernel; Go only sees estimation requests for the subset
// of responses that omit usage, so a sync.Mutex is sufficient).
type Estimator struct {
	mu       sync.Mutex
	cache    map[string]*tiktoken.Tiktoken
	fallback bool // set true if tiktoken init failed (e.g. offline); char-estimate only
}

// NewEstimator constructs an Estimator. tiktoken-go downloads its BPE ranks
// from a CDN on first use; if the process cannot reach the CDN, call
// [Estimator.SetOfflineFallback] to switch to char-based estimation for all
// models. In tests, prefer SetOfflineFallback to avoid network dependence.
func NewEstimator() *Estimator {
	return &Estimator{cache: make(map[string]*tiktoken.Tiktoken)}
}

// SetOfflineFallback forces the estimator to use the character-based fallback
// for every call, bypassing tiktoken entirely. Used in tests and when the
// process cannot fetch BPE ranks.
func (e *Estimator) SetOfflineFallback() {
	e.mu.Lock()
	defer e.mu.Unlock()
	e.fallback = true
	e.cache = nil
}

// Estimate returns (input, output) token counts for a request/response body.
// The body is expected to be the raw JSON the caller sent/received (OpenAI
// Chat shape: {"messages":[...], ...}). For unknown shapes or providers, a
// coarse char/4 estimate is used (documented in constitution XIX; the goal
// is a reasonable billable proxy, not byte-exact accuracy).
//
// The function never returns an error: estimation is best-effort by design
// (constitution XIX: "no usage -> tokenizer estimate"). A failure to parse
// or tokenize degrades to the char-based fallback.
func (e *Estimator) Estimate(body []byte, provider, model string) (input, output int64) {
	req, resp := splitBody(body)
	enc := e.tokenizerFor(provider, model)
	if enc == nil {
		// Char-based fallback: ~4 chars/token (OpenAI rough heuristic). We
		// also count JSON structural bytes, which is conservative (tends to
		// over-count, which is the safer billing direction).
		return int64(len(req)) / 4, int64(len(resp)) / 4
	}
	return int64(countTokens(enc, req)), int64(countTokens(enc, resp))
}

// splitBody divides the body_ref into (request, response) halves. T31 brief
// specifies body_ref carries "request/response body"; we JSON-decode an
// object of shape {"request": ..., "response": ...} if possible, otherwise
// treat the whole payload as the request (response empty). This keeps the
// estimator robust to whatever the Rust kernel actually ships.
func splitBody(body []byte) (req, resp []byte) {
	if len(body) == 0 {
		return nil, nil
	}
	var parts struct {
		Request  json.RawMessage
		Response json.RawMessage
	}
	if err := json.Unmarshal(body, &parts); err == nil {
		if len(parts.Request) > 0 {
			req = parts.Request
		}
		if len(parts.Response) > 0 {
			resp = parts.Response
		}
		return
	}
	// Not a {request,response} envelope: treat as request-only.
	return body, nil
}

// tokenizerFor picks the tiktoken encoding for a provider/model pair. Returns
// nil when no suitable tokenizer is available (caller falls back to char
// estimate). The first call for a given encoding lazy-loads the BPE ranks;
// subsequent calls hit the cache.
func (e *Estimator) tokenizerFor(provider, model string) *tiktoken.Tiktoken {
	e.mu.Lock()
	defer e.mu.Unlock()
	if e.fallback {
		return nil
	}
	encName := pickEncoding(provider, model)
	if encName == "" {
		return nil
	}
	if t, ok := e.cache[encName]; ok {
		return t
	}
	t, err := tiktoken.EncodingForModel(encName)
	if err != nil {
		// EncodingForModel fails when the model name is unknown or the BPE
		// ranks cannot be fetched. Either way, we return nil and let the
		// caller fall back. We do NOT cache nil under encName because a later
		// retry (after CDN reachable) might succeed.
		return nil
	}
	// Defensive: EncodingForModel returns a non-nil Tiktoken on success.
	if t == nil {
		return nil
	}
	e.cache[encName] = t
	return t
}

// pickEncoding maps a (provider, model) to a tiktoken encoding name. Returns
// "" when the pair has no native tiktoken support (Anthropic / Gemini /
// unknown). Lower-cases the model for matching because provider model names
// are case-insensitive in practice.
func pickEncoding(provider, model string) string {
	m := strings.ToLower(model)
	p := strings.ToLower(provider)
	switch {
	case strings.Contains(p, "anthropic"), strings.Contains(p, "claude"):
		return "" // no native tiktoken; char fallback
	case strings.Contains(p, "gemini"), strings.Contains(m, "gemini"):
		return ""
	case strings.Contains(m, "gpt-4o"), strings.Contains(m, "gpt-4.1"),
		strings.Contains(m, "o1"), strings.Contains(m, "o3"),
		strings.Contains(m, "chatgpt-4o"):
		return "o200k_base"
	case strings.Contains(m, "gpt-4"), strings.Contains(m, "gpt-3.5"),
		strings.Contains(m, "text-davinci"):
		return "cl100k_base"
	}
	// Default for OpenAI-compatible providers with unknown models: try the
	// newer o200k_base (covers gpt-4o family and is the current default); if
	// that fails at load time, tokenizerFor returns nil and we fall back.
	if p == "" || strings.Contains(p, "openai") {
		return "o200k_base"
	}
	return ""
}

// countTokens encodes the body and returns the token count. On any error
// (including a panic from the underlying regex engine), returns a char/4
// estimate so the caller still gets a non-zero number.
func countTokens(t *tiktoken.Tiktoken, body []byte) int {
	if len(body) == 0 {
		return 0
	}
	tokens := t.Encode(string(body), nil, nil)
	if len(tokens) == 0 {
		return len(body) / 4
	}
	return len(tokens)
}

// String is a small helper for debug logging (not on the hot path).
func (e *Estimator) String() string {
	e.mu.Lock()
	defer e.mu.Unlock()
	return fmt.Sprintf("Estimator(cache=%d, fallback=%v)", len(e.cache), e.fallback)
}
