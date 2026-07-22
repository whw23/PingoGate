//! Platform-mode virtual-key authenticator (constitution XX; S3).
//!
//! [`VirtualKeyAuth`] implements [`crate::KeyAuth`] for platform mode: the
//! caller presents an opaque virtual-key token, the authenticator hashes it
//! with SHA-256, looks up the hash in `snapshot.virtual_keys` (constant-time
//! compare on the hex digest), checks `enabled` + `expires_at`, and enforces a
//! per-vkey concurrency quota via an in-memory atomic counter.
//!
//! Concurrency is tracked in a `Mutex<HashMap<String, AtomicI64>>` keyed by
//! virtual-key id. [`KeyAuth::authenticate`] increments the counter (CAS loop:
//! reject if `current >= max_concurrency`); [`KeyAuth::release`] decrements
//! it. The pipeline calls `release` exactly once per authenticated request, at
//! request end (success or error), so the counter never leaks.
//!
//! `max_concurrency == 0` means unlimited (no counter check, no increment).
//!
//! The hash is SHA-256 (not bcrypt) for hot-path performance: the presented
//! token has ~128 bits of entropy (PingoGate-generated `pg_vkey_…`), so a
//! fast hash is sufficient and brute-force is infeasible (constitution XX:
//! the token is a credential, not a password).

use std::collections::HashMap;
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::Mutex;

use pingogate_core_types::{now_unix_secs, Principal, PrincipalKind};
use pingogate_snapshot::RuntimeSnapshot;
use sha2::{Digest, Sha256};
use subtle::ConstantTimeEq;

use crate::key_auth::KeyAuth;

/// Platform-mode authenticator: SHA-256 hash + virtual-key lookup + concurrency
/// quota. Stateful (per-vkey atomic counter); the pipeline must call
/// [`KeyAuth::release`] at request end to decrement the counter.
pub struct VirtualKeyAuth {
    /// `vkey_id -> in-flight count`. Protected by a `Mutex` for entry
    /// lookup; the `AtomicI64` allows the check+increment CAS loop to run
    /// without holding the lock (the `Arc<AtomicI64>` is cloned out under the
    /// lock, then operated on lock-free).
    concurrency: Mutex<HashMap<String, std::sync::Arc<AtomicI64>>>,
}

impl VirtualKeyAuth {
    /// Construct with an empty concurrency map. The map grows lazily as new
    /// virtual keys authenticate; entries are never removed (vkey ids are
    /// bounded by the snapshot).
    pub fn new() -> Self {
        Self {
            concurrency: Mutex::new(HashMap::new()),
        }
    }

    /// Read out the current in-flight count for `vkey_id` (0 if unknown).
    /// Test-only: the hot path never reads the count after increment.
    #[cfg(test)]
    pub(crate) fn in_flight(&self, vkey_id: &str) -> i64 {
        let map = self.concurrency.lock().expect("concurrency map poisoned");
        map.get(vkey_id)
            .map(|c| c.load(Ordering::Relaxed))
            .unwrap_or(0)
    }

    /// CAS loop: increment `counter` only if `current < max`. Returns `true`
    /// on success, `false` if the quota is exhausted.
    fn try_increment(counter: &AtomicI64, max: i64) -> bool {
        let mut seen = counter.load(Ordering::Relaxed);
        loop {
            if seen >= max {
                return false;
            }
            match counter.compare_exchange(seen, seen + 1, Ordering::Relaxed, Ordering::Relaxed) {
                Ok(_) => return true,
                Err(actual) => seen = actual,
            }
        }
    }
}

impl Default for VirtualKeyAuth {
    fn default() -> Self {
        Self::new()
    }
}

impl KeyAuth for VirtualKeyAuth {
    fn authenticate(&self, credential: &str, snapshot: &RuntimeSnapshot) -> Option<Principal> {
        // 1. SHA-256(credential) -> hex digest.
        let digest = hex_sha256(credential.as_bytes());

        // 2. Find the virtual key whose token_hash matches (constant-time
        //    compare on the hex digest; the digest is already a hash, but
        //    ct_eq avoids leaking which prefix matched).
        let vk = snapshot.virtual_keys.iter().find(|k| {
            bool::from(digest.as_bytes().ct_eq(k.token_hash.as_bytes()))
        })?;

        // 3. Check enabled + expiry (virtual keys are mirrored verbatim from
        //    the proto, not filtered at build time, so these checks are live).
        if !vk.enabled {
            return None;
        }
        if vk.expires_at > 0 && now_unix_secs() as i64 >= vk.expires_at {
            return None;
        }

        // 4. Concurrency quota (max_concurrency == 0 means unlimited).
        if vk.max_concurrency > 0 {
            let counter = {
                let mut map = self.concurrency.lock().expect("concurrency map poisoned");
                map.entry(vk.id.clone())
                    .or_insert_with(|| std::sync::Arc::new(AtomicI64::new(0)))
                    .clone()
            };
            if !Self::try_increment(&counter, vk.max_concurrency as i64) {
                return None; // quota exhausted
            }
        }

        Some(Principal::virtual_key(&vk.id, &vk.owner_user_id))
    }

    fn release(&self, principal: &Principal) {
        // Only virtual-key principals carry a concurrency reservation.
        if principal.kind != PrincipalKind::VirtualKey {
            return;
        }
        let counter = {
            let map = self.concurrency.lock().expect("concurrency map poisoned");
            map.get(&principal.id).cloned()
        };
        if let Some(counter) = counter {
            // fetch_sub with saturating semantics: never go below 0 (a stray
            // double-release would underflow to a huge positive via wrapping,
            // so clamp at 0).
            let prev = counter.fetch_sub(1, Ordering::Relaxed);
            if prev <= 0 {
                // Underflow: another release raced ahead. Restore to 0.
                counter.store(0, Ordering::Relaxed);
            }
        }
    }
}

/// SHA-256 -> lowercase hex string (no `0x` prefix). Matches the Go control
/// plane's `token_hash` format (S3: `hex.EncodeToString(sha256(token))`).
fn hex_sha256(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    let digest = hasher.finalize();
    // Manual hex encode (no `hex` crate dependency; 32 bytes -> 64 hex chars).
    let mut out = String::with_capacity(64);
    for byte in digest {
        out.push(hex_nibble(byte >> 4));
        out.push(hex_nibble(byte & 0x0f));
    }
    out
}

fn hex_nibble(n: u8) -> char {
    match n {
        0..=9 => (b'0' + n) as char,
        10..=15 => (b'a' + (n - 10)) as char,
        _ => unreachable!("hex nibble is 0..=15"),
    }
}

#[cfg(test)]
#[path = "virtual_key_auth_tests.rs"]
mod tests;
