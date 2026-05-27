//! Phase 8 workflow-fidelity evals.
//!
//! These evals exercise the semantic GUI workflow pipeline without running
//! desktop actions. They judge workflow-mode correctness, fidelity tier
//! selection, visible-vs-structural contrast, fallback honesty, verifier
//! authority boundaries, hybrid synchronization invalidation, HITL handling,
//! and weak-model-safe ambiguity behavior.

use kria_core::agent::browser_media_governance::{
    BrowserMediaGovernanceAction, BrowserMediaGovernanceEvaluator,
};
use kria_core::agent::execution_mode_reasoner::{
    EnvironmentCapabilities, ExecutionMode, ExecutionModeDecision, ExecutionModeReasoner,
    PolicyContext, RequiredVerifier, WorkflowContractId,
};
use kria_core::agent::hybrid_synchronization::{
    HybridSynchronizationEvaluator, HybridSynchronizationObservation,
    HybridSynchronizationOverallVerdict,
};
use kria_core::agent::intent_compiler::{Ambiguity, ContentClass, GuiTaskSpec, TargetRef, Verb};
use kria_core::agent::semantic_workflow::{
    analyze_semantic_workflow, SemanticWorkflowAnalysis, WorkflowFidelityTier,
};
use kria_core::agent::verifier_authority::{
    AuthorityConfidenceTier, EvidenceFreshnessStatus, ObservedVerifierEvidence,
    VerifierAuthorityEvaluator, VerifierAuthorityLevel, VerifierAuthorityOverallVerdict,
};
use kria_core::agent::workflow_intent_contract::{
    ContractCheck, ForbiddenDegradation, WorkflowIntentContractRegistry,
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkflowFidelityEvalDimension {
    ModeCorrectness,
    FidelityResolution,
    VisibleVsStructural,
    FallbackHonesty,
    VerifierAuthorityConflict,
    HybridSynchronization,
    HitlPrivateSession,
    AmbiguityGovernance,
    BrowserMediaContract,
    WeakModelProfile,
}

impl WorkflowFidelityEvalDimension {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::ModeCorrectness => "mode_correctness",
            Self::FidelityResolution => "fidelity_resolution",
            Self::VisibleVsStructural => "visible_vs_structural",
            Self::FallbackHonesty => "fallback_honesty",
            Self::VerifierAuthorityConflict => "verifier_authority_conflict",
            Self::HybridSynchronization => "hybrid_synchronization",
            Self::HitlPrivateSession => "hitl_private_session",
            Self::AmbiguityGovernance => "ambiguity_governance",
            Self::BrowserMediaContract => "browser_media_contract",
            Self::WeakModelProfile => "weak_model_profile",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkflowFidelitySpecTemplate {
    VisibleCodingRunShow,
    StructuralCodingOnly,
    VisibleTerminalOutput,
    PublicBrowserPage,
    BrowserAccountUpload,
    PrivateMediaPlaylist,
    VisibleCodingPolicyBlocked,
    BrowserAccountSurfaceConflict,
    HybridCodingStaleVisibleState,
    WeakModelAmbiguousDeictic,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkflowFidelityExpectation {
    pub expected_mode: Option<ExecutionMode>,
    pub expected_contract_id: Option<WorkflowContractId>,
    pub expected_fidelity: Option<WorkflowFidelityTier>,
    pub clarification_required: Option<bool>,
    pub required_verifiers: Vec<RequiredVerifier>,
    pub expected_governance_action: Option<BrowserMediaGovernanceAction>,
    pub expected_forbidden_degradations: Vec<ForbiddenDegradation>,
    pub expected_verifier_verdict: Option<VerifierAuthorityOverallVerdict>,
    pub expected_hybrid_sync_verdict: Option<HybridSynchronizationOverallVerdict>,
    pub partial_completion_required: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkflowFidelityEvalCase {
    pub id: String,
    pub description: String,
    pub prompt: String,
    pub template: WorkflowFidelitySpecTemplate,
    pub dimensions: Vec<WorkflowFidelityEvalDimension>,
    pub capability_ids: Vec<String>,
    pub failure_mode_ids: Vec<String>,
    pub expectation: WorkflowFidelityExpectation,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkflowFidelityObservation {
    pub case_id: String,
    pub mode: ExecutionMode,
    pub contract_id: WorkflowContractId,
    pub requested_fidelity: WorkflowFidelityTier,
    pub clarification_required: bool,
    pub required_verifiers: Vec<RequiredVerifier>,
    pub forbidden_degradations: Vec<ForbiddenDegradation>,
    pub partial_completion_required: bool,
    pub browser_media_action: BrowserMediaGovernanceAction,
    pub browser_media_hitl_pause_required: bool,
    pub verifier_verdict: Option<VerifierAuthorityOverallVerdict>,
    pub hybrid_sync_verdict: Option<HybridSynchronizationOverallVerdict>,
    pub trace_labels: Vec<String>,
    pub evidence: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkflowFidelityVerdict {
    pub case_id: String,
    pub passed: bool,
    pub explanation: String,
    pub failures: Vec<String>,
    pub evidence: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkflowFidelityCaseResult {
    pub case: WorkflowFidelityEvalCase,
    pub observation: WorkflowFidelityObservation,
    pub verdict: WorkflowFidelityVerdict,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkflowFidelityDimensionSummary {
    pub dimension: WorkflowFidelityEvalDimension,
    pub total: usize,
    pub passed: usize,
    pub failed: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkflowFidelitySummary {
    pub total: usize,
    pub passed: usize,
    pub failed: usize,
    pub mode_correctness_cases: usize,
    pub visible_vs_structural_cases: usize,
    pub fallback_honesty_cases: usize,
    pub verifier_authority_conflict_cases: usize,
    pub hybrid_synchronization_cases: usize,
    pub hitl_cases: usize,
    pub weak_model_profile_cases: usize,
    pub dimension_summaries: Vec<WorkflowFidelityDimensionSummary>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkflowFidelityReport {
    pub run_id: String,
    pub generated_at: String,
    pub report_version: String,
    pub summary: WorkflowFidelitySummary,
    pub case_results: Vec<WorkflowFidelityCaseResult>,
}

pub fn workflow_fidelity_suite() -> Vec<WorkflowFidelityEvalCase> {
    vec![
        case(
            "fidelity-001-visible-coding-hybrid",
            "IDE coding prompt with run/show must select hybrid workflow and visible output.",
            "open code and write a program to print pascal triangle and run it and show output",
            WorkflowFidelitySpecTemplate::VisibleCodingRunShow,
            &[
                WorkflowFidelityEvalDimension::ModeCorrectness,
                WorkflowFidelityEvalDimension::FidelityResolution,
                WorkflowFidelityEvalDimension::VisibleVsStructural,
                WorkflowFidelityEvalDimension::HybridSynchronization,
            ],
            expectation(
                Some(ExecutionMode::HybridWorkflow),
                Some(WorkflowContractId::VisibleCodingWorkflow),
                Some(WorkflowFidelityTier::WorkflowStageFidelity),
                Some(false),
                &[
                    RequiredVerifier::IdeFileVisible,
                    RequiredVerifier::OutputSurfaced,
                    RequiredVerifier::WorkflowStageVisible,
                ],
            ),
            &["workflow.semantic_frame", "mode.hybrid", "verifier.output_surfaced"],
            &["backend_only_collapse", "output_file_only"],
        ),
        case(
            "fidelity-002-structural-coding-contrast",
            "Coding prompt without app/visibility anchor may stay structural.",
            "write a python program that prints pascal triangle",
            WorkflowFidelitySpecTemplate::StructuralCodingOnly,
            &[
                WorkflowFidelityEvalDimension::ModeCorrectness,
                WorkflowFidelityEvalDimension::FidelityResolution,
                WorkflowFidelityEvalDimension::VisibleVsStructural,
            ],
            expectation(
                Some(ExecutionMode::StructuralExecution),
                Some(WorkflowContractId::SilentExecutionWorkflow),
                Some(WorkflowFidelityTier::MinimalResultFidelity),
                Some(false),
                &[RequiredVerifier::StructuralResult],
            ),
            &["mode.structural", "fidelity.minimal"],
            &["over_visible_planning"],
        ),
        case(
            "fidelity-003-visible-terminal-output",
            "Terminal prompt with show output requires terminal/output visible evidence.",
            "open terminal and run df -h and show output",
            WorkflowFidelitySpecTemplate::VisibleTerminalOutput,
            &[
                WorkflowFidelityEvalDimension::ModeCorrectness,
                WorkflowFidelityEvalDimension::FidelityResolution,
                WorkflowFidelityEvalDimension::VisibleVsStructural,
            ],
            expectation(
                Some(ExecutionMode::HybridWorkflow),
                Some(WorkflowContractId::GeneralVisibleWorkflow),
                Some(WorkflowFidelityTier::WorkflowStageFidelity),
                Some(false),
                &[
                    RequiredVerifier::TerminalOutputVisible,
                    RequiredVerifier::OutputSurfaced,
                ],
            ),
            &["terminal.output_visible", "verifier.output_surfaced"],
            &["hidden_terminal_output"],
        ),
        case(
            "fidelity-004-public-browser-visible",
            "Public browser page requires visible page verifier, not navigation-only success.",
            "open example.com and show me the page",
            WorkflowFidelitySpecTemplate::PublicBrowserPage,
            &[
                WorkflowFidelityEvalDimension::ModeCorrectness,
                WorkflowFidelityEvalDimension::BrowserMediaContract,
            ],
            expectation(
                Some(ExecutionMode::VisibleAppWorkflow),
                Some(WorkflowContractId::VisibleBrowserWorkflow),
                Some(WorkflowFidelityTier::AppContextFidelity),
                Some(false),
                &[RequiredVerifier::BrowserPageVisible],
            )
            .with_governance_action(BrowserMediaGovernanceAction::RequireVisibleVerifier),
            &["browser.page_visible"],
            &["navigation_only_claimed_complete"],
        ),
        case(
            "fidelity-005-browser-account-hitl",
            "Browser login/upload requires account context and HITL pause.",
            "open browser, sign in to my account, and upload this file",
            WorkflowFidelitySpecTemplate::BrowserAccountUpload,
            &[
                WorkflowFidelityEvalDimension::ModeCorrectness,
                WorkflowFidelityEvalDimension::FidelityResolution,
                WorkflowFidelityEvalDimension::HitlPrivateSession,
                WorkflowFidelityEvalDimension::BrowserMediaContract,
            ],
            expectation(
                Some(ExecutionMode::HumanCollaborativeWorkflow),
                Some(WorkflowContractId::HumanReviewWorkflow),
                Some(WorkflowFidelityTier::HumanObservedFidelity),
                Some(true),
                &[
                    RequiredVerifier::BrowserPageVisible,
                    RequiredVerifier::BrowserAccountContext,
                    RequiredVerifier::HumanReviewPending,
                    RequiredVerifier::UserConfirmation,
                ],
            )
            .with_governance_action(BrowserMediaGovernanceAction::RequireHitlPause),
            &["browser.account_context", "hitl.review_pending"],
            &["account_session_assumed", "upload_without_review"],
        ),
        case(
            "fidelity-006-private-media-hitl",
            "Personal media playlist requires media playback verifier and account/HITL.",
            "open youtube and play my playlist",
            WorkflowFidelitySpecTemplate::PrivateMediaPlaylist,
            &[
                WorkflowFidelityEvalDimension::ModeCorrectness,
                WorkflowFidelityEvalDimension::HitlPrivateSession,
                WorkflowFidelityEvalDimension::BrowserMediaContract,
            ],
            expectation(
                Some(ExecutionMode::HumanCollaborativeWorkflow),
                Some(WorkflowContractId::HumanReviewWorkflow),
                Some(WorkflowFidelityTier::HumanObservedFidelity),
                Some(true),
                &[
                    RequiredVerifier::MediaPlaybackVisible,
                    RequiredVerifier::BrowserAccountContext,
                    RequiredVerifier::HumanReviewPending,
                    RequiredVerifier::UserConfirmation,
                ],
            )
            .with_governance_action(BrowserMediaGovernanceAction::RequireHitlPause),
            &["media.playback_visible", "browser.account_context", "hitl.review_pending"],
            &["personal_playlist_assumed", "media_navigation_as_playback"],
        ),
        case(
            "fidelity-007-fallback-honesty",
            "Policy-blocked visible workflow must be marked degraded/partial, not silent structural success.",
            "open code and write a program to print pascal triangle and run it and show output",
            WorkflowFidelitySpecTemplate::VisibleCodingPolicyBlocked,
            &[
                WorkflowFidelityEvalDimension::FallbackHonesty,
                WorkflowFidelityEvalDimension::VisibleVsStructural,
            ],
            expectation(
                Some(ExecutionMode::StructuralExecution),
                Some(WorkflowContractId::VisibleCodingWorkflow),
                Some(WorkflowFidelityTier::WorkflowStageFidelity),
                Some(false),
                &[RequiredVerifier::OutputSurfaced],
            )
            .with_forbidden_degradation(ForbiddenDegradation::SilentVisibleToStructural)
            .with_partial_completion_required(true),
            &["fallback.honesty", "contract.visible_coding"],
            &["silent_backend_downgrade"],
        ),
        case(
            "fidelity-008-verifier-authority-conflict",
            "Browser account surface evidence must not satisfy user-confirmed account authority.",
            "open browser, sign in to my account, and upload this file",
            WorkflowFidelitySpecTemplate::BrowserAccountSurfaceConflict,
            &[WorkflowFidelityEvalDimension::VerifierAuthorityConflict],
            expectation(
                Some(ExecutionMode::HumanCollaborativeWorkflow),
                Some(WorkflowContractId::HumanReviewWorkflow),
                Some(WorkflowFidelityTier::HumanObservedFidelity),
                Some(true),
                &[RequiredVerifier::BrowserAccountContext],
            )
            .with_verifier_verdict(VerifierAuthorityOverallVerdict::Partial),
            &["verifier.browser_account_context", "authority.user_confirmed"],
            &["focused_browser_claimed_authenticated"],
        ),
        case(
            "fidelity-009-hybrid-stale-visible-state",
            "Hybrid coding stale visible file hash invalidates full completion.",
            "open code and write a program to print pascal triangle and run it and show output",
            WorkflowFidelitySpecTemplate::HybridCodingStaleVisibleState,
            &[WorkflowFidelityEvalDimension::HybridSynchronization],
            expectation(
                Some(ExecutionMode::HybridWorkflow),
                Some(WorkflowContractId::VisibleCodingWorkflow),
                Some(WorkflowFidelityTier::WorkflowStageFidelity),
                Some(false),
                &[RequiredVerifier::IdeFileVisible],
            )
            .with_hybrid_sync_verdict(HybridSynchronizationOverallVerdict::Invalidated),
            &["hybrid.file_hash_sync", "stale_state.invalidation"],
            &["ide_stale_buffer_claimed_fresh"],
        ),
        case(
            "fidelity-010-weak-model-ambiguity",
            "Weak/local model profile must ask on unresolved this/there/show ambiguity.",
            "run this there and show me",
            WorkflowFidelitySpecTemplate::WeakModelAmbiguousDeictic,
            &[
                WorkflowFidelityEvalDimension::AmbiguityGovernance,
                WorkflowFidelityEvalDimension::WeakModelProfile,
            ],
            expectation(
                Some(ExecutionMode::VerificationVisibleWorkflow),
                Some(WorkflowContractId::VerificationVisibleWorkflow),
                Some(WorkflowFidelityTier::WorkflowStageFidelity),
                Some(true),
                &[RequiredVerifier::OutputSurfaced],
            ),
            &["ambiguity.identity", "weak_model.safe_default"],
            &["llm_invented_target", "silent_inference_on_deictic"],
        ),
    ]
}

pub fn run_workflow_fidelity_suite(run_id: impl Into<String>) -> WorkflowFidelityReport {
    let case_results = workflow_fidelity_suite()
        .into_iter()
        .map(run_workflow_fidelity_case)
        .collect::<Vec<_>>();
    let total = case_results.len();
    let passed = case_results
        .iter()
        .filter(|result| result.verdict.passed)
        .count();
    let summary = WorkflowFidelitySummary {
        total,
        passed,
        failed: total.saturating_sub(passed),
        mode_correctness_cases: count_dimension(
            &case_results,
            WorkflowFidelityEvalDimension::ModeCorrectness,
        ),
        visible_vs_structural_cases: count_dimension(
            &case_results,
            WorkflowFidelityEvalDimension::VisibleVsStructural,
        ),
        fallback_honesty_cases: count_dimension(
            &case_results,
            WorkflowFidelityEvalDimension::FallbackHonesty,
        ),
        verifier_authority_conflict_cases: count_dimension(
            &case_results,
            WorkflowFidelityEvalDimension::VerifierAuthorityConflict,
        ),
        hybrid_synchronization_cases: count_dimension(
            &case_results,
            WorkflowFidelityEvalDimension::HybridSynchronization,
        ),
        hitl_cases: count_dimension(
            &case_results,
            WorkflowFidelityEvalDimension::HitlPrivateSession,
        ),
        weak_model_profile_cases: count_dimension(
            &case_results,
            WorkflowFidelityEvalDimension::WeakModelProfile,
        ),
        dimension_summaries: dimension_summaries(&case_results),
    };

    WorkflowFidelityReport {
        run_id: run_id.into(),
        generated_at: unix_now(),
        report_version: "phase8_workflow_fidelity_v1".to_string(),
        summary,
        case_results,
    }
}

pub fn run_workflow_fidelity_case(case: WorkflowFidelityEvalCase) -> WorkflowFidelityCaseResult {
    let spec = spec_for_template(case.template);
    let analysis = analyze_semantic_workflow(&spec, &case.prompt);
    let policy = policy_for_template(case.template);
    let decision = ExecutionModeReasoner.decide(
        &spec,
        &analysis,
        &EnvironmentCapabilities::unchecked_default(),
        &policy,
    );
    let contract_check = WorkflowIntentContractRegistry.evaluate(&decision, &analysis);
    let workflow_attempt_id = format!("phase8:{}", case.id);
    let verifier_authority = VerifierAuthorityEvaluator.assess(
        &contract_check,
        &decision,
        &analysis,
        workflow_attempt_id.clone(),
    );
    let hybrid_assessment = HybridSynchronizationEvaluator.assess(
        &decision,
        &analysis,
        &verifier_authority,
        workflow_attempt_id,
    );
    let browser_media = BrowserMediaGovernanceEvaluator.assess(&analysis, &decision, &case.prompt);

    let verifier_verdict =
        verifier_verdict_for_template(case.template, &verifier_authority.requirements);
    let hybrid_sync_verdict = hybrid_verdict_for_template(case.template, &hybrid_assessment);

    let observation = build_observation(
        &case,
        &analysis,
        &decision,
        &contract_check,
        &verifier_authority,
        &browser_media,
        verifier_verdict,
        hybrid_sync_verdict,
    );
    let verdict = judge_workflow_fidelity_case(&case, &observation);

    WorkflowFidelityCaseResult {
        case,
        observation,
        verdict,
    }
}

pub fn print_workflow_fidelity_report(report: &WorkflowFidelityReport) {
    println!("Workflow Fidelity Eval");
    println!(
        "  total={} passed={} failed={}",
        report.summary.total, report.summary.passed, report.summary.failed
    );
    for summary in &report.summary.dimension_summaries {
        println!(
            "  {}: {}/{} passed",
            summary.dimension.as_str(),
            summary.passed,
            summary.total
        );
    }
    for result in &report.case_results {
        let status = if result.verdict.passed {
            "PASS"
        } else {
            "FAIL"
        };
        println!(
            "  {} {:42} {}",
            status, result.case.id, result.verdict.explanation
        );
    }
}

pub fn write_workflow_fidelity_markdown(
    report: &WorkflowFidelityReport,
    path: impl AsRef<std::path::Path>,
) -> std::io::Result<()> {
    let mut markdown = String::new();
    markdown.push_str("# Workflow Fidelity Eval\n\n");
    markdown.push_str(&format!("**Run ID:** `{}`\n\n", report.run_id));
    markdown.push_str(&format!(
        "**Summary:** {}/{} passed\n\n",
        report.summary.passed, report.summary.total
    ));
    markdown.push_str("| Dimension | Passed | Total |\n");
    markdown.push_str("|---|---:|---:|\n");
    for summary in &report.summary.dimension_summaries {
        markdown.push_str(&format!(
            "| {} | {} | {} |\n",
            summary.dimension.as_str(),
            summary.passed,
            summary.total
        ));
    }
    markdown.push_str("\n| Case | Verdict | Explanation |\n");
    markdown.push_str("|---|---|---|\n");
    for result in &report.case_results {
        markdown.push_str(&format!(
            "| `{}` | {} | {} |\n",
            result.case.id,
            if result.verdict.passed {
                "pass"
            } else {
                "fail"
            },
            result.verdict.explanation.replace('|', "\\|")
        ));
    }
    std::fs::write(path, markdown)
}

fn build_observation(
    case: &WorkflowFidelityEvalCase,
    analysis: &SemanticWorkflowAnalysis,
    decision: &ExecutionModeDecision,
    contract_check: &ContractCheck,
    verifier_authority: &kria_core::agent::verifier_authority::VerifierAuthorityAssessment,
    browser_media: &kria_core::agent::browser_media_governance::BrowserMediaGovernanceAssessment,
    verifier_verdict: Option<VerifierAuthorityOverallVerdict>,
    hybrid_sync_verdict: Option<HybridSynchronizationOverallVerdict>,
) -> WorkflowFidelityObservation {
    let mut trace_labels = Vec::new();
    trace_labels.extend(analysis.frame.trace.signal_labels.clone());
    trace_labels.extend(decision.trace.reason_labels.clone());
    trace_labels.extend(contract_check.trace.trace_labels.clone());
    trace_labels.extend(verifier_authority.trace.trace_labels.clone());
    trace_labels.extend(browser_media.trace.trace_labels.clone());

    let mut evidence = Vec::new();
    evidence.push(format!("mode::{:?}", decision.mode));
    evidence.push(format!("contract::{:?}", decision.workflow_contract_id));
    evidence.push(format!(
        "fidelity::{:?}",
        analysis.fidelity.requested_fidelity
    ));
    evidence.push(format!("verifiers::{}", decision.required_verifiers.len()));
    if !contract_check.forbidden_degradations_triggered.is_empty() {
        evidence.push(format!(
            "forbidden_degradations::{:?}",
            contract_check.forbidden_degradations_triggered
        ));
    }
    if let Some(verdict) = verifier_verdict {
        evidence.push(format!("verifier_verdict::{:?}", verdict));
    }
    if let Some(verdict) = hybrid_sync_verdict {
        evidence.push(format!("hybrid_sync_verdict::{:?}", verdict));
    }

    WorkflowFidelityObservation {
        case_id: case.id.clone(),
        mode: decision.mode,
        contract_id: decision.workflow_contract_id,
        requested_fidelity: analysis.fidelity.requested_fidelity,
        clarification_required: decision.clarification.required,
        required_verifiers: contract_check.verifier_requirements.clone(),
        forbidden_degradations: contract_check.forbidden_degradations_triggered.clone(),
        partial_completion_required: verifier_authority.partial_completion_required,
        browser_media_action: browser_media.action,
        browser_media_hitl_pause_required: browser_media.requires_hitl_pause,
        verifier_verdict,
        hybrid_sync_verdict,
        trace_labels,
        evidence,
    }
}

fn judge_workflow_fidelity_case(
    case: &WorkflowFidelityEvalCase,
    observation: &WorkflowFidelityObservation,
) -> WorkflowFidelityVerdict {
    let mut failures = Vec::new();
    let expectation = &case.expectation;

    if let Some(expected) = expectation.expected_mode {
        if observation.mode != expected {
            failures.push(format!(
                "mode mismatch: expected {:?}, got {:?}",
                expected, observation.mode
            ));
        }
    }
    if let Some(expected) = expectation.expected_contract_id {
        if observation.contract_id != expected {
            failures.push(format!(
                "contract mismatch: expected {:?}, got {:?}",
                expected, observation.contract_id
            ));
        }
    }
    if let Some(expected) = expectation.expected_fidelity {
        if observation.requested_fidelity != expected {
            failures.push(format!(
                "fidelity mismatch: expected {:?}, got {:?}",
                expected, observation.requested_fidelity
            ));
        }
    }
    if let Some(expected) = expectation.clarification_required {
        if observation.clarification_required != expected {
            failures.push(format!(
                "clarification mismatch: expected {}, got {}",
                expected, observation.clarification_required
            ));
        }
    }
    for verifier in &expectation.required_verifiers {
        if !observation.required_verifiers.contains(verifier) {
            failures.push(format!("missing verifier::{:?}", verifier));
        }
    }
    if let Some(expected) = expectation.expected_governance_action {
        if observation.browser_media_action != expected {
            failures.push(format!(
                "governance action mismatch: expected {:?}, got {:?}",
                expected, observation.browser_media_action
            ));
        }
    }
    for degradation in &expectation.expected_forbidden_degradations {
        if !observation.forbidden_degradations.contains(degradation) {
            failures.push(format!("missing forbidden degradation::{:?}", degradation));
        }
    }
    if let Some(expected) = expectation.partial_completion_required {
        if observation.partial_completion_required != expected {
            failures.push(format!(
                "partial completion mismatch: expected {}, got {}",
                expected, observation.partial_completion_required
            ));
        }
    }
    if let Some(expected) = expectation.expected_verifier_verdict {
        if observation.verifier_verdict != Some(expected) {
            failures.push(format!(
                "verifier verdict mismatch: expected {:?}, got {:?}",
                expected, observation.verifier_verdict
            ));
        }
    }
    if let Some(expected) = expectation.expected_hybrid_sync_verdict {
        if observation.hybrid_sync_verdict != Some(expected) {
            failures.push(format!(
                "hybrid sync verdict mismatch: expected {:?}, got {:?}",
                expected, observation.hybrid_sync_verdict
            ));
        }
    }

    WorkflowFidelityVerdict {
        case_id: case.id.clone(),
        passed: failures.is_empty(),
        explanation: if failures.is_empty() {
            "workflow fidelity contract satisfied".to_string()
        } else {
            failures.join("; ")
        },
        failures,
        evidence: observation.evidence.clone(),
    }
}

fn verifier_verdict_for_template(
    template: WorkflowFidelitySpecTemplate,
    requirements: &[kria_core::agent::verifier_authority::VerifierAuthorityRequirement],
) -> Option<VerifierAuthorityOverallVerdict> {
    if template != WorkflowFidelitySpecTemplate::BrowserAccountSurfaceConflict {
        return None;
    }

    let observed = vec![ObservedVerifierEvidence {
        required_verifier: RequiredVerifier::BrowserAccountContext,
        authority_level: VerifierAuthorityLevel::SurfaceAuthority,
        evidence_time_unix_ms: Some(200),
        workflow_attempt_id: Some("phase8-verifier-conflict".to_string()),
        target_identity: Some("browser:account".to_string()),
        freshness_status: EvidenceFreshnessStatus::Fresh,
        confidence_tier: AuthorityConfidenceTier::Strong,
        evidence_summary: "browser surface is visible but account identity is not user-confirmed"
            .to_string(),
    }];
    Some(
        VerifierAuthorityEvaluator
            .evaluate_observed(requirements, &observed)
            .overall,
    )
}

fn hybrid_verdict_for_template(
    template: WorkflowFidelitySpecTemplate,
    assessment: &kria_core::agent::hybrid_synchronization::HybridSynchronizationAssessment,
) -> Option<HybridSynchronizationOverallVerdict> {
    if template != WorkflowFidelitySpecTemplate::HybridCodingStaleVisibleState {
        return None;
    }

    let observations = vec![HybridSynchronizationObservation {
        checkpoint_id: None,
        kind: kria_core::agent::hybrid_synchronization::HybridSynchronizationCheckpointKind::FileHashSync,
        structural_identity: None,
        visible_identity: None,
        structural_hash: Some("structural-hash-current".to_string()),
        visible_hash: Some("visible-hash-stale".to_string()),
        visible_open_hash: Some("visible-hash-stale".to_string()),
        expected_workspace: None,
        observed_workspace: None,
        current_run_marker: None,
        observed_run_marker: None,
        expected_account_identity: None,
        observed_account_identity: None,
        action_started_unix_ms: Some(100),
        visible_observed_unix_ms: Some(200),
        browser_navigation_unix_ms: None,
        external_mutation_unix_ms: None,
        evidence_summary: "IDE-visible buffer hash does not match structural file hash".to_string(),
    }];
    Some(
        HybridSynchronizationEvaluator
            .evaluate_observed(assessment, &observations)
            .overall,
    )
}

fn spec_for_template(template: WorkflowFidelitySpecTemplate) -> GuiTaskSpec {
    match template {
        WorkflowFidelitySpecTemplate::VisibleCodingRunShow
        | WorkflowFidelitySpecTemplate::VisibleCodingPolicyBlocked
        | WorkflowFidelitySpecTemplate::HybridCodingStaleVisibleState => spec(
            Verb::Open,
            vec![TargetRef::App("VS Code".to_string())],
            Some(ContentClass::Generated {
                hint: "pascal triangle program".to_string(),
                language: Some("python".to_string()),
            }),
            Vec::new(),
        ),
        WorkflowFidelitySpecTemplate::StructuralCodingOnly => spec(
            Verb::Run,
            Vec::new(),
            Some(ContentClass::Generated {
                hint: "pascal triangle program".to_string(),
                language: Some("python".to_string()),
            }),
            Vec::new(),
        ),
        WorkflowFidelitySpecTemplate::VisibleTerminalOutput => spec(
            Verb::Open,
            vec![TargetRef::App("Terminal".to_string())],
            None,
            Vec::new(),
        ),
        WorkflowFidelitySpecTemplate::PublicBrowserPage => spec(
            Verb::Open,
            vec![TargetRef::Url("https://example.com".to_string())],
            None,
            Vec::new(),
        ),
        WorkflowFidelitySpecTemplate::BrowserAccountUpload
        | WorkflowFidelitySpecTemplate::BrowserAccountSurfaceConflict => spec(
            Verb::Open,
            vec![TargetRef::App("Firefox".to_string())],
            None,
            Vec::new(),
        ),
        WorkflowFidelitySpecTemplate::PrivateMediaPlaylist => spec(
            Verb::Open,
            vec![TargetRef::App("YouTube".to_string())],
            None,
            Vec::new(),
        ),
        WorkflowFidelitySpecTemplate::WeakModelAmbiguousDeictic => spec(
            Verb::Run,
            Vec::new(),
            None,
            vec![Ambiguity::FileNotSpecified],
        ),
    }
}

fn policy_for_template(template: WorkflowFidelitySpecTemplate) -> PolicyContext {
    if template == WorkflowFidelitySpecTemplate::VisibleCodingPolicyBlocked {
        PolicyContext {
            allow_structural_execution: true,
            allow_visible_workflows: false,
            allow_human_collaboration: true,
        }
    } else {
        PolicyContext::default()
    }
}

fn spec(
    primary_verb: Verb,
    targets: Vec<TargetRef>,
    content: Option<ContentClass>,
    ambiguities: Vec<Ambiguity>,
) -> GuiTaskSpec {
    GuiTaskSpec {
        primary_verb,
        targets,
        content,
        declared_preconditions: Vec::new(),
        declared_success_criteria: Vec::new(),
        ambiguities,
    }
}

fn case(
    id: &str,
    description: &str,
    prompt: &str,
    template: WorkflowFidelitySpecTemplate,
    dimensions: &[WorkflowFidelityEvalDimension],
    expectation: WorkflowFidelityExpectation,
    capability_ids: &[&str],
    failure_mode_ids: &[&str],
) -> WorkflowFidelityEvalCase {
    WorkflowFidelityEvalCase {
        id: id.to_string(),
        description: description.to_string(),
        prompt: prompt.to_string(),
        template,
        dimensions: dimensions.to_vec(),
        capability_ids: strings(capability_ids),
        failure_mode_ids: strings(failure_mode_ids),
        expectation,
    }
}

fn expectation(
    expected_mode: Option<ExecutionMode>,
    expected_contract_id: Option<WorkflowContractId>,
    expected_fidelity: Option<WorkflowFidelityTier>,
    clarification_required: Option<bool>,
    required_verifiers: &[RequiredVerifier],
) -> WorkflowFidelityExpectation {
    WorkflowFidelityExpectation {
        expected_mode,
        expected_contract_id,
        expected_fidelity,
        clarification_required,
        required_verifiers: required_verifiers.to_vec(),
        expected_governance_action: None,
        expected_forbidden_degradations: Vec::new(),
        expected_verifier_verdict: None,
        expected_hybrid_sync_verdict: None,
        partial_completion_required: None,
    }
}

impl WorkflowFidelityExpectation {
    fn with_governance_action(mut self, action: BrowserMediaGovernanceAction) -> Self {
        self.expected_governance_action = Some(action);
        self
    }

    fn with_forbidden_degradation(mut self, degradation: ForbiddenDegradation) -> Self {
        self.expected_forbidden_degradations.push(degradation);
        self
    }

    fn with_partial_completion_required(mut self, required: bool) -> Self {
        self.partial_completion_required = Some(required);
        self
    }

    fn with_verifier_verdict(mut self, verdict: VerifierAuthorityOverallVerdict) -> Self {
        self.expected_verifier_verdict = Some(verdict);
        self
    }

    fn with_hybrid_sync_verdict(mut self, verdict: HybridSynchronizationOverallVerdict) -> Self {
        self.expected_hybrid_sync_verdict = Some(verdict);
        self
    }
}

fn count_dimension(
    results: &[WorkflowFidelityCaseResult],
    dimension: WorkflowFidelityEvalDimension,
) -> usize {
    results
        .iter()
        .filter(|result| result.case.dimensions.contains(&dimension))
        .count()
}

fn dimension_summaries(
    results: &[WorkflowFidelityCaseResult],
) -> Vec<WorkflowFidelityDimensionSummary> {
    let mut counts = BTreeMap::<WorkflowFidelityEvalDimension, (usize, usize)>::new();
    for result in results {
        for dimension in &result.case.dimensions {
            let entry = counts.entry(*dimension).or_insert((0, 0));
            entry.0 += 1;
            if result.verdict.passed {
                entry.1 += 1;
            }
        }
    }
    counts
        .into_iter()
        .map(
            |(dimension, (total, passed))| WorkflowFidelityDimensionSummary {
                dimension,
                total,
                passed,
                failed: total.saturating_sub(passed),
            },
        )
        .collect()
}

fn strings(values: &[&str]) -> Vec<String> {
    values.iter().map(|value| value.to_string()).collect()
}

fn unix_now() -> String {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_secs().to_string())
        .unwrap_or_else(|_| "0".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn suite_covers_required_phase8_dimensions() {
        let suite = workflow_fidelity_suite();
        let all_dimensions = suite
            .iter()
            .flat_map(|case| case.dimensions.iter().copied())
            .collect::<Vec<_>>();

        for dimension in [
            WorkflowFidelityEvalDimension::ModeCorrectness,
            WorkflowFidelityEvalDimension::VisibleVsStructural,
            WorkflowFidelityEvalDimension::FallbackHonesty,
            WorkflowFidelityEvalDimension::VerifierAuthorityConflict,
            WorkflowFidelityEvalDimension::HybridSynchronization,
            WorkflowFidelityEvalDimension::WeakModelProfile,
        ] {
            assert!(
                all_dimensions.contains(&dimension),
                "missing dimension {dimension:?}"
            );
        }
    }

    #[test]
    fn workflow_fidelity_suite_passes_all_cases() {
        let report = run_workflow_fidelity_suite("test-run");

        assert_eq!(report.summary.total, 10);
        assert_eq!(report.summary.failed, 0);
        assert_eq!(report.summary.passed, report.summary.total);
    }

    #[test]
    fn fallback_honesty_case_detects_silent_visible_to_structural_degradation() {
        let case = workflow_fidelity_suite()
            .into_iter()
            .find(|case| case.id == "fidelity-007-fallback-honesty")
            .expect("fallback case exists");
        let result = run_workflow_fidelity_case(case);

        assert!(result.verdict.passed);
        assert!(result
            .observation
            .forbidden_degradations
            .contains(&ForbiddenDegradation::SilentVisibleToStructural));
        assert!(result.observation.partial_completion_required);
    }

    #[test]
    fn private_media_case_requires_hitl_and_account_context() {
        let case = workflow_fidelity_suite()
            .into_iter()
            .find(|case| case.id == "fidelity-006-private-media-hitl")
            .expect("private media case exists");
        let result = run_workflow_fidelity_case(case);

        assert!(result.verdict.passed);
        assert_eq!(
            result.observation.mode,
            ExecutionMode::HumanCollaborativeWorkflow
        );
        assert!(result.observation.browser_media_hitl_pause_required);
        assert!(result
            .observation
            .required_verifiers
            .contains(&RequiredVerifier::BrowserAccountContext));
    }

    #[test]
    fn report_serializes_to_json() {
        let report = run_workflow_fidelity_suite("test-run");
        let json = serde_json::to_string(&report).expect("report serializes");
        let roundtrip: WorkflowFidelityReport =
            serde_json::from_str(&json).expect("report deserializes");

        assert_eq!(roundtrip.summary.total, report.summary.total);
        assert_eq!(roundtrip.summary.failed, 0);
    }

    #[test]
    fn markdown_report_contains_dimension_rows() {
        let report = run_workflow_fidelity_suite("test-run");
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("workflow_fidelity.md");

        write_workflow_fidelity_markdown(&report, &path).expect("markdown write");
        let markdown = std::fs::read_to_string(path).expect("markdown read");

        assert!(markdown.contains("mode_correctness"));
        assert!(markdown.contains("fallback_honesty"));
    }
}
