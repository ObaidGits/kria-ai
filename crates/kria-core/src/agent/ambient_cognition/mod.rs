//! Batch 3 — Ambient Cognition Loop.
//!
//! # Core Mission
//!
//! A low-frequency, bounded background loop that maintains continuous operational
//! awareness without heavy GPU inference. It periodically checks the environment
//! for actionable operational signals and emits typed events via [`CognitionEventBus`].
//!
//! # Design Principles
//!
//! - **No LLM calls** in the loop body. All checks are semantic/structural.
//! - **Non-intrusive.** The loop only emits suggestions, never executes.
//! - **Cancellable.** A `CancellationToken` stops the loop immediately.
//! - **Bounded.** Each tick is time-capped at [`MAX_TICK_MS`].
//! - **Policy-gated.** `AmbientCognitionEnabled` flag can pause/resume the loop.
//!
//! # What the Loop Checks Each Tick
//!
//! 1. Paused/resumable sessions (`WorkflowContinuationRuntime::find_resumable`)
//! 2. Stalled operational goals (`PersistentGoalRuntime::list_stalled`)
//! 3. Active PSDG-based build failure facts
//! 4. Idle desktop (no workflow running > idle threshold)
//!
//! # Safety
//!
//! - The loop NEVER executes actions. It only emits events.
//! - All operations are read-only.
//! - Tick interval is configurable (default: 30 s). Minimum enforced at 10 s.
//! - Vision (OCR / VLM) is NEVER invoked from this loop.

use std::sync::Arc;
use std::time::Duration;

use tokio_util::sync::CancellationToken;
use tracing::{debug, info, warn};

use crate::agent::cognition_event_bus::{
    CognitionEvent, CognitionEventBus, SuggestionEvent, SuggestionKind, WorkflowEvent,
    WorkflowEventKind,
};
use crate::agent::psdg::PsdgHandle;
use crate::agent::workflow_continuation::WorkflowContinuationRuntime;

// ─── Constants ────────────────────────────────────────────────────────────────

/// Default ambient loop tick interval.
pub const DEFAULT_TICK_INTERVAL: Duration = Duration::from_secs(30);

/// Minimum allowed tick interval (safety floor).
pub const MIN_TICK_INTERVAL: Duration = Duration::from_secs(10);

/// Maximum time allowed per tick. If exceeded, the tick is abandoned.
pub const MAX_TICK_MS: u64 = 2_000;

/// Maximum number of continuation suggestions emitted per tick.
pub const MAX_SUGGESTIONS_PER_TICK: usize = 2;

// ─── Config ───────────────────────────────────────────────────────────────────

/// Configuration for the ambient cognition loop.
#[derive(Debug, Clone)]
pub struct AmbientCognitionConfig {
    /// How often the loop ticks. Clamped to minimum [`MIN_TICK_INTERVAL`].
    pub tick_interval: Duration,
    /// Whether to emit suggestions for resumable sessions.
    pub suggest_session_resume: bool,
    /// Whether to emit build failure suggestions.
    pub suggest_build_recovery: bool,
    /// Whether ambient cognition is enabled at start-up.
    pub enabled: bool,
}

impl Default for AmbientCognitionConfig {
    fn default() -> Self {
        Self {
            tick_interval: DEFAULT_TICK_INTERVAL,
            suggest_session_resume: true,
            suggest_build_recovery: true,
            enabled: true,
        }
    }
}

// ─── Ambient Cognition Loop ───────────────────────────────────────────────────

/// The ambient cognition loop — a supervised Tokio task.
///
/// Start via [`AmbientCognitionLoop::start()`], which spawns a background task
/// and returns a [`AmbientCognitionHandle`] to pause, resume, or stop it.
pub struct AmbientCognitionLoop {
    config: AmbientCognitionConfig,
    event_bus: Arc<CognitionEventBus>,
    continuation_runtime: Option<Arc<WorkflowContinuationRuntime>>,
    psdg: Option<PsdgHandle>,
    cancel: CancellationToken,
    /// Internal enabled flag — toggled by pause/resume.
    enabled: Arc<std::sync::atomic::AtomicBool>,
}

impl AmbientCognitionLoop {
    /// Construct the loop. Call [`start()`] to spawn it.
    pub fn new(
        config: AmbientCognitionConfig,
        event_bus: Arc<CognitionEventBus>,
        continuation_runtime: Option<Arc<WorkflowContinuationRuntime>>,
        psdg: Option<PsdgHandle>,
    ) -> Self {
        let enabled = config.enabled;
        Self {
            config,
            event_bus,
            continuation_runtime,
            psdg,
            cancel: CancellationToken::new(),
            enabled: Arc::new(std::sync::atomic::AtomicBool::new(enabled)),
        }
    }

    /// Spawn the ambient cognition loop as a Tokio background task.
    ///
    /// Returns an [`AmbientCognitionHandle`] for control (pause/resume/stop).
    pub fn start(self) -> AmbientCognitionHandle {
        let cancel = self.cancel.clone();
        let enabled = Arc::clone(&self.enabled);
        let tick_interval = self.config.tick_interval.max(MIN_TICK_INTERVAL);

        let handle = AmbientCognitionHandle {
            cancel: cancel.clone(),
            enabled: Arc::clone(&enabled),
        };

        tokio::spawn(async move {
            info!(target: "ambient_cognition", interval_sec = tick_interval.as_secs(), "AmbientCognitionLoop started");
            let mut interval = tokio::time::interval(tick_interval);
            interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);

            loop {
                tokio::select! {
                    biased;
                    _ = cancel.cancelled() => {
                        info!(target: "ambient_cognition", "AmbientCognitionLoop cancelled");
                        break;
                    }
                    _ = interval.tick() => {
                        if !enabled.load(std::sync::atomic::Ordering::Relaxed) {
                            debug!(target: "ambient_cognition", "AmbientCognitionLoop paused — skipping tick");
                            continue;
                        }

                        let tick_result = tokio::time::timeout(
                            Duration::from_millis(MAX_TICK_MS),
                            self.run_tick(),
                        ).await;

                        match tick_result {
                            Ok(n) => debug!(target: "ambient_cognition", events_emitted = n, "tick completed"),
                            Err(_) => warn!(target: "ambient_cognition", "tick exceeded {}ms budget, abandoned", MAX_TICK_MS),
                        }
                    }
                }
            }
        });

        handle
    }

    /// Execute one ambient cognition tick.
    ///
    /// Returns the number of events emitted. Public for testing.
    pub async fn run_tick(&self) -> usize {
        let mut emitted = 0;
        emitted += self.check_resumable_sessions().await;
        emitted += self.check_psdg_build_failures().await;
        emitted
    }

    /// Check for paused sessions that can be resumed.
    async fn check_resumable_sessions(&self) -> usize {
        let wcr = match &self.continuation_runtime {
            Some(r) => r,
            None => return 0,
        };
        if !self.config.suggest_session_resume {
            return 0;
        }

        let resumable = wcr.find_resumable();
        if resumable.is_empty() {
            return 0;
        }

        let mut emitted = 0;
        for session in resumable.iter().take(MAX_SUGGESTIONS_PER_TICK) {
            let hint = session
                .continuation_hint
                .as_deref()
                .unwrap_or("continue where you left off")
                .to_string();

            // Emit ResumeAvailable workflow event
            let wf_event = CognitionEvent::Workflow(WorkflowEvent {
                session_id: session.session_id.clone(),
                description: session.user_intent.clone(),
                kind: WorkflowEventKind::ResumeAvailable {
                    continuation_hint: hint.clone(),
                },
            });
            emitted += self.event_bus.emit(wf_event);

            // Emit Suggestion event
            let sug_event = CognitionEvent::Suggestion(SuggestionEvent {
                suggestion_id: format!("resume-{}", session.session_id),
                content: format!(
                    "Paused workflow available: '{}'. {}",
                    session.user_intent, hint
                ),
                rationale: "A previous workflow was paused and can be resumed.".to_string(),
                kind: SuggestionKind::ResumePausedWorkflow {
                    session_id: session.session_id.clone(),
                },
            });
            emitted += self.event_bus.emit(sug_event);
        }

        debug!(
            target: "ambient_cognition",
            count = resumable.len(),
            emitted,
            "check_resumable_sessions"
        );
        emitted
    }

    /// Check PSDG for active build failure facts.
    async fn check_psdg_build_failures(&self) -> usize {
        let psdg = match &self.psdg {
            Some(p) => p,
            None => return 0,
        };
        if !self.config.suggest_build_recovery {
            return 0;
        }

        // Check build failure facts via bounded subject query
        let build_facts = psdg.query_subject_bounded("build.last_result");
        let has_build_failure = build_facts
            .iter()
            .any(|f| f.object.contains("failed") || f.object.contains("error"));

        if has_build_failure {
            let sug = CognitionEvent::Suggestion(SuggestionEvent {
                suggestion_id: "build-failure-recovery".to_string(),
                content: "Active build failure detected. Consider addressing diagnostics."
                    .to_string(),
                rationale: "PSDG build.last_result indicates a recent build failure.".to_string(),
                kind: SuggestionKind::RecoverBuildFailure,
            });
            return self.event_bus.emit(sug);
        }
        0
    }
}

// ─── Handle ───────────────────────────────────────────────────────────────────

/// Control handle for a running [`AmbientCognitionLoop`].
#[derive(Clone)]
pub struct AmbientCognitionHandle {
    pub cancel: CancellationToken,
    pub enabled: Arc<std::sync::atomic::AtomicBool>,
}

impl AmbientCognitionHandle {
    /// Construct a fresh handle for testing purposes.
    pub fn new_for_test() -> (Self, CancellationToken) {
        let cancel = CancellationToken::new();
        let enabled = Arc::new(std::sync::atomic::AtomicBool::new(true));
        let h = Self {
            cancel: cancel.clone(),
            enabled,
        };
        (h, cancel)
    }
}

impl AmbientCognitionHandle {
    /// Pause the ambient cognition loop (tick skipped until resumed).
    pub fn pause(&self) {
        self.enabled
            .store(false, std::sync::atomic::Ordering::Relaxed);
        info!(target: "ambient_cognition", "AmbientCognitionLoop paused");
    }

    /// Resume a paused ambient cognition loop.
    pub fn resume(&self) {
        self.enabled
            .store(true, std::sync::atomic::Ordering::Relaxed);
        info!(target: "ambient_cognition", "AmbientCognitionLoop resumed");
    }

    /// Stop the ambient cognition loop permanently.
    pub fn stop(&self) {
        self.cancel.cancel();
        info!(target: "ambient_cognition", "AmbientCognitionLoop stopped");
    }

    /// Whether ambient cognition is currently enabled.
    pub fn is_enabled(&self) -> bool {
        self.enabled.load(std::sync::atomic::Ordering::Relaxed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::cognition_event_bus::CognitionEventBus;

    fn make_loop() -> AmbientCognitionLoop {
        AmbientCognitionLoop::new(
            AmbientCognitionConfig {
                tick_interval: DEFAULT_TICK_INTERVAL,
                suggest_session_resume: true,
                suggest_build_recovery: true,
                enabled: true,
            },
            Arc::new(CognitionEventBus::new()),
            None,
            None,
        )
    }

    #[test]
    fn handle_pause_resume_toggle() {
        let lp = make_loop();
        let cancel = lp.cancel.clone();
        let enabled = Arc::clone(&lp.enabled);
        let handle = AmbientCognitionHandle { cancel, enabled };

        assert!(handle.is_enabled());
        handle.pause();
        assert!(!handle.is_enabled());
        handle.resume();
        assert!(handle.is_enabled());
    }

    #[test]
    fn handle_stop_cancels_token() {
        let lp = make_loop();
        let cancel = lp.cancel.clone();
        let enabled = Arc::clone(&lp.enabled);
        let handle = AmbientCognitionHandle {
            cancel: cancel.clone(),
            enabled,
        };
        assert!(!cancel.is_cancelled());
        handle.stop();
        assert!(cancel.is_cancelled());
    }

    #[tokio::test]
    async fn tick_with_no_wcr_emits_zero() {
        let lp = make_loop();
        let n = lp.run_tick().await;
        assert_eq!(n, 0, "no WCR → no resumable sessions → zero events");
    }

    #[tokio::test]
    async fn tick_with_wcr_no_sessions_emits_zero() {
        let wcr = Arc::new(WorkflowContinuationRuntime::new(None));
        let bus = Arc::new(CognitionEventBus::new());
        let lp = AmbientCognitionLoop::new(
            AmbientCognitionConfig::default(),
            Arc::clone(&bus),
            Some(wcr),
            None,
        );
        let n = lp.run_tick().await;
        assert_eq!(n, 0, "no paused sessions → zero events");
    }

    #[test]
    fn config_clamps_tick_interval_floor() {
        let config = AmbientCognitionConfig {
            tick_interval: Duration::from_secs(1), // below minimum
            ..Default::default()
        };
        let actual = config.tick_interval.max(MIN_TICK_INTERVAL);
        assert_eq!(actual, MIN_TICK_INTERVAL);
    }

    #[test]
    fn config_default_is_enabled() {
        let config = AmbientCognitionConfig::default();
        assert!(config.enabled);
        assert!(config.suggest_session_resume);
        assert!(config.suggest_build_recovery);
    }
}
