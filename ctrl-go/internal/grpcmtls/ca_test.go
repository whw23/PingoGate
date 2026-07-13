package grpcmtls

import (
	"crypto/x509"
	"encoding/pem"
	"os"
	"path/filepath"
	"testing"
)

// parsePEM decodes the first PEM block of the given type from data.
func parsePEM(t *testing.T, data []byte, blockType string) []byte {
	t.Helper()
	block, _ := pem.Decode(data)
	if block == nil {
		t.Fatalf("failed to decode PEM block of type %s", blockType)
	}
	if block.Type != blockType {
		t.Fatalf("expected PEM type %s, got %s", blockType, block.Type)
	}
	return block.Bytes
}

// TestEnsureCerts_GeneratesAllFiles verifies that EnsureCerts creates the CA,
// ctrl, and core cert/key files on first call.
func TestEnsureCerts_GeneratesAllFiles(t *testing.T) {
	dir := t.TempDir()

	if err := EnsureCerts(dir); err != nil {
		t.Fatalf("EnsureCerts: %v", err)
	}

	files := []string{caCertFile, caKeyFile, ctrlCertFile, ctrlKeyFile, coreCertFile, coreKeyFile}
	for _, f := range files {
		path := filepath.Join(dir, f)
		if _, err := os.Stat(path); err != nil {
			t.Errorf("expected file %s to exist: %v", f, err)
		}
	}
}

// TestEnsureCerts_Idempotent verifies that calling EnsureCerts twice on the
// same directory does not error and preserves existing files.
func TestEnsureCerts_Idempotent(t *testing.T) {
	dir := t.TempDir()

	if err := EnsureCerts(dir); err != nil {
		t.Fatalf("first EnsureCerts: %v", err)
	}

	caBefore, _ := os.ReadFile(filepath.Join(dir, caCertFile))

	if err := EnsureCerts(dir); err != nil {
		t.Fatalf("second EnsureCerts: %v", err)
	}

	caAfter, _ := os.ReadFile(filepath.Join(dir, caCertFile))
	if string(caBefore) != string(caAfter) {
		t.Error("CA cert changed on second EnsureCerts call; expected idempotent")
	}
}

// TestClientCredentials_LoadsAfterGenerate verifies that ClientCredentials can
// load the generated cert/key/CA and produce valid gRPC transport credentials.
func TestClientCredentials_LoadsAfterGenerate(t *testing.T) {
	dir := t.TempDir()

	if err := EnsureCerts(dir); err != nil {
		t.Fatalf("EnsureCerts: %v", err)
	}

	creds, err := ClientCredentials(dir)
	if err != nil {
		t.Fatalf("ClientCredentials: %v", err)
	}
	if creds == nil {
		t.Error("ClientCredentials returned nil credentials")
	}
}

// TestGeneratedCerts_Parse verifies that the generated PEM files are valid
// X.509 certificates with correct key usage and signing chain.
func TestGeneratedCerts_Parse(t *testing.T) {
	dir := t.TempDir()

	if err := EnsureCerts(dir); err != nil {
		t.Fatalf("EnsureCerts: %v", err)
	}

	// Parse CA cert.
	caPEM, _ := os.ReadFile(filepath.Join(dir, caCertFile))
	caCert, err := x509.ParseCertificate(parsePEM(t, caPEM, "CERTIFICATE"))
	if err != nil {
		t.Fatalf("parse CA cert: %v", err)
	}
	if !caCert.IsCA {
		t.Error("CA cert should have IsCA=true")
	}

	// Parse ctrl (client) cert and verify it is signed by the CA.
	ctrlPEM, _ := os.ReadFile(filepath.Join(dir, ctrlCertFile))
	ctrlCert, err := x509.ParseCertificate(parsePEM(t, ctrlPEM, "CERTIFICATE"))
	if err != nil {
		t.Fatalf("parse ctrl cert: %v", err)
	}
	if err := ctrlCert.CheckSignatureFrom(caCert); err != nil {
		t.Errorf("ctrl cert not signed by CA: %v", err)
	}

	// Verify ctrl cert has client auth ext key usage.
	foundClientAuth := false
	for _, eku := range ctrlCert.ExtKeyUsage {
		if eku == x509.ExtKeyUsageClientAuth {
			foundClientAuth = true
		}
	}
	if !foundClientAuth {
		t.Error("ctrl cert should have ExtKeyUsageClientAuth")
	}

	// Parse core (server) cert and verify server auth.
	corePEM, _ := os.ReadFile(filepath.Join(dir, coreCertFile))
	coreCert, err := x509.ParseCertificate(parsePEM(t, corePEM, "CERTIFICATE"))
	if err != nil {
		t.Fatalf("parse core cert: %v", err)
	}
	foundServerAuth := false
	for _, eku := range coreCert.ExtKeyUsage {
		if eku == x509.ExtKeyUsageServerAuth {
			foundServerAuth = true
		}
	}
	if !foundServerAuth {
		t.Error("core cert should have ExtKeyUsageServerAuth")
	}
}
