//! PsdgCoordinator — background PerceptionBus subscriber that writes semantic
//! desktop state into WorldModelStore.
//!
//! # Design
//!
//! The coordinator subscribes to `PerceptionBus` and translates system events
//! into WorldModelStore writes via `PsdgHandle`:
//!
//! ```text
//! PerceptionBus
//!   DesktopOp::FocusChanged   → record_app_focus
//!   DesktopOp::WindowCreated  → record desktop topology fact
//!   DesktopOp::WindowDestroyed → prune desktop topology fact
//!   FilesystemOp::*           → record file change fact (scoped to watched dirs)
//!   ProcessLifecycle          → record process state fact
//! ```
//!
//! # Safety Invariants
//!
//! - Coordinator runs at `Priority::Background` in `ExecutiveController`.
//! - Coordinator NEVER calls any tool, executes any action, or bypasses HITL.
//! - All writes are fire-and-forget via `PsdgHandle`. Coordinator task failures
//!   are non-fatal — they log at `warn` and the coordinator restarts.
//! - The coordinator loop yields on `CancellationToken` cancellation immediately.

use tokio::sync::broadcast;
use tokio_util::sync::CancellationToken;
use tracing::{debug, warn};

use crate::agent::perception::{DesktopOp, EventKind, FilesystemOp, PerceptionEvent};
use crate::agent::psdg::PsdgHandle;
use crate::agent::world_model::FactSource;

/// Configuration for the `PsdgCoordinator`.
#[derive(Debug, Clone)]
pub struct PsdgCoordinatorConfig {
    /// Enable writing desktop focus events to WorldModelStore.
    pub track_focus: bool,
    /// Enable writing window lifecycle events.
    pub track_window_lifecycle: bool,
    /// Enable writing filesystem change events (scoped paths only).
    pub track_filesystem: bool,
    /// Enable writing process lifecycle events.
    pub track_processes: bool,
    /// Maximum filesystem events to write per second (rate-limiting).
    pub max_fs_events_per_sec: u32,
}

impl Default for PsdgCoordinatorConfig {
    fn default() -> Self {
        Self {
            track_focus: true,
            track_window_lifecycle: true,
            track_filesystem: true,
            track_processes: true,
            max_fs_events_per_sec: 10,
        }
    }
}

/// Background coordinator that subscribes to `PerceptionBus` and drives
/// WorldModelStore updates for desktop semantic state.
pub struct PsdgCoordinator {
    psdg: PsdgHandle,
    config: PsdgCoordinatorConfig,
}

impl PsdgCoordinator {
    pub fn new(psdg: PsdgHandle, config: PsdgCoordinatorConfig) -> Self {
        Self { psdg, config }
    }

    /// Spawn the coordinator as a background Tokio task.
    ///
    /// The task runs until `cancel` is triggered. Returns the `JoinHandle`
    /// so the caller can await shutdown.
    pub fn spawn(
        self,
        mut rx: broadcast::Receiver<PerceptionEvent>,
        cancel: CancellationToken,
    ) -> tokio::task::JoinHandle<()> {
        tokio::spawn(async move {
            debug!(target: "psdg::coordinator", "PsdgCoordinator started");
            let mut fs_event_count_this_sec: u32 = 0;
            let mut fs_window_start = tokio::time::Instant::now();

            // Storm detection: track total events per second across all types.
            // If throughput exceeds STORM_THRESHOLD_PER_SEC, log a warning and
            // begin dropping non-focus events to prevent graph bloat.
            // Threshold is set to 2000/s — app launches (VS Code, browsers)
            // legitimately generate 2500–14000 AT-SPI events/sec during startup.
            // The old 200/s threshold fired on every app open, dropping valid
            // focus events and causing verifier false-negatives.
            //
            // Carry-over: if the previous second was a storm, pre-arm the drop
            // for the next second so the window boundary doesn't momentarily
            // let non-focus events through before the counter reaches 2000 again.
            let mut total_events_this_sec: u32 = 0;
            let mut total_window_start = tokio::time::Instant::now();
            let mut prev_second_was_storm = false;
            let mut lagged_events_since_warn: u64 = 0;
            let mut lagged_drops_since_warn: u64 = 0;
            let mut lagged_window_start = tokio::time::Instant::now();
            const STORM_THRESHOLD_PER_SEC: u32 = 2000;
            const LAG_WARNING_INTERVAL_SECS: u64 = 5;

            loop {
                tokio::select! {
                    _ = cancel.cancelled() => {
                        debug!(target: "psdg::coordinator", "PsdgCoordinator cancelled");
                        break;
                    }
                    event = rx.recv() => {
                        match event {
                            Ok(ev) => {
                                // ── Storm detection ──────────────────────────
                                let total_elapsed = total_window_start.elapsed();
                                if total_elapsed.as_secs() >= 1 {
                                    if total_events_this_sec > STORM_THRESHOLD_PER_SEC {
                                        warn!(
                                            target: "psdg::coordinator",
                                            events_per_sec = total_events_this_sec,
                                            threshold = STORM_THRESHOLD_PER_SEC,
                                            "Event storm detected — non-focus events will be dropped this second"
                                        );
                                        prev_second_was_storm = true;
                                    } else {
                                        if prev_second_was_storm {
                                            tracing::debug!(
                                                target: "psdg::coordinator",
                                                events_prev_sec = total_events_this_sec,
                                                "Event storm subsided — resuming normal processing"
                                            );
                                        }
                                        prev_second_was_storm = false;
                                    }
                                    total_events_this_sec = 0;
                                    total_window_start = tokio::time::Instant::now();
                                }
                                total_events_this_sec += 1;

                                // Drop non-focus events during a storm to prevent bloat.
                                // Also apply carry-over: if the previous second was a storm,
                                // keep dropping until the current second proves it's calm
                                // (avoids the 2000-event grace window at each boundary).
                                let under_storm = total_events_this_sec > STORM_THRESHOLD_PER_SEC
                                    || prev_second_was_storm;
                                if under_storm {
                                    // Only allow focus events through during a storm
                                    if !matches!(ev.kind, EventKind::DesktopEvent(DesktopOp::FocusChanged)) {
                                        continue;
                                    }
                                }

                                self.handle_event(
                                    ev,
                                    &mut fs_event_count_this_sec,
                                    &mut fs_window_start,
                                );
                            }
                            Err(broadcast::error::RecvError::Lagged(n)) => {
                                lagged_events_since_warn += 1;
                                lagged_drops_since_warn =
                                    lagged_drops_since_warn.saturating_add(n);
                                if lagged_window_start.elapsed().as_secs()
                                    >= LAG_WARNING_INTERVAL_SECS
                                {
                                    warn!(
                                        target: "psdg::coordinator",
                                        dropped = lagged_drops_since_warn,
                                        lagged_events = lagged_events_since_warn,
                                        window_secs = lagged_window_start.elapsed().as_secs(),
                                        "PerceptionBus lagged — events dropped (coalesced, non-fatal)"
                                    );
                                    lagged_events_since_warn = 0;
                                    lagged_drops_since_warn = 0;
                                    lagged_window_start = tokio::time::Instant::now();
                                }
                            }
                            Err(broadcast::error::RecvError::Closed) => {
                                debug!(target: "psdg::coordinator", "PerceptionBus closed — coordinator stopping");
                                break;
                            }
                        }
                    }
                }
            }
            debug!(target: "psdg::coordinator", "PsdgCoordinator stopped");
        })
    }

    fn handle_event(
        &self,
        event: PerceptionEvent,
        fs_event_count: &mut u32,
        fs_window_start: &mut tokio::time::Instant,
    ) {
        match &event.kind {
            EventKind::DesktopEvent(op) if self.config.track_focus => {
                self.handle_desktop_op(op, &event);
            }
            EventKind::Filesystem(op) if self.config.track_filesystem => {
                self.handle_filesystem_event(op, &event, fs_event_count, fs_window_start);
            }
            EventKind::ProcessLifecycle(proc_event) if self.config.track_processes => {
                self.handle_process_event(proc_event, &event);
            }
            _ => {}
        }
    }

    fn handle_desktop_op(&self, op: &DesktopOp, event: &PerceptionEvent) {
        match op {
            DesktopOp::FocusChanged => {
                // Extract app name from the event primary_path or summary
                // The AT-SPI engine or X11 listener populates primary_path with the focused app
                if let Some(ref app_path) = event.primary_path {
                    let app_id = sanitize_app_id(app_path);
                    let app_name = extract_app_name(app_path);
                    self.psdg.record_app_focus(&app_id, &app_name);
                    debug!(
                        target: "psdg::coordinator",
                        app_id = %app_id,
                        "FocusChanged → recorded app focus"
                    );
                }
            }
            DesktopOp::WindowCreated => {
                if self.config.track_window_lifecycle {
                    if let Some(ref window_path) = event.primary_path {
                        let app_id = sanitize_app_id(window_path);
                        self.psdg.record_fact(
                            &app_id,
                            "window_state",
                            "open",
                            0.90,
                            FactSource::Detected,
                            "perception:WindowCreated",
                        );
                    }
                }
            }
            DesktopOp::WindowDestroyed => {
                if self.config.track_window_lifecycle {
                    if let Some(ref window_path) = event.primary_path {
                        let app_id = sanitize_app_id(window_path);
                        // Mark as closed — Bayesian update will reduce confidence
                        self.psdg.record_fact(
                            &app_id,
                            "window_state",
                            "closed",
                            0.95,
                            FactSource::Detected,
                            "perception:WindowDestroyed",
                        );
                    }
                }
            }
            DesktopOp::WorkspaceChanged => {
                if let Some(ref ws) = event.primary_path {
                    self.psdg.record_fact(
                        "desktop_environment",
                        "active_workspace",
                        ws,
                        0.95,
                        FactSource::Detected,
                        "perception:WorkspaceChanged",
                    );
                }
            }
        }
    }

    fn handle_filesystem_event(
        &self,
        op: &FilesystemOp,
        event: &PerceptionEvent,
        fs_event_count: &mut u32,
        fs_window_start: &mut tokio::time::Instant,
    ) {
        // Rate-limit filesystem events to avoid graph bloat from rapid changes
        let elapsed = fs_window_start.elapsed();
        if elapsed.as_secs() >= 1 {
            *fs_event_count = 0;
            *fs_window_start = tokio::time::Instant::now();
        }
        if *fs_event_count >= self.config.max_fs_events_per_sec {
            return;
        }
        *fs_event_count += 1;

        if let Some(ref path) = event.primary_path {
            let predicate = match op {
                FilesystemOp::Created => "file_created",
                FilesystemOp::Modified => "file_modified",
                FilesystemOp::Deleted => "file_deleted",
                FilesystemOp::Renamed => "file_renamed",
            };
            // Subject is the parent directory (not full path to avoid graph explosion)
            let subject = parent_dir_subject(path);
            self.psdg.record_fact(
                &subject,
                predicate,
                path,
                0.80,
                FactSource::Detected,
                &format!("perception:Filesystem:{predicate}"),
            );
        }
    }

    fn handle_process_event(&self, proc_event: &str, event: &PerceptionEvent) {
        // proc_event is a string like "started:firefox" or "crashed:code"
        let (state, process) = proc_event.split_once(':').unwrap_or(("event", proc_event));

        let subject = format!("process:{process}");
        self.psdg.record_fact(
            &subject,
            "lifecycle_state",
            state,
            0.90,
            FactSource::Detected,
            &format!("perception:ProcessLifecycle:{state}"),
        );

        if let Some(ref path) = event.primary_path {
            self.psdg.record_fact(
                &subject,
                "binary_path",
                path,
                0.95,
                FactSource::Detected,
                "perception:ProcessLifecycle",
            );
        }
    }
}

// ─── Helpers ─────────────────────────────────────────────────────────────────

/// Convert a raw app path/name into a stable, sanitized PSDG subject ID.
fn sanitize_app_id(raw: &str) -> String {
    // Use last path component, lowercase, alphanumeric+hyphen only
    let base = raw.rsplit(['/', '\\']).next().unwrap_or(raw);
    let sanitized: String = base
        .to_lowercase()
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || c == '-' || c == '.' {
                c
            } else {
                '_'
            }
        })
        .collect();
    // Strip trailing extension
    sanitized
        .rsplit_once('.')
        .map(|(stem, _)| stem.to_string())
        .unwrap_or(sanitized)
}

/// Extract a human-readable app name from a path/window title.
fn extract_app_name(raw: &str) -> String {
    sanitize_app_id(raw)
        .replace('_', " ")
        .split_whitespace()
        .map(|w| {
            let mut c = w.chars();
            match c.next() {
                None => String::new(),
                Some(f) => f.to_uppercase().to_string() + c.as_str(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// Map a file path to a directory-level subject for the PSDG.
///
/// This prevents graph explosion from tracking every individual file.
fn parent_dir_subject(path: &str) -> String {
    std::path::Path::new(path)
        .parent()
        .and_then(|p| p.to_str())
        .filter(|p| !p.is_empty())
        .map(|p| format!("dir:{p}"))
        .unwrap_or_else(|| "dir:unknown".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitize_app_id_strips_extension() {
        assert_eq!(sanitize_app_id("/usr/bin/firefox"), "firefox");
        assert_eq!(sanitize_app_id("code.desktop"), "code");
        assert_eq!(sanitize_app_id("Google Chrome"), "google_chrome");
    }

    #[test]
    fn extract_app_name_capitalizes() {
        assert_eq!(extract_app_name("/usr/bin/firefox"), "Firefox");
        assert_eq!(extract_app_name("visual_studio_code"), "Visual Studio Code");
    }

    #[test]
    fn parent_dir_subject_extracts_directory() {
        assert_eq!(
            parent_dir_subject("/home/obaid/projects/kria/src/main.rs"),
            "dir:/home/obaid/projects/kria/src"
        );
        assert_eq!(parent_dir_subject("file.txt"), "dir:unknown");
    }
}
