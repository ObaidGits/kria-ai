//! Batch 3 — Desktop Awareness Runtime.
//!
//! # Core Mission
//!
//! Maintain a live, lightweight operational snapshot of the user's desktop
//! environment — combining browser, IDE, and AT-SPI state into one queryable
//! struct. This snapshot is updated by consuming [`CognitionEvent`]s from the
//! [`CognitionEventBus`] and persisted as PSDG facts.
//!
//! # What Is Tracked
//!
//! | Field                  | Source                             |
//! |------------------------|------------------------------------|
//! | `browser_state`        | `BrowserCognitionEvent` events     |
//! | `ide_state`            | `IdeCognitionEvent` events         |
//! | `active_window`        | `DesktopCognitionEvent::FocusChanged` |
//! | `active_app`           | `DesktopCognitionEvent`            |
//! | `has_dialog`           | `DesktopCognitionEvent::WindowAppeared { is_dialog: true }` |
//! | `active_workflow_id`   | `WorkflowEvent::Started` / `Completed` |
//!
//! # Usage
//!
//! ```no_run
//! let runtime = DesktopAwarenessRuntime::new(None);
//! // Feed events manually or via the bus listener task
//! runtime.apply_event(&event);
//! let snapshot = runtime.snapshot();
//! ```
//!
//! # Invariants
//!
//! - Snapshot reads are O(1) — lock on `RwLock`.
//! - Updates are O(1) — atomic field writes.
//! - NO LLM calls. NO vision inference.
//! - All state is derived from typed [`CognitionEvent`]s only.
//! - Vision (OCR / VLM) is NEVER invoked here.

use std::sync::{Arc, RwLock};

use serde::{Deserialize, Serialize};
use tracing::debug;

use crate::agent::cognition_event_bus::{
    BrowserEventKind, CognitionEvent, DesktopCognitionEventKind, IdeEventKind, WorkflowEventKind,
};
use crate::agent::psdg::PsdgHandle;
use crate::agent::world_model::FactSource;

// ─── Snapshot Types ───────────────────────────────────────────────────────────

/// Lightweight browser state snapshot.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct BrowserStateSnapshot {
    /// Current page URL (empty if unknown).
    pub url: String,
    /// Current page title.
    pub title: String,
    /// Whether a browser auth interruption is active.
    pub auth_interrupt: bool,
    /// Hint about the service requiring auth.
    pub auth_service_hint: Option<String>,
}

/// Lightweight IDE state snapshot.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct IdeStateSnapshot {
    /// Workspace root (if known).
    pub workspace_root: Option<String>,
    /// Last build succeeded?
    pub last_build_ok: Option<bool>,
    /// Active error count.
    pub error_count: usize,
    /// Active warning count.
    pub warning_count: usize,
    /// Active file (if known).
    pub active_file: Option<String>,
}

impl IdeStateSnapshot {
    /// Whether there are active build errors.
    pub fn has_errors(&self) -> bool {
        self.error_count > 0
    }
}

/// Unified desktop awareness snapshot.
///
/// A cheap, cloneable view of the last-known desktop environment state.
/// All fields default to "unknown" / empty — callers must handle absence.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct DesktopAwarenessSnapshot {
    /// Last known browser state.
    pub browser: BrowserStateSnapshot,
    /// Last known IDE state.
    pub ide: IdeStateSnapshot,
    /// Currently focused application name.
    pub active_app: Option<String>,
    /// Currently focused window title.
    pub active_window_title: Option<String>,
    /// Whether a dialog is active (popup, auth prompt, etc.).
    pub has_dialog: bool,
    /// Active workflow session ID, if any.
    pub active_workflow_id: Option<String>,
    /// Epoch seconds when the snapshot was last updated.
    pub last_updated_at: u64,
}

impl DesktopAwarenessSnapshot {
    /// Whether the desktop is in a "clean" state for new workflow execution.
    ///
    /// Clean = no active dialog, no active workflow.
    pub fn is_clean(&self) -> bool {
        !self.has_dialog && self.active_workflow_id.is_none()
    }
}

// ─── Runtime ─────────────────────────────────────────────────────────────────

/// Live desktop awareness runtime.
///
/// Maintains an up-to-date snapshot of the desktop operational state by
/// consuming [`CognitionEvent`]s. Writes selected facts to PSDG on each update.
pub struct DesktopAwarenessRuntime {
    psdg: Option<PsdgHandle>,
    snapshot: RwLock<DesktopAwarenessSnapshot>,
}

impl DesktopAwarenessRuntime {
    /// Create a new runtime.
    pub fn new(psdg: Option<PsdgHandle>) -> Self {
        Self {
            psdg,
            snapshot: RwLock::new(DesktopAwarenessSnapshot::default()),
        }
    }

    /// Apply a [`CognitionEvent`] and update the snapshot accordingly.
    pub fn apply_event(&self, event: &CognitionEvent) {
        let mut snap = self.snapshot.write().unwrap();
        let changed = Self::apply_to_snapshot(&mut snap, event);
        if changed {
            snap.last_updated_at = now_epoch();
            debug!(
                target: "desktop_awareness",
                summary = %event.summary(),
                "Snapshot updated"
            );
            let snap_clone = snap.clone();
            drop(snap);
            self.persist_snapshot(&snap_clone);
        }
    }

    /// Return a clone of the current snapshot.
    pub fn snapshot(&self) -> DesktopAwarenessSnapshot {
        self.snapshot.read().unwrap().clone()
    }

    /// Spawn a task that continuously applies events from a bus receiver.
    ///
    /// The task terminates when the receiver is dropped (bus closed) or the
    /// `CancellationToken` fires.
    pub fn start_listener(
        self: Arc<Self>,
        mut rx: tokio::sync::broadcast::Receiver<CognitionEvent>,
        cancel: tokio_util::sync::CancellationToken,
    ) {
        tokio::spawn(async move {
            loop {
                tokio::select! {
                    biased;
                    _ = cancel.cancelled() => break,
                    result = rx.recv() => {
                        match result {
                            Ok(event) => self.apply_event(&event),
                            Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                                debug!(
                                    target: "desktop_awareness",
                                    skipped = n,
                                    "Receiver lagged — skipping events"
                                );
                            }
                            Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                        }
                    }
                }
            }
        });
    }

    // ── Internal ─────────────────────────────────────────────────────────────

    /// Apply an event to a snapshot. Returns `true` if the snapshot changed.
    fn apply_to_snapshot(snap: &mut DesktopAwarenessSnapshot, event: &CognitionEvent) -> bool {
        match event {
            CognitionEvent::Browser(b) => {
                snap.browser.url = b.url.clone();
                snap.browser.title = b.title.clone();
                match &b.kind {
                    BrowserEventKind::AuthInterrupt { service_hint } => {
                        snap.browser.auth_interrupt = true;
                        snap.browser.auth_service_hint = Some(service_hint.clone());
                    }
                    BrowserEventKind::Navigated | BrowserEventKind::PageLoaded => {
                        snap.browser.auth_interrupt = false;
                        snap.browser.auth_service_hint = None;
                    }
                    _ => {}
                }
                true
            }

            CognitionEvent::Ide(i) => {
                if let Some(ref root) = i.workspace_root {
                    snap.ide.workspace_root = Some(root.clone());
                }
                match &i.kind {
                    IdeEventKind::BuildSucceeded => {
                        snap.ide.last_build_ok = Some(true);
                    }
                    IdeEventKind::BuildFailed { error_count, .. } => {
                        snap.ide.last_build_ok = Some(false);
                        snap.ide.error_count = *error_count;
                    }
                    IdeEventKind::DiagnosticsChanged {
                        error_count,
                        warning_count,
                    } => {
                        snap.ide.error_count = *error_count;
                        snap.ide.warning_count = *warning_count;
                    }
                    IdeEventKind::ActiveFileChanged { path } => {
                        snap.ide.active_file = Some(path.clone());
                    }
                    IdeEventKind::RuntimeFailure { .. } => {
                        snap.ide.last_build_ok = Some(false);
                    }
                }
                true
            }

            CognitionEvent::Desktop(d) => {
                match &d.kind {
                    DesktopCognitionEventKind::FocusChanged { to, .. } => {
                        snap.active_app = Some(to.clone());
                        snap.active_window_title = Some(d.app_name.clone());
                    }
                    DesktopCognitionEventKind::WindowAppeared { title, is_dialog } => {
                        if *is_dialog {
                            snap.has_dialog = true;
                        }
                        snap.active_window_title = Some(title.clone());
                    }
                    DesktopCognitionEventKind::WindowClosed { .. } => {
                        snap.has_dialog = false;
                    }
                    DesktopCognitionEventKind::AppLaunched => {
                        snap.active_app = Some(d.app_name.clone());
                    }
                    _ => {}
                }
                true
            }

            CognitionEvent::Workflow(w) => {
                match &w.kind {
                    WorkflowEventKind::Started => {
                        snap.active_workflow_id = Some(w.session_id.clone());
                    }
                    WorkflowEventKind::Completed { .. }
                    | WorkflowEventKind::Failed { .. }
                    | WorkflowEventKind::Paused { .. } => {
                        if snap.active_workflow_id.as_deref() == Some(&w.session_id) {
                            snap.active_workflow_id = None;
                        }
                    }
                    _ => {}
                }
                true
            }

            _ => false, // Continuation/Policy/Suggestion events don't update snapshot
        }
    }

    fn persist_snapshot(&self, snap: &DesktopAwarenessSnapshot) {
        if let Some(ref psdg) = self.psdg {
            if !snap.browser.url.is_empty() {
                psdg.record_fact(
                    "desktop.browser.url",
                    "aware_of",
                    &snap.browser.url,
                    0.90,
                    FactSource::Detected,
                    "",
                );
            }
            if let Some(ref root) = snap.ide.workspace_root {
                psdg.record_fact(
                    "desktop.ide.workspace",
                    "aware_of",
                    root,
                    0.90,
                    FactSource::Detected,
                    "",
                );
            }
            if let Some(ref app) = snap.active_app {
                psdg.record_fact(
                    "desktop.active_app",
                    "aware_of",
                    app,
                    0.85,
                    FactSource::Detected,
                    "",
                );
            }
        }
    }
}

fn now_epoch() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::cognition_event_bus::*;

    fn runtime() -> DesktopAwarenessRuntime {
        DesktopAwarenessRuntime::new(None)
    }

    fn browser_nav(url: &str) -> CognitionEvent {
        CognitionEvent::Browser(BrowserCognitionEvent {
            url: url.to_string(),
            title: "Test Page".to_string(),
            kind: BrowserEventKind::Navigated,
        })
    }

    fn ide_build_ok() -> CognitionEvent {
        CognitionEvent::Ide(IdeCognitionEvent {
            workspace_root: Some("/proj".to_string()),
            kind: IdeEventKind::BuildSucceeded,
        })
    }

    fn ide_build_fail(errors: usize) -> CognitionEvent {
        CognitionEvent::Ide(IdeCognitionEvent {
            workspace_root: Some("/proj".to_string()),
            kind: IdeEventKind::BuildFailed {
                error_count: errors,
                first_error: "undefined symbol".to_string(),
            },
        })
    }

    fn wf_started(id: &str) -> CognitionEvent {
        CognitionEvent::Workflow(WorkflowEvent {
            session_id: id.to_string(),
            description: "test".to_string(),
            kind: WorkflowEventKind::Started,
        })
    }

    fn wf_completed(id: &str) -> CognitionEvent {
        CognitionEvent::Workflow(WorkflowEvent {
            session_id: id.to_string(),
            description: "test".to_string(),
            kind: WorkflowEventKind::Completed { duration_ms: 100 },
        })
    }

    #[test]
    fn browser_nav_updates_url() {
        let r = runtime();
        r.apply_event(&browser_nav("https://example.com"));
        assert_eq!(r.snapshot().browser.url, "https://example.com");
    }

    #[test]
    fn ide_build_ok_updates_last_build() {
        let r = runtime();
        r.apply_event(&ide_build_ok());
        assert_eq!(r.snapshot().ide.last_build_ok, Some(true));
    }

    #[test]
    fn ide_build_fail_updates_error_count() {
        let r = runtime();
        r.apply_event(&ide_build_fail(3));
        let snap = r.snapshot();
        assert_eq!(snap.ide.error_count, 3);
        assert_eq!(snap.ide.last_build_ok, Some(false));
    }

    #[test]
    fn workflow_started_sets_active_id() {
        let r = runtime();
        r.apply_event(&wf_started("s1"));
        assert_eq!(r.snapshot().active_workflow_id.as_deref(), Some("s1"));
    }

    #[test]
    fn workflow_completed_clears_active_id() {
        let r = runtime();
        r.apply_event(&wf_started("s1"));
        r.apply_event(&wf_completed("s1"));
        assert!(r.snapshot().active_workflow_id.is_none());
    }

    #[test]
    fn is_clean_no_workflow_no_dialog() {
        let r = runtime();
        assert!(r.snapshot().is_clean());
    }

    #[test]
    fn is_clean_false_when_workflow_active() {
        let r = runtime();
        r.apply_event(&wf_started("s1"));
        assert!(!r.snapshot().is_clean());
    }

    #[test]
    fn dialog_appears_sets_has_dialog() {
        let r = runtime();
        r.apply_event(&CognitionEvent::Desktop(DesktopCognitionEvent {
            app_name: "system".to_string(),
            kind: DesktopCognitionEventKind::WindowAppeared {
                title: "Permission required".to_string(),
                is_dialog: true,
            },
        }));
        assert!(r.snapshot().has_dialog);
    }

    #[test]
    fn dialog_close_clears_has_dialog() {
        let r = runtime();
        r.apply_event(&CognitionEvent::Desktop(DesktopCognitionEvent {
            app_name: "system".to_string(),
            kind: DesktopCognitionEventKind::WindowAppeared {
                title: "Dialog".to_string(),
                is_dialog: true,
            },
        }));
        r.apply_event(&CognitionEvent::Desktop(DesktopCognitionEvent {
            app_name: "system".to_string(),
            kind: DesktopCognitionEventKind::WindowClosed {
                title: "Dialog".to_string(),
            },
        }));
        assert!(!r.snapshot().has_dialog);
    }

    #[test]
    fn auth_interrupt_sets_flag() {
        let r = runtime();
        r.apply_event(&CognitionEvent::Browser(BrowserCognitionEvent {
            url: "https://accounts.google.com".to_string(),
            title: "Sign in".to_string(),
            kind: BrowserEventKind::AuthInterrupt {
                service_hint: "google".to_string(),
            },
        }));
        assert!(r.snapshot().browser.auth_interrupt);
        assert_eq!(
            r.snapshot().browser.auth_service_hint.as_deref(),
            Some("google")
        );
    }

    #[test]
    fn ide_workspace_root_tracked() {
        let r = runtime();
        r.apply_event(&ide_build_ok());
        assert_eq!(r.snapshot().ide.workspace_root.as_deref(), Some("/proj"));
    }
}
