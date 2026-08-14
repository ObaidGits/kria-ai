//! Native-OS action authority boundary (linux-os-control-production Task 0.3).
//!
//! # Purpose
//!
//! Requirements OSC-001/002/004 and design §2.1 ("Existing authority
//! reconciliation") mandate that KRIA have **exactly one** native operating-system
//! admission authority: the `ExecutionGate` → (future) `OsControlRuntime` path.
//! The extension/marketplace capability plane (`CapabilityPlatform`,
//! `DefaultPermissionEngine`, `GrantStore`) and the command-policy defence layer
//! (`CapabilityPolicyGate`) must remain in their **narrower** roles and can never
//! approve, execute, or broaden a native host-OS mutation.
//!
//! This module is the single, code-owned definition of that boundary:
//!
//! 1. [`is_native_os_action`] — is a given tool action one of the frozen §10.4
//!    canonical OS-control operations (the same manifest Task 0.1 froze)? A
//!    "typed" native OS action is governed by `ExecutionGate`, which mints an
//!    [`crate::agent::execution_gate::OsActionGrant`] for it. Generic execution
//!    (`execute_bash`, a raw `reboot` binary, …) is *not* a native OS action and
//!    stays governed by generic-shell policy (and stays blocked where dangerous).
//!
//! 2. [`NATIVE_OS_EFFECT`] + [`effects_request_native_os`] — the effect-class
//!    marker an extension/marketplace descriptor would use to *request* a native
//!    host-OS effect. The capability plane refuses any descriptor/grant/request
//!    carrying it: extensions receive no host handle and must re-enter a
//!    canonical registered OS tool through `ExecutionGate`.
//!
//! # Source of truth and deferral
//!
//! The canonical action set is projected directly from the frozen contract
//! manifest fixture (Task 0.1). Task 1.2 will make `ToolRegistry`'s strict
//! `ToolContractMetadata` the authoritative runtime source; until then this
//! projection is the single non-duplicated list, kept honest by
//! [`tests`] which re-parse the fixture. No name is invented here.

use once_cell::sync::Lazy;
use std::collections::HashSet;

/// The closed set of canonical native-OS control tool names (§10.4), projected
/// from the single embedded frozen manifest owned by
/// [`crate::os_control::manifest`]. Sourcing it there keeps exactly one embedded
/// copy of the manifest and one source of truth for the canonical OS tool names.
static NATIVE_OS_TOOL_NAMES: Lazy<HashSet<String>> =
    Lazy::new(|| crate::os_control::frozen_tool_names().into_iter().collect());

/// True when `action` is a canonical native-OS control tool (a typed operation
/// that must be admitted by `ExecutionGate` and carries an
/// [`crate::agent::execution_gate::OsActionGrant`]).
///
/// This is deliberately name-exact against the frozen manifest: generic
/// primitives such as `execute_bash`, `execute_python`, or an ad-hoc `reboot`
/// shell binary are **not** native OS actions and are governed by the separate
/// generic-shell / command-policy surface.
pub fn is_native_os_action(action: &str) -> bool {
    NATIVE_OS_TOOL_NAMES.contains(action)
}

/// Number of canonical native-OS actions (frozen manifest size). Used by tests.
pub fn native_os_action_count() -> usize {
    NATIVE_OS_TOOL_NAMES.len()
}

/// The reserved effect-class marker a marketplace/extension capability descriptor
/// would use to declare (or request) a native host-OS effect. The capability
/// plane treats any descriptor, grant, or request carrying this marker as
/// **excluded**: it is never registered as executable, never authorized by the
/// extension permission engine, never persisted as a grant, and never executed
/// by `CapabilityPlatform`. Native host effects must instead re-enter a canonical
/// registered OS tool through `ExecutionGate`.
pub const NATIVE_OS_EFFECT: &str = "os.native";

/// True when a set of declared/granted effect classes attempts to reach a native
/// host-OS effect. Matches the explicit [`NATIVE_OS_EFFECT`] marker plus a small
/// closed set of unambiguous host-control synonyms, so an extension cannot smuggle
/// a native OS effect past the plane exclusion under an alias. It intentionally
/// does **not** match generic capability effects (`read`, `write`, `network`,
/// `subprocess`, `gpu`), which remain valid on the extension plane.
pub fn effects_request_native_os(classes: &[String]) -> bool {
    classes.iter().any(|class| {
        let c = class.trim().to_ascii_lowercase();
        c == NATIVE_OS_EFFECT
            || c == "host_os"
            || c == "host-os"
            || c == "os.mutate"
            || c.starts_with("os.native")
            || c.starts_with("hostoscontrol")
            || c.starts_with("host_os_control")
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frozen_manifest_projects_exactly_149_native_os_actions() {
        // Parity with the frozen §10.4 manifest (Task 0.1): the projection is the
        // canonical closed set, neither more nor fewer.
        assert_eq!(
            native_os_action_count(),
            149,
            "native-OS action set must equal the frozen 149-operation manifest"
        );
    }

    #[test]
    fn typed_native_os_actions_are_recognized() {
        // A representative sample across power/session, connectivity, files, apps.
        for action in [
            "reboot_system",
            "shutdown_system",
            "logout_session",
            "hibernate",
            "toggle_wifi",
            "connect_wifi",
            "set_brightness",
            "set_volume",
            "install_package",
            "mount_device",
        ] {
            assert!(
                is_native_os_action(action),
                "`{action}` must be a canonical native-OS action"
            );
        }
    }

    #[test]
    fn generic_execution_primitives_are_not_native_os_actions() {
        // Generic shell / code execution and a raw `reboot` binary are NOT native
        // OS actions: they stay on the separately-governed generic surface.
        for action in [
            "execute_bash",
            "execute_python",
            "execute_powershell",
            "reboot",
            "shutdown",
            "openclaw",
        ] {
            assert!(
                !is_native_os_action(action),
                "`{action}` must not be classified as a typed native-OS action"
            );
        }
    }

    #[test]
    fn native_os_effect_marker_is_detected_but_generic_effects_pass() {
        assert!(effects_request_native_os(&[NATIVE_OS_EFFECT.to_string()]));
        assert!(effects_request_native_os(&["host_os".to_string()]));
        assert!(effects_request_native_os(&[
            "read".to_string(),
            "os.native.power".to_string(),
        ]));
        // Ordinary extension effect classes must NOT trip the exclusion.
        assert!(!effects_request_native_os(&[
            "read".to_string(),
            "write".to_string(),
            "network".to_string(),
            "subprocess".to_string(),
            "gpu".to_string(),
        ]));
        assert!(!effects_request_native_os(&[]));
    }
}
