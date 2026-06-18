//! Deterministic task priority engine (Phase 2.2).
//!
//! Pure, testable classification: maps a task's status, due date, and text into
//! a [`PriorityBucket`] plus a numeric score for ordering. No LLM, no I/O.
//! LLM-based refinement is a later enhancement (Phase 6).

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// Coarse priority category used for grouping in the UI.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PriorityBucket {
    Urgent,
    Important,
    Blocked,
    Waiting,
    Normal,
}

impl PriorityBucket {
    pub fn as_str(&self) -> &'static str {
        match self {
            PriorityBucket::Urgent => "urgent",
            PriorityBucket::Important => "important",
            PriorityBucket::Blocked => "blocked",
            PriorityBucket::Waiting => "waiting",
            PriorityBucket::Normal => "normal",
        }
    }

    pub fn from_str(s: &str) -> PriorityBucket {
        match s.trim().to_ascii_lowercase().as_str() {
            "urgent" => PriorityBucket::Urgent,
            "important" => PriorityBucket::Important,
            "blocked" => PriorityBucket::Blocked,
            "waiting" => PriorityBucket::Waiting,
            _ => PriorityBucket::Normal,
        }
    }
}

const URGENT_KEYWORDS: &[&str] = &[
    "asap",
    "urgent",
    "deadline",
    "emergency",
    "immediately",
    "today",
    "right now",
    "critical",
];

/// Classify a task into a (bucket, score). Higher score sorts first.
///
/// Rules (first match wins for status; otherwise due/keyword driven):
/// - `blocked` status → Blocked
/// - `waiting` status → Waiting
/// - `done`/`cancelled` → Normal, score 0
/// - overdue or due within 24h → Urgent
/// - due within 72h or urgent keyword → Important
/// - otherwise Normal
pub fn classify(
    status: &str,
    due_at: Option<DateTime<Utc>>,
    text: &str,
    now: DateTime<Utc>,
) -> (PriorityBucket, i64) {
    let status = status.trim().to_ascii_lowercase();
    match status.as_str() {
        "blocked" => return (PriorityBucket::Blocked, 700),
        "waiting" => return (PriorityBucket::Waiting, 400),
        "done" | "cancelled" => return (PriorityBucket::Normal, 0),
        _ => {}
    }

    let lower = text.to_ascii_lowercase();
    let has_urgent_keyword = URGENT_KEYWORDS.iter().any(|k| lower.contains(k));
    let in_progress_bonus = if status == "in_progress" { 50 } else { 0 };

    if let Some(due) = due_at {
        let minutes_until = (due - now).num_minutes();
        if minutes_until < 0 {
            // Overdue: the more overdue, the higher (capped).
            let overdue = (-minutes_until).min(10_000);
            return (PriorityBucket::Urgent, 1000 + overdue + in_progress_bonus);
        }
        if minutes_until <= 24 * 60 {
            return (PriorityBucket::Urgent, 900 + in_progress_bonus);
        }
        if minutes_until <= 72 * 60 {
            return (PriorityBucket::Important, 600 + in_progress_bonus);
        }
    }

    if has_urgent_keyword {
        return (PriorityBucket::Important, 600 + in_progress_bonus);
    }

    (PriorityBucket::Normal, 200 + in_progress_bonus)
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Duration;

    fn now() -> DateTime<Utc> {
        Utc::now()
    }

    #[test]
    fn overdue_is_urgent() {
        let (bucket, score) = classify("open", Some(now() - Duration::hours(2)), "pay bill", now());
        assert_eq!(bucket, PriorityBucket::Urgent);
        assert!(score > 1000);
    }

    #[test]
    fn due_soon_is_urgent() {
        let (bucket, _) = classify("open", Some(now() + Duration::hours(3)), "call", now());
        assert_eq!(bucket, PriorityBucket::Urgent);
    }

    #[test]
    fn due_this_week_is_important() {
        let (bucket, _) = classify("open", Some(now() + Duration::hours(48)), "review", now());
        assert_eq!(bucket, PriorityBucket::Important);
    }

    #[test]
    fn blocked_status_wins() {
        let (bucket, _) = classify("blocked", Some(now() - Duration::hours(5)), "x", now());
        assert_eq!(bucket, PriorityBucket::Blocked);
    }

    #[test]
    fn waiting_status_wins() {
        let (bucket, _) = classify("waiting", None, "x", now());
        assert_eq!(bucket, PriorityBucket::Waiting);
    }

    #[test]
    fn keyword_promotes_to_important() {
        let (bucket, _) = classify("open", None, "Finish report ASAP", now());
        assert_eq!(bucket, PriorityBucket::Important);
    }

    #[test]
    fn plain_task_is_normal() {
        let (bucket, score) = classify("open", None, "tidy desk", now());
        assert_eq!(bucket, PriorityBucket::Normal);
        assert_eq!(score, 200);
    }

    #[test]
    fn done_is_zero() {
        let (bucket, score) = classify("done", Some(now() - Duration::hours(1)), "x", now());
        assert_eq!(bucket, PriorityBucket::Normal);
        assert_eq!(score, 0);
    }

    #[test]
    fn urgent_outranks_important_outranks_normal() {
        let (_, urgent) = classify("open", Some(now() + Duration::hours(1)), "", now());
        let (_, important) = classify("open", Some(now() + Duration::hours(48)), "", now());
        let (_, normal) = classify("open", None, "", now());
        assert!(urgent > important);
        assert!(important > normal);
    }
}
