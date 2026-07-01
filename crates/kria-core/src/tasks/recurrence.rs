//! Reminder recurrence (Phase 2 intelligence upgrade).
//!
//! A deterministic, fully-testable recurrence model covering the common cases
//! (every-N-minutes, daily, weekly@weekday, monthly@day-of-month). Full
//! RFC-5545 (`rrule` crate) is a documented future upgrade; this enum handles
//! ~90% of real reminders without a fragile dependency.

use chrono::{DateTime, Datelike, Duration, Utc, Weekday};

/// How a reminder repeats.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Recurrence {
    None,
    /// Every N minutes.
    EveryMinutes(u32),
    /// Every day at the same time.
    Daily,
    /// Every week on the given weekday.
    Weekly(Weekday),
    /// Every month on the given day-of-month (1–31, clamped to month length).
    Monthly(u32),
}

impl Recurrence {
    /// Parse from the stored string form. `None`/empty → `Recurrence::None`.
    pub fn parse(s: Option<&str>) -> Recurrence {
        let raw = match s {
            Some(v) if !v.trim().is_empty() => v.trim().to_ascii_lowercase(),
            _ => return Recurrence::None,
        };
        match raw.as_str() {
            "daily" => Recurrence::Daily,
            "none" => Recurrence::None,
            other => {
                if let Some(rest) = other.strip_prefix("every:") {
                    return parse_every(rest);
                }
                if let Some(rest) = other.strip_prefix("weekly:") {
                    return weekday_from_str(rest)
                        .map(Recurrence::Weekly)
                        .unwrap_or(Recurrence::None);
                }
                if let Some(rest) = other.strip_prefix("monthly:") {
                    return rest
                        .parse::<u32>()
                        .ok()
                        .filter(|d| (1..=31).contains(d))
                        .map(Recurrence::Monthly)
                        .unwrap_or(Recurrence::None);
                }
                Recurrence::None
            }
        }
    }

    /// Serialize to the stored string form (`None` → `None`).
    pub fn to_storage(&self) -> Option<String> {
        match self {
            Recurrence::None => None,
            Recurrence::EveryMinutes(m) => Some(format!("every:{m}m")),
            Recurrence::Daily => Some("daily".into()),
            Recurrence::Weekly(wd) => Some(format!("weekly:{}", weekday_str(*wd))),
            Recurrence::Monthly(d) => Some(format!("monthly:{d}")),
        }
    }

    pub fn is_recurring(&self) -> bool {
        !matches!(self, Recurrence::None)
    }

    /// Next occurrence strictly after `from`, or `None` for non-recurring.
    pub fn next_after(&self, from: DateTime<Utc>) -> Option<DateTime<Utc>> {
        match self {
            Recurrence::None => None,
            Recurrence::EveryMinutes(m) => Some(from + Duration::minutes(*m as i64)),
            Recurrence::Daily => Some(from + Duration::days(1)),
            Recurrence::Weekly(wd) => {
                let mut next = from + Duration::days(1);
                while next.weekday() != *wd {
                    next += Duration::days(1);
                }
                Some(next)
            }
            Recurrence::Monthly(day) => Some(next_monthly(from, *day)),
        }
    }
}

fn parse_every(rest: &str) -> Recurrence {
    // forms: "30m", "30", "2h"
    let r = rest.trim();
    if let Some(h) = r.strip_suffix('h') {
        if let Ok(n) = h.trim().parse::<u32>() {
            return Recurrence::EveryMinutes(n * 60);
        }
    }
    let digits = r.trim_end_matches('m');
    digits
        .trim()
        .parse::<u32>()
        .ok()
        .filter(|n| *n > 0)
        .map(Recurrence::EveryMinutes)
        .unwrap_or(Recurrence::None)
}

fn next_monthly(from: DateTime<Utc>, day: u32) -> DateTime<Utc> {
    let (mut year, mut month) = (from.year(), from.month());
    // advance to next month
    if month == 12 {
        year += 1;
        month = 1;
    } else {
        month += 1;
    }
    let dim = days_in_month(year, month);
    let d = day.min(dim);
    // Build from NaiveDate + original time to avoid with_month/day ordering traps.
    match chrono::NaiveDate::from_ymd_opt(year, month, d) {
        Some(date) => DateTime::<Utc>::from_naive_utc_and_offset(date.and_time(from.time()), Utc),
        None => from + Duration::days(30),
    }
}

fn days_in_month(year: i32, month: u32) -> u32 {
    let (ny, nm) = if month == 12 {
        (year + 1, 1)
    } else {
        (year, month + 1)
    };
    let first_next = chrono::NaiveDate::from_ymd_opt(ny, nm, 1).unwrap();
    let first_this = chrono::NaiveDate::from_ymd_opt(year, month, 1).unwrap();
    (first_next - first_this).num_days() as u32
}

fn weekday_from_str(s: &str) -> Option<Weekday> {
    match s.trim().to_ascii_lowercase().as_str() {
        "mon" | "monday" => Some(Weekday::Mon),
        "tue" | "tuesday" => Some(Weekday::Tue),
        "wed" | "wednesday" => Some(Weekday::Wed),
        "thu" | "thursday" => Some(Weekday::Thu),
        "fri" | "friday" => Some(Weekday::Fri),
        "sat" | "saturday" => Some(Weekday::Sat),
        "sun" | "sunday" => Some(Weekday::Sun),
        _ => None,
    }
}

fn weekday_str(wd: Weekday) -> &'static str {
    match wd {
        Weekday::Mon => "mon",
        Weekday::Tue => "tue",
        Weekday::Wed => "wed",
        Weekday::Thu => "thu",
        Weekday::Fri => "fri",
        Weekday::Sat => "sat",
        Weekday::Sun => "sun",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn dt(y: i32, m: u32, d: u32, h: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(y, m, d, h, 0, 0).unwrap()
    }

    #[test]
    fn parse_and_roundtrip() {
        assert_eq!(Recurrence::parse(Some("daily")), Recurrence::Daily);
        assert_eq!(
            Recurrence::parse(Some("every:30m")),
            Recurrence::EveryMinutes(30)
        );
        assert_eq!(
            Recurrence::parse(Some("every:2h")),
            Recurrence::EveryMinutes(120)
        );
        assert_eq!(
            Recurrence::parse(Some("weekly:fri")),
            Recurrence::Weekly(Weekday::Fri)
        );
        assert_eq!(
            Recurrence::parse(Some("monthly:15")),
            Recurrence::Monthly(15)
        );
        assert_eq!(Recurrence::parse(None), Recurrence::None);
        assert_eq!(
            Recurrence::Weekly(Weekday::Fri).to_storage().as_deref(),
            Some("weekly:fri")
        );
    }

    #[test]
    fn daily_next() {
        let n = Recurrence::Daily.next_after(dt(2026, 6, 18, 9)).unwrap();
        assert_eq!(n, dt(2026, 6, 19, 9));
    }

    #[test]
    fn every_minutes_next() {
        let n = Recurrence::EveryMinutes(30)
            .next_after(dt(2026, 6, 18, 9))
            .unwrap();
        assert_eq!(n, dt(2026, 6, 18, 9) + Duration::minutes(30));
    }

    #[test]
    fn weekly_next_lands_on_weekday() {
        // 2026-06-18 is a Thursday; next Friday = 06-19.
        let n = Recurrence::Weekly(Weekday::Fri)
            .next_after(dt(2026, 6, 18, 9))
            .unwrap();
        assert_eq!(n.weekday(), Weekday::Fri);
        assert_eq!(n, dt(2026, 6, 19, 9));
    }

    #[test]
    fn monthly_next_clamps() {
        // Jan 31 → Feb (clamped to 28/29).
        let n = Recurrence::Monthly(31)
            .next_after(dt(2026, 1, 31, 9))
            .unwrap();
        assert_eq!(n.month(), 2);
        assert!(n.day() <= 29);
    }

    #[test]
    fn none_has_no_next() {
        assert!(Recurrence::None.next_after(dt(2026, 6, 18, 9)).is_none());
    }
}
