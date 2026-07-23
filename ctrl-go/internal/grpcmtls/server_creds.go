// Package grpcmtls - server-side credentials for the Go UsageService gRPC
// server (T31). The Rust kernel dials Go as a client to push UsageEvents;
// Go presents the ctrl cert (dual-usage after T31) as its server identity.
// Symmetric to ClientCredentials but for the server side.
package grpcmtls

import (
	"crypto/tls"
	"crypto/x509"
	"fmt"
	"os"
	"path/filepath"

	"google.golang.org/grpc/credentials"
)

// ServerCredentials loads the ctrl cert/key (now also used as the Go server
// identity for the UsageService gRPC server; T31) and the CA cert (to verify
// the Rust client), returning gRPC transport credentials configured for
// mutual TLS. Symmetric with ClientCredentials but presented as a server.
func ServerCredentials(dir string) (credentials.TransportCredentials, error) {
	certPath := filepath.Join(dir, ctrlCertFile)
	keyPath := filepath.Join(dir, ctrlKeyFile)
	caPath := filepath.Join(dir, caCertFile)

	cert, err := tls.LoadX509KeyPair(certPath, keyPath)
	if err != nil {
		return nil, fmt.Errorf("grpcmtls: load server cert/key: %w", err)
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
		ClientCAs:    pool,
		ClientAuth:   tls.RequireAndVerifyClientCert,
		MinVersion:   tls.VersionTLS13,
	}
	return credentials.NewTLS(tlsConfig), nil
}
