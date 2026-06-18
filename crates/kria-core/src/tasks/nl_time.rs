//! Natural-language time parsing (Phase 2 intelligence upgrade).
//!
//! English fast-path via the `interim` crate (maintained `chrono-english`
//! fork). For Hinglish / messy input the agent's LLM extraction (llama +
//! llguidance) supplies an ISO timestamp instead — see the plan doc.

use chrono::{DateTime, Utc};
use interim::{parse_date_string, Dialect};

/// Parse an English natural-language time expression ("tomorrow 5pm",
/// "next friday", "in 2 hours") relative to `now`. Returns `None` if it
/// isn't a recognisable English expression.
pub fn parse(text: &str, now: DateTime<Utc>) -> Option<DateTime<Utc>> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return None;
    }
    // Already an ISO timestamp? Use it directly.
    if let Ok(dt) = DateTime::parse_from_rfc3339(trimmed) {
        return Some(dt.with_timezone(&Utc));
    }
    parse_date_string(trimmed, now, Dialect::Us)
        .ok()
        .map(|dt| dt.with_timezone(&Utc))
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{Duration, TimeZone};

    fn now() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 6, 18, 9, 0, 0).unwrap()
    }

    #[test]
    fn parses_relative_day() {
        let r = parse("tomorrow", now()).unwrap();
        assert_eq!(r.date_naive(), (now() + Duration::days(1)).date_naive());
    }

    #[test]
    fn parses_iso_passthrough() {
        let r = parse("2026-06-19T10:00:00Z", now()).unwrap();
        assert_eq!(r, Utc.with_ymd_and_hms(2026, 6, 19, 10, 0, 0).unwrap());
    }

    #[test]
    fn rejects_garbage() {
        assert!(parse("kuch bhi random xyz", now()).is_none());
    }
}
