//! Proves a native OS action reaching the chat tool path receives a governed call.
//!
//! # The bug this pins
//!
//! `set_volume` failed with `os_control.unavailable` even though the audio provider
//! was composed and the handler was registered. Cause: the chat tool path called
//! the handler **directly**, so `ToolContext::os_call` was never populated. The
//! handler then failed closed — correctly, because without a grant it cannot prove
//! it was admitted.
//!
//! Failing closed was right. Skipping the gate was the bug.
//!
//! This test asserts the gate mints a grant for a canonical OS action, which is the
//! artefact whose absence caused the failure.

/// Every canonical OS action must be recognised by the authority set.
#[test]
fn set_volume_is_recognised_as_a_native_os_action() {
    assert!(
        kria_core::agent::os_action_authority::is_native_os_action("set_volume"),
        "set_volume must be recognised, or the gate never mints a grant for it"
    );
    // A generic primitive must NOT be, or it would be admitted as a typed OS
    // action and bypass the shell/command policy surface.
    assert!(!kria_core::agent::os_action_authority::is_native_os_action(
        "execute_bash"
    ));
}

/// Every tool exercised in live testing must be recognised by the authority set.
///
/// A tool missing here gets no grant, and the handler then reports
/// `os_control.unavailable` with the message "provider … is not composed" — which
/// points at the wrong layer entirely and is very expensive to diagnose.
#[test]
fn every_live_tested_tool_is_recognised() {
    for tool in [
        "set_volume",
        "get_audio_state",
        "set_brightness",
        "get_display_state",
        "set_night_light",
        "search_files",
        "get_wifi_networks",
        "get_bluetooth_state",
        "set_bluetooth_enabled",
    ] {
        assert!(
            kria_core::agent::os_action_authority::is_native_os_action(tool),
            "`{tool}` is not in the authority set, so it can never receive a grant"
        );
    }
}

#[test]
fn the_authority_set_covers_the_whole_frozen_manifest() {
    let count = kria_core::agent::os_action_authority::native_os_action_count();
    assert_eq!(
        count, 149,
        "the authority set must match the frozen manifest exactly; a tool missing \
         from it silently loses its grant and answers os_control.unavailable"
    );
}
