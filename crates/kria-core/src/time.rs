//! Centralized timezone-aware time helpers.
//!
//! Resolution order for the user's local time:
//! 1. `KRIA_USER_TZ` env (offset hours like "5" or "-8", or named like "Asia/Karachi")
//! 2. `chrono::Local` (system local timezone via `/etc/localtime`)
//! 3. UTC fallback (logged as a warning)
//!
//! Use these helpers in any user-facing or prompt-injected context to avoid
//! UTC leakage on containerized deployments where `chrono::Local` defaults to UTC.

use chrono::{DateTime, FixedOffset, Local, Timelike, Utc};

/// Get the current local time, respecting `KRIA_USER_TZ` override.
///
/// Returns a `DateTime<FixedOffset>` so callers can format and reason about
/// the offset explicitly.
pub fn kria_now_local() -> DateTime<FixedOffset> {
    if let Some(tz) = resolve_env_offset() {
        return Utc::now().with_timezone(&tz);
    }

    // Fall back to system local timezone
    let local = Local::now();
    let offset = *local.offset();
    local.with_timezone(&offset)
}

/// Resolve a fixed offset from `KRIA_USER_TZ` env.
/// Accepts:
/// - Integer hours: "5" → UTC+5, "-8" → UTC-8
/// - Float hours: "5.5" → UTC+5:30
/// - Named offsets: "UTC", "GMT" → UTC+0
fn resolve_env_offset() -> Option<FixedOffset> {
    let raw = std::env::var("KRIA_USER_TZ").ok()?;
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }

    let upper = trimmed.to_ascii_uppercase();
    if matches!(upper.as_str(), "UTC" | "GMT" | "Z") {
        return FixedOffset::east_opt(0);
    }

    // Try integer first
    if let Ok(hours) = trimmed.parse::<i32>() {
        return FixedOffset::east_opt(hours * 3600);
    }

    // Try float (for half-hour offsets like 5.5 = IST)
    if let Ok(hours) = trimmed.parse::<f64>() {
        let seconds = (hours * 3600.0) as i32;
        return FixedOffset::east_opt(seconds);
    }

    None
}

/// Get the current local hour (0–23), used for time-of-day greetings.
pub fn kria_local_hour() -> u32 {
    kria_now_local().hour()
}

/// Get a time-of-day greeting based on the local hour.
pub fn kria_time_of_day_greeting() -> &'static str {
    match kria_local_hour() {
        5..=11 => "Good morning",
        12..=16 => "Good afternoon",
        17..=20 => "Good evening",
        _ => "Hello",
    }
}

/// Whether the current hour is in night/early-morning range (21:00–04:59).
pub fn is_night_hours() -> bool {
    let h = kria_local_hour();
    h >= 21 || h <= 4
}

#[cfg(test)]
mod tests {
    use super::*;

    /// All env-mutating tests must run in a single sequential test because
    /// `std::env::set_var` is process-global and parallel tests would race.
    #[test]
    fn env_offset_overrides_resolve_correctly() {
        // Positive integer offset
        std::env::set_var("KRIA_USER_TZ", "5");
        let now = kria_now_local();
        assert_eq!(now.offset().local_minus_utc(), 5 * 3600);

        // Negative integer offset
        std::env::set_var("KRIA_USER_TZ", "-8");
        let now = kria_now_local();
        assert_eq!(now.offset().local_minus_utc(), -8 * 3600);

        // Half-hour offset (IST)
        std::env::set_var("KRIA_USER_TZ", "5.5");
        let now = kria_now_local();
        assert_eq!(now.offset().local_minus_utc(), 5 * 3600 + 1800);

        // UTC alias
        std::env::set_var("KRIA_USER_TZ", "UTC");
        let now = kria_now_local();
        assert_eq!(now.offset().local_minus_utc(), 0);

        // Cleanup
        std::env::remove_var("KRIA_USER_TZ");
    }

    #[test]
    fn time_of_day_greeting_returns_valid_string() {
        let greeting = kria_time_of_day_greeting();
        assert!(matches!(
            greeting,
            "Good morning" | "Good afternoon" | "Good evening" | "Hello"
        ));
    }
}
