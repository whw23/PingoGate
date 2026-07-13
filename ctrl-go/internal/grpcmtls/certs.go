// Package grpcmtls - certificate loading for the Go gRPC client.
package grpcmtls

import (
	"crypto/tls"
	"crypto/x509"
	"fmt"
	"os"
	"path/filepath"

	"google.golang.org/grpc/credentials"
)

// ClientCredentials loads the ctrl cert/key (client identity) and the CA cert
// (to verify the Rust server), returning gRPC transport credentials configured
// for mutual TLS. The server's certificate must list 127.0.0.1 or localhost in
// its SANs, matching the loopback-only gRPC listener (spec §12A).
func ClientCredentials(dir string) (credentials.TransportCredentials, error) {
	certPath := filepath.Join(dir, ctrlCertFile)
	keyPath := filepath.Join(dir, ctrlKeyFile)
	caPath := filepath.Join(dir, caCertFile)

	cert, err := tls.LoadX509KeyPair(certPath, keyPath)
	if err != nil {
		return nil, fmt.Errorf("grpcmtls: load client cert/key: %w", err)
	}

	caPEM, err := os.ReadFile(caPath)
	if err != nil {
		return nil, fmt.Errorf("grpcmtls: read CA cert: %w", err)
	}

	pool := x509.NewCertPool()
	if !pool.AppendCertsFromPEM(caPEM) {
		return nil, fmt.Errorf("grpcmtls: failed to parse CA cert from %s", caPath)
	}

	tlsConfig := &tls.Config{
		Certificates: []tls.Certificate{cert},
		RootCAs:      pool,
		// The Rust kernel presents its own cert signed by the same CA; verify
		// it against the CA pool and require the server cert (mTLS).
		ServerName:         "localhost",
		MinVersion:         tls.VersionTLS13,
		InsecureSkipVerify: false,
	}

	return credentials.NewTLS(tlsConfig), nil
}
