// Package keymgmt - KeyVaultClient is the Go gRPC client for the Rust
// KeyVaultService (T15). It wraps pingogatepb.KeyVaultServiceClient so the
// handler layer depends on a focused interface (constitution VII) and so tests
// can substitute a fake without standing up the Rust binary.
//
// Flow at Create time: handler holds plaintext briefly, calls
// KeyVaultClient.Encrypt, stores the returned ciphertext. The plaintext is
// never persisted and never logged (constitution XX). Decrypt is not used by
// the S2 control plane (only the Rust hot path decrypts); it is exposed here
// so S3's reveal endpoint (created_by only) can call it without a new client.
package keymgmt

import (
	"context"
	"fmt"

	pb "github.com/whw23/pingogate/ctrl-go/internal/proto"
)

// KeyVaultClient is the focused interface the handler depends on. The real
// implementation (grpcKeyVaultClient) wraps pb.KeyVaultServiceClient; tests
// substitute a fake without standing up the Rust binary.
//
// The interface is intentionally narrow (Encrypt/Decrypt) so the handler
// doesn't accidentally depend on gRPC machinery (constitution VII: focused
// trait, injected dep).
type KeyVaultClient interface {
	// Encrypt sends plaintext to the Rust KeyVault and returns the AES-GCM
	// ciphertext. The plaintext is never persisted by Go; only the returned
	// ciphertext is stored in user_provider_keys.encrypted_key.
	Encrypt(ctx context.Context, plaintext []byte) ([]byte, error)

	// Decrypt sends ciphertext to the Rust KeyVault and returns the plaintext.
	// Used by S3's reveal endpoint (created_by only); not used by S2 control
	// plane list/get handlers. requesterID/keyID/intent are forwarded to Rust
	// for the S3 dual-defense visibility check (constitution XX).
	Decrypt(ctx context.Context, ciphertext []byte, requesterID, keyID, intent string) ([]byte, error)
}

// grpcKeyVaultClient is the production implementation. It owns a
// pb.KeyVaultServiceClient (constructed by main.go with mTLS + token
// interceptors from T11) and forwards calls to it.
type grpcKeyVaultClient struct {
	client pb.KeyVaultServiceClient
}

// NewKeyVaultClient wraps a pb.KeyVaultServiceClient. The caller (main.go) is
// responsible for constructing the underlying *grpc.ClientConn with mTLS +
// token interceptors (T11 grpcmtls); this wrapper only adds the focused
// KeyVaultClient interface on top.
func NewKeyVaultClient(client pb.KeyVaultServiceClient) KeyVaultClient {
	return &grpcKeyVaultClient{client: client}
}

// Encrypt calls the Rust KeyVaultService.Encrypt. A non-empty Error field in
// the response indicates the Rust side rejected the request (e.g., MKEK not
// loaded); we surface it as an error so the handler maps to 500/503.
func (c *grpcKeyVaultClient) Encrypt(ctx context.Context, plaintext []byte) ([]byte, error) {
	resp, err := c.client.Encrypt(ctx, &pb.EncryptRequest{Plaintext: plaintext})
	if err != nil {
		return nil, fmt.Errorf("keymgmt: keyvault Encrypt RPC: %w", err)
	}
	if resp.GetError() != "" {
		return nil, fmt.Errorf("keymgmt: keyvault Encrypt rejected: %s", resp.GetError())
	}
	if len(resp.GetCiphertext()) == 0 {
		return nil, fmt.Errorf("keymgmt: keyvault Encrypt returned empty ciphertext")
	}
	return resp.GetCiphertext(), nil
}

// Decrypt calls the Rust KeyVaultService.Decrypt. Used by S3's reveal
// endpoint; not on the S2 control-plane read path. A non-empty Error field
// indicates the Rust side rejected the request (e.g., visibility check
// failed); we surface it as an error so the handler maps to 403/500.
func (c *grpcKeyVaultClient) Decrypt(ctx context.Context, ciphertext []byte, requesterID, keyID, intent string) ([]byte, error) {
	resp, err := c.client.Decrypt(ctx, &pb.DecryptRequest{
		Ciphertext:      ciphertext,
		RequesterUserId: requesterID,
		KeyId:           keyID,
		Intent:          intent,
	})
	if err != nil {
		return nil, fmt.Errorf("keymgmt: keyvault Decrypt RPC: %w", err)
	}
	if resp.GetError() != "" {
		return nil, fmt.Errorf("keymgmt: keyvault Decrypt rejected: %s", resp.GetError())
	}
	return resp.GetPlaintext(), nil
}

// fakeKeyVaultClient is a test-only KeyVaultClient that performs a reversible
// marker-prefix transformation (NOT real cryptography). It lets handler
// tests verify the Create flow (plaintext -> ciphertext -> stored -> list
// returns last4) without standing up the Rust binary. The prefix marks the
// ciphertext so tests can assert it is not the plaintext.
type fakeKeyVaultClient struct {
	// marker is prepended to the ciphertext so tests can distinguish ciphertext
	// from plaintext (real KeyVault uses AES-GCM; the fake uses a marker).
	marker []byte
}

// newFakeKeyVaultClient returns a fake KeyVaultClient suitable for tests. The
// marker makes it easy to assert that what is stored is not the plaintext.
func newFakeKeyVaultClient() *fakeKeyVaultClient {
	return &fakeKeyVaultClient{marker: []byte("ENC:")}
}

// Encrypt reversibly transforms plaintext for test purposes. NOT secure; test
// only. The transformation is marker || plaintext so Decrypt can reverse it.
func (f *fakeKeyVaultClient) Encrypt(_ context.Context, plaintext []byte) ([]byte, error) {
	out := make([]byte, 0, len(f.marker)+len(plaintext))
	out = append(out, f.marker...)
	out = append(out, plaintext...)
	return out, nil
}

// Decrypt reverses the fake Encrypt transformation. Returns the plaintext
// with the marker stripped; if the marker is absent the input is returned
// as-is (defensive, should not happen in tests).
func (f *fakeKeyVaultClient) Decrypt(_ context.Context, ciphertext []byte, _, _, _ string) ([]byte, error) {
	if len(ciphertext) < len(f.marker) {
		return ciphertext, nil
	}
	return ciphertext[len(f.marker):], nil
}
