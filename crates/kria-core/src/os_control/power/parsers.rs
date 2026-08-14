//! Pure, table-driven parsers for the power-profile (`powerprofilesctl`)
//! fallback adapter.
//!
//! linux-os-control-production **Task 2.3** — "Migrate Wi-Fi and power-profile
//! controls" (OSC-020, OSC-031), design §9.7.
//!
//! These functions are the migrated home of the power-profile parser that
//! previously lived (and directly drove subprocesses) in
//! `tools/system_config.rs`. Here it is a **pure** string→value function with
//! no process access, so the governed [`super::PowerControl`] provider and its
//! transports can be tested entirely with captured fixtures.

use super::PowerProfile;

/// Parse a `powerprofilesctl get` reply into a [`PowerProfile`]. Returns
/// `None` on unrecognized output so ambiguity never reports a fabricated
/// profile (OSC-031).
#[must_use]
pub fn parse_power_profile(output: &str) -> Option<PowerProfile> {
    PowerProfile::parse(output.trim())
}

#[cfg(all(test, feature = "os-control-test"))]
mod tests {
    use super::*;

    #[test]
    fn power_profile_output_table() {
        assert_eq!(parse_power_profile("balanced\n"), Some(PowerProfile::Balanced));
        assert_eq!(
            parse_power_profile("power-saver"),
            Some(PowerProfile::PowerSaver)
        );
        assert_eq!(
            parse_power_profile("  performance  \n"),
            Some(PowerProfile::Performance)
        );
        assert_eq!(parse_power_profile("garbage"), None);
        assert_eq!(parse_power_profile(""), None);
    }
}
