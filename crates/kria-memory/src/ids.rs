//! Identifiers, timestamps, hashing, and the Hybrid Logical Clock (HLC).
//!
//! Memory-upgrade design §14/§17 + architecture N10/D-15: event ordering uses a
//! monotonic HLC + UUID v7, **never** wall-clock comparison, so ordering stays
//! correct under clock drift, DST changes, and timezone travel.

use std::sync::Mutex;

use chrono::{DateTime, Utc};
use uuid::Uuid;

/// A UTC instant. All timestamps in the memory system are stored in UTC; the
/// originating timezone offset is stored separately on events (design §14).
pub type Timestamp = DateTime<Utc>;

/// Generate a fresh time-ordered identifier (UUID v7). Used for events,
/// memories, sessions, episodes, etc.
#[inline]
pub fn new_id() -> Uuid {
    Uuid::now_v7()
}

/// BLAKE3 content/payload hash, hex-encoded. Used for `content_hash`,
/// idempotent outbox keys, and event payload checksums (design §14).
#[inline]
pub fn blake3_hex(bytes: &[u8]) -> String {
    blake3::hash(bytes).to_hex().to_string()
}

/// Normalize text for stable dedup hashing: NFC normalization + trim +
/// collapse internal whitespace + lowercase. Ensures equivalent Unicode forms
/// hash identically (architecture §38.10 i18n Unicode hygiene).
pub fn normalized_content_hash(content: &str) -> String {
    use unicode_normalization::UnicodeNormalization;
    let nfc: String = content.nfc().collect();
    let collapsed = nfc.split_whitespace().collect::<Vec<_>>().join(" ");
    blake3_hex(collapsed.to_lowercase().as_bytes())
}

/// A Hybrid Logical Clock timestamp: a wall-clock millisecond component plus a
/// logical counter that guarantees strict monotonicity even when the wall clock
/// stalls or jumps backward.
///
/// Encoded as a fixed-width, lexicographically-sortable hex string so that
/// `ORDER BY hlc` in SQLite reproduces causal order (design §14 event ordering).
#[derive(
    Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, serde::Serialize, serde::Deserialize,
)]
pub struct Hlc {
    /// Wall-clock milliseconds since the Unix epoch (monotonic-guarded).
    pub wall_ms: u64,
    /// Logical counter, incremented when two ticks share a `wall_ms`.
    pub counter: u32,
}

impl Hlc {
    /// The zero clock (before any event).
    pub const ZERO: Hlc = Hlc {
        wall_ms: 0,
        counter: 0,
    };

    /// Fixed-width, sortable encoding: 16 hex digits of `wall_ms` (u64) followed
    /// by 8 hex digits of `counter` (u32). String order == `(wall_ms, counter)`
    /// order.
    pub fn encode(&self) -> String {
        format!("{:016x}{:08x}", self.wall_ms, self.counter)
    }

    /// Parse a previously-encoded HLC. Returns `None` on malformed input.
    pub fn decode(s: &str) -> Option<Hlc> {
        if s.len() != 24 {
            return None;
        }
        let wall_ms = u64::from_str_radix(&s[0..16], 16).ok()?;
        let counter = u32::from_str_radix(&s[16..24], 16).ok()?;
        Some(Hlc { wall_ms, counter })
    }
}

impl std::fmt::Display for Hlc {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.encode())
    }
}

/// Thread-safe monotonic HLC generator. There is one per process; the memory
/// system's single writer drives it, but it is safe to share across the read
/// pool for read-time comparisons.
#[derive(Debug)]
pub struct HlcGenerator {
    last: Mutex<Hlc>,
}

impl HlcGenerator {
    pub fn new() -> Self {
        Self {
            last: Mutex::new(Hlc::ZERO),
        }
    }

    /// Restore a generator from the highest HLC seen so far (e.g. read from the
    /// event log on startup) so post-restart ticks continue monotonically.
    pub fn from_last(last: Hlc) -> Self {
        Self {
            last: Mutex::new(last),
        }
    }

    /// Produce the next strictly-increasing HLC given the current wall clock.
    ///
    /// Monotonicity rules (drift/DST/backward-jump safe, architecture N10):
    /// * `wall_ms = max(now_ms, last.wall_ms)` — never goes backward.
    /// * if `wall_ms == last.wall_ms` → `counter += 1`; else `counter = 0`.
    pub fn tick(&self, now: Timestamp) -> Hlc {
        let now_ms = now.timestamp_millis().max(0) as u64;
        let mut guard = self.last.lock().unwrap_or_else(|p| p.into_inner());
        let next = if now_ms > guard.wall_ms {
            Hlc {
                wall_ms: now_ms,
                counter: 0,
            }
        } else {
            Hlc {
                wall_ms: guard.wall_ms,
                counter: guard.counter.saturating_add(1),
            }
        };
        *guard = next;
        next
    }

    /// Convenience: tick against the real wall clock now.
    pub fn now(&self) -> Hlc {
        self.tick(Utc::now())
    }
}

impl Default for HlcGenerator {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;
    use proptest::prelude::*;

    #[test]
    fn encode_is_sortable() {
        let a = Hlc {
            wall_ms: 1000,
            counter: 5,
        };
        let b = Hlc {
            wall_ms: 1000,
            counter: 6,
        };
        let c = Hlc {
            wall_ms: 1001,
            counter: 0,
        };
        assert!(a.encode() < b.encode());
        assert!(b.encode() < c.encode());
        assert_eq!(Hlc::decode(&a.encode()), Some(a));
    }

    #[test]
    fn backward_clock_jump_stays_monotonic() {
        let gen = HlcGenerator::new();
        let t1 = Utc.timestamp_opt(1_000, 0).unwrap();
        let t0 = Utc.timestamp_opt(500, 0).unwrap(); // jumps backward
        let a = gen.tick(t1);
        let b = gen.tick(t0); // backward wall clock
        let c = gen.tick(t0);
        assert!(a < b, "HLC must not go backward on clock rewind");
        assert!(b < c, "counter must advance when wall stalls/rewinds");
    }

    proptest! {
        /// CP-18: for any sequence of (possibly non-monotonic) wall-clock
        /// readings, the emitted HLC sequence is strictly increasing and its
        /// encoded string order matches emission order.
        #[test]
        fn hlc_strictly_increasing_under_drift(millis in proptest::collection::vec(0i64..4_000_000_000i64, 1..200)) {
            let gen = HlcGenerator::new();
            let mut prev: Option<Hlc> = None;
            let mut prev_enc: Option<String> = None;
            for ms in millis {
                let ts = Utc.timestamp_millis_opt(ms).single().unwrap_or_else(Utc::now);
                let h = gen.tick(ts);
                if let Some(p) = prev {
                    prop_assert!(h > p, "HLC not strictly increasing: {:?} !> {:?}", h, p);
                }
                let enc = h.encode();
                if let Some(pe) = &prev_enc {
                    prop_assert!(&enc > pe, "encoded HLC order broke monotonicity");
                }
                prev = Some(h);
                prev_enc = Some(enc);
            }
        }
    }
}
