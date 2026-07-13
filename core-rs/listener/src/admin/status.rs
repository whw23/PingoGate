//! In-memory reload status, read lock-free by the Admin API (FR-020).
//!
//! Records the outcome of the most recent reload attempt so operators can tell,
//! after triggering a reload, whether the candidate config was applied or
//! rejected - and, on rejection, why. A failed attempt never advances
//! `active_version`: the previously active snapshot keeps serving (FR-021).

use std::sync::Arc;

use arc_swap::ArcSwap;
use pingogate_core_types::now_rfc3339;

/// Outcome of the most recent reload attempt.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReloadOutcome {
    /// No reload has been attempted since startup.
    Pending,
    /// The candidate config was validated, built, and swapped in.
    Success,
    /// The candidate was rejected; the active snapshot is unchanged.
    Failure,
}

impl ReloadOutcome {
    /// Stable lowercase label for JSON / logs.
    pub fn as_str(self) -> &'static str {
        match self {
            ReloadOutcome::Pending => "pending",
            ReloadOutcome::Success => "success",
            ReloadOutcome::Failure => "failure",
        }
    }
}

/// A point-in-time view of the reload subsystem.
#[derive(Debug, Clone)]
pub struct ReloadStatus {
    /// Version of the snapshot currently serving traffic.
    pub active_version: u64,
    /// Version assigned to the most recent attempt (success or failure).
    pub last_attempt_version: Option<u64>,
    pub outcome: ReloadOutcome,
    /// Human-readable detail; on failure, names what was wrong (never a secret).
    pub message: String,
    /// UTC RFC 3339 timestamp of when this status was recorded.
    pub timestamp: String,
    /// Total reload attempts since startup.
    pub attempts: u64,
}

/// Lock-free store for the latest [`ReloadStatus`], swapped atomically.
pub struct ReloadStatusStore {
    inner: ArcSwap<ReloadStatus>,
}

impl ReloadStatusStore {
    /// Create a store seeded with the bootstrap snapshot version.
    pub fn new(active_version: u64) -> Self {
        Self {
            inner: ArcSwap::from_pointee(ReloadStatus {
                active_version,
                last_attempt_version: None,
                outcome: ReloadOutcome::Pending,
                message: "no reload attempted since startup".to_string(),
                timestamp: now_rfc3339(),
                attempts: 0,
            }),
        }
    }

    /// Snapshot the current status for a lock-free read.
    pub fn current(&self) -> Arc<ReloadStatus> {
        self.inner.load_full()
    }

    /// Record a successful reload that advanced the active version.
    pub fn record_success(&self, version: u64) {
        let attempts = self.inner.load().attempts;
        self.inner.store(Arc::new(ReloadStatus {
            active_version: version,
            last_attempt_version: Some(version),
            outcome: ReloadOutcome::Success,
            message: "configuration reloaded".to_string(),
            timestamp: now_rfc3339(),
            attempts: attempts.saturating_add(1),
        }));
    }

    /// Record a rejected reload; `active_version` is preserved (FR-021).
    pub fn record_failure(&self, attempt_version: u64, message: impl Into<String>) {
        let prev = self.inner.load();
        self.inner.store(Arc::new(ReloadStatus {
            active_version: prev.active_version,
            last_attempt_version: Some(attempt_version),
            outcome: ReloadOutcome::Failure,
            message: message.into(),
            timestamp: now_rfc3339(),
            attempts: prev.attempts.saturating_add(1),
        }));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn starts_pending_at_bootstrap_version() {
        let store = ReloadStatusStore::new(1);
        let s = store.current();
        assert_eq!(s.active_version, 1);
        assert_eq!(s.outcome, ReloadOutcome::Pending);
        assert_eq!(s.attempts, 0);
        assert!(s.last_attempt_version.is_none());
    }

    #[test]
    fn success_advances_active_version() {
        let store = ReloadStatusStore::new(1);
        store.record_success(2);
        let s = store.current();
        assert_eq!(s.active_version, 2);
        assert_eq!(s.outcome, ReloadOutcome::Success);
        assert_eq!(s.attempts, 1);
    }

    #[test]
    fn failure_preserves_active_version_and_records_reason() {
        let store = ReloadStatusStore::new(3);
        store.record_failure(4, "routes[0].provider: unknown provider");
        let s = store.current();
        assert_eq!(s.active_version, 3);
        assert_eq!(s.last_attempt_version, Some(4));
        assert_eq!(s.outcome, ReloadOutcome::Failure);
        assert!(s.message.contains("unknown provider"));
        assert_eq!(s.attempts, 1);
    }
}
