//! Phase 4 — Workflow Continuation Runtime.
//!
//! # Core Mission
//!
//! Enable KRIA to safely pause, resume, and recover interrupted long-horizon
//! workflows. This is the operational layer that bridges:
//!
//! - `SessionManager` (file checkpoint persistence)
//! - `PsdgHandle` (semantic context rehydration)
//! - `CollaborativeAutonomyEngine` (interruption decisions)
//!
//! # Key Capabilities
//!
//! 1. **Pause** a workflow at any stage boundary — preserves all context.
//! 2. **Resume** from a checkpoint — rehydrates PSDG + semantic context.
//! 3. **Classify interruptions** — popup/auth/focus-theft/crash/network drop.
//! 4. **Plan bounded recovery** — retry/skip/escalate/rollback.
//! 5. **Context rehydration** — restore WorldModelStore snapshot to pre-pause state.
//!
//! # Architectural Invariants
//!
//! - Recovery trees are bounded to depth 2 (no infinite loops).
//! - All pause/resume operations are atomic (write-then-rename file semantics).
//! - Semantic context rehydration uses PSDG; ephemeral state is not carried.
//! - KRIA never autonomously retries more than `MAX_RECOVERY_DEPTH` times.

use serde::{Deserialize, Serialize};
use tracing::{debug, info, warn};

use crate::agent::psdg::PsdgHandle;
use crate::agent::workflow_session::{SessionManager, SessionStep, WorkflowSession};
use crate::agent::world_model::FactSource;

/// Maximum recovery depth before escalating to HITL.
pub const MAX_RECOVERY_DEPTH: u8 = 2;

// ─── Interruption Classification ──────────────────────────────────────────────

/// Classifies the type of interruption that paused a workflow.
///
/// Each class has a distinct recovery strategy and urgency level.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum InterruptionClass {
    /// A popup dialog appeared (system notification, update prompt, error dialog).
    Popup {
        title: String,
        /// True if this is an authentication prompt.
        is_auth: bool,
    },
    /// Another application stole keyboard focus.
    FocusTheft { stolen_by: String },
    /// Authentication is required to continue (sudo, SSH, OAuth).
    AuthRequired { service: String },
    /// The window compositor or display server crashed/restarted.
    CompositorEvent { description: String },
    /// The IDE has a conflicting edit lock on a file.
    IdeConflict { file: String },
    /// The browser navigated away or reloaded unexpectedly.
    BrowserStateChanged { url: String },
    /// Network connectivity was lost.
    NetworkDropped,
    /// A process in the workflow crashed.
    ProcessCrashed { binary: String },
    /// The user manually intervened (pressed a key, moved focus, cancelled).
    UserIntervened { description: String },
    /// The workflow timed out.
    Timeout { stage_label: String },
    /// The system resource was exhausted (disk full, OOM).
    ResourceExhausted { resource: String },
    /// Window focus verification failed after a launch/focus action.
    /// The app may not have finished raising its window yet.
    WindowFocusFailed {
        /// The app that was expected to be focused.
        app: String,
        /// The checkpoint failure reason from the stage result.
        reason: String,
    },
    /// The uinput daemon or vision sidecar stopped responding mid-workflow.
    /// This is an infrastructure-level failure, not a desktop event.
    InfrastructureFailure {
        /// Which service failed ("uinput_daemon", "vision_sidecar", or "gui_services").
        service: String,
        /// The raw halt/error reason from the service layer.
        reason: String,
    },
    /// Interruption cause could not be determined.
    Unknown,
}

impl InterruptionClass {
    /// Whether this interruption is typically transient (auto-resolvable).
    pub fn is_transient(&self) -> bool {
        matches!(
            self,
            InterruptionClass::Popup { is_auth: false, .. }
                | InterruptionClass::FocusTheft { .. }
                | InterruptionClass::BrowserStateChanged { .. }
                | InterruptionClass::Timeout { .. }
                | InterruptionClass::WindowFocusFailed { .. }
                | InterruptionClass::InfrastructureFailure { .. }
        )
    }

    /// Whether this interruption requires human action to resolve.
    pub fn requires_human(&self) -> bool {
        matches!(
            self,
            InterruptionClass::AuthRequired { .. }
                | InterruptionClass::Popup { is_auth: true, .. }
                | InterruptionClass::UserIntervened { .. }
                | InterruptionClass::ResourceExhausted { .. }
        )
    }

    /// Human-readable description for the pause message.
    pub fn user_message(&self) -> String {
        match self {
            Self::Popup { title, is_auth } => {
                if *is_auth {
                    format!("Authentication required: {}", title)
                } else {
                    format!("Popup dialog interrupted workflow: {}", title)
                }
            }
            Self::FocusTheft { stolen_by } => {
                format!("Focus moved to {} — workflow paused", stolen_by)
            }
            Self::AuthRequired { service } => format!("Authentication required for {}", service),
            Self::CompositorEvent { description } => format!("Display event: {}", description),
            Self::IdeConflict { file } => format!("IDE has conflicting edit on {}", file),
            Self::BrowserStateChanged { url } => format!("Browser navigated away from {}", url),
            Self::NetworkDropped => "Network connection dropped".into(),
            Self::ProcessCrashed { binary } => format!("{} crashed", binary),
            Self::UserIntervened { description } => format!("User intervention: {}", description),
            Self::Timeout { stage_label } => format!("Stage '{}' timed out", stage_label),
            Self::ResourceExhausted { resource } => format!("Resource exhausted: {}", resource),
            Self::WindowFocusFailed { app, .. } => {
                format!(
                    "Window focus verification failed for '{}' — app may not have raised yet",
                    app
                )
            }
            Self::InfrastructureFailure { service, .. } => {
                format!(
                    "GUI input service ('{}') stopped responding. \
                     KRIA is attempting automatic recovery. \
                     If this persists: restart KRIA and verify your sudoers file grants \
                     `NOPASSWD: /path/to/kria-uinput-daemon` without using `env` as an intermediary.",
                    service
                )
            }
            Self::Unknown => "Unknown interruption".into(),
        }
    }
}

// ─── Recovery Action ──────────────────────────────────────────────────────────

/// What should happen after an interruption is classified.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum RecoveryAction {
    /// Continue from the current stage (interruption was harmless).
    Continue,
    /// Retry the current stage after a delay.
    Retry { delay_ms: u64 },
    /// Skip the current stage and proceed to the next.
    SkipStage { reason: String },
    /// Escalate to the human — KRIA cannot safely recover.
    Escalate { reason: String },
    /// Roll back the last action and re-verify state.
    Rollback { description: String },
    /// Request human intervention to resolve the interruption.
    RequestHumanIntervention { question: String, context: String },
    /// Abort the workflow entirely.
    Abort { reason: String },
}

impl RecoveryAction {
    /// Returns `true` if this action requires pausing the workflow.
    pub fn requires_pause(&self) -> bool {
        matches!(
            self,
            RecoveryAction::Escalate { .. }
                | RecoveryAction::RequestHumanIntervention { .. }
                | RecoveryAction::Abort { .. }
        )
    }
}

// ─── Recovery Branch ─────────────────────────────────────────────────────────

/// A single branch in the recovery decision tree.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecoveryBranch {
    /// The interruption class this branch handles.
    pub class: InterruptionClass,
    /// The recommended recovery action.
    pub action: RecoveryAction,
    /// Confidence in this recovery recommendation (0.0–1.0).
    pub confidence: f32,
    /// Depth of this branch in the recovery tree.
    pub depth: u8,
}

// ─── Recovery Plan ────────────────────────────────────────────────────────────

/// A bounded recovery tree for an interrupted workflow.
///
/// At most `MAX_RECOVERY_DEPTH` levels deep.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecoveryPlan {
    /// The classified interruption.
    pub interruption: InterruptionClass,
    /// The primary recommended action.
    pub primary_action: RecoveryAction,
    /// Fallback actions if the primary fails (max MAX_RECOVERY_DEPTH entries).
    pub fallbacks: Vec<RecoveryBranch>,
    /// Human-readable explanation of the plan.
    pub explanation: String,
}

// ─── Pause Checkpoint ─────────────────────────────────────────────────────────

/// Extended checkpoint for a paused workflow.
///
/// Wraps `WorkflowSession` with additional pause-specific metadata.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PauseCheckpoint {
    /// The underlying session.
    pub session: WorkflowSession,
    /// Interruption class that caused the pause.
    pub interruption: InterruptionClass,
    /// Recovery plan at the time of pause.
    pub recovery_plan: RecoveryPlan,
    /// PSDG snapshot (key facts) captured at pause time.
    pub psdg_snapshot: Vec<PsdgFact>,
    /// Workflow category for context rehydration.
    pub workflow_category: String,
    /// Epoch seconds when paused.
    pub paused_at: u64,
}

/// A single PSDG fact captured at pause time.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PsdgFact {
    pub subject: String,
    pub predicate: String,
    pub object: String,
    pub confidence: f64,
}

// ─── Resume Result ────────────────────────────────────────────────────────────

/// Result of attempting to resume a paused workflow.
#[derive(Debug, Clone)]
pub struct ResumeResult {
    /// Whether resumption was successful.
    pub success: bool,
    /// The session to resume (if successful).
    pub session: Option<WorkflowSession>,
    /// The recovery action to execute (if resumption needs a step).
    pub next_action: RecoveryAction,
    /// PSDG context restored from the checkpoint.
    pub restored_context: Vec<PsdgFact>,
    /// Human-readable summary of the resume decision.
    pub summary: String,
}

// ─── WorkflowContinuationRuntime ─────────────────────────────────────────────

/// Manages workflow pause, resume, interruption classification, and recovery.
pub struct WorkflowContinuationRuntime {
    pub(crate) session_mgr: SessionManager,
    psdg: Option<PsdgHandle>,
}

impl WorkflowContinuationRuntime {
    /// Create a new continuation runtime.
    pub fn new(psdg: Option<PsdgHandle>) -> Self {
        Self {
            session_mgr: SessionManager::new(),
            psdg,
        }
    }

    // ── Interruption Classification ────────────────────────────────────────

    /// Classify an interruption from available context.
    ///
    /// Uses PSDG state (focused app, browser URL) to narrow the class.
    pub fn classify_interruption(&self, context: &InterruptionContext) -> InterruptionClass {
        // Auth patterns
        if context
            .window_title
            .as_deref()
            .map(|t| {
                let tl = t.to_lowercase();
                tl.contains("password")
                    || tl.contains("authentication")
                    || tl.contains("sudo")
                    || tl.contains("polkit")
                    || tl.contains("authorize")
            })
            .unwrap_or(false)
        {
            return InterruptionClass::Popup {
                title: context.window_title.clone().unwrap_or_default(),
                is_auth: true,
            };
        }

        // Infrastructure failure: GLOBAL_SAFETY_HALT caused by a dead uinput/vision service.
        // This must be checked before generic popup/focus patterns because the error string
        // looks nothing like a desktop event.
        if let Some(ref reason) = context.checkpoint_failure_reason {
            if reason.contains("GLOBAL_SAFETY_HALT") || reason.contains("service not ready") {
                let service = if reason.contains("uinput") {
                    "uinput_daemon"
                } else if reason.contains("vision") {
                    "vision_sidecar"
                } else {
                    "gui_services"
                };
                return InterruptionClass::InfrastructureFailure {
                    service: service.to_string(),
                    reason: reason.clone(),
                };
            }
        }

        // Window focus checkpoint failure (checkpoint failed after open/switch action)
        if let Some(ref reason) = context.checkpoint_failure_reason {
            let reason_lc = reason.to_lowercase();
            if reason_lc.contains("checkpoint failed") {
                let app = context
                    .current_stage_label
                    .clone()
                    .unwrap_or_else(|| "unknown".into());
                return InterruptionClass::WindowFocusFailed {
                    app,
                    reason: reason.clone(),
                };
            }
        }

        // Focus theft
        if let Some(ref new_app) = context.new_focused_app {
            if context.expected_focused_app.as_deref() != Some(new_app.as_str()) {
                return InterruptionClass::FocusTheft {
                    stolen_by: new_app.clone(),
                };
            }
        }

        // Process crash
        if let Some(ref binary) = context.crashed_process {
            return InterruptionClass::ProcessCrashed {
                binary: binary.clone(),
            };
        }

        // Network drop
        if context.network_dropped {
            return InterruptionClass::NetworkDropped;
        }

        // Browser state change
        if let Some(ref url) = context.browser_url_changed_from {
            return InterruptionClass::BrowserStateChanged { url: url.clone() };
        }

        // Timeout
        if context.stage_timed_out {
            return InterruptionClass::Timeout {
                stage_label: context
                    .current_stage_label
                    .clone()
                    .unwrap_or_else(|| "unknown".into()),
            };
        }

        // Generic popup
        if context.window_title.is_some() && context.expected_focused_app.is_none() {
            return InterruptionClass::Popup {
                title: context.window_title.clone().unwrap_or_default(),
                is_auth: false,
            };
        }

        InterruptionClass::Unknown
    }

    // ── Recovery Planning ──────────────────────────────────────────────────

    /// Plan bounded recovery for a classified interruption.
    ///
    /// Recovery trees are bounded to MAX_RECOVERY_DEPTH levels.
    pub fn plan_recovery(&self, interruption: &InterruptionClass, attempt: u8) -> RecoveryPlan {
        if attempt >= MAX_RECOVERY_DEPTH {
            return RecoveryPlan {
                interruption: interruption.clone(),
                primary_action: RecoveryAction::Escalate {
                    reason: format!("Recovery depth limit ({}) reached", MAX_RECOVERY_DEPTH),
                },
                fallbacks: vec![],
                explanation: format!(
                    "Exceeded maximum recovery attempts ({}). Human intervention required.",
                    MAX_RECOVERY_DEPTH
                ),
            };
        }

        let (primary, fallback_action, explanation) = match interruption {
            InterruptionClass::Popup {
                is_auth: true,
                title,
            } => (
                RecoveryAction::RequestHumanIntervention {
                    question: format!("Authentication required: {}. Please authenticate.", title),
                    context: "Workflow paused pending authentication".into(),
                },
                RecoveryAction::Abort {
                    reason: "Authentication declined".into(),
                },
                format!(
                    "Authentication dialog '{}' must be resolved before workflow can continue.",
                    title
                ),
            ),

            InterruptionClass::Popup {
                is_auth: false,
                title,
            } => (
                RecoveryAction::Continue,
                RecoveryAction::SkipStage {
                    reason: format!("Popup '{}' was unresolvable", title),
                },
                format!(
                    "Non-auth popup '{}' detected. Attempting to continue.",
                    title
                ),
            ),

            InterruptionClass::FocusTheft { stolen_by } => (
                RecoveryAction::Retry { delay_ms: 1000 },
                RecoveryAction::SkipStage {
                    reason: format!("Focus stolen by {}", stolen_by),
                },
                format!("Focus moved to '{}'. Waiting 1s then retrying.", stolen_by),
            ),

            InterruptionClass::AuthRequired { service } => (
                RecoveryAction::RequestHumanIntervention {
                    question: format!("Please authenticate with {} to continue.", service),
                    context: "Workflow needs authentication to proceed".into(),
                },
                RecoveryAction::Abort {
                    reason: format!("Cannot authenticate with {}", service),
                },
                format!("Authentication with '{}' required.", service),
            ),

            InterruptionClass::NetworkDropped => (
                RecoveryAction::Retry { delay_ms: 3000 },
                RecoveryAction::Escalate {
                    reason: "Network unavailable after retry".into(),
                },
                "Network dropped. Waiting 3s then retrying.".into(),
            ),

            InterruptionClass::ProcessCrashed { binary } => (
                RecoveryAction::Rollback {
                    description: format!("Rollback after {} crash", binary),
                },
                RecoveryAction::Escalate {
                    reason: format!("{} crashed and rollback failed", binary),
                },
                format!("Process '{}' crashed. Attempting rollback.", binary),
            ),

            InterruptionClass::BrowserStateChanged { url } => (
                RecoveryAction::Retry { delay_ms: 500 },
                RecoveryAction::SkipStage {
                    reason: format!("Browser navigated away from {}", url),
                },
                format!("Browser left '{}'. Retrying navigation.", url),
            ),

            InterruptionClass::IdeConflict { file } => (
                RecoveryAction::RequestHumanIntervention {
                    question: format!(
                        "IDE has conflicting edit on '{}'. Please close the conflict dialog.",
                        file
                    ),
                    context: "File conflict must be resolved manually".into(),
                },
                RecoveryAction::SkipStage {
                    reason: format!("IDE conflict on {} unresolved", file),
                },
                format!("IDE conflict on '{}'. Human intervention needed.", file),
            ),

            InterruptionClass::Timeout { stage_label } => (
                RecoveryAction::Retry { delay_ms: 2000 },
                RecoveryAction::Escalate {
                    reason: format!("Stage '{}' timed out twice", stage_label),
                },
                format!("Stage '{}' timed out. Retrying once.", stage_label),
            ),

            InterruptionClass::ResourceExhausted { resource } => (
                RecoveryAction::Escalate {
                    reason: format!("Resource exhausted: {}", resource),
                },
                RecoveryAction::Abort {
                    reason: format!("Cannot recover from {} exhaustion", resource),
                },
                format!("Resource '{}' is exhausted. Cannot auto-recover.", resource),
            ),

            InterruptionClass::UserIntervened { description } => (
                RecoveryAction::RequestHumanIntervention {
                    question: format!(
                        "Workflow paused due to: {}. Should I continue?",
                        description
                    ),
                    context: "User intervention detected".into(),
                },
                RecoveryAction::Abort {
                    reason: "User intervened and declined continuation".into(),
                },
                format!("User intervention: {}.", description),
            ),

            InterruptionClass::CompositorEvent { description } => (
                RecoveryAction::Retry { delay_ms: 2000 },
                RecoveryAction::Escalate {
                    reason: format!("Compositor unstable: {}", description),
                },
                format!(
                    "Compositor event: {}. Waiting 2s then retrying.",
                    description
                ),
            ),

            InterruptionClass::WindowFocusFailed { app, .. } => (
                RecoveryAction::Retry { delay_ms: 500 },
                RecoveryAction::SkipStage {
                    reason: format!("Window focus for '{}' could not be verified after retry", app),
                },
                format!(
                    "Window focus verification for '{}' failed — retrying after 500ms to allow OS focus transition.",
                    app
                ),
            ),

            InterruptionClass::InfrastructureFailure { service, .. } => (
                RecoveryAction::Retry { delay_ms: 5000 },
                RecoveryAction::Escalate {
                    reason: format!(
                        "'{}' unavailable after retry — manual restart required",
                        service
                    ),
                },
                format!(
                    "GUI infrastructure service '{}' is not responding. \
                     KRIA will wait 5s and retry (automatic restart may be in progress). \
                     If this persists: restart KRIA or check daemon sudo permissions.",
                    service
                ),
            ),

            InterruptionClass::Unknown => (
                RecoveryAction::Escalate {
                    reason: "Unknown interruption".into(),
                },
                RecoveryAction::Abort {
                    reason: "Could not classify interruption".into(),
                },
                "Unknown interruption. Cannot auto-recover.".into(),
            ),
        };

        let fallbacks = vec![RecoveryBranch {
            class: interruption.clone(),
            action: fallback_action,
            confidence: 0.5,
            depth: 1,
        }];

        RecoveryPlan {
            interruption: interruption.clone(),
            primary_action: primary,
            fallbacks,
            explanation,
        }
    }

    // ── Pause Workflow ─────────────────────────────────────────────────────

    /// Pause a workflow at the current stage boundary.
    ///
    /// Captures a PSDG snapshot, writes an atomic checkpoint, and returns
    /// the pause checkpoint for the caller to surface to the user.
    pub fn pause_workflow(
        &self,
        workflow_id: &str,
        current_session: &WorkflowSession,
        interruption: InterruptionClass,
        workflow_category: &str,
    ) -> PauseCheckpoint {
        let plan = self.plan_recovery(&interruption, 0);
        let psdg_snapshot = self.capture_psdg_snapshot();
        let now = now_epoch();

        let mut session = current_session.clone();
        session.mark_failed(
            interruption.user_message(),
            Some(format!(
                "Resume {} after resolving: {}",
                workflow_category,
                interruption.user_message()
            )),
        );
        if let Err(e) = self.session_mgr.save(&session) {
            warn!(
                target: "workflow_continuation",
                workflow_id = %workflow_id,
                error = %e,
                "Failed to save pause checkpoint (non-fatal)"
            );
        }

        info!(
            target: "workflow_continuation",
            workflow_id = %workflow_id,
            interruption = ?interruption,
            "Workflow paused"
        );

        PauseCheckpoint {
            session,
            interruption,
            recovery_plan: plan,
            psdg_snapshot,
            workflow_category: workflow_category.to_string(),
            paused_at: now,
        }
    }

    // ── Resume Workflow ────────────────────────────────────────────────────

    /// Attempt to resume a paused workflow by session ID.
    ///
    /// 1. Loads the checkpoint from disk.
    /// 2. Rehydrates PSDG context from the snapshot.
    /// 3. Determines the appropriate recovery action.
    pub fn resume_workflow(&self, session_id: &str) -> ResumeResult {
        let session = match self.session_mgr.load(session_id) {
            Some(s) => s,
            None => {
                return ResumeResult {
                    success: false,
                    session: None,
                    next_action: RecoveryAction::Abort {
                        reason: format!("Session '{}' not found", session_id),
                    },
                    restored_context: vec![],
                    summary: format!("Cannot resume: session '{}' not found on disk.", session_id),
                };
            }
        };

        if session.complete {
            return ResumeResult {
                success: false,
                session: Some(session),
                next_action: RecoveryAction::Abort {
                    reason: "Workflow already completed".into(),
                },
                restored_context: vec![],
                summary: "Workflow is already complete — nothing to resume.".into(),
            };
        }

        let steps_completed = session.completed_steps.len();
        let hint = session
            .continuation_hint
            .clone()
            .unwrap_or_else(|| "Continue from last checkpoint".into());

        // Determine next action based on session state.
        let next_action = if session.error.is_some() {
            RecoveryAction::Retry { delay_ms: 0 }
        } else {
            RecoveryAction::Continue
        };

        let summary = format!(
            "Resuming workflow '{}': {} steps completed. Next: {}",
            session_id, steps_completed, hint
        );

        info!(
            target: "workflow_continuation",
            session_id = %session_id,
            steps_completed,
            "Workflow resume initiated"
        );

        ResumeResult {
            success: true,
            session: Some(session),
            next_action,
            restored_context: vec![], // PSDG snapshot rehydration happens separately
            summary,
        }
    }

    /// Find all workflows that can be resumed.
    pub fn find_resumable(&self) -> Vec<WorkflowSession> {
        self.session_mgr.find_continuable()
    }

    /// Record that one exact action blocked on a collaborative decision was
    /// verified complete. This is action-level progress only; it does not mark
    /// the stage or workflow complete.
    pub fn record_decision_action_completed(
        &self,
        workflow_id: &str,
        action_id: &str,
        params: serde_json::Value,
        evidence: String,
    ) -> Result<WorkflowSession, String> {
        let mut session = self.session_mgr.load(workflow_id).unwrap_or_else(|| {
            WorkflowSession::new(
                workflow_id.to_string(),
                "decision-bound workflow continuation".to_string(),
                "CollaborativeDecision".to_string(),
            )
        });

        let canonical_action = format!("decision_action:{action_id}");
        if !session
            .completed_steps
            .iter()
            .any(|step| step.action == canonical_action)
        {
            let step = SessionStep {
                step: session.completed_steps.len() + 1,
                action: canonical_action,
                params,
                success: true,
                evidence,
                timestamp: now_epoch(),
            };
            session.add_step(step);
        }

        session.error = None;
        session.continuation_hint = Some(
            "Prior decision-bound action verified. No automatic workflow autoplay performed."
                .to_string(),
        );
        self.session_mgr.save(&session)?;
        Ok(session)
    }

    // ── Context Rehydration ────────────────────────────────────────────────

    /// Restore PSDG context from a pause checkpoint.
    ///
    /// Re-writes key facts that were captured at pause time back into
    /// WorldModelStore. This allows context injection in the resumed turn
    /// to reconstruct the pre-pause operational state.
    pub fn rehydrate_context(&self, checkpoint: &PauseCheckpoint) {
        let psdg = match &self.psdg {
            Some(h) => h.clone(),
            None => return,
        };

        let facts = checkpoint.psdg_snapshot.clone();
        let store = psdg.store_arc();
        tokio::task::spawn_blocking(move || {
            for fact in &facts {
                let _ = store.upsert(
                    &fact.subject,
                    &fact.predicate,
                    &fact.object,
                    // Reduce confidence slightly (fact is from the past)
                    (fact.confidence * 0.9).max(0.5),
                    FactSource::Inferred,
                    "workflow_resume",
                );
            }
            debug!(
                target: "workflow_continuation",
                facts = facts.len(),
                "PSDG context rehydrated from pause checkpoint"
            );
        });
    }

    // ── PSDG Snapshot ─────────────────────────────────────────────────────

    /// Capture the current PSDG state as a portable snapshot.
    ///
    /// Only captures the high-confidence key facts needed for rehydration.
    fn capture_psdg_snapshot(&self) -> Vec<PsdgFact> {
        let psdg = match &self.psdg {
            Some(h) => h,
            None => return vec![],
        };

        let subjects = [
            "desktop_environment",
            "browser_primary",
            "ide_primary",
            "terminal_primary",
        ];

        let mut snapshot = Vec::new();
        for subject in &subjects {
            let facts = psdg.store().query_subject(subject).unwrap_or_default();
            for fact in facts {
                if fact.confidence >= 0.6 {
                    snapshot.push(PsdgFact {
                        subject: fact.subject,
                        predicate: fact.predicate,
                        object: fact.object,
                        confidence: fact.confidence,
                    });
                }
            }
        }

        snapshot
    }
}

/// Context provided when classifying an interruption.
#[derive(Debug, Default)]
pub struct InterruptionContext {
    /// The title of the unexpected window that appeared.
    pub window_title: Option<String>,
    /// The app that is now focused (if focus theft occurred).
    pub new_focused_app: Option<String>,
    /// The app that was expected to be focused.
    pub expected_focused_app: Option<String>,
    /// Binary name that crashed (if process crash).
    pub crashed_process: Option<String>,
    /// Whether network connectivity dropped.
    pub network_dropped: bool,
    /// URL the browser was at before the change.
    pub browser_url_changed_from: Option<String>,
    /// Whether the current stage timed out.
    pub stage_timed_out: bool,
    /// Label of the current stage.
    pub current_stage_label: Option<String>,
    /// The failure reason from the stage result (e.g. "Checkpoint failed after N recovery attempts").
    pub checkpoint_failure_reason: Option<String>,
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
    use crate::agent::workflow_session::SessionStep;

    fn runtime() -> WorkflowContinuationRuntime {
        WorkflowContinuationRuntime::new(None)
    }

    // ── Interruption classification ────────────────────────────────────────

    #[test]
    fn classify_password_window_is_auth_popup() {
        let rt = runtime();
        let ctx = InterruptionContext {
            window_title: Some("Enter Password".into()),
            ..Default::default()
        };
        let class = rt.classify_interruption(&ctx);
        assert!(matches!(
            class,
            InterruptionClass::Popup { is_auth: true, .. }
        ));
    }

    #[test]
    fn classify_focus_theft_by_new_app() {
        let rt = runtime();
        let ctx = InterruptionContext {
            new_focused_app: Some("slack".into()),
            expected_focused_app: Some("code".into()),
            ..Default::default()
        };
        let class = rt.classify_interruption(&ctx);
        assert!(
            matches!(class, InterruptionClass::FocusTheft { stolen_by } if stolen_by == "slack")
        );
    }

    #[test]
    fn classify_network_drop() {
        let rt = runtime();
        let ctx = InterruptionContext {
            network_dropped: true,
            ..Default::default()
        };
        assert_eq!(
            rt.classify_interruption(&ctx),
            InterruptionClass::NetworkDropped
        );
    }

    #[test]
    fn classify_process_crash() {
        let rt = runtime();
        let ctx = InterruptionContext {
            crashed_process: Some("cargo".into()),
            ..Default::default()
        };
        assert!(matches!(
            rt.classify_interruption(&ctx),
            InterruptionClass::ProcessCrashed { .. }
        ));
    }

    #[test]
    fn classify_timeout() {
        let rt = runtime();
        let ctx = InterruptionContext {
            stage_timed_out: true,
            current_stage_label: Some("build".into()),
            ..Default::default()
        };
        assert!(matches!(
            rt.classify_interruption(&ctx),
            InterruptionClass::Timeout { .. }
        ));
    }

    // ── Recovery planning ──────────────────────────────────────────────────

    #[test]
    fn recovery_auth_popup_requests_human() {
        let rt = runtime();
        let plan = rt.plan_recovery(
            &InterruptionClass::Popup {
                title: "polkit".into(),
                is_auth: true,
            },
            0,
        );
        assert!(matches!(
            plan.primary_action,
            RecoveryAction::RequestHumanIntervention { .. }
        ));
    }

    #[test]
    fn recovery_network_drop_retries() {
        let rt = runtime();
        let plan = rt.plan_recovery(&InterruptionClass::NetworkDropped, 0);
        assert!(matches!(plan.primary_action, RecoveryAction::Retry { .. }));
    }

    #[test]
    fn recovery_max_depth_escalates() {
        let rt = runtime();
        let plan = rt.plan_recovery(&InterruptionClass::NetworkDropped, MAX_RECOVERY_DEPTH);
        assert!(matches!(
            plan.primary_action,
            RecoveryAction::Escalate { .. }
        ));
    }

    #[test]
    fn recovery_tree_bounded() {
        let rt = runtime();
        let plan = rt.plan_recovery(&InterruptionClass::Unknown, 0);
        assert!(plan.fallbacks.len() <= MAX_RECOVERY_DEPTH as usize);
    }

    // ── Pause/resume ───────────────────────────────────────────────────────

    #[test]
    fn pause_workflow_saves_session() {
        let rt = runtime();
        let mut session = WorkflowSession::new(
            "pause-test-001".into(),
            "build project".into(),
            "Coding".into(),
        );
        session.add_step(SessionStep {
            step: 1,
            action: "cargo build".into(),
            params: serde_json::Value::Null,
            success: true,
            evidence: "built".into(),
            timestamp: 0,
        });

        let checkpoint = rt.pause_workflow(
            "pause-test-001",
            &session,
            InterruptionClass::NetworkDropped,
            "Deployment",
        );

        assert!(matches!(
            checkpoint.interruption,
            InterruptionClass::NetworkDropped
        ));
        assert!(!checkpoint.session.complete);
        assert!(checkpoint.session.continuation_hint.is_some());

        // Cleanup
        rt.session_mgr.delete("pause-test-001");
    }

    #[test]
    fn resume_nonexistent_session_returns_failure() {
        let rt = runtime();
        let result = rt.resume_workflow("definitely-does-not-exist-12345");
        assert!(!result.success);
    }

    #[test]
    fn resume_complete_session_returns_failure() {
        let rt = runtime();
        let mut session =
            WorkflowSession::new("resume-complete-001".into(), "x".into(), "y".into());
        session.mark_complete(vec![]);
        rt.session_mgr.save(&session).unwrap();

        let result = rt.resume_workflow("resume-complete-001");
        assert!(!result.success);
        rt.session_mgr.delete("resume-complete-001");
    }

    #[test]
    fn resume_failed_session_returns_success() {
        let rt = runtime();
        let mut session = WorkflowSession::new(
            "resume-failed-001".into(),
            "deploy".into(),
            "Deployment".into(),
        );
        session.mark_failed("timeout".into(), Some("retry step 2".into()));
        rt.session_mgr.save(&session).unwrap();

        let result = rt.resume_workflow("resume-failed-001");
        assert!(result.success);
        assert!(matches!(result.next_action, RecoveryAction::Retry { .. }));
        rt.session_mgr.delete("resume-failed-001");
    }

    // ── Interruption classification properties ─────────────────────────────

    #[test]
    fn auth_interruption_requires_human() {
        assert!(InterruptionClass::AuthRequired {
            service: "ssh".into()
        }
        .requires_human());
    }

    #[test]
    fn focus_theft_is_transient() {
        assert!(InterruptionClass::FocusTheft {
            stolen_by: "slack".into()
        }
        .is_transient());
    }

    #[test]
    fn resource_exhausted_requires_human() {
        assert!(InterruptionClass::ResourceExhausted {
            resource: "disk".into()
        }
        .requires_human());
    }

    // ── InfrastructureFailure classification ───────────────────────────────

    #[test]
    fn classify_global_safety_halt_as_infrastructure_failure() {
        let rt = runtime();
        let ctx = InterruptionContext {
            checkpoint_failure_reason: Some(
                "Action 'type_text' failed: GLOBAL_SAFETY_HALT: service not ready \
                 (vision=ok, uinput=stopped)"
                    .into(),
            ),
            ..Default::default()
        };
        let class = rt.classify_interruption(&ctx);
        assert!(
            matches!(class, InterruptionClass::InfrastructureFailure { ref service, .. }
                if service == "uinput_daemon"),
            "expected InfrastructureFailure(uinput_daemon), got {:?}",
            class
        );
    }

    #[test]
    fn classify_service_not_ready_uinput_failed_as_infrastructure_failure() {
        let rt = runtime();
        let ctx = InterruptionContext {
            checkpoint_failure_reason: Some("service not ready (vision=ok, uinput=FAILED)".into()),
            ..Default::default()
        };
        let class = rt.classify_interruption(&ctx);
        assert!(matches!(
            class,
            InterruptionClass::InfrastructureFailure { .. }
        ));
    }

    #[test]
    fn infrastructure_failure_is_transient() {
        assert!(InterruptionClass::InfrastructureFailure {
            service: "uinput_daemon".into(),
            reason: "GLOBAL_SAFETY_HALT".into(),
        }
        .is_transient());
    }

    #[test]
    fn infrastructure_failure_does_not_require_human() {
        assert!(!InterruptionClass::InfrastructureFailure {
            service: "uinput_daemon".into(),
            reason: "GLOBAL_SAFETY_HALT".into(),
        }
        .requires_human());
    }

    #[test]
    fn infrastructure_failure_recovery_plan_retries() {
        let rt = runtime();
        let class = InterruptionClass::InfrastructureFailure {
            service: "uinput_daemon".into(),
            reason: "GLOBAL_SAFETY_HALT: service not ready (vision=ok, uinput=stopped)".into(),
        };
        let plan = rt.plan_recovery(&class, 0);
        assert!(
            matches!(plan.primary_action, RecoveryAction::Retry { delay_ms } if delay_ms == 5000),
            "expected Retry{{delay_ms: 5000}}, got {:?}",
            plan.primary_action
        );
    }

    #[test]
    fn infrastructure_failure_user_message_names_service() {
        let class = InterruptionClass::InfrastructureFailure {
            service: "uinput_daemon".into(),
            reason: "GLOBAL_SAFETY_HALT".into(),
        };
        let msg = class.user_message();
        assert!(
            msg.contains("uinput_daemon"),
            "user_message should name the service, got: {}",
            msg
        );
    }
}
