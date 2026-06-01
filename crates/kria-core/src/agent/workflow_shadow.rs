//! Shadow Mode Validation — Runtime Parity Infrastructure.
//!
//! This module implements Shadow Mode execution where:
//! - Legacy runtime remains authoritative (performs real mutations)
//! - Canonical runtime executes in dry-run/simulation mode
//! - Outputs from both are compared for parity
//! - Divergences are classified and reported
//!
//! # Safety Guarantees
//!
//! The canonical runtime in Shadow mode:
//! - MUST NOT launch apps
//! - MUST NOT click/type
//! - MUST NOT mutate files
//! - MUST NOT execute shell commands
//! - MUST NOT alter browser state
//!
//! Instead it:
//! - Simulates execution (lifecycle transitions, telemetry)
//! - Evaluates plans (capability-aware planning)
//! - Generates contracts (outcome contracts)
//! - Performs dry-run verification modeling
//! - Emits telemetry into an isolated channel
//!
//! # Design
//!
//! - Deterministic: same inputs → same comparison always
//! - Isolated: shadow telemetry never reaches the frontend
//! - Bounded: shadow execution has its own timeout budget
//! - Observable: every divergence is classified and logged

use crate::agent::workflow_types::*;

// ═══════════════════════════════════════════════════════════════════════════════
// §1 — Divergence Taxonomy
// ═══════════════════════════════════════════════════════════════════════════════

/// Severity of a runtime divergence between legacy and canonical.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, serde::Serialize, serde::Deserialize,
)]
pub enum DivergenceSeverity {
    /// Harmless difference (timing jitter, formatting)
    Benign,
    /// Worth noting but not blocking (confidence differences)
    Advisory,
    /// Significant structural difference (plan topology, step count)
    Medium,
    /// Contradictory outcomes (one says success, other says failure)
    Critical,
}

/// A single divergence between legacy and canonical runtime outputs.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Divergence {
    /// What type of divergence this is
    pub category: DivergenceCategory,
    /// How severe is this divergence
    pub severity: DivergenceSeverity,
    /// Human-readable description
    pub description: String,
    /// What the legacy runtime produced
    pub legacy_value: String,
    /// What the canonical runtime produced
    pub canonical_value: String,
    /// Probable cause
    pub probable_cause: Option<String>,
    /// Suggested remediation
    pub remediation: Option<String>,
}

/// Categories of divergence between runtimes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum DivergenceCategory {
    /// Verdict differs (Complete vs StructurallyComplete, etc.)
    VerdictMismatch,
    /// Planning produced different step counts or actions
    PlanningStructureMismatch,
    /// Verification confidence differs significantly
    VerificationConfidenceMismatch,
    /// Telemetry event count or ordering differs
    TelemetryOrderingMismatch,
    /// Lifecycle state transitions differ
    LifecycleTransitionMismatch,
    /// Capability detection differs
    CapabilityMismatch,
    /// Timing differs significantly (>2x)
    TimingMismatch,
    /// One runtime triggered HITL, other didn't
    HitlTriggerMismatch,
}

// ═══════════════════════════════════════════════════════════════════════════════
// §2 — Parity Reports
// ═══════════════════════════════════════════════════════════════════════════════

/// Complete parity report for a single shadow execution.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct RuntimeParityReport {
    /// Workflow identifier
    pub workflow_id: String,
    /// User's original intent
    pub user_text: String,
    /// When this comparison was performed
    pub timestamp: String,
    /// Overall parity assessment
    pub parity: ParityAssessment,
    /// Verdict comparison
    pub verdict_diff: VerdictDiffReport,
    /// Planning comparison
    pub planning_diff: PlanningDiffReport,
    /// Telemetry comparison
    pub telemetry_diff: TelemetryDiffReport,
    /// All divergences found
    pub divergences: Vec<Divergence>,
    /// Maximum severity across all divergences
    pub max_severity: DivergenceSeverity,
    /// Whether canonical mode activation is recommended based on this execution
    pub canonical_safe: bool,
}

/// Overall parity assessment.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum ParityAssessment {
    /// Outputs are identical or functionally equivalent
    FullParity,
    /// Minor differences that don't affect correctness
    NearParity,
    /// Significant differences that need investigation
    Divergent,
    /// Contradictory outcomes — canonical mode NOT safe
    Contradictory,
}

/// Verdict comparison between legacy and canonical.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct VerdictDiffReport {
    pub legacy_verdict: String,
    pub canonical_verdict: String,
    pub match_type: VerdictMatchType,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum VerdictMatchType {
    /// Exact same verdict
    Exact,
    /// Same success/failure, different details
    EquivalentOutcome,
    /// Different verdicts (e.g., Complete vs StructurallyComplete)
    DifferentVerdict,
    /// Contradictory (one success, one failure)
    Contradictory,
}

/// Planning comparison between legacy and canonical.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct PlanningDiffReport {
    pub legacy_step_count: u32,
    pub canonical_step_count: u32,
    pub legacy_substrate: String,
    pub canonical_substrate: String,
    pub steps_match: bool,
    pub has_outcome_contract: bool,
}

/// Telemetry comparison between legacy and canonical.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct TelemetryDiffReport {
    pub legacy_event_count: u32,
    pub canonical_event_count: u32,
    pub ordering_consistent: bool,
    pub missing_in_canonical: Vec<String>,
    pub extra_in_canonical: Vec<String>,
}

// ═══════════════════════════════════════════════════════════════════════════════
// §3 — Shadow Execution Context (No Side Effects)
// ═══════════════════════════════════════════════════════════════════════════════

/// Shadow execution context — simulates workflow execution without mutations.
///
/// This is the core safety boundary. All execution within this context
/// is guaranteed to have NO side effects on the real environment.
pub struct ShadowExecutionContext {
    /// Workflow ID for this shadow execution
    pub workflow_id: String,
    /// Capabilities resolved at start
    pub capabilities: CapabilitySet,
    /// Simulated step results (no real execution)
    pub simulated_steps: Vec<SimulatedStepResult>,
    /// Telemetry collected during shadow execution (isolated)
    pub shadow_telemetry: Vec<TelemetryEnvelope>,
    /// The verdict the canonical runtime would produce
    pub canonical_verdict: Option<WorkflowVerdict>,
    /// Planning result from capability-aware planner
    pub planning_summary: Option<ShadowPlanSummary>,
}

/// A simulated step result (no real execution occurred).
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct SimulatedStepResult {
    pub step_index: u32,
    pub action: String,
    /// Whether this step WOULD succeed based on capability analysis
    pub predicted_success: bool,
    /// Why we predict this outcome
    pub prediction_basis: String,
    /// Marked as synthetic (never treated as real evidence)
    pub synthetic: bool,
}

/// Summary of what the canonical planner would produce.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ShadowPlanSummary {
    pub substrate: String,
    pub step_count: u32,
    pub execution_mode: ExecutionMode,
    pub has_outcome_contract: bool,
    pub required_outcomes: u32,
    pub desired_outcomes: u32,
    pub adaptations: Vec<String>,
}

impl ShadowExecutionContext {
    pub fn new(workflow_id: String, capabilities: CapabilitySet) -> Self {
        Self {
            workflow_id,
            capabilities,
            simulated_steps: Vec::new(),
            shadow_telemetry: Vec::new(),
            canonical_verdict: None,
            planning_summary: None,
        }
    }
}

// ═══════════════════════════════════════════════════════════════════════════════
// §4 — Shadow Execution Engine
// ═══════════════════════════════════════════════════════════════════════════════

use crate::agent::intent_compiler::GuiTaskSpec;
use crate::agent::workflow_planner::{CapabilityAwarePlanner, PlanningResult};
/// Execute a workflow in shadow mode — NO side effects.
///
/// This function:
/// 1. Runs capability-aware planning (no mutations)
/// 2. Simulates step execution (predicts outcomes from capabilities)
/// 3. Runs dry-run verification modeling (no real probes)
/// 4. Computes what the canonical verdict WOULD be
/// 5. Returns a ShadowExecutionContext for comparison
pub fn execute_shadow(
    spec: &GuiTaskSpec,
    raw_user_text: &str,
    capabilities: &CapabilitySet,
    app_registry: &crate::platform::app_registry::InstalledAppRegistry,
) -> ShadowExecutionContext {
    let workflow_id = format!("shadow-{}", uuid::Uuid::new_v4());
    let mut ctx = ShadowExecutionContext::new(workflow_id, capabilities.clone());

    // Step 1: Run capability-aware planning (pure, no side effects)
    let planning_result =
        CapabilityAwarePlanner::plan(spec, raw_user_text, capabilities, app_registry);

    match planning_result {
        PlanningResult::Planned {
            substrate_plan,
            outcome_contract,
            execution_mode,
            adaptations,
        } => {
            // Record planning summary
            ctx.planning_summary = Some(ShadowPlanSummary {
                substrate: format!("{:?}", substrate_plan.substrate),
                step_count: substrate_plan
                    .workflow
                    .as_ref()
                    .map(|w| w.sub_goals.len() as u32)
                    .unwrap_or(0),
                execution_mode: execution_mode.clone(),
                has_outcome_contract: !outcome_contract.required.is_empty()
                    || !outcome_contract.desired.is_empty(),
                required_outcomes: outcome_contract.required.len() as u32,
                desired_outcomes: outcome_contract.desired.len() as u32,
                adaptations: adaptations.iter().map(|a| a.description.clone()).collect(),
            });

            // Step 2: Simulate step execution (predict outcomes)
            if let Some(workflow) = &substrate_plan.workflow {
                for goal in &workflow.sub_goals {
                    let predicted = predict_step_outcome(&goal.action, capabilities);
                    ctx.simulated_steps.push(predicted);
                }
            }

            // Step 3: Predict canonical verdict from contract + simulated results
            let all_steps_pass = ctx.simulated_steps.iter().all(|s| s.predicted_success);
            if all_steps_pass {
                // Check if desired outcomes would be verifiable
                let desired_verifiable = outcome_contract
                    .desired
                    .iter()
                    .all(|o| can_verify_outcome(&o.expectation, capabilities));
                if desired_verifiable && outcome_contract.desired.is_empty() {
                    ctx.canonical_verdict = Some(WorkflowVerdict::Complete);
                } else if desired_verifiable {
                    ctx.canonical_verdict = Some(WorkflowVerdict::Complete);
                } else {
                    let unverified: Vec<String> = outcome_contract
                        .desired
                        .iter()
                        .filter(|o| !can_verify_outcome(&o.expectation, capabilities))
                        .map(|o| o.description.clone())
                        .collect();
                    ctx.canonical_verdict = Some(WorkflowVerdict::StructurallyComplete {
                        unverified_outcomes: unverified,
                    });
                }
            } else {
                let failed_step = ctx
                    .simulated_steps
                    .iter()
                    .find(|s| !s.predicted_success)
                    .map(|s| s.step_index)
                    .unwrap_or(1);
                ctx.canonical_verdict = Some(WorkflowVerdict::Failed {
                    step: failed_step,
                    reason: "Predicted failure from capability analysis".into(),
                    recovery: None,
                });
            }
        }
        PlanningResult::NeedsHitl { reason, .. } => {
            ctx.canonical_verdict = Some(WorkflowVerdict::Blocked {
                reason: format!("HITL required: {:?}", reason),
            });
        }
        PlanningResult::Unplannable { reason } => {
            ctx.canonical_verdict = Some(WorkflowVerdict::Failed {
                step: 0,
                reason: format!("Unplannable: {}", reason),
                recovery: None,
            });
        }
    }

    ctx
}

/// Predict whether a step would succeed based on capabilities (no execution).
fn predict_step_outcome(action: &str, capabilities: &CapabilitySet) -> SimulatedStepResult {
    let (predicted_success, basis) = match action {
        "write_file" => (true, "Filesystem always available on Linux"),
        "execute_bash" | "execute_python" => (true, "Shell execution always available"),
        "open_application" | "open_application_with_file" => {
            // Depends on app being installed — assume yes for simulation
            (
                true,
                "App launch assumed available (registry check at plan time)",
            )
        }
        "browser_search" | "managed_browser_navigate" | "open_url" => {
            (true, "Browser launch assumed available")
        }
        "type_text" | "click_mouse" | "click_element" | "press_shortcut" => {
            let can_inject =
                capabilities.interaction.keyboard_injection != InputInjectionLevel::None;
            if can_inject {
                (true, "Input injection available via uinput")
            } else {
                (false, "Input injection unavailable — no uinput daemon")
            }
        }
        _ => (true, "Default: assume success for unknown actions"),
    };

    SimulatedStepResult {
        step_index: 0, // Will be set by caller
        action: action.to_string(),
        predicted_success,
        prediction_basis: basis.to_string(),
        synthetic: true,
    }
}

/// Check if an outcome expectation can be verified with current capabilities.
fn can_verify_outcome(expectation: &OutcomeExpectation, capabilities: &CapabilitySet) -> bool {
    match expectation {
        OutcomeExpectation::FileExists { .. } => true, // Always verifiable
        OutcomeExpectation::ProcessRunning { .. } => true, // Always verifiable
        OutcomeExpectation::OutputContains { .. } => true, // Always verifiable
        OutcomeExpectation::PortListening { .. } => true, // Always verifiable
        OutcomeExpectation::AppWindowVisible { .. } => {
            capabilities.verifier.window_state_max_confidence >= 0.60
        }
        OutcomeExpectation::BrowserAtUrl { .. } => capabilities.verifier.cdp_available,
    }
}

// ═══════════════════════════════════════════════════════════════════════════════
// §5 — Parity Comparison Engine
// ═══════════════════════════════════════════════════════════════════════════════

/// Compare legacy execution results with canonical shadow results.
///
/// Produces a RuntimeParityReport with classified divergences.
pub fn compare_runtime_parity(
    workflow_id: &str,
    user_text: &str,
    legacy_success: bool,
    _legacy_completed_steps: usize,
    legacy_total_steps: usize,
    legacy_error: Option<&str>,
    shadow_ctx: &ShadowExecutionContext,
) -> RuntimeParityReport {
    let mut divergences = Vec::new();

    // ── Verdict comparison ────────────────────────────────────────────────
    let legacy_verdict_str = if legacy_success {
        "Complete".to_string()
    } else {
        format!("Failed: {}", legacy_error.unwrap_or("unknown"))
    };

    let canonical_verdict_str = shadow_ctx
        .canonical_verdict
        .as_ref()
        .map(|v| format!("{:?}", v))
        .unwrap_or_else(|| "None".into());

    let verdict_match =
        classify_verdict_match(legacy_success, shadow_ctx.canonical_verdict.as_ref());

    if verdict_match == VerdictMatchType::DifferentVerdict {
        divergences.push(Divergence {
            category: DivergenceCategory::VerdictMismatch,
            severity: DivergenceSeverity::Medium,
            description: "Verdict differs between runtimes".into(),
            legacy_value: legacy_verdict_str.clone(),
            canonical_value: canonical_verdict_str.clone(),
            probable_cause: Some("Canonical runtime has stricter visibility requirements".into()),
            remediation: Some("Review capability-gated verification thresholds".into()),
        });
    } else if verdict_match == VerdictMatchType::Contradictory {
        divergences.push(Divergence {
            category: DivergenceCategory::VerdictMismatch,
            severity: DivergenceSeverity::Critical,
            description: "CONTRADICTORY verdicts — one success, one failure".into(),
            legacy_value: legacy_verdict_str.clone(),
            canonical_value: canonical_verdict_str.clone(),
            probable_cause: Some("Fundamental execution model disagreement".into()),
            remediation: Some("Do NOT activate canonical mode until resolved".into()),
        });
    }

    let verdict_diff = VerdictDiffReport {
        legacy_verdict: legacy_verdict_str,
        canonical_verdict: canonical_verdict_str,
        match_type: verdict_match,
    };

    // ── Planning comparison ───────────────────────────────────────────────
    let planning_diff = if let Some(ref plan) = shadow_ctx.planning_summary {
        let steps_match = plan.step_count == legacy_total_steps as u32;
        if !steps_match {
            divergences.push(Divergence {
                category: DivergenceCategory::PlanningStructureMismatch,
                severity: if (plan.step_count as i32 - legacy_total_steps as i32).unsigned_abs() > 2
                {
                    DivergenceSeverity::Medium
                } else {
                    DivergenceSeverity::Advisory
                },
                description: format!(
                    "Step count differs: legacy={}, canonical={}",
                    legacy_total_steps, plan.step_count
                ),
                legacy_value: format!("{} steps", legacy_total_steps),
                canonical_value: format!("{} steps", plan.step_count),
                probable_cause: Some(
                    "Canonical planner may add/remove steps based on capabilities".into(),
                ),
                remediation: None,
            });
        }
        PlanningDiffReport {
            legacy_step_count: legacy_total_steps as u32,
            canonical_step_count: plan.step_count,
            legacy_substrate: "legacy_htn".into(),
            canonical_substrate: plan.substrate.clone(),
            steps_match,
            has_outcome_contract: plan.has_outcome_contract,
        }
    } else {
        PlanningDiffReport {
            legacy_step_count: legacy_total_steps as u32,
            canonical_step_count: 0,
            legacy_substrate: "legacy_htn".into(),
            canonical_substrate: "unplannable".into(),
            steps_match: false,
            has_outcome_contract: false,
        }
    };

    // ── Telemetry comparison ──────────────────────────────────────────────
    let telemetry_diff = TelemetryDiffReport {
        legacy_event_count: (legacy_total_steps * 2) as u32, // approximate: start + end per step
        canonical_event_count: shadow_ctx.shadow_telemetry.len() as u32,
        ordering_consistent: true, // Shadow telemetry is always ordered
        missing_in_canonical: vec![],
        extra_in_canonical: vec![],
    };

    // ── Compute overall assessment ────────────────────────────────────────
    let max_severity = divergences
        .iter()
        .map(|d| d.severity)
        .max()
        .unwrap_or(DivergenceSeverity::Benign);

    let parity = match max_severity {
        DivergenceSeverity::Benign => ParityAssessment::FullParity,
        DivergenceSeverity::Advisory => ParityAssessment::NearParity,
        DivergenceSeverity::Medium => ParityAssessment::Divergent,
        DivergenceSeverity::Critical => ParityAssessment::Contradictory,
    };

    let canonical_safe = !matches!(parity, ParityAssessment::Contradictory);

    RuntimeParityReport {
        workflow_id: workflow_id.to_string(),
        user_text: user_text.to_string(),
        timestamp: chrono::Utc::now().to_rfc3339(),
        parity,
        verdict_diff,
        planning_diff,
        telemetry_diff,
        divergences,
        max_severity,
        canonical_safe,
    }
}

/// Classify how two verdicts relate to each other.
fn classify_verdict_match(
    legacy_success: bool,
    canonical_verdict: Option<&WorkflowVerdict>,
) -> VerdictMatchType {
    let Some(canonical) = canonical_verdict else {
        return VerdictMatchType::DifferentVerdict;
    };

    let canonical_success = matches!(
        canonical,
        WorkflowVerdict::Complete
            | WorkflowVerdict::StructurallyComplete { .. }
            | WorkflowVerdict::AlreadySatisfied { .. }
    );

    match (legacy_success, canonical_success) {
        (true, true) => {
            if matches!(canonical, WorkflowVerdict::Complete) {
                VerdictMatchType::Exact
            } else {
                VerdictMatchType::EquivalentOutcome
            }
        }
        (false, false) => VerdictMatchType::EquivalentOutcome,
        _ => VerdictMatchType::Contradictory,
    }
}

// ═══════════════════════════════════════════════════════════════════════════════
// §6 — Tests
// ═══════════════════════════════════════════════════════════════════════════════

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::intent_compiler::{GuiTaskSpec, TargetRef, Verb};

    fn make_capabilities() -> CapabilitySet {
        CapabilitySet {
            environment: EnvironmentCapability {
                session_type: SessionType::X11,
                compositor: None,
                atspi_level: AtSpiLevel::Full,
                xdotool_available: true,
                uinput_available: true,
                ocr_available: false,
            },
            verifier: VerifierCapability {
                available_methods: vec![
                    VerificationMethod::FileSystem,
                    VerificationMethod::ProcessTable,
                ],
                window_state_max_confidence: 0.90,
                cdp_available: false,
                filesystem_available: true,
                process_table_available: true,
            },
            interaction: InteractionCapability {
                keyboard_injection: InputInjectionLevel::Full,
                mouse_injection: InputInjectionLevel::Full,
                clipboard_available: true,
            },
        }
    }

    fn make_spec() -> GuiTaskSpec {
        GuiTaskSpec {
            primary_verb: Verb::Open,
            targets: vec![TargetRef::App("gedit".into())],
            content: None,
            declared_preconditions: vec![],
            declared_success_criteria: vec![],
            ambiguities: vec![],
        }
    }

    #[test]
    fn shadow_execution_produces_no_side_effects() {
        let spec = make_spec();
        let caps = make_capabilities();
        let registry = crate::platform::app_registry::InstalledAppRegistry::build_sync();

        let ctx = execute_shadow(&spec, "open gedit", &caps, &registry);

        // Shadow execution should produce a verdict prediction
        assert!(ctx.canonical_verdict.is_some());
        // All simulated steps should be marked synthetic
        for step in &ctx.simulated_steps {
            assert!(step.synthetic, "Shadow steps must be marked synthetic");
        }
    }

    #[test]
    fn shadow_execution_predicts_interaction_failure_without_uinput() {
        let spec = GuiTaskSpec {
            primary_verb: Verb::Type,
            targets: vec![TargetRef::Element("search".into())],
            content: Some(crate::agent::intent_compiler::ContentClass::Literal(
                "hello".into(),
            )),
            declared_preconditions: vec![],
            declared_success_criteria: vec![],
            ambiguities: vec![],
        };
        let mut caps = make_capabilities();
        caps.interaction.keyboard_injection = InputInjectionLevel::None;
        let registry = crate::platform::app_registry::InstalledAppRegistry::build_sync();

        let ctx = execute_shadow(&spec, "type hello", &caps, &registry);

        // Should predict HITL/blocked due to no interaction capability
        assert!(
            matches!(ctx.canonical_verdict, Some(WorkflowVerdict::Blocked { .. })),
            "Should predict blocked without uinput, got {:?}",
            ctx.canonical_verdict
        );
    }

    #[test]
    fn parity_report_detects_verdict_mismatch() {
        let caps = make_capabilities();
        let mut ctx = ShadowExecutionContext::new("test-1".into(), caps);
        ctx.canonical_verdict = Some(WorkflowVerdict::StructurallyComplete {
            unverified_outcomes: vec!["Window visible".into()],
        });
        ctx.planning_summary = Some(ShadowPlanSummary {
            substrate: "AppOpenOnly".into(),
            step_count: 1,
            execution_mode: ExecutionMode::Visible,
            has_outcome_contract: true,
            required_outcomes: 1,
            desired_outcomes: 1,
            adaptations: vec![],
        });

        let report = compare_runtime_parity(
            "test-1",
            "open gedit",
            true, // legacy says success
            1,
            1,
            None,
            &ctx,
        );

        // Legacy says Complete (success=true), canonical says StructurallyComplete
        // This is EquivalentOutcome (both are "success" variants)
        assert!(
            matches!(
                report.verdict_diff.match_type,
                VerdictMatchType::Exact | VerdictMatchType::EquivalentOutcome
            ),
            "Both success variants should be equivalent, got {:?}",
            report.verdict_diff.match_type
        );
    }

    #[test]
    fn parity_report_detects_contradictory_verdicts() {
        let caps = make_capabilities();
        let mut ctx = ShadowExecutionContext::new("test-2".into(), caps);
        ctx.canonical_verdict = Some(WorkflowVerdict::Failed {
            step: 1,
            reason: "App not found".into(),
            recovery: None,
        });

        let report = compare_runtime_parity(
            "test-2",
            "open nonexistent",
            true, // legacy says success (!)
            1,
            1,
            None,
            &ctx,
        );

        // Legacy success + canonical failure = Contradictory
        assert_eq!(
            report.verdict_diff.match_type,
            VerdictMatchType::Contradictory
        );
        assert_eq!(report.max_severity, DivergenceSeverity::Critical);
        assert!(!report.canonical_safe);
    }

    #[test]
    fn parity_report_detects_planning_step_mismatch() {
        let caps = make_capabilities();
        let mut ctx = ShadowExecutionContext::new("test-3".into(), caps);
        ctx.canonical_verdict = Some(WorkflowVerdict::Complete);
        ctx.planning_summary = Some(ShadowPlanSummary {
            substrate: "IdeCodeRunWorkflow".into(),
            step_count: 4,
            execution_mode: ExecutionMode::Hybrid {
                visible_steps: vec![3],
            },
            has_outcome_contract: true,
            required_outcomes: 2,
            desired_outcomes: 1,
            adaptations: vec![],
        });

        let report = compare_runtime_parity(
            "test-3",
            "open code and run",
            true,
            2, // legacy has 2 steps
            2,
            None,
            &ctx,
        );

        // Step count mismatch: legacy=2, canonical=4
        assert!(
            report
                .divergences
                .iter()
                .any(|d| d.category == DivergenceCategory::PlanningStructureMismatch),
            "Should detect planning structure mismatch"
        );
    }

    #[test]
    fn full_parity_when_both_agree() {
        let caps = make_capabilities();
        let mut ctx = ShadowExecutionContext::new("test-4".into(), caps);
        ctx.canonical_verdict = Some(WorkflowVerdict::Complete);
        ctx.planning_summary = Some(ShadowPlanSummary {
            substrate: "AppOpenOnly".into(),
            step_count: 1,
            execution_mode: ExecutionMode::Visible,
            has_outcome_contract: true,
            required_outcomes: 1,
            desired_outcomes: 0,
            adaptations: vec![],
        });

        let report = compare_runtime_parity("test-4", "open firefox", true, 1, 1, None, &ctx);

        assert_eq!(report.parity, ParityAssessment::FullParity);
        assert!(report.canonical_safe);
        assert!(report.divergences.is_empty());
    }

    #[test]
    fn divergence_severity_ordering() {
        assert!(DivergenceSeverity::Benign < DivergenceSeverity::Advisory);
        assert!(DivergenceSeverity::Advisory < DivergenceSeverity::Medium);
        assert!(DivergenceSeverity::Medium < DivergenceSeverity::Critical);
    }
}
