//! Persistent nonce replay semantics for the broker.
//!
//! linux-os-control-production **Task 1.5**, design §12 (OSC-001, OSC-007).
//!
//! Replay storage is keyed by **caller binding + nonce** and persists through
//! the request expiry window (design §12). A replay never dispatches:
//!
//! * If a completed response is already cached for the key, the broker returns
//!   that **identical bound response** (idempotent completion).
//! * If the key is reserved but not yet completed (an in-flight duplicate), the
//!   broker returns [`super::protocol::BrokerPreDispatchError::ReplayDetected`].
//! * A fresh key is reserved before dispatch and completed afterwards.
//!
//! The trait allows a durable SQLite-backed store in the live composition; the
//! in-memory implementation here backs deny-live tests.

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::SystemTime;

/// The result of reserving a `(caller_binding, nonce)` key before dispatch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReplayCheck {
    /// The key was unseen (or its window expired) and is now reserved; the
    /// broker may proceed toward dispatch.
    Fresh,
    /// The key is reserved but has no completed response yet (an in-flight
    /// duplicate); the broker returns `ReplayDetected` without dispatching.
    ReplayInFlight,
    /// A completed response is cached; the broker returns this identical frame
    /// without re-dispatching.
    ReplayCompleted(Vec<u8>),
}

/// Persistent replay store keyed by caller binding + nonce.
pub trait NonceReplayStore: Send + Sync {
    /// Atomically check the key and, when fresh, reserve it through `expires_at`.
    fn check_and_reserve(
        &self,
        caller_binding_hex: &str,
        nonce: &str,
        expires_at: SystemTime,
        now: SystemTime,
    ) -> ReplayCheck;

    /// Record the completed response frame for a reserved key so a later replay
    /// returns the identical bound response.
    fn record_completion(&self, caller_binding_hex: &str, nonce: &str, response_frame: Vec<u8>);
}

#[derive(Debug, Clone)]
struct ReplayEntry {
    expires_at: SystemTime,
    completed: Option<Vec<u8>>,
}

/// An in-memory [`NonceReplayStore`] for deny-live tests.
#[derive(Default)]
pub struct InMemoryNonceStore {
    entries: Mutex<HashMap<String, ReplayEntry>>,
}

impl InMemoryNonceStore {
    /// Create an empty store.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    fn key(caller_binding_hex: &str, nonce: &str) -> String {
        format!("{caller_binding_hex}\x1f{nonce}")
    }

    /// Number of live (unexpired-tracking) entries; diagnostics only.
    #[must_use]
    pub fn len(&self) -> usize {
        self.entries.lock().expect("nonce store poisoned").len()
    }

    /// Whether the store is empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

impl NonceReplayStore for InMemoryNonceStore {
    fn check_and_reserve(
        &self,
        caller_binding_hex: &str,
        nonce: &str,
        expires_at: SystemTime,
        now: SystemTime,
    ) -> ReplayCheck {
        let key = Self::key(caller_binding_hex, nonce);
        let mut map = self.entries.lock().expect("nonce store poisoned");

        // Purge entries whose expiry window has fully elapsed so a key can be
        // reused only after it can no longer be replayed.
        map.retain(|_, e| e.expires_at > now);

        match map.get(&key) {
            Some(entry) => match &entry.completed {
                Some(frame) => ReplayCheck::ReplayCompleted(frame.clone()),
                None => ReplayCheck::ReplayInFlight,
            },
            None => {
                map.insert(
                    key,
                    ReplayEntry {
                        expires_at,
                        completed: None,
                    },
                );
                ReplayCheck::Fresh
            }
        }
    }

    fn record_completion(&self, caller_binding_hex: &str, nonce: &str, response_frame: Vec<u8>) {
        let key = Self::key(caller_binding_hex, nonce);
        let mut map = self.entries.lock().expect("nonce store poisoned");
        if let Some(entry) = map.get_mut(&key) {
            entry.completed = Some(response_frame);
        }
    }
}

#[cfg(all(test, feature = "os-control-test"))]
mod tests {
    use super::*;
    use std::time::Duration;

    fn future() -> SystemTime {
        SystemTime::now() + Duration::from_secs(60)
    }

    #[test]
    fn fresh_then_inflight_then_completed() {
        let store = InMemoryNonceStore::new();
        let now = SystemTime::now();
        let exp = future();

        assert_eq!(
            store.check_and_reserve("caller", "nonce", exp, now),
            ReplayCheck::Fresh
        );
        // Duplicate before completion is an in-flight replay.
        assert_eq!(
            store.check_and_reserve("caller", "nonce", exp, now),
            ReplayCheck::ReplayInFlight
        );
        // After completion, a replay returns the identical cached frame.
        store.record_completion("caller", "nonce", vec![1, 2, 3]);
        assert_eq!(
            store.check_and_reserve("caller", "nonce", exp, now),
            ReplayCheck::ReplayCompleted(vec![1, 2, 3])
        );
    }

    #[test]
    fn distinct_caller_or_nonce_is_independent() {
        let store = InMemoryNonceStore::new();
        let now = SystemTime::now();
        let exp = future();
        assert_eq!(
            store.check_and_reserve("caller-a", "nonce", exp, now),
            ReplayCheck::Fresh
        );
        assert_eq!(
            store.check_and_reserve("caller-b", "nonce", exp, now),
            ReplayCheck::Fresh
        );
        assert_eq!(
            store.check_and_reserve("caller-a", "other", exp, now),
            ReplayCheck::Fresh
        );
    }

    #[test]
    fn expired_window_allows_reservation_again() {
        let store = InMemoryNonceStore::new();
        let past = SystemTime::now() - Duration::from_secs(10);
        let now = SystemTime::now();
        // Reserve with an already-past expiry.
        assert_eq!(
            store.check_and_reserve("caller", "nonce", past, past - Duration::from_secs(1)),
            ReplayCheck::Fresh
        );
        // A later check past the window purges and treats it as fresh again.
        assert_eq!(
            store.check_and_reserve("caller", "nonce", future(), now),
            ReplayCheck::Fresh
        );
    }
}
