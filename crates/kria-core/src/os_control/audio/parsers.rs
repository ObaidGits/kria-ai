//! Pure, table-driven parsers for the audio fallback adapters.
//!
//! linux-os-control-production **Task 2.1** — "Migrate audio volume and add
//! getters/mute" (OSC-005, OSC-006, OSC-018, OSC-031), design §3.
//!
//! These functions are the migrated home of the volume parsers that previously
//! lived (and directly drove subprocesses) in `tools/system_config.rs`. Here
//! they are **pure** string→value functions with no process access, so the
//! governed [`super::AudioControl`] provider and its transports can be tested
//! entirely with captured fixtures.
//!
//! # Ambiguity never reports success
//!
//! Every parser returns `None` when the backend output cannot be parsed into an
//! unambiguous value (empty, non-numeric, contradictory channel states, …).
//! The provider maps a `None` to a non-success outcome (`Unavailable` /
//! `Unverified`) — parser ambiguity is **never** reported as a satisfied state
//! (OSC-018, OSC-031, and the task's "parser ambiguity never reports success").

/// Parse a single percentage token such as `"60%"`, `"[60%]"`, or `"(60%)"`.
///
/// Returns `None` when the token is not a bounded percentage. The value is
/// clamped to `0..=100`.
#[must_use]
pub fn parse_percent_token(token: &str) -> Option<u8> {
    let cleaned = token
        .trim()
        .trim_matches(|c: char| matches!(c, '[' | ']' | '(' | ')' | ','));
    let without_percent = cleaned.strip_suffix('%')?;
    let value = without_percent.trim().parse::<f64>().ok()?;
    if !value.is_finite() {
        return None;
    }
    Some(value.round().clamp(0.0, 100.0) as u8)
}

/// Parse a WirePlumber `wpctl get-volume` volume value.
///
/// `wpctl` prints a normalized `0.0..=1.0(+)` float (e.g. `Volume: 0.60`); some
/// locales/tools print a `NN%` token instead. The first recognizable value
/// wins; unparseable output yields `None`.
#[must_use]
pub fn parse_wpctl_percent(output: &str) -> Option<u8> {
    for token in output.split_whitespace() {
        let cleaned = token.trim_matches(|c: char| matches!(c, ',' | ';'));

        if let Ok(value) = cleaned.parse::<f64>() {
            if value.is_finite() && (0.0..=1.5).contains(&value) {
                return Some((value * 100.0).round().clamp(0.0, 100.0) as u8);
            }
        }

        if let Some(percent) = parse_percent_token(cleaned) {
            return Some(percent);
        }
    }
    None
}

/// Parse the first bounded `NN%` token found anywhere in the output. Used by the
/// `pactl` / `amixer` volume adapters.
#[must_use]
pub fn parse_any_percent(output: &str) -> Option<u8> {
    output.split_whitespace().find_map(parse_percent_token)
}

/// Parse a `wpctl get-volume` line into `(volume_percent, muted)`.
///
/// `wpctl` prints `Volume: 0.60` or `Volume: 0.60 [MUTED]`. The `[MUTED]` marker
/// is the authoritative mute signal. Returns `None` when no volume value can be
/// parsed (ambiguous output never reports a state).
#[must_use]
pub fn parse_wpctl_state(output: &str) -> Option<(u8, bool)> {
    let volume = parse_wpctl_percent(output)?;
    let muted = output.to_ascii_uppercase().contains("[MUTED]");
    Some((volume, muted))
}

/// Parse a `pactl get-sink-mute` line (`Mute: yes` / `Mute: no`).
///
/// Returns `None` when the mute state is absent or contradictory (both `yes`
/// and `no` present) so ambiguity never reports success.
#[must_use]
pub fn parse_pactl_mute(output: &str) -> Option<bool> {
    let lower = output.to_ascii_lowercase();
    // Only consider the token immediately following a `mute:` label so stray
    // "yes"/"no" elsewhere cannot flip the result.
    let mut result: Option<bool> = None;
    for line in lower.lines() {
        if let Some((_, rest)) = line.split_once("mute:") {
            let token = rest.trim();
            let parsed = if token.starts_with("yes") {
                Some(true)
            } else if token.starts_with("no") {
                Some(false)
            } else {
                None
            };
            match (result, parsed) {
                (None, Some(v)) => result = Some(v),
                // Contradictory mute lines → ambiguous → None.
                (Some(existing), Some(v)) if existing != v => return None,
                _ => {}
            }
        }
    }
    result
}

/// Parse an `amixer get Master` block into `(volume_percent, muted)`.
///
/// `amixer` prints per-channel lines like `Front Left: Playback 39321 [60%]
/// [on]`. All present channel mute states must agree; a contradiction (one
/// `[on]`, one `[off]`) is ambiguous and yields `None`. Returns `None` when no
/// percentage is present.
#[must_use]
pub fn parse_amixer_state(output: &str) -> Option<(u8, bool)> {
    let volume = parse_any_percent(output)?;

    let mut mute: Option<bool> = None;
    let lower = output.to_ascii_lowercase();
    for token in lower.split_whitespace() {
        let cleaned = token.trim_matches(|c: char| matches!(c, '[' | ']'));
        let parsed = match cleaned {
            "on" => Some(false),  // amixer `[on]` = playing = not muted
            "off" => Some(true),  // amixer `[off]` = muted
            _ => None,
        };
        match (mute, parsed) {
            (None, Some(v)) => mute = Some(v),
            (Some(existing), Some(v)) if existing != v => return None, // contradictory
            _ => {}
        }
    }

    // No mute marker at all: default to not-muted (volume present, unambiguous).
    Some((volume, mute.unwrap_or(false)))
}

#[cfg(all(test, feature = "os-control-test"))]
mod tests {
    use super::*;

    #[test]
    fn percent_token_table() {
        let cases: &[(&str, Option<u8>)] = &[
            ("60%", Some(60)),
            ("[60%]", Some(60)),
            ("(0%)", Some(0)),
            ("100%", Some(100)),
            ("150%", Some(100)), // clamped
            ("42", None),        // no percent suffix
            ("", None),
            ("%", None),
            ("abc%", None),
            ("NaN%", None),
        ];
        for (input, expected) in cases {
            assert_eq!(parse_percent_token(input), *expected, "token {input:?}");
        }
    }

    #[test]
    fn wpctl_percent_table() {
        let cases: &[(&str, Option<u8>)] = &[
            ("Volume: 0.60", Some(60)),
            ("Volume: 1.00", Some(100)),
            ("Volume: 0.00", Some(0)),
            ("Volume: 1.50", Some(100)), // clamp on the *100 result
            ("Volume: 0.42 [MUTED]", Some(42)),
            ("Volume: 60%", Some(60)),
            ("garbage output", None),
            ("", None),
        ];
        for (input, expected) in cases {
            assert_eq!(parse_wpctl_percent(input), *expected, "wpctl {input:?}");
        }
    }

    #[test]
    fn wpctl_state_table() {
        assert_eq!(parse_wpctl_state("Volume: 0.60"), Some((60, false)));
        assert_eq!(parse_wpctl_state("Volume: 0.60 [MUTED]"), Some((60, true)));
        assert_eq!(parse_wpctl_state("Volume: 0.00 [MUTED]"), Some((0, true)));
        // Ambiguous / empty never reports a state.
        assert_eq!(parse_wpctl_state("no volume here"), None);
        assert_eq!(parse_wpctl_state(""), None);
    }

    #[test]
    fn any_percent_table() {
        assert_eq!(
            parse_any_percent("Front Left: Playback 39321 [60%] [on]"),
            Some(60)
        );
        assert_eq!(parse_any_percent("Volume: 0 / 0% / -inf dB"), Some(0));
        assert_eq!(parse_any_percent("no percentage"), None);
        assert_eq!(parse_any_percent(""), None);
    }

    #[test]
    fn pactl_mute_table() {
        assert_eq!(parse_pactl_mute("Mute: yes"), Some(true));
        assert_eq!(parse_pactl_mute("Mute: no"), Some(false));
        assert_eq!(parse_pactl_mute("Mute:   yes"), Some(true));
        // Absent → None (ambiguous, never success).
        assert_eq!(parse_pactl_mute("Volume: 60%"), None);
        assert_eq!(parse_pactl_mute(""), None);
        // Contradictory mute lines → None.
        assert_eq!(parse_pactl_mute("Mute: yes\nMute: no"), None);
    }

    #[test]
    fn amixer_state_table() {
        assert_eq!(
            parse_amixer_state("Front Left: Playback 39321 [60%] [on]\nFront Right: Playback 39321 [60%] [on]"),
            Some((60, false))
        );
        assert_eq!(
            parse_amixer_state("Mono: Playback 0 [30%] [off]"),
            Some((30, true))
        );
        // No mute marker → default not-muted, volume unambiguous.
        assert_eq!(parse_amixer_state("Mono: Playback 0 [45%]"), Some((45, false)));
        // Contradictory channel mute → ambiguous → None.
        assert_eq!(
            parse_amixer_state("Front Left: [60%] [on]\nFront Right: [60%] [off]"),
            None
        );
        // No percentage → None.
        assert_eq!(parse_amixer_state("Simple mixer control 'Master'"), None);
        assert_eq!(parse_amixer_state(""), None);
    }
}
