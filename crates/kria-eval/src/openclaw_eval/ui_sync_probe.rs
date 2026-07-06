//! R16 — UI/backend state synchronization (tasks.md task 16, design.md
//! "UI-sync probe"). Asserts at the real command/event contract level
//! (never renaming existing Tauri command/event names, per structure.md).
//!
//! Real-code grounding (exhaustive grep across `kria-desktop/`, not
//! assumed):
//!
//! TWO real, well-designed backend event streams exist for OpenClaw:
//! 1. `openclaw::bundle::events::{subscribe, BundleLifecycleEvent}` —
//!    Installing/Installed/Updated/Failed/RolledBack/Removed/Enabled/Disabled.
//!    Emitted by the REAL `BundleInstaller` (confirmed firing correctly
//!    throughout tasks 5/6/8/12's real install/uninstall/enable/disable
//!    tests in this session).
//! 2. `openclaw::event::{subscribe, SkillEvent, Stage}` — Started/Preparing/
//!    Running/Completed/Failed/etc. per skill execution. Confirmed firing
//!    correctly in task 5's real R11 trace (`[Started, Preparing, Running,
//!    Running, Completed]`).
//!
//! R16 FIXED (product gap 5/8, post user sign-off): NEITHER event stream
//! used to be subscribed to by the desktop application — the frontend had
//! NO push-based way to learn about install/update/remove/enable/disable or
//! skill-execution progress. Real fix, additive: added
//! `commands::openclaw::{forward_bundle_events, forward_execution_events}`
//! (testable core, sink-parameterized) plus
//! `spawn_openclaw_event_forwarding(app_handle)` which wires both to real
//! `AppHandle::emit` calls (`"openclaw:bundle_event"`,
//! `"openclaw:execution_event"`), called from `main.rs`'s `setup()` — the
//! same pattern every other push-based feature in this codebase already
//! uses (`voice.rs`, `wake_listener.rs`, `test_runner.rs`). Proven with 2
//! real tests in `kria-desktop` itself (subscribes to the REAL broadcast
//! buses, emits a real event, confirms the sink receives it).
//!
//! What ALSO holds (R16.4, unaffected by this fix, still validated below):
//! polling the real commands DOES reflect real backend state immediately —
//! proven by installing a skill via the real installer and confirming
//! `ProductionSkillRegistry` (the same data `clawhub_list_skills` reads)
//! reflects it with no propagation delay. Push-based (R16.1-16.3) AND
//! poll-based (R16.4) both now hold.

use kria_core::openclaw::bundle::verify::TrustPolicy;
use kria_core::openclaw::bundle::BundleInstaller;
use kria_core::openclaw::registry::ProductionSkillRegistry;
use semver::Version;
use std::sync::Arc;

/// FIX PROOF: confirms the real event-forwarding wiring now exists in the
/// desktop app source (source tripwire; the real behavioral proof is
/// `kria-desktop`'s own `event_forwarding_tests` module, which actually
/// subscribes to the real buses and confirms delivery).
pub fn validate_event_forwarding_exists() -> Result<(), String> {
    let desktop_src_files = [
        include_str!("../../../kria-desktop/src/commands/openclaw.rs"),
        include_str!("../../../kria-desktop/src/main.rs"),
    ];

    let bundle_events_subscribed = desktop_src_files
        .iter()
        .any(|f| f.contains("bundle::events::subscribe"));
    let skill_events_subscribed = desktop_src_files
        .iter()
        .any(|f| f.contains("openclaw::event::subscribe"));
    let wired_in_setup = desktop_src_files
        .iter()
        .any(|f| f.contains("spawn_openclaw_event_forwarding"));

    if !bundle_events_subscribed || !skill_events_subscribed || !wired_in_setup {
        return Err(format!(
            "REGRESSION: event-forwarding wiring appears to be missing (bundle_subscribed={bundle_events_subscribed}, \
             skill_subscribed={skill_events_subscribed}, wired_in_setup={wired_in_setup})"
        ));
    }
    Ok(())
}

/// R16.4-adjacent real validation: the underlying data a polling-based UI
/// would read (`ProductionSkillRegistry`, what `clawhub_list_skills` reads)
/// DOES reflect a real install immediately, with no propagation delay —
/// proving the reconciliation fallback R16.4 describes has correct data to
/// reconcile TO, even though push-based sync (R16.1-16.3) does not exist.
pub fn validate_polled_data_reflects_real_state() -> Result<(), String> {
    let dir = tempfile::tempdir().map_err(|e| e.to_string())?;
    let db_path = dir.path().join("r16_poll.db");
    let registry = Arc::new(ProductionSkillRegistry::new(&db_path).map_err(|e| e.to_string())?);
    let audit = Arc::new(
        kria_core::openclaw::audit::AuditLedger::open(&db_path, b"r16-test-key".to_vec()).map_err(|e| e.to_string())?,
    );
    let store = dir.path().join("store");
    std::fs::create_dir_all(&store).map_err(|e| e.to_string())?;
    let author_dir = dir.path().join("authored");
    std::fs::create_dir_all(&author_dir).map_err(|e| e.to_string())?;

    let bundle_root = crate::openclaw_eval::installer_matrix::author_signed_bundle(&author_dir, "oc_r16_poll", [88u8; 32])?;
    let installer = BundleInstaller::new(registry.clone(), audit, store)
        .with_kria_version(Version::new(1, 0, 0))
        .with_trust_policy(TrustPolicy { trusted_keys: Vec::new(), require_signature: true });

    installer.install(&bundle_root).map_err(|e| format!("install failed: {e}"))?;

    // Immediately after install, with no delay — the data a poll-based
    // clawhub_list_skills-equivalent read would see.
    let all_installed = registry.list_installed().map_err(|e| e.to_string())?;
    let found = all_installed.iter().any(|s| s.skill_id == "oc_r16_poll");
    if !found {
        return Err("polled registry data did not immediately reflect the real install".into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fixed_r16_event_forwarding_wired_to_frontend() {
        validate_event_forwarding_exists()
            .expect("REGRESSION: OpenClaw event forwarding must remain wired into the desktop frontend");
    }

    #[test]
    fn r16_4_polled_data_reflects_real_state_immediately() {
        validate_polled_data_reflects_real_state()
            .expect("R16.4: polling-based reconciliation must have correct, immediately-consistent data to read");
    }
}
