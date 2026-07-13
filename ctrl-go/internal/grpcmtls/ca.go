// Package grpcmtls manages the self-signed CA and mutual TLS certificates used
// to secure the internal gRPC channel between the Go control plane
// (pingogate-ctrl) and the Rust kernel (pingogate-core).
//
// R10 decision (spec §13): on first start the Go binary generates a self-signed
// CA and two leaf certs - ctrl.pem/ctrl.key (presented by Go as the gRPC client)
// and core.pem/core.key (presented by Rust as the gRPC server). Both are signed
// by the same CA so each side can verify the other. Certs are written to a
// on-disk directory (gitignored); subsequent starts reuse existing material.
package grpcmtls

import (
	"crypto/ecdsa"
	"crypto/elliptic"
	"crypto/rand"
	"crypto/x509"
	"crypto/x509/pkix"
	"encoding/pem"
	"fmt"
	"math/big"
	"net"
	"os"
	"path/filepath"
	"time"
)

// File names written under the certs directory.
const (
	caCertFile   = "ca.pem"
	caKeyFile    = "ca.key"
	ctrlCertFile = "ctrl.pem"
	ctrlKeyFile  = "ctrl.key"
	coreCertFile = "core.pem"
	coreKeyFile  = "core.key"
)

// EnsureCerts generates the CA + ctrl + core cert/key set in dir if any file is
// missing. If all files exist it is a no-op. The directory is created with
// 0700 if it does not exist.
func EnsureCerts(dir string) error {
	if err := os.MkdirAll(dir, 0o700); err != nil {
		return fmt.Errorf("grpcmtls: create cert dir: %w", err)
	}

	if allPresent(dir) {
		return nil
	}

	caKey, caCert, err := generateCA()
	if err != nil {
		return fmt.Errorf("grpcmtls: generate CA: %w", err)
	}

	if err := writeCert(filepath.Join(dir, caCertFile), caCert); err != nil {
		return err
	}
	if err := writeKey(filepath.Join(dir, caKeyFile), caKey); err != nil {
		return err
	}

	if err := generateAndWriteLeaf(dir, ctrlCertFile, ctrlKeyFile, "pingogate-ctrl", caCert, caKey, true); err != nil {
		return fmt.Errorf("grpcmtls: generate ctrl cert: %w", err)
	}
	if err := generateAndWriteLeaf(dir, coreCertFile, coreKeyFile, "pingogate-core", caCert, caKey, false); err != nil {
		return fmt.Errorf("grpcmtls: generate core cert: %w", err)
	}

	return nil
}

// allPresent returns true if every CA/ctrl/core cert+key file already exists.
func allPresent(dir string) bool {
	files := []string{
		caCertFile, caKeyFile,
		ctrlCertFile, ctrlKeyFile,
		coreCertFile, coreKeyFile,
	}
	for _, f := range files {
		if _, err := os.Stat(filepath.Join(dir, f)); err != nil {
			return false
		}
	}
	return true
}

// generateCA creates a self-signed root CA using an ECDSA P-256 key.
func generateCA() (*ecdsa.PrivateKey, []byte, error) {
	key, err := ecdsa.GenerateKey(elliptic.P256(), rand.Reader)
	if err != nil {
		return nil, nil, err
	}

	serial, err := randSerial()
	if err != nil {
		return nil, nil, err
	}

	tmpl := &x509.Certificate{
		SerialNumber: serial,
		Subject:      pkix.Name{CommonName: "PingoGate Internal CA"},
		NotBefore:    time.Now().Add(-time.Minute),
		NotAfter:     time.Now().AddDate(10, 0, 0),
		IsCA:         true,
		KeyUsage:     x509.KeyUsageCertSign | x509.KeyUsageCRLSign,
		BasicConstraintsValid: true,
	}

	certDER, err := x509.CreateCertificate(rand.Reader, tmpl, tmpl, &key.PublicKey, key)
	if err != nil {
		return nil, nil, err
	}

	return key, certDER, nil
}

// generateAndWriteLeaf creates a leaf cert signed by the CA and writes cert+key.
// isClient=true sets ExtKeyUsage ClientAuth (for Go ctrl), false sets ServerAuth
// (for Rust core). Both IP 127.0.0.1 and DNS localhost are in the SANs so the
// cert validates for the loopback gRPC listener.
func generateAndWriteLeaf(dir, certName, keyName, cn string, caCert []byte, caKey *ecdsa.PrivateKey, isClient bool) error {
	key, err := ecdsa.GenerateKey(elliptic.P256(), rand.Reader)
	if err != nil {
		return err
	}

	caTemplate, err := x509.ParseCertificate(caCert)
	if err != nil {
		return fmt.Errorf("parse CA cert: %w", err)
	}

	serial, err := randSerial()
	if err != nil {
		return err
	}

	extKeyUsage := []x509.ExtKeyUsage{x509.ExtKeyUsageServerAuth}
	if isClient {
		extKeyUsage = []x509.ExtKeyUsage{x509.ExtKeyUsageClientAuth}
	}

	tmpl := &x509.Certificate{
		SerialNumber: serial,
		Subject:      pkix.Name{CommonName: cn},
		NotBefore:    time.Now().Add(-time.Minute),
		NotAfter:     time.Now().AddDate(10, 0, 0),
		KeyUsage:     x509.KeyUsageDigitalSignature | x509.KeyUsageKeyEncipherment,
		ExtKeyUsage:  extKeyUsage,
		IPAddresses:  []net.IP{net.IPv4(127, 0, 0, 1)},
		DNSNames:     []string{"localhost"},
	}

	certDER, err := x509.CreateCertificate(rand.Reader, tmpl, caTemplate, &key.PublicKey, caKey)
	if err != nil {
		return fmt.Errorf("sign leaf cert: %w", err)
	}

	if err := writeCert(filepath.Join(dir, certName), certDER); err != nil {
		return err
	}
	return writeKey(filepath.Join(dir, keyName), key)
}

// writeCert encodes a DER cert as PEM and writes it to path (0644: certs are
// public, world-readable is fine).
func writeCert(path string, certDER []byte) error {
	f, err := os.OpenFile(path, os.O_WRONLY|os.O_CREATE|os.O_TRUNC, 0o644)
	if err != nil {
		return fmt.Errorf("write cert %s: %w", path, err)
	}
	defer f.Close()
	return pem.Encode(f, &pem.Block{Type: "CERTIFICATE", Bytes: certDER})
}

// writeKey encodes an ECDSA private key as SEC1 PEM and writes it to path (0600).
func writeKey(path string, key *ecdsa.PrivateKey) error {
	der, err := x509.MarshalECPrivateKey(key)
	if err != nil {
		return fmt.Errorf("marshal key: %w", err)
	}
	f, err := os.OpenFile(path, os.O_WRONLY|os.O_CREATE|os.O_TRUNC, 0o600)
	if err != nil {
		return fmt.Errorf("write key %s: %w", path, err)
	}
	defer f.Close()
	return pem.Encode(f, &pem.Block{Type: "EC PRIVATE KEY", Bytes: der})
}

// randSerial returns a positive 128-bit serial number for a certificate.
func randSerial() (*big.Int, error) {
	max := new(big.Int).Lsh(big.NewInt(1), 128)
	serial, err := rand.Int(rand.Reader, max)
	if err != nil {
		return nil, err
	}
	return serial, nil
}
