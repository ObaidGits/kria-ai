//! Workflow Telemetry Emitter — Structured Observable Runtime Events.
//!
//! This module provides the canonical telemetry emission layer for workflow
//! execution. It emits typed `TelemetryEnvelope` events that:
//!
//! - Are frontend-agnostic (consumed by Tauri adapter, evals, debugger, etc.)
//! - Have monotonic ordering (sequence numbers + timestamps)
//! - Are deterministic (same workflow → same event sequence)
//! - Support persistence (serializable, bounded trace)
//!
//! # Design
//!
//! The emitter is a lightweight wrapper around a channel sender. It does NOT:
//! - Make I/O calls
//! - Call the LLM
//! - Block on the receiver
//! - Grow unboundedly (bounded channel with backpressure)
//!
//! # Usage
//!
//! ```ignore
//! let (emitter, receiver) = WorkflowTelemetryEmitter::new("wf-123", WorkflowSource::SubstrateRouter);
//! emitter.emit_started("Generate website", &steps, ExecutionMode::Hybrid { visible_steps: vec![2, 4] });
//! // ... execution ...
//! emitter.emit_step_completed(1, true, VisibilityConfidence::NotApplicable, vec![]);
//! // ... final ...
//! emitter.emit_completed(verdict, "All done", vec![], vec![]);
//! ```

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Instant;
use tokio::sync::mpsc;

use crate::agent::workflow_types::{
    ContinuationAction, ExecutionMode, HitlOption, HitlReason, StepExecutionMode, StepPreview,
    StepType, TelemetryEnvelope, VisibilityConfidence, WorkflowSource, WorkflowTelemetry,
    WorkflowVerdict, TELEMETRY_CHANNEL_CAPACITY, TELEMETRY_VERSION,
};

// ═══════════════════════════════════════════════════════════════════════════════
// §1 — Telemetry Emitter
// ═══════════════════════════════════════════════════════════════════════════════

/// Emits structured workflow telemetry events with monotonic ordering.
#[derive(Clone)]
pub struct WorkflowTelemetryEmitter {
    workflow_id: String,
    source: WorkflowSource,
    seq: Arc<AtomicU64>,
    start_time: Instant,
    tx: mpsc::Sender<TelemetryEnvelope>,
}

/// Receives telemetry events (consumed by adapters, persistence, frontend).
pub struct WorkflowTelemetryReceiver {
    pub rx: mpsc::Receiver<TelemetryEnvelope>,
}

impl WorkflowTelemetryEmitter {
    /// Create a new emitter/receiver pair for a workflow.
    pub fn new(
        workflow_id: impl Into<String>,
        source: WorkflowSource,
    ) -> (Self, WorkflowTelemetryReceiver) {
        let (tx, rx) = mpsc::channel(TELEMETRY_CHANNEL_CAPACITY);
        let emitter = Self {
            workflow_id: workflow_id.into(),
            source,
            seq: Arc::new(AtomicU64::new(0)),
            start_time: Instant::now(),
            tx,
        };
        let receiver = WorkflowTelemetryReceiver { rx };
        (emitter, receiver)
    }

    /// Get the workflow ID.
    pub fn workflow_id(&self) -> &str {
        &self.workflow_id
    }

    /// Emit a telemetry event. Non-blocking for non-critical events.
    /// Critical events use blocking_send semantics to guarantee delivery.
    fn emit(&self, event: WorkflowTelemetry, critical: bool) {
        let seq = self.seq.fetch_add(1, Ordering::SeqCst) + 1;
        let envelope = TelemetryEnvelope {
            version: TELEMETRY_VERSION,
            seq,
            event,
            timestamp_ms: self.start_time.elapsed().as_millis() as u64,
            source: self.source,
        };

        if critical {
            // Critical events MUST be delivered. Use blocking_send to wait until space
            // is available. This may block briefly under heavy load but ensures the
            // frontend never misses HitlRequired/Completed/Cancelled events.
            //
            // We use a Tokio runtime-aware approach: if we're inside a runtime, spawn
            // a blocking task that does the send. Otherwise fall back to try_send.
            match self.tx.try_send(envelope.clone()) {
                Ok(_) => {}
                Err(tokio::sync::mpsc::error::TrySendError::Full(envelope)) => {
                    // Channel full — block until space available (critical path)
                    let tx = self.tx.clone();
                    tokio::spawn(async move {
                        if let Err(e) = tx.send(envelope).await {
                            tracing::warn!(
                                target: "workflow_telemetry",
                                error = %e,
                                "Critical event delivery failed (receiver dropped)"
                            );
                        }
                    });
                }
                Err(tokio::sync::mpsc::error::TrySendError::Closed(_)) => {
                    tracing::warn!(
                        target: "workflow_telemetry",
                        "Critical event dropped: channel closed"
                    );
                }
            }
        } else {
            // Non-critical events are dropped if channel is full (backpressure protection)
            let _ = self.tx.try_send(envelope);
        }
    }

    // ─── Lifecycle Events ─────────────────────────────────────────────────

    /// Emit workflow started event.
    pub fn emit_started(
        &self,
        title: &str,
        steps: &[StepPreview],
        execution_mode: ExecutionMode,
        estimated_duration_ms: Option<u64>,
    ) {
        self.emit(
            WorkflowTelemetry::Started {
                workflow_id: self.workflow_id.clone(),
                title: title.to_string(),
                steps: steps.to_vec(),
                execution_mode,
                estimated_duration_ms,
            },
            true,
        );
    }

    /// Emit step started event.
    pub fn emit_step_started(&self, step_index: u32, description: &str, step_type: StepType) {
        self.emit(
            WorkflowTelemetry::StepStarted {
                workflow_id: self.workflow_id.clone(),
                step_index,
                description: description.to_string(),
                step_type,
            },
            false,
        );
    }

    /// Emit step completed event.
    pub fn emit_step_completed(
        &self,
        step_index: u32,
        structural_success: bool,
        visibility_confidence: VisibilityConfidence,
        artifacts: Vec<String>,
    ) {
        self.emit(
            WorkflowTelemetry::StepCompleted {
                workflow_id: self.workflow_id.clone(),
                step_index,
                structural_success,
                visibility_confidence,
                artifacts,
            },
            false,
        );
    }

    /// Emit HITL required event (critical — never dropped).
    pub fn emit_hitl_required(&self, reason: HitlReason, options: Vec<HitlOption>, context: &str) {
        self.emit(
            WorkflowTelemetry::HitlRequired {
                workflow_id: self.workflow_id.clone(),
                reason,
                options,
                context: context.to_string(),
            },
            true,
        );
    }

    /// Emit workflow completed event (critical — never dropped).
    pub fn emit_completed(
        &self,
        verdict: WorkflowVerdict,
        summary: &str,
        artifacts: Vec<String>,
        continuation: Vec<ContinuationAction>,
    ) {
        self.emit(
            WorkflowTelemetry::Completed {
                workflow_id: self.workflow_id.clone(),
                verdict,
                summary: summary.to_string(),
                artifacts,
                continuation,
            },
            true,
        );
    }

    /// Emit workflow cancelled event (critical — never dropped).
    pub fn emit_cancelled(&self, reason: &str, completed_steps: u32, total_steps: u32) {
        self.emit(
            WorkflowTelemetry::Cancelled {
                workflow_id: self.workflow_id.clone(),
                reason: reason.to_string(),
                completed_steps,
                total_steps,
            },
            true,
        );
    }
}

// ═══════════════════════════════════════════════════════════════════════════════
// §2 — Tauri Event Adapter (Transitional Compatibility Layer)
// ═══════════════════════════════════════════════════════════════════════════════

/// Adapts structured telemetry into the existing StreamEvent format.
///
/// This is the transitional compatibility layer that allows the frontend to
/// continue working with existing event types while progressively migrating
/// to structured telemetry rendering.
///
/// The adapter consumes `TelemetryEnvelope` events and produces:
/// - `StreamEvent::TaskStep` for step progress (existing UI)
/// - `StreamEvent::Plan` for workflow start (existing UI)
/// - `StreamEvent::Token` / `StreamEvent::Done` for completion (existing UI)
/// - Raw JSON telemetry event for new frontend components (future)
pub fn adapt_telemetry_to_legacy_events(envelope: &TelemetryEnvelope) -> Vec<LegacyEventAdapter> {
    match &envelope.event {
        WorkflowTelemetry::Started { title, steps, .. } => {
            let mut events = vec![LegacyEventAdapter::Plan(format!(
                "Starting workflow: {} ({} steps)",
                title,
                steps.len()
            ))];
            // Also emit initial task steps
            for step in steps {
                events.push(LegacyEventAdapter::TaskStep {
                    index: step.index,
                    total: steps.len() as u32,
                    description: step.description.clone(),
                    status: "starting".into(),
                });
            }
            events
        }
        WorkflowTelemetry::StepStarted {
            step_index,
            description,
            ..
        } => {
            vec![LegacyEventAdapter::TaskStep {
                index: *step_index,
                total: 0, // Will be filled by caller
                description: description.clone(),
                status: "running".into(),
            }]
        }
        WorkflowTelemetry::StepCompleted {
            step_index,
            structural_success,
            ..
        } => {
            vec![LegacyEventAdapter::TaskStep {
                index: *step_index,
                total: 0,
                description: String::new(),
                status: if *structural_success {
                    "done".into()
                } else {
                    "failed".into()
                },
            }]
        }
        WorkflowTelemetry::HitlRequired {
            reason: _, context, ..
        } => {
            // For now, surface as a text message (legacy compatibility)
            vec![LegacyEventAdapter::Token(format!(
                "⏸ Action needed: {}",
                context
            ))]
        }
        WorkflowTelemetry::Completed {
            verdict, summary, ..
        } => {
            let is_error = matches!(verdict, WorkflowVerdict::Failed { .. });
            if is_error {
                vec![
                    LegacyEventAdapter::Error(summary.clone()),
                    LegacyEventAdapter::Done(summary.clone()),
                ]
            } else {
                vec![
                    LegacyEventAdapter::Token(summary.clone()),
                    LegacyEventAdapter::Done(summary.clone()),
                ]
            }
        }
        WorkflowTelemetry::Cancelled { reason, .. } => {
            vec![
                LegacyEventAdapter::Token(format!("Workflow cancelled: {}", reason)),
                LegacyEventAdapter::Done(format!("Cancelled: {}", reason)),
            ]
        }
        _ => vec![],
    }
}

/// Legacy event types that the existing frontend understands.
/// These map 1:1 to the existing `StreamEvent` variants.
#[derive(Debug, Clone)]
pub enum LegacyEventAdapter {
    Plan(String),
    Token(String),
    Error(String),
    Done(String),
    TaskStep {
        index: u32,
        total: u32,
        description: String,
        status: String,
    },
}

// ═══════════════════════════════════════════════════════════════════════════════
// §3 — Helper: Build StepPreviews from GuiWorkflow
// ═══════════════════════════════════════════════════════════════════════════════

/// Convert a legacy `GuiWorkflow` into `StepPreview` list for telemetry.
pub fn step_previews_from_workflow(
    workflow: &crate::agent::htn_executor::GuiWorkflow,
) -> Vec<StepPreview> {
    workflow
        .sub_goals
        .iter()
        .map(|goal| {
            let step_type = match goal.action.as_str() {
                "write_file" => StepType::FileWrite,
                "open_application" | "open_application_with_file" => StepType::AppLaunch,
                "execute_bash" | "execute_python" => StepType::CommandExecution,
                "browser_search" | "managed_browser_navigate" | "open_url" => {
                    StepType::BrowserNavigation
                }
                "click_element" | "click_mouse" | "type_text" | "press_shortcut" => {
                    StepType::Interaction
                }
                _ => StepType::CommandExecution,
            };
            let execution_mode = match goal.action.as_str() {
                "write_file" | "execute_bash" | "execute_python" => StepExecutionMode::Backend,
                "open_application"
                | "open_application_with_file"
                | "browser_search"
                | "managed_browser_navigate"
                | "open_url" => StepExecutionMode::Visible,
                "click_element" | "click_mouse" | "type_text" | "press_shortcut" => {
                    StepExecutionMode::Interactive
                }
                _ => StepExecutionMode::Backend,
            };
            StepPreview {
                index: goal.step as u32,
                description: gui_action_label_for_telemetry(&goal.action),
                step_type,
                execution_mode,
            }
        })
        .collect()
}

/// Determine execution mode from step previews.
pub fn execution_mode_from_previews(previews: &[StepPreview]) -> ExecutionMode {
    let visible_steps: Vec<u32> = previews
        .iter()
        .filter(|p| {
            matches!(
                p.execution_mode,
                StepExecutionMode::Visible | StepExecutionMode::Interactive
            )
        })
        .map(|p| p.index)
        .collect();

    if visible_steps.is_empty() {
        ExecutionMode::Structural
    } else if visible_steps.len() == previews.len() {
        ExecutionMode::Visible
    } else {
        ExecutionMode::Hybrid { visible_steps }
    }
}

fn gui_action_label_for_telemetry(action: &str) -> String {
    match action {
        "write_file" => "Write generated file".into(),
        "execute_bash" => "Run command".into(),
        "open_application_with_file" => "Open file in application".into(),
        "open_application" => "Open application".into(),
        "browser_search" | "managed_browser_navigate" | "open_url" => "Open browser target".into(),
        "click_element" | "click_mouse" => "Click target".into(),
        "type_text" => "Type text".into(),
        "press_shortcut" => "Press shortcut".into(),
        "focus_window" => "Focus window".into(),
        _ => format!("Execute: {}", action),
    }
}

// ═══════════════════════════════════════════════════════════════════════════════
// §4 — Tests
// ═══════════════════════════════════════════════════════════════════════════════

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn emitter_produces_monotonic_sequence() {
        let (emitter, mut receiver) =
            WorkflowTelemetryEmitter::new("test-1", WorkflowSource::SubstrateRouter);

        emitter.emit_step_started(1, "Write file", StepType::FileWrite);
        emitter.emit_step_started(2, "Open app", StepType::AppLaunch);
        emitter.emit_step_started(3, "Run command", StepType::CommandExecution);

        let e1 = receiver.rx.recv().await.unwrap();
        let e2 = receiver.rx.recv().await.unwrap();
        let e3 = receiver.rx.recv().await.unwrap();

        assert_eq!(e1.seq, 1);
        assert_eq!(e2.seq, 2);
        assert_eq!(e3.seq, 3);
        assert!(e2.timestamp_ms >= e1.timestamp_ms);
        assert!(e3.timestamp_ms >= e2.timestamp_ms);
    }

    #[tokio::test]
    async fn emitter_includes_correct_version_and_source() {
        let (emitter, mut receiver) =
            WorkflowTelemetryEmitter::new("test-2", WorkflowSource::LegacyShim);

        emitter.emit_step_started(1, "Test", StepType::FileWrite);

        let envelope = receiver.rx.recv().await.unwrap();
        assert_eq!(envelope.version, TELEMETRY_VERSION);
        assert_eq!(envelope.source, WorkflowSource::LegacyShim);
    }

    #[tokio::test]
    async fn emitter_started_event_contains_workflow_id() {
        let (emitter, mut receiver) =
            WorkflowTelemetryEmitter::new("wf-abc", WorkflowSource::SubstrateRouter);

        emitter.emit_started(
            "Test Workflow",
            &[StepPreview {
                index: 1,
                description: "Step 1".into(),
                step_type: StepType::FileWrite,
                execution_mode: StepExecutionMode::Backend,
            }],
            ExecutionMode::Structural,
            Some(5000),
        );

        let envelope = receiver.rx.recv().await.unwrap();
        match envelope.event {
            WorkflowTelemetry::Started {
                workflow_id,
                title,
                steps,
                ..
            } => {
                assert_eq!(workflow_id, "wf-abc");
                assert_eq!(title, "Test Workflow");
                assert_eq!(steps.len(), 1);
            }
            _ => panic!("Expected Started event"),
        }
    }

    #[tokio::test]
    async fn emitter_completed_event_carries_verdict() {
        let (emitter, mut receiver) =
            WorkflowTelemetryEmitter::new("wf-done", WorkflowSource::SubstrateRouter);

        emitter.emit_completed(
            WorkflowVerdict::StructurallyComplete {
                unverified_outcomes: vec!["VS Code visible".into()],
            },
            "All steps done structurally",
            vec!["/tmp/test.py".into()],
            vec![],
        );

        let envelope = receiver.rx.recv().await.unwrap();
        match envelope.event {
            WorkflowTelemetry::Completed {
                verdict,
                summary,
                artifacts,
                ..
            } => {
                assert!(matches!(
                    verdict,
                    WorkflowVerdict::StructurallyComplete { .. }
                ));
                assert_eq!(summary, "All steps done structurally");
                assert_eq!(artifacts, vec!["/tmp/test.py"]);
            }
            _ => panic!("Expected Completed event"),
        }
    }

    #[test]
    fn legacy_adapter_converts_started_to_plan_and_steps() {
        let envelope = TelemetryEnvelope {
            version: 1,
            seq: 1,
            event: WorkflowTelemetry::Started {
                workflow_id: "test".into(),
                title: "Generate website".into(),
                steps: vec![
                    StepPreview {
                        index: 1,
                        description: "Write files".into(),
                        step_type: StepType::FileWrite,
                        execution_mode: StepExecutionMode::Backend,
                    },
                    StepPreview {
                        index: 2,
                        description: "Open IDE".into(),
                        step_type: StepType::AppLaunch,
                        execution_mode: StepExecutionMode::Visible,
                    },
                ],
                execution_mode: ExecutionMode::Hybrid {
                    visible_steps: vec![2],
                },
                estimated_duration_ms: Some(10000),
            },
            timestamp_ms: 0,
            source: WorkflowSource::SubstrateRouter,
        };

        let legacy = adapt_telemetry_to_legacy_events(&envelope);
        assert!(legacy.len() >= 1);
        assert!(
            matches!(&legacy[0], LegacyEventAdapter::Plan(s) if s.contains("Generate website"))
        );
    }

    #[test]
    fn legacy_adapter_converts_completed_success_to_token_done() {
        let envelope = TelemetryEnvelope {
            version: 1,
            seq: 5,
            event: WorkflowTelemetry::Completed {
                workflow_id: "test".into(),
                verdict: WorkflowVerdict::Complete,
                summary: "All done".into(),
                artifacts: vec![],
                continuation: vec![],
            },
            timestamp_ms: 5000,
            source: WorkflowSource::SubstrateRouter,
        };

        let legacy = adapt_telemetry_to_legacy_events(&envelope);
        assert_eq!(legacy.len(), 2);
        assert!(matches!(&legacy[0], LegacyEventAdapter::Token(s) if s == "All done"));
        assert!(matches!(&legacy[1], LegacyEventAdapter::Done(s) if s == "All done"));
    }

    #[test]
    fn legacy_adapter_converts_failed_to_error_done() {
        let envelope = TelemetryEnvelope {
            version: 1,
            seq: 3,
            event: WorkflowTelemetry::Completed {
                workflow_id: "test".into(),
                verdict: WorkflowVerdict::Failed {
                    step: 2,
                    reason: "app not found".into(),
                    recovery: None,
                },
                summary: "Failed at step 2".into(),
                artifacts: vec![],
                continuation: vec![],
            },
            timestamp_ms: 3000,
            source: WorkflowSource::SubstrateRouter,
        };

        let legacy = adapt_telemetry_to_legacy_events(&envelope);
        assert_eq!(legacy.len(), 2);
        assert!(matches!(&legacy[0], LegacyEventAdapter::Error(s) if s.contains("Failed")));
    }

    #[test]
    fn execution_mode_detection_works() {
        let all_backend = vec![
            StepPreview {
                index: 1,
                description: "".into(),
                step_type: StepType::FileWrite,
                execution_mode: StepExecutionMode::Backend,
            },
            StepPreview {
                index: 2,
                description: "".into(),
                step_type: StepType::CommandExecution,
                execution_mode: StepExecutionMode::Backend,
            },
        ];
        assert!(matches!(
            execution_mode_from_previews(&all_backend),
            ExecutionMode::Structural
        ));

        let hybrid = vec![
            StepPreview {
                index: 1,
                description: "".into(),
                step_type: StepType::FileWrite,
                execution_mode: StepExecutionMode::Backend,
            },
            StepPreview {
                index: 2,
                description: "".into(),
                step_type: StepType::AppLaunch,
                execution_mode: StepExecutionMode::Visible,
            },
        ];
        assert!(matches!(
            execution_mode_from_previews(&hybrid),
            ExecutionMode::Hybrid { .. }
        ));

        let all_visible = vec![
            StepPreview {
                index: 1,
                description: "".into(),
                step_type: StepType::AppLaunch,
                execution_mode: StepExecutionMode::Visible,
            },
            StepPreview {
                index: 2,
                description: "".into(),
                step_type: StepType::BrowserNavigation,
                execution_mode: StepExecutionMode::Visible,
            },
        ];
        assert!(matches!(
            execution_mode_from_previews(&all_visible),
            ExecutionMode::Visible
        ));
    }
}
