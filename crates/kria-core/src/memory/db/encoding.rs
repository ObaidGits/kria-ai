//! Canonical value encodings enforced at the authority write boundary
//! (design §4: canonical time / UUID / boolean encodings).
//!
//! These conventions are *also* enforced structurally by schema `CHECK`
//! constraints, but the Rust write path canonicalises/validates values **before**
//! they reach SQLite so a bad value fails with a clear domain error rather than a
//! bare constraint violation. Keeping the invariant anchored here (next to the
//! connection-open pragma assertions) documents the single boundary.
//!
//! Conventions:
//! * **UUID** — stored lower-case, hyphenated `8-4-4-4-12` (see
//!   [`crate::memory::ids::new_id`], which emits UUID v7). [`canonical_uuid`]
//!   validates the shape and returns the canonical lower-case form.
//! * **Time** — stored as RFC 3339 in **UTC** (`…Z` or `+00:00`); the
//!   originating timezone offset is stored separately on events (design §14).
//!   [`assert_rfc3339_utc`] rejects non-UTC / malformed timestamps.
//! * **Boolean** — stored as `INTEGER` `0` / `1` (enforced by schema `CHECK`).
//!   [`canonical_bool`] maps a Rust `bool` to that integer for write helpers.

use crate::memory::error::{MemoryResult, StorageError};

/// Validate a UUID string and return its canonical lower-case hyphenated form
/// (`8-4-4-4-12`). Accepts mixed-case hex input (normalising to lower-case) but
/// rejects any other shape: wrong length, misplaced/missing hyphens, non-hex
/// characters, braces, or URN prefixes.
///
/// Hand-rolled (no new dependency) so the accepted format is exactly the
/// canonical one the authority stores — `Uuid::parse_str` is intentionally more
/// permissive than we want here.
pub fn canonical_uuid(s: &str) -> MemoryResult<String> {
    // Canonical form is exactly 36 chars: 32 hex digits + 4 hyphens.
    if s.len() != 36 {
        return Err(encoding_err(format!(
            "uuid must be 36 chars (8-4-4-4-12), got {} in {s:?}",
            s.len()
        )));
    }
    let bytes = s.as_bytes();
    // Hyphens at fixed positions.
    for &pos in &[8usize, 13, 18, 23] {
        if bytes[pos] != b'-' {
            return Err(encoding_err(format!(
                "uuid missing hyphen at pos {pos}: {s:?}"
            )));
        }
    }
    // Every other char must be a hex digit.
    for (i, &b) in bytes.iter().enumerate() {
        if matches!(i, 8 | 13 | 18 | 23) {
            continue;
        }
        if !b.is_ascii_hexdigit() {
            return Err(encoding_err(format!(
                "uuid has non-hex char {:?} at pos {i}: {s:?}",
                b as char
            )));
        }
    }
    Ok(s.to_ascii_lowercase())
}

/// Assert a timestamp string is RFC 3339 **in UTC** (offset zero, i.e. `…Z` or
/// `+00:00`). All authority timestamps are stored in UTC (design §14); a value
/// carrying a non-zero offset would corrupt cross-event ordering.
pub fn assert_rfc3339_utc(s: &str) -> MemoryResult<()> {
    let parsed = chrono::DateTime::parse_from_rfc3339(s)
        .map_err(|e| encoding_err(format!("timestamp {s:?} is not valid RFC 3339: {e}")))?;
    if parsed.offset().local_minus_utc() != 0 {
        return Err(encoding_err(format!(
            "timestamp {s:?} is not UTC (offset {} min)",
            parsed.offset().local_minus_utc() / 60
        )));
    }
    Ok(())
}

/// The canonical integer encoding for a boolean column (`0` / `1`). Booleans are
/// stored as `INTEGER` and constrained to `IN (0,1)` by schema `CHECK`s; write
/// helpers use this so the convention lives in one place.
#[inline]
pub fn canonical_bool(value: bool) -> i64 {
    value as i64
}

fn encoding_err(msg: String) -> crate::memory::error::MemoryError {
    StorageError::Encoding(msg).into()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn canonical_uuid_accepts_lower_case() {
        let u = "018f4e2a-1c3b-7d4e-8f90-abcdef012345";
        assert_eq!(canonical_uuid(u).unwrap(), u);
    }

    #[test]
    fn canonical_uuid_normalizes_upper_case() {
        let upper = "018F4E2A-1C3B-7D4E-8F90-ABCDEF012345";
        let lower = "018f4e2a-1c3b-7d4e-8f90-abcdef012345";
        assert_eq!(canonical_uuid(upper).unwrap(), lower);
    }

    #[test]
    fn canonical_uuid_matches_generated_ids() {
        // The authority's own id generator must round-trip through the validator.
        let id = crate::memory::ids::new_id().to_string();
        assert_eq!(canonical_uuid(&id).unwrap(), id);
    }

    #[test]
    fn canonical_uuid_rejects_garbage() {
        assert!(canonical_uuid("not-a-uuid").is_err());
        assert!(canonical_uuid("").is_err());
        // Wrong length (35 chars).
        assert!(canonical_uuid("018f4e2a-1c3b-7d4e-8f90-abcdef01234").is_err());
        // Non-hex character.
        assert!(canonical_uuid("018f4e2a-1c3b-7d4e-8f90-abcdef012zzz").is_err());
        // Braced form is not canonical.
        assert!(canonical_uuid("{018f4e2a-1c3b-7d4e-8f90-abcdef012345}").is_err());
        // Missing hyphen (32 hex, no separators, padded to 36 with spaces).
        assert!(canonical_uuid("018f4e2a1c3b7d4e8f90abcdef012345    ").is_err());
    }

    #[test]
    fn rfc3339_utc_accepts_zulu_and_zero_offset() {
        assert!(assert_rfc3339_utc("2026-01-01T00:00:00Z").is_ok());
        assert!(assert_rfc3339_utc("2026-01-01T00:00:00+00:00").is_ok());
        assert!(assert_rfc3339_utc("2026-01-01T12:34:56.789Z").is_ok());
    }

    #[test]
    fn rfc3339_utc_rejects_non_utc_and_garbage() {
        // Non-zero offset.
        assert!(assert_rfc3339_utc("2026-01-01T00:00:00+05:30").is_err());
        assert!(assert_rfc3339_utc("2026-01-01T00:00:00-08:00").is_err());
        // Not RFC 3339 at all.
        assert!(assert_rfc3339_utc("2026-01-01 00:00:00").is_err());
        assert!(assert_rfc3339_utc("garbage").is_err());
        assert!(assert_rfc3339_utc("").is_err());
    }

    #[test]
    fn bool_encodes_to_zero_one() {
        assert_eq!(canonical_bool(false), 0);
        assert_eq!(canonical_bool(true), 1);
    }
}
