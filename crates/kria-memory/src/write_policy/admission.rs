//! Write admission control — write-storm guard (memory-upgrade design §47.5).
//!
//! Ambient / high-frequency sources (file watchers, desktop context, GUI loops)
//! are debounced + coalesced by `(source, entity)` so 1000 ticks become one
//! observation. `TriggerProvenance::User` (i.e. [`Source::User`]) writes are
//! **never** throttled. This is the minimal MVP form (no multi-tier token
//! buckets); it protects the fast-path latency budget and bounded growth (R20).

use std::time::{Duration, Instant};

use dashmap::DashMap;

use crate::types::Source;

/// Admission outcome for a candidate.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Admit {
    /// Accept and persist now.
    Accept,
    /// Coalesced/throttled — a recent equivalent write already happened.
    Throttled,
}

/// Per-source debounce registry.
#[derive(Debug)]
pub struct Admission {
    last_seen: DashMap<String, Instant>,
    debounce: Duration,
}

impl Admission {
    pub fn new(debounce: Duration) -> Self {
        Self {
            last_seen: DashMap::new(),
            debounce,
        }
    }

    /// Decide whether to admit a write from `source` about `entity_key`.
    ///
    /// * `Source::User` → always [`Admit::Accept`] (never throttle the user).
    /// * Other sources → [`Admit::Throttled`] if an equivalent `(source, key)`
    ///   was admitted within the debounce window; otherwise accept and record.
    pub fn admit(&self, source: &Source, entity_key: &str) -> Admit {
        if matches!(source, Source::User) {
            return Admit::Accept;
        }
        let key = format!("{}::{}", source.tag(), entity_key);
        let now = Instant::now();
        if let Some(prev) = self.last_seen.get(&key) {
            if now.duration_since(*prev) < self.debounce {
                return Admit::Throttled;
            }
        }
        self.last_seen.insert(key, now);
        Admit::Accept
    }

    /// Drop debounce entries older than a horizon to bound memory (called by
    /// the scheduler's maintenance sweep; safe to call anytime).
    pub fn evict_older_than(&self, horizon: Duration) {
        let now = Instant::now();
        self.last_seen
            .retain(|_, t| now.duration_since(*t) < horizon);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn user_never_throttled() {
        let a = Admission::new(Duration::from_secs(60));
        assert_eq!(a.admit(&Source::User, "x"), Admit::Accept);
        assert_eq!(a.admit(&Source::User, "x"), Admit::Accept);
    }

    #[test]
    fn ambient_source_debounced_by_key() {
        let a = Admission::new(Duration::from_secs(60));
        let s = Source::Tool("file_watcher".into());
        assert_eq!(a.admit(&s, "/tmp/a"), Admit::Accept);
        assert_eq!(a.admit(&s, "/tmp/a"), Admit::Throttled); // same key, within window
        assert_eq!(a.admit(&s, "/tmp/b"), Admit::Accept); // different key
    }

    #[test]
    fn expired_window_admits_again() {
        let a = Admission::new(Duration::from_millis(1));
        let s = Source::Tool("desktop".into());
        assert_eq!(a.admit(&s, "focus"), Admit::Accept);
        std::thread::sleep(Duration::from_millis(5));
        assert_eq!(a.admit(&s, "focus"), Admit::Accept);
    }
}
