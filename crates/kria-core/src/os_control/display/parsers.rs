//! Pure, table-driven parsers for the display/brightness fallback adapters.
//!
//! linux-os-control-production **Task 2.2** — "Migrate brightness and prepare
//! display provider seam" (OSC-019, OSC-031, OSC-032), design §9.6.
//!
//! These functions are the migrated home of the brightness parsers that
//! previously lived (and directly drove subprocesses) in
//! `tools/system_config.rs`. Here they are **pure** string→value functions
//! with no process access, so the governed [`super::DisplayControl`] provider
//! and its transports can be tested entirely with captured fixtures.
//!
//! # Ambiguity never reports success
//!
//! Every parser returns `None` when the backend output cannot be parsed into
//! an unambiguous value. The provider maps a `None` to a non-success outcome
//! (`Unavailable`/`Unverified`) — parser ambiguity is **never** reported as a
//! satisfied state (OSC-031, OSC-019, and the task's "parser ambiguity never
//! reports success").

/// Clamp a raw percentage value into `0..=100`.
#[must_use]
fn clamp_percent(value: u64) -> u8 {
    value.min(100) as u8
}

/// Parse `brightnessctl get`/`brightnessctl max` integer output into a
/// percentage, given both raw current and max values. Returns `None` when
/// either side is unparseable or `max` is zero (division is meaningless).
#[must_use]
pub fn parse_brightnessctl_percent(current_output: &str, max_output: &str) -> Option<u8> {
    let current = parse_u64_line(current_output)?;
    let max = parse_u64_line(max_output)?;
    if max == 0 {
        return None;
    }
    let percent = ((current as f64 / max as f64) * 100.0)
        .round()
        .clamp(0.0, 100.0) as u8;
    Some(percent)
}

/// Parse the first whitespace-delimited unsigned integer found on any line.
#[must_use]
pub fn parse_u64_line(output: &str) -> Option<u64> {
    output
        .lines()
        .find_map(|line| line.trim().parse::<u64>().ok())
}

/// Parse a `gdbus call … org.freedesktop.DBus.Properties.Get … Brightness`
/// reply into a percentage. GNOME's `SettingsDaemon.Power.Screen.Brightness`
/// property is already an integer 0..=100 wrapped in a GVariant tuple like
/// `(<int32 60>,)`; this extracts the embedded integer. The `int32`/`uint32`
/// GVariant type tag is stripped first so its own embedded digits (e.g. the
/// `32` in `int32`) can never be mistaken for the brightness value. Returns
/// `None` on unparseable output.
#[must_use]
pub fn parse_gdbus_brightness_percent(output: &str) -> Option<u8> {
    let without_type_tags = output
        .replace("int32", " ")
        .replace("uint32", " ")
        .replace("int64", " ")
        .replace("uint64", " ");
    let normalized: String = without_type_tags
        .chars()
        .map(|c| if c.is_ascii_digit() { c } else { ' ' })
        .collect();

    normalized
        .split_whitespace()
        .find_map(|part| part.parse::<u64>().ok())
        .map(clamp_percent)
}

/// Parse an `xrandr --verbose` block's `Brightness: <fraction>` line into a
/// percentage. XRandR reports a `0.0..=1.0`(+) gamma-scale fraction, **not**
/// physical brightness (OSC-019.2). Returns `None` when no valid fraction is
/// present.
#[must_use]
pub fn parse_xrandr_brightness_percent(output: &str) -> Option<u8> {
    for line in output.lines() {
        if let Some((_, value)) = line.split_once("Brightness:") {
            if let Ok(fraction) = value.trim().parse::<f64>() {
                if fraction.is_finite() {
                    return Some((fraction * 100.0).round().clamp(0.0, 100.0) as u8);
                }
            }
        }
    }
    None
}

/// Parse an `xrandr` (non-verbose) listing for the first ` connected` output's
/// connector name. Used to bind the concrete `--output` target for a
/// brightness-gamma mutation; never fabricated when no connected display is
/// reported.
#[must_use]
pub fn first_connected_display(xrandr_output: &str) -> Option<String> {
    xrandr_output
        .lines()
        .find(|line| line.contains(" connected"))
        .and_then(|line| line.split_whitespace().next())
        .map(str::to_string)
}

#[cfg(all(test, feature = "os-control-test"))]
mod tests {
    use super::*;

    #[test]
    fn brightnessctl_percent_table() {
        assert_eq!(parse_brightnessctl_percent("300\n", "600\n"), Some(50));
        assert_eq!(parse_brightnessctl_percent("0\n", "255\n"), Some(0));
        assert_eq!(parse_brightnessctl_percent("255\n", "255\n"), Some(100));
        // Zero max is meaningless — ambiguous, never success.
        assert_eq!(parse_brightnessctl_percent("10\n", "0\n"), None);
        assert_eq!(parse_brightnessctl_percent("garbage", "255"), None);
        assert_eq!(parse_brightnessctl_percent("", ""), None);
    }

    #[test]
    fn u64_line_table() {
        assert_eq!(parse_u64_line("300\n"), Some(300));
        assert_eq!(parse_u64_line("  42  \nfoo"), Some(42));
        assert_eq!(parse_u64_line("no numbers here"), None);
        assert_eq!(parse_u64_line(""), None);
    }

    #[test]
    fn gdbus_brightness_table() {
        assert_eq!(parse_gdbus_brightness_percent("(<int32 60>,)"), Some(60));
        assert_eq!(parse_gdbus_brightness_percent("(<int32 0>,)"), Some(0));
        assert_eq!(parse_gdbus_brightness_percent("(<int32 100>,)"), Some(100));
        assert_eq!(parse_gdbus_brightness_percent(""), None);
        assert_eq!(parse_gdbus_brightness_percent("()"), None);
    }

    #[test]
    fn xrandr_brightness_table() {
        assert_eq!(
            parse_xrandr_brightness_percent("Brightness: 0.60\n"),
            Some(60)
        );
        assert_eq!(
            parse_xrandr_brightness_percent("Brightness: 1.00\n"),
            Some(100)
        );
        assert_eq!(
            parse_xrandr_brightness_percent("Brightness: 0.00\n"),
            Some(0)
        );
        // Clamp on an out-of-range fraction rather than reporting a lie >100%.
        assert_eq!(
            parse_xrandr_brightness_percent("Brightness: 1.50\n"),
            Some(100)
        );
        assert_eq!(parse_xrandr_brightness_percent("no brightness here"), None);
        assert_eq!(parse_xrandr_brightness_percent(""), None);
    }

    #[test]
    fn first_connected_display_table() {
        let listing = "Screen 0: minimum 320 x 200\neDP-1 connected primary 1920x1080+0+0\nHDMI-1 disconnected";
        assert_eq!(
            first_connected_display(listing),
            Some("eDP-1".to_string())
        );
        assert_eq!(first_connected_display("HDMI-1 disconnected"), None);
        assert_eq!(first_connected_display(""), None);
    }
}
