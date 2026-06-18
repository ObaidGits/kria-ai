//! Calendar availability engine (Phase 1.2).
//!
//! Pure, dependency-light logic for turning a set of calendar events (busy
//! intervals) into **free slots** and **conflicts**. No network or MCP here —
//! this is the testable core that the `gw_calendar_availability` tool feeds
//! with events fetched from Google Calendar.
//!
//! Reused later by schedule intelligence (Phase 6).

use chrono::{DateTime, Utc};
use serde::Serialize;

/// A busy time block (typically a calendar event).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct BusyInterval {
    pub start: DateTime<Utc>,
    pub end: DateTime<Utc>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
}

/// A free gap between busy blocks.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct FreeSlot {
    pub start: DateTime<Utc>,
    pub end: DateTime<Utc>,
    pub minutes: i64,
}

/// Two overlapping events.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Conflict {
    pub first: BusyInterval,
    pub second: BusyInterval,
    pub overlap_minutes: i64,
}

/// Merge overlapping/adjacent busy intervals into disjoint `(start, end)` pairs,
/// sorted ascending.
pub fn merge_busy(intervals: &[BusyInterval]) -> Vec<(DateTime<Utc>, DateTime<Utc>)> {
    let mut spans: Vec<(DateTime<Utc>, DateTime<Utc>)> = intervals
        .iter()
        .filter(|i| i.end > i.start)
        .map(|i| (i.start, i.end))
        .collect();
    spans.sort_by_key(|(s, _)| *s);

    let mut merged: Vec<(DateTime<Utc>, DateTime<Utc>)> = Vec::new();
    for (start, end) in spans {
        if let Some(last) = merged.last_mut() {
            if start <= last.1 {
                if end > last.1 {
                    last.1 = end;
                }
                continue;
            }
        }
        merged.push((start, end));
    }
    merged
}

/// Compute free slots within `[window_start, window_end]` not covered by any
/// busy interval. Only slots of at least `min_minutes` are returned.
pub fn free_slots(
    window_start: DateTime<Utc>,
    window_end: DateTime<Utc>,
    busy: &[BusyInterval],
    min_minutes: i64,
) -> Vec<FreeSlot> {
    if window_end <= window_start {
        return Vec::new();
    }

    // Clamp busy spans to the window.
    let clamped: Vec<BusyInterval> = busy
        .iter()
        .filter_map(|i| {
            let s = i.start.max(window_start);
            let e = i.end.min(window_end);
            (e > s).then(|| BusyInterval {
                start: s,
                end: e,
                title: i.title.clone(),
            })
        })
        .collect();

    let merged = merge_busy(&clamped);

    let mut slots = Vec::new();
    let mut cursor = window_start;
    for (start, end) in merged {
        if start > cursor {
            push_slot(&mut slots, cursor, start, min_minutes);
        }
        if end > cursor {
            cursor = end;
        }
    }
    if cursor < window_end {
        push_slot(&mut slots, cursor, window_end, min_minutes);
    }
    slots
}

fn push_slot(out: &mut Vec<FreeSlot>, start: DateTime<Utc>, end: DateTime<Utc>, min_minutes: i64) {
    let minutes = (end - start).num_minutes();
    if minutes >= min_minutes {
        out.push(FreeSlot { start, end, minutes });
    }
}

/// Detect pairs of events that overlap in time (touching boundaries do not count).
pub fn detect_conflicts(busy: &[BusyInterval]) -> Vec<Conflict> {
    let mut sorted: Vec<&BusyInterval> = busy.iter().filter(|i| i.end > i.start).collect();
    sorted.sort_by_key(|i| i.start);

    let mut conflicts = Vec::new();
    for (i, a) in sorted.iter().enumerate() {
        for b in sorted.iter().skip(i + 1) {
            if b.start >= a.end {
                break; // sorted by start: no later event can overlap `a`
            }
            // overlap = [max(start), min(end))
            let overlap_start = a.start.max(b.start);
            let overlap_end = a.end.min(b.end);
            let overlap = (overlap_end - overlap_start).num_minutes();
            if overlap > 0 {
                conflicts.push(Conflict {
                    first: (*a).clone(),
                    second: (*b).clone(),
                    overlap_minutes: overlap,
                });
            }
        }
    }
    conflicts
}

/// Parse Google Calendar `listCalendarEvents` JSON into busy intervals.
///
/// Tolerant of the common shapes: an `items`/`events`/`results` array (or a
/// bare array) of event objects with `start`/`end` carrying either
/// `dateTime` (RFC3339) or `date` (all-day, treated as a full UTC day).
pub fn parse_google_events(payload: &serde_json::Value) -> Vec<BusyInterval> {
    let items = find_events_array(payload);
    let mut out = Vec::new();
    for item in items {
        if let (Some(start), Some(end)) = (
            parse_event_edge(item.get("start")),
            parse_event_edge(item.get("end")),
        ) {
            if end > start {
                out.push(BusyInterval {
                    start,
                    end,
                    title: item
                        .get("summary")
                        .or_else(|| item.get("title"))
                        .and_then(|v| v.as_str())
                        .map(|s| s.to_string()),
                });
            }
        }
    }
    out
}

fn find_events_array(payload: &serde_json::Value) -> Vec<serde_json::Value> {
    for key in ["items", "events", "results"] {
        if let Some(arr) = payload.get(key).and_then(|v| v.as_array()) {
            return arr.clone();
        }
    }
    if let Some(arr) = payload.as_array() {
        return arr.clone();
    }
    // Recurse one level into objects (e.g. {"data": {"items": [...]}}).
    if let Some(obj) = payload.as_object() {
        for v in obj.values() {
            let nested = find_events_array(v);
            if !nested.is_empty() {
                return nested;
            }
        }
    }
    Vec::new()
}

/// Parse a Google Calendar `start`/`end` edge. All-day `date` values map to
/// midnight UTC of that day; Google's `end.date` is already exclusive (the day
/// after the last full day), so no offset is applied.
fn parse_event_edge(edge: Option<&serde_json::Value>) -> Option<DateTime<Utc>> {
    let edge = edge?;
    if let Some(dt) = edge.get("dateTime").and_then(|v| v.as_str()) {
        if let Ok(parsed) = DateTime::parse_from_rfc3339(dt) {
            return Some(parsed.with_timezone(&Utc));
        }
    }
    if let Some(date) = edge.get("date").and_then(|v| v.as_str()) {
        if let Ok(naive) = chrono::NaiveDate::parse_from_str(date, "%Y-%m-%d") {
            let midnight = naive.and_hms_opt(0, 0, 0)?;
            return Some(DateTime::<Utc>::from_naive_utc_and_offset(midnight, Utc));
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{Duration, TimeZone};

    fn t(h: i64, m: i64) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 6, 18, 0, 0, 0).unwrap() + Duration::hours(h) + Duration::minutes(m)
    }

    fn ev(sh: i64, eh: i64, title: &str) -> BusyInterval {
        BusyInterval {
            start: t(sh, 0),
            end: t(eh, 0),
            title: Some(title.into()),
        }
    }

    #[test]
    fn merges_overlapping_busy() {
        let merged = merge_busy(&[ev(9, 10, "a"), ev(9, 11, "b"), ev(13, 14, "c")]);
        assert_eq!(merged.len(), 2);
        assert_eq!(merged[0], (t(9, 0), t(11, 0)));
        assert_eq!(merged[1], (t(13, 0), t(14, 0)));
    }

    #[test]
    fn computes_free_slots_between_meetings() {
        let busy = vec![ev(10, 11, "standup"), ev(14, 15, "review")];
        let slots = free_slots(t(9, 0), t(18, 0), &busy, 30);
        // free: 9-10, 11-14, 15-18
        assert_eq!(slots.len(), 3);
        assert_eq!((slots[0].start, slots[0].end), (t(9, 0), t(10, 0)));
        assert_eq!((slots[1].start, slots[1].end), (t(11, 0), t(14, 0)));
        assert_eq!((slots[2].start, slots[2].end), (t(15, 0), t(18, 0)));
        assert_eq!(slots[1].minutes, 180);
    }

    #[test]
    fn min_minutes_filters_small_gaps() {
        let busy = vec![ev(9, 10, "a"), ev(10, 11, "b")]; // back-to-back, no gap
        let slots = free_slots(t(9, 0), t(11, 0), &busy, 15);
        assert!(slots.is_empty());
    }

    #[test]
    fn detects_overlapping_conflict() {
        let busy = vec![ev(9, 11, "a"), ev(10, 12, "b"), ev(13, 14, "c")];
        let conflicts = detect_conflicts(&busy);
        assert_eq!(conflicts.len(), 1);
        assert_eq!(conflicts[0].overlap_minutes, 60); // 10-11
    }

    #[test]
    fn touching_events_are_not_conflicts() {
        let busy = vec![ev(9, 10, "a"), ev(10, 11, "b")];
        assert!(detect_conflicts(&busy).is_empty());
    }

    #[test]
    fn parses_google_calendar_payload() {
        let payload = serde_json::json!({
            "items": [
                { "summary": "Standup",
                  "start": { "dateTime": "2026-06-18T10:00:00Z" },
                  "end":   { "dateTime": "2026-06-18T10:30:00Z" } },
                { "summary": "All hands",
                  "start": { "date": "2026-06-18" },
                  "end":   { "date": "2026-06-19" } }
            ]
        });
        let events = parse_google_events(&payload);
        assert_eq!(events.len(), 2);
        assert_eq!(events[0].title.as_deref(), Some("Standup"));
        assert_eq!(events[0].start, t(10, 0));
        assert_eq!(events[0].end, t(10, 30));
        // all-day spans the full UTC day
        assert_eq!(events[1].start, t(0, 0));
        assert_eq!(events[1].end, t(24, 0));
    }

    #[test]
    fn parses_nested_data_array() {
        let payload = serde_json::json!({
            "data": { "events": [
                { "summary": "X",
                  "start": { "dateTime": "2026-06-18T09:00:00Z" },
                  "end":   { "dateTime": "2026-06-18T09:45:00Z" } }
            ]}
        });
        let events = parse_google_events(&payload);
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].end - events[0].start, Duration::minutes(45));
    }
}
