//! S3 security no-leak audit (spec §12D, constitution XX).
//!
//! This test enforces the "plaintext key never leaks" property end-to-end at
//! the Rust gRPC KeyVault layer. It exercises every `Decrypt` path
//! (owner-success, non-owner-denied, unknown-key-denied, hot_path_inject) and
//! asserts that a known plaintext marker (`LEAKMARKER`) never appears in:
//!
//! - tracing log output (the audit lines the service emits)
//! - gRPC error messages (returned to the caller)
//!
//! The successful `Decrypt` response's `plaintext` field IS allowed to contain
//! the marker (the caller asked to view_plaintext and is authorized) - but it
//! must not leak via logs or error messages.
//!
//! The test uses a `Mutex<Vec<u8>>` MakeWriter to capture all tracing output
//! produced during the exercise, then scans the buffer for the marker. It is
//! hermetic (no real mTLS, no Go control plane, no real snapshot push): the
//! owner map is a static `MapOwnerLookup`; the keyvault is the real AES-GCM
//! implementation.
//!
//! Constitution XX: "明文只在 `KeyVault::decrypt()` 返回的 `SecretString` 中
//! 存活，MUST NOT 进日志 / 指标 / 任何对外输出."

#![cfg(test)]

use std::collections::HashMap;
use std::io::{self, Write};
use std::sync::{Arc, Mutex};

use tonic::{Request, Status};
use tracing_subscriber::fmt::MakeWriter;

use pingogate_keyvault::{
    proto::key_vault_service_server::KeyVaultService, AesGcmKeyVault,
    INTENT_HOT_PATH_INJECT, INTENT_VIEW_PLAINTEXT, KeyVaultGrpcService, OwnerLookup,
};
use pingogate_keyvault::proto::DecryptRequest;

/// Marker embedded in the test plaintext. If this string appears anywhere in
/// captured tracing output or in gRPC error messages, the test fails - the
/// KeyVault has leaked plaintext beyond the `Decrypt` response.
const LEAK_MARKER: &str = "LEAKMARKER";

/// Thread-safe buffer that captures all bytes written by the tracing
/// subscriber. `Mutex<Vec<u8>>` is fine here because the tests are single-
/// threaded (no concurrent tracing from other threads during this test).
#[derive(Clone, Default)]
struct CapturingWriter {
    buf: Arc<Mutex<Vec<u8>>>,
}

impl Write for CapturingWriter {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.buf
            .lock()
            .expect("capturing writer mutex poisoned")
            .extend_from_slice(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl<'a> MakeWriter<'a> for CapturingWriter {
    type Writer = CapturingWriter;
    fn make_writer(&'a self) -> Self::Writer {
        self.clone()
    }
}

/// Static `OwnerLookup` for tests: `k1 -> user-A`. The production impl reads
/// from the live `RuntimeSnapshot`; tests substitute this to avoid standing
/// up the snapshot pipeline.
struct MapOwnerLookup(HashMap<String, String>);

impl OwnerLookup for MapOwnerLookup {
    fn owner_of(&self, key_id: &str) -> Option<String> {
        self.0.get(key_id).cloned()
    }
}

/// Exercise every Decrypt path and assert the plaintext marker never appears
/// in tracing logs or gRPC error messages (constitution XX). The successful
/// `view_plaintext` response IS allowed to contain the marker (the caller is
/// authorized to see it); everything else must be clean.
#[test]
fn no_plaintext_key_in_logs_or_errors() {
    // A single-threaded tokio runtime is enough: the KeyVault gRPC methods do
    // no I/O (AES-GCM ops are in-memory), so they complete synchronously on
    // the first poll.
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("tokio runtime for security_no_leak test");

    let writer = CapturingWriter::default();
    let subscriber = tracing_subscriber::fmt()
        .with_max_level(tracing::Level::TRACE)
        .with_writer(writer.clone())
        .with_ansi(false) // strip ANSI so the marker scan isn't confused by escapes
        .finish();

    // set_default scopes the subscriber to this thread for the duration of the
    // guard. Other threads are unaffected; tests in other files are unaffected.
    let _guard = tracing::subscriber::set_default(subscriber);

    // Build the service with a real AES-GCM keyvault and a static owner map
    // where k1 is owned by user-A.
    let kv = Arc::new(AesGcmKeyVault::from_master_key(&[42u8; 32]));
    let mut owners = HashMap::new();
    owners.insert("k1".to_string(), "user-A".to_string());
    let lookup: Arc<dyn OwnerLookup> = Arc::new(MapOwnerLookup(owners));
    let svc = KeyVaultGrpcService::new(kv.clone(), lookup);

    // Encrypt a known plaintext with the LEAK_MARKER embedded.
    let plaintext = format!("sk-test-{}-1234567890", LEAK_MARKER);
    let ct = kv
        .encrypt(plaintext.as_bytes())
        .expect("encrypt must succeed");

    // --- Scenario 1: owner success (view_plaintext) ---
    // The response plaintext IS allowed to contain the marker; logs must not.
    let resp = runtime
        .block_on(svc.decrypt(Request::new(DecryptRequest {
            ciphertext: ct.clone(),
            requester_user_id: "user-A".to_string(),
            key_id: "k1".to_string(),
            intent: INTENT_VIEW_PLAINTEXT.to_string(),
        })))
        .expect("owner view_plaintext must succeed");
    assert_eq!(
        resp.into_inner().plaintext,
        plaintext.as_bytes().to_vec(),
        "owner view_plaintext must return the plaintext"
    );

    // --- Scenario 2: non-owner denied (view_plaintext) ---
    // The error message must not contain the marker; logs must not either.
    let err = runtime
        .block_on(svc.decrypt(Request::new(DecryptRequest {
            ciphertext: ct.clone(),
            requester_user_id: "user-B".to_string(),
            key_id: "k1".to_string(),
            intent: INTENT_VIEW_PLAINTEXT.to_string(),
        })))
        .expect_err("non-owner view_plaintext must be denied");
    assert_eq!(
        err.code(),
        tonic::Code::PermissionDenied,
        "expected permission_denied; got {:?}: {}",
        err.code(),
        err.message()
    );
    assert!(
        !err.message().contains(LEAK_MARKER),
        "error message leaked plaintext marker: {}",
        err.message()
    );

    // --- Scenario 3: unknown key_id denied (view_plaintext) ---
    let err = runtime
        .block_on(svc.decrypt(Request::new(DecryptRequest {
            ciphertext: ct.clone(),
            requester_user_id: "user-A".to_string(),
            key_id: "missing-key".to_string(),
            intent: INTENT_VIEW_PLAINTEXT.to_string(),
        })))
        .expect_err("unknown key_id view_plaintext must be denied");
    assert_eq!(
        err.code(),
        tonic::Code::PermissionDenied,
        "expected permission_denied; got {:?}: {}",
        err.code(),
        err.message()
    );
    assert!(
        !err.message().contains(LEAK_MARKER),
        "error message leaked plaintext marker (unknown key): {}",
        err.message()
    );

    // --- Scenario 4: hot_path_inject success (skips owner check) ---
    // Requester is user-B but hot_path_inject is allowed; response plaintext
    // IS the marker (allowed); logs must not contain it.
    let resp = runtime
        .block_on(svc.decrypt(Request::new(DecryptRequest {
            ciphertext: ct,
            requester_user_id: "user-B".to_string(),
            key_id: "k1".to_string(),
            intent: INTENT_HOT_PATH_INJECT.to_string(),
        })))
        .expect("hot_path_inject must succeed");
    assert_eq!(
        resp.into_inner().plaintext,
        plaintext.as_bytes().to_vec(),
        "hot_path_inject must return the plaintext"
    );

    // --- The audit: scan captured tracing output for the marker. ---
    let captured = writer
        .buf
        .lock()
        .expect("capturing writer mutex poisoned")
        .clone();
    let captured_str = String::from_utf8_lossy(&captured);
    assert!(
        !captured_str.contains(LEAK_MARKER),
        "tracing output leaked plaintext marker; captured output:\n{}",
        captured_str
    );
}

/// Sanity: the AES-GCM ciphertext itself must not contain the plaintext as a
/// substring (this is a basic property of authenticated encryption, but worth
/// asserting explicitly as part of the no-leak audit).
#[test]
fn aesgcm_ciphertext_does_not_contain_plaintext_substring() {
    let kv = AesGcmKeyVault::from_master_key(&[42u8; 32]);
    let plaintext = format!("sk-test-{}-1234567890", LEAK_MARKER);
    let ct = kv
        .encrypt(plaintext.as_bytes())
        .expect("encrypt must succeed");

    let captured_str = String::from_utf8_lossy(&ct);
    assert!(
        !captured_str.contains(LEAK_MARKER),
        "AES-GCM ciphertext contains plaintext marker (catastrophic cipher failure)"
    );
    assert_ne!(
        &ct[..],
        plaintext.as_bytes(),
        "AES-GCM ciphertext must not equal plaintext"
    );
}

/// Suppress the unused-import warning for `Status` - it's re-exported via
/// tonic and used in the `expect_err` calls above as a return type, but rustc
/// can't always see that through the `.expect_err` desugaring.
#[allow(dead_code)]
fn _status_type_anchor(_s: Status) {}
