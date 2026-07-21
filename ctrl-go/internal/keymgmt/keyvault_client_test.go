// Package keymgmt - tests for the KeyVault client and helpers (constitution
// XIII: TDD). Uses a fake KeyVaultClient so tests are hermetic and run
// without the Rust binary.
package keymgmt

import (
	"bytes"
	"context"
	"errors"
	"testing"
)

// TestKeyVaultClient_FakeReversible verifies the fake KeyVault client is
// actually reversible (Encrypt then Decrypt returns the original plaintext).
func TestKeyVaultClient_FakeReversible(t *testing.T) {
	kv := newFakeKeyVaultClient()
	ctx := context.Background()
	plaintext := []byte("sk-reversible-test-XYZ")

	ciphertext, err := kv.Encrypt(ctx, plaintext)
	if err != nil {
		t.Fatalf("encrypt: %v", err)
	}
	if bytes.Equal(ciphertext, plaintext) {
		t.Fatal("fake ciphertext equals plaintext; marker not applied")
	}
	decrypted, err := kv.Decrypt(ctx, ciphertext, "u_test", "pk_test", "view_plaintext")
	if err != nil {
		t.Fatalf("decrypt: %v", err)
	}
	if !bytes.Equal(decrypted, plaintext) {
		t.Fatalf("decrypt mismatch: got %q, want %q", decrypted, plaintext)
	}
}

// TestComputeLast4 covers the last4 helper directly.
func TestComputeLast4(t *testing.T) {
	tests := []struct {
		input string
		want  string
	}{
		{"sk-1234567890abcdef", "cdef"},
		{"abc", "abc"},
		{"abcd", "abcd"},
		{"abcde", "bcde"},
		{"", ""},
	}
	for _, tc := range tests {
		t.Run(tc.input, func(t *testing.T) {
			got := computeLast4(tc.input)
			if got != tc.want {
				t.Fatalf("computeLast4(%q) = %q, want %q", tc.input, got, tc.want)
			}
		})
	}
}

// failingKeyVaultClient is a test-only KeyVaultClient whose Encrypt always
// returns an error.
type failingKeyVaultClient struct{}

func (f *failingKeyVaultClient) Encrypt(_ context.Context, _ []byte) ([]byte, error) {
	return nil, errFakeEncrypt
}

func (f *failingKeyVaultClient) Decrypt(_ context.Context, _ []byte, _, _, _ string) ([]byte, error) {
	return nil, errFakeEncrypt
}

// errFakeEncrypt is the sentinel returned by failingKeyVaultClient.
var errFakeEncrypt = errors.New("fake keyvault: encrypt failed")
