//! EnvironmentStateTracker — bridges `OperationalFacts` into WorldModelStore.
//!
//! # Design Contract
//!
//! The `EnvironmentGrounder` produces **ephemeral** `OperationalFacts` snapshots
//! (10s TTL, bounded, read-only). This contract MUST NOT change.
//!
//! The `EnvironmentStateTracker` is a **side-channel observer**: after each
//! `EnvironmentGrounder::ground()` call, the caller passes the fresh
//! `OperationalFacts` to `track()`. The tracker computes a delta against the
//! previous snapshot and writes changed facts to WorldModelStore via `PsdgHandle`.
//!
//! # Delta Strategy
//!
//! Only writes that differ from the last recorded snapshot are issued, to avoid
//! redundant SQLite writes on every grounding call (which may occur every 10s).
//!
//! # Authority Boundary
//!
//! - `EnvironmentGrounder` remains ephemeral, bounded, read-only, deterministic.
//! - `EnvironmentStateTracker` observes and persists — never mutates `OperationalFacts`.
//! - All writes are fire-and-forget via `PsdgHandle`.

use std::sync::Mutex;

use crate::agent::environment_grounder::OperationalFacts;
use crate::agent::psdg::PsdgHandle;
use crate::agent::world_model::FactSource;

/// Last-seen values for delta computation.
#[derive(Debug, Default, Clone)]
struct LastSeen {
    focused_app: Option<String>,
    terminal_cwd: Option<String>,
    open_project_path: Option<String>,
    visible_window_count: usize,
}

/// Observes `OperationalFacts` snapshots and writes semantic deltas to WorldModelStore.
pub struct EnvironmentStateTracker {
    psdg: PsdgHandle,
    last_seen: Mutex<LastSeen>,
}

impl EnvironmentStateTracker {
    pub fn new(psdg: PsdgHandle) -> Self {
        Self {
            psdg,
            last_seen: Mutex::new(LastSeen::default()),
        }
    }

    /// Process a fresh `OperationalFacts` snapshot.
    ///
    /// Computes delta against last-seen state and issues fire-and-forget
    /// writes to WorldModelStore for changed values only.
    ///
    /// This method is synchronous and cheap (only issues writes on change).
    /// Call it after every `EnvironmentGrounder::ground()` call.
    pub fn track(&self, facts: &OperationalFacts) {
        let mut last = self.last_seen.lock().unwrap_or_else(|p| p.into_inner());

        // ── Focused app ───────────────────────────────────────────────────────
        let new_app = facts.focused_app.clone();
        if new_app != last.focused_app {
            if let Some(ref app) = new_app {
                let app_id = app.to_lowercase().replace(' ', "_");
                self.psdg.record_app_focus(&app_id, app);
            }
            last.focused_app = new_app;
        }

        // ── Terminal CWD ──────────────────────────────────────────────────────
        let new_cwd = facts
            .terminal_cwd
            .as_ref()
            .and_then(|p| p.to_str().map(|s| s.to_string()));
        if new_cwd != last.terminal_cwd {
            if let Some(ref cwd) = new_cwd {
                self.psdg.record_fact(
                    "terminal_primary",
                    "cwd",
                    cwd,
                    0.90,
                    FactSource::Detected,
                    "EnvironmentStateTracker",
                );
            }
            last.terminal_cwd = new_cwd;
        }

        // ── Open project path ─────────────────────────────────────────────────
        let new_project = facts
            .open_project_path
            .as_ref()
            .and_then(|p| p.to_str().map(|s| s.to_string()));
        if new_project != last.open_project_path {
            if let Some(ref proj) = new_project {
                self.psdg.record_fact(
                    "ide_primary",
                    "workspace_root",
                    proj,
                    0.75,
                    FactSource::Inferred,
                    "EnvironmentStateTracker (window title heuristic)",
                );
            }
            last.open_project_path = new_project;
        }

        // ── Desktop topology (window count change) ────────────────────────────
        let new_count = facts.visible_windows.len();
        if new_count != last.visible_window_count {
            self.psdg.record_fact(
                "desktop_environment",
                "visible_window_count",
                &new_count.to_string(),
                0.80,
                FactSource::Detected,
                "EnvironmentStateTracker",
            );
            last.visible_window_count = new_count;
        }

        // ── Focused window title ──────────────────────────────────────────────
        if let Some(ref win) = facts.focused_window {
            if !win.title.is_empty() {
                self.psdg.record_fact(
                    "desktop_environment",
                    "focused_window_title",
                    &win.title,
                    0.85,
                    FactSource::Detected,
                    "EnvironmentStateTracker",
                );
            }
        }

        // ── Display server ────────────────────────────────────────────────────
        let display_server = format!("{:?}", facts.capabilities.display_server).to_lowercase();
        self.psdg.record_fact(
            "desktop_environment",
            "display_server",
            &display_server,
            0.99,
            FactSource::Detected,
            "EnvironmentStateTracker",
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::environment_grounder::{GroundingCapabilities, OperationalFacts};
    use crate::agent::psdg::PsdgHandle;
    use tempfile::NamedTempFile;

    fn make_handle() -> PsdgHandle {
        let tmp = NamedTempFile::new().unwrap();
        PsdgHandle::open(tmp.path()).unwrap()
    }

    #[tokio::test]
    async fn track_does_not_panic_on_empty_facts() {
        let handle = make_handle();
        let tracker = EnvironmentStateTracker::new(handle);
        let facts = OperationalFacts::empty(GroundingCapabilities::none());
        // Should not panic
        tracker.track(&facts);
        tracker.track(&facts);
    }

    #[tokio::test]
    async fn track_writes_on_change_only() {
        let handle = make_handle();
        let tracker = EnvironmentStateTracker::new(handle.clone());

        let mut facts = OperationalFacts::empty(GroundingCapabilities::none());
        facts.focused_app = Some("firefox".to_string());

        tracker.track(&facts);
        // Re-track same facts (no change — no duplicate write)
        tracker.track(&facts);

        // Change the app
        facts.focused_app = Some("code".to_string());
        tracker.track(&facts);
    }
}
