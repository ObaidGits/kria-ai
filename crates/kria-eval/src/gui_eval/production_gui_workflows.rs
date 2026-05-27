//! Phase 9 production GUI workflow evals.
//!
//! These evals score workflow-mode correctness and production prompt coverage
//! across realistic GUI requests. They are deterministic metadata evals only:
//! no desktop actions, no LLM calls, no hidden planners, and no tool-success
//! scoring as a proxy for workflow fidelity.

use crate::gui_eval::workflow_fidelity::run_workflow_fidelity_suite;
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
    analyze_semantic_workflow, AppClass, SemanticWorkflowAnalysis, TaskFamily,
    VisibilityExpectation, WorkflowFidelityTier,
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
pub enum ProductionGuiEvalDimension {
    ModeCorrectness,
    FidelityResolution,
    AppAnchorFidelity,
    VisibilitySatisfaction,
    HybridSynchronization,
    VerifierAuthority,
    FallbackHonesty,
    ClarificationQuality,
    RecoverySemantics,
    BrowserMediaContract,
    PromptCoverage,
}

impl ProductionGuiEvalDimension {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::ModeCorrectness => "mode_correctness",
            Self::FidelityResolution => "fidelity_resolution",
            Self::AppAnchorFidelity => "app_anchor_fidelity",
            Self::VisibilitySatisfaction => "visibility_satisfaction",
            Self::HybridSynchronization => "hybrid_synchronization",
            Self::VerifierAuthority => "verifier_authority",
            Self::FallbackHonesty => "fallback_honesty",
            Self::ClarificationQuality => "clarification_quality",
            Self::RecoverySemantics => "recovery_semantics",
            Self::BrowserMediaContract => "browser_media_contract",
            Self::PromptCoverage => "prompt_coverage",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProductionPromptShape {
    RealWorld,
    LongPrompt,
    IndirectPrompt,
    MessyPrompt,
    LaymanPrompt,
    DeveloperPrompt,
    IdeWorkflow,
    BrowserWorkflow,
    MediaWorkflow,
    DocumentWorkflow,
    SpreadsheetWorkflow,
    CommunicationWorkflow,
    StructuralContrast,
}

impl ProductionPromptShape {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::RealWorld => "real_world",
            Self::LongPrompt => "long_prompt",
            Self::IndirectPrompt => "indirect_prompt",
            Self::MessyPrompt => "messy_prompt",
            Self::LaymanPrompt => "layman_prompt",
            Self::DeveloperPrompt => "developer_prompt",
            Self::IdeWorkflow => "ide_workflow",
            Self::BrowserWorkflow => "browser_workflow",
            Self::MediaWorkflow => "media_workflow",
            Self::DocumentWorkflow => "document_workflow",
            Self::SpreadsheetWorkflow => "spreadsheet_workflow",
            Self::CommunicationWorkflow => "communication_workflow",
            Self::StructuralContrast => "structural_contrast",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProductionGuiSpecTemplate {
    VisibleCodingRunShow,
    LongVisibleCodingRunShow,
    StructuralCodingContrast,
    BrowserVisibleCheck,
    BrowserAccountUpload,
    PrivateMediaPlaylist,
    CommunicationReviewBeforeSend,
    DocumentEditorVisibleUpdate,
    SpreadsheetLaymanTotals,
    MessyDeicticRunThereShow,
    VisibleCodingPolicyBlocked,
    BrowserAccountSurfaceConflict,
    HybridCodingStaleVisibleState,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProductionGuiExpectation {
    pub expected_mode: Option<ExecutionMode>,
    pub expected_contract_id: Option<WorkflowContractId>,
    pub expected_fidelity: Option<WorkflowFidelityTier>,
    pub expected_task_family: Option<TaskFamily>,
    pub expected_visibility: Option<VisibilityExpectation>,
    pub required_app_classes: Vec<AppClass>,
    pub clarification_required: Option<bool>,
    pub required_verifiers: Vec<RequiredVerifier>,
    pub expected_governance_action: Option<BrowserMediaGovernanceAction>,
    pub expected_forbidden_degradations: Vec<ForbiddenDegradation>,
    pub expected_verifier_verdict: Option<VerifierAuthorityOverallVerdict>,
    pub expected_hybrid_sync_verdict: Option<HybridSynchronizationOverallVerdict>,
    pub partial_completion_required: Option<bool>,
    pub recovery_pause_or_disclosure_required: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProductionGuiEvalCase {
    pub id: String,
    pub description: String,
    pub prompt: String,
    pub template: ProductionGuiSpecTemplate,
    pub dimensions: Vec<ProductionGuiEvalDimension>,
    pub prompt_shapes: Vec<ProductionPromptShape>,
    pub capability_ids: Vec<String>,
    pub workflow_contract_id: WorkflowContractId,
    pub failure_mode_ids: Vec<String>,
    pub expectation: ProductionGuiExpectation,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProductionGuiObservation {
    pub case_id: String,
    pub task_family: TaskFamily,
    pub visibility: VisibilityExpectation,
    pub app_classes: Vec<AppClass>,
    pub mode: ExecutionMode,
    pub contract_id: WorkflowContractId,
    pub requested_fidelity: WorkflowFidelityTier,
    pub clarification_required: bool,
    pub required_verifiers: Vec<RequiredVerifier>,
    pub forbidden_degradations: Vec<ForbiddenDegradation>,
    pub partial_completion_required: bool,
    pub recovery_pause_or_disclosure_required: bool,
    pub browser_media_action: BrowserMediaGovernanceAction,
    pub verifier_verdict: Option<VerifierAuthorityOverallVerdict>,
    pub hybrid_sync_verdict: Option<HybridSynchronizationOverallVerdict>,
    pub trace_labels: Vec<String>,
    pub evidence: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProductionGuiVerdict {
    pub case_id: String,
    pub passed: bool,
    pub explanation: String,
    pub failures: Vec<String>,
    pub evidence: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProductionGuiCaseResult {
    pub case: ProductionGuiEvalCase,
    pub observation: ProductionGuiObservation,
    pub verdict: ProductionGuiVerdict,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProductionGuiPrerequisiteStatus {
    pub phase8_workflow_fidelity_total: usize,
    pub phase8_workflow_fidelity_passed: usize,
    pub phase8_workflow_fidelity_failed: usize,
    pub phase8_workflow_fidelity_passed_all: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProductionGuiDimensionSummary {
    pub dimension: ProductionGuiEvalDimension,
    pub total: usize,
    pub passed: usize,
    pub failed: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProductionPromptShapeSummary {
    pub prompt_shape: ProductionPromptShape,
    pub total: usize,
    pub passed: usize,
    pub failed: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProductionGuiSummary {
    pub total: usize,
    pub passed: usize,
    pub failed: usize,
    pub prerequisites_satisfied: bool,
    pub prompt_shape_summaries: Vec<ProductionPromptShapeSummary>,
    pub dimension_summaries: Vec<ProductionGuiDimensionSummary>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProductionGuiWorkflowReport {
    pub run_id: String,
    pub generated_at: String,
    pub report_version: String,
    pub prerequisite_status: ProductionGuiPrerequisiteStatus,
    pub summary: ProductionGuiSummary,
    pub case_results: Vec<ProductionGuiCaseResult>,
}

pub fn production_gui_workflow_suite() -> Vec<ProductionGuiEvalCase> {
    vec![
        case(
            "prod-gui-001-visible-coding-hybrid",
            "Canonical visible coding workflow must not collapse to backend-only execution.",
            "open code and write a program to print pascal triangle and run it and show output",
            ProductionGuiSpecTemplate::VisibleCodingRunShow,
            &[
                ProductionGuiEvalDimension::ModeCorrectness,
                ProductionGuiEvalDimension::FidelityResolution,
                ProductionGuiEvalDimension::AppAnchorFidelity,
                ProductionGuiEvalDimension::VisibilitySatisfaction,
                ProductionGuiEvalDimension::HybridSynchronization,
            ],
            &[
                ProductionPromptShape::RealWorld,
                ProductionPromptShape::DeveloperPrompt,
                ProductionPromptShape::IdeWorkflow,
            ],
            expectation(
                Some(ExecutionMode::HybridWorkflow),
                Some(WorkflowContractId::VisibleCodingWorkflow),
                Some(WorkflowFidelityTier::WorkflowStageFidelity),
                Some(TaskFamily::Coding),
                Some(VisibilityExpectation::WorkflowVisible),
                &[AppClass::Ide],
                Some(false),
                &[
                    RequiredVerifier::IdeFileVisible,
                    RequiredVerifier::WorkflowStageVisible,
                    RequiredVerifier::OutputSurfaced,
                ],
            ),
            &["ide.open_file_visible", "ide.workspace_context", "terminal.output_visible"],
            &["backend_only_collapse", "output_file_only"],
        ),
        case(
            "prod-gui-002-long-visible-coding",
            "Long developer prompt must preserve IDE and surfaced-output expectations.",
            "quick thing: please open code, create a small python script, run it, and show me the terminal output so i can check it before moving on",
            ProductionGuiSpecTemplate::LongVisibleCodingRunShow,
            &[
                ProductionGuiEvalDimension::ModeCorrectness,
                ProductionGuiEvalDimension::FidelityResolution,
                ProductionGuiEvalDimension::PromptCoverage,
                ProductionGuiEvalDimension::VisibilitySatisfaction,
            ],
            &[
                ProductionPromptShape::LongPrompt,
                ProductionPromptShape::DeveloperPrompt,
                ProductionPromptShape::IdeWorkflow,
            ],
            expectation(
                Some(ExecutionMode::HybridWorkflow),
                Some(WorkflowContractId::VisibleCodingWorkflow),
                Some(WorkflowFidelityTier::WorkflowStageFidelity),
                Some(TaskFamily::Coding),
                Some(VisibilityExpectation::WorkflowVisible),
                &[AppClass::Ide],
                Some(false),
                &[
                    RequiredVerifier::IdeFileVisible,
                    RequiredVerifier::WorkflowStageVisible,
                    RequiredVerifier::OutputSurfaced,
                ],
            ),
            &["ide.open_file_visible", "terminal.output_visible"],
            &["long_prompt_flattening"],
        ),
        case(
            "prod-gui-003-structural-coding-contrast",
            "Pure coding generation without app or visibility wording may remain structural.",
            "write a python program that prints pascal triangle",
            ProductionGuiSpecTemplate::StructuralCodingContrast,
            &[
                ProductionGuiEvalDimension::ModeCorrectness,
                ProductionGuiEvalDimension::FidelityResolution,
                ProductionGuiEvalDimension::PromptCoverage,
            ],
            &[ProductionPromptShape::StructuralContrast],
            expectation(
                Some(ExecutionMode::StructuralExecution),
                Some(WorkflowContractId::SilentExecutionWorkflow),
                Some(WorkflowFidelityTier::MinimalResultFidelity),
                Some(TaskFamily::Coding),
                Some(VisibilityExpectation::None),
                &[],
                Some(false),
                &[RequiredVerifier::StructuralResult],
            ),
            &["structural.result"],
            &["over_visible_planning"],
        ),
        case(
            "prod-gui-004-browser-visible-check",
            "Indirect browser check requires visible browser/page verifier.",
            "open the browser at https://example.com and see if it works",
            ProductionGuiSpecTemplate::BrowserVisibleCheck,
            &[
                ProductionGuiEvalDimension::ModeCorrectness,
                ProductionGuiEvalDimension::AppAnchorFidelity,
                ProductionGuiEvalDimension::VisibilitySatisfaction,
                ProductionGuiEvalDimension::BrowserMediaContract,
            ],
            &[
                ProductionPromptShape::IndirectPrompt,
                ProductionPromptShape::BrowserWorkflow,
            ],
            expectation(
                Some(ExecutionMode::VisibleAppWorkflow),
                Some(WorkflowContractId::VisibleBrowserWorkflow),
                Some(WorkflowFidelityTier::WorkflowStageFidelity),
                Some(TaskFamily::Browser),
                Some(VisibilityExpectation::WorkflowVisible),
                &[AppClass::Browser],
                Some(false),
                &[RequiredVerifier::BrowserPageVisible],
            ),
            &["browser.navigate_visible", "browser.page_visible"],
            &["navigation_as_completion"],
        ),
        case(
            "prod-gui-005-browser-account-hitl",
            "Account/session browser upload must pause for human confirmation.",
            "open browser and upload this file to my account",
            ProductionGuiSpecTemplate::BrowserAccountUpload,
            &[
                ProductionGuiEvalDimension::ModeCorrectness,
                ProductionGuiEvalDimension::ClarificationQuality,
                ProductionGuiEvalDimension::BrowserMediaContract,
                ProductionGuiEvalDimension::RecoverySemantics,
            ],
            &[
                ProductionPromptShape::RealWorld,
                ProductionPromptShape::BrowserWorkflow,
            ],
            expectation(
                Some(ExecutionMode::HumanCollaborativeWorkflow),
                Some(WorkflowContractId::HumanReviewWorkflow),
                Some(WorkflowFidelityTier::HumanObservedFidelity),
                Some(TaskFamily::Browser),
                Some(VisibilityExpectation::HumanObserved),
                &[AppClass::Browser],
                Some(true),
                &[
                    RequiredVerifier::BrowserAccountContext,
                    RequiredVerifier::HumanReviewPending,
                    RequiredVerifier::UserConfirmation,
                ],
            )
            .with_governance_action(BrowserMediaGovernanceAction::RequireHitlPause)
            .with_recovery_pause_or_disclosure(true),
            &["browser.account_context", "browser.form_interaction", "hitl.pause"],
            &["account_session_assumed", "external_side_effect_without_hitl"],
        ),
        case(
            "prod-gui-006-private-media-hitl",
            "Private media playlist request must require visible media and account/HITL handling.",
            "open youtube and play my playlist",
            ProductionGuiSpecTemplate::PrivateMediaPlaylist,
            &[
                ProductionGuiEvalDimension::ModeCorrectness,
                ProductionGuiEvalDimension::BrowserMediaContract,
                ProductionGuiEvalDimension::ClarificationQuality,
            ],
            &[
                ProductionPromptShape::MediaWorkflow,
                ProductionPromptShape::RealWorld,
            ],
            expectation(
                Some(ExecutionMode::HumanCollaborativeWorkflow),
                Some(WorkflowContractId::HumanReviewWorkflow),
                Some(WorkflowFidelityTier::HumanObservedFidelity),
                Some(TaskFamily::Media),
                Some(VisibilityExpectation::HumanObserved),
                &[AppClass::Media],
                Some(true),
                &[
                    RequiredVerifier::MediaPlaybackVisible,
                    RequiredVerifier::BrowserAccountContext,
                    RequiredVerifier::HumanReviewPending,
                ],
            )
            .with_governance_action(BrowserMediaGovernanceAction::RequireHitlPause),
            &["browser.account_context", "media.playback_visible", "hitl.pause"],
            &["private_media_selected_arbitrarily"],
        ),
        case(
            "prod-gui-007-communication-review",
            "Draft/send workflow with approval wording must select human collaborative review.",
            "draft an email in gmail but do not send it until i approve",
            ProductionGuiSpecTemplate::CommunicationReviewBeforeSend,
            &[
                ProductionGuiEvalDimension::ModeCorrectness,
                ProductionGuiEvalDimension::ClarificationQuality,
                ProductionGuiEvalDimension::RecoverySemantics,
            ],
            &[
                ProductionPromptShape::CommunicationWorkflow,
                ProductionPromptShape::RealWorld,
            ],
            expectation(
                Some(ExecutionMode::HumanCollaborativeWorkflow),
                Some(WorkflowContractId::HumanReviewWorkflow),
                Some(WorkflowFidelityTier::HumanObservedFidelity),
                Some(TaskFamily::Communication),
                Some(VisibilityExpectation::HumanObserved),
                &[AppClass::Communication],
                Some(true),
                &[RequiredVerifier::HumanReviewPending, RequiredVerifier::UserConfirmation],
            )
            .with_recovery_pause_or_disclosure(true),
            &["communication.review_surface", "hitl.approval"],
            &["send_without_approval"],
        ),
        case(
            "prod-gui-008-document-visible-update",
            "Document editor workflow must require app-visible document state.",
            "open writer and update the document summary, then show me the result",
            ProductionGuiSpecTemplate::DocumentEditorVisibleUpdate,
            &[
                ProductionGuiEvalDimension::ModeCorrectness,
                ProductionGuiEvalDimension::AppAnchorFidelity,
                ProductionGuiEvalDimension::VisibilitySatisfaction,
                ProductionGuiEvalDimension::PromptCoverage,
            ],
            &[
                ProductionPromptShape::DocumentWorkflow,
                ProductionPromptShape::RealWorld,
            ],
            expectation(
                Some(ExecutionMode::VisibleAppWorkflow),
                Some(WorkflowContractId::GeneralVisibleWorkflow),
                Some(WorkflowFidelityTier::AppContextFidelity),
                Some(TaskFamily::DocumentEditing),
                Some(VisibilityExpectation::AppVisible),
                &[AppClass::DocumentEditor],
                Some(false),
                &[RequiredVerifier::DocumentContentVisible, RequiredVerifier::AppContextVisible],
            ),
            &["document.content_visible", "app.context_visible"],
            &["artifact_only_document_update"],
        ),
        case(
            "prod-gui-009-spreadsheet-layman",
            "Layman spreadsheet wording must preserve spreadsheet app anchor.",
            "open excel and make the totals correct",
            ProductionGuiSpecTemplate::SpreadsheetLaymanTotals,
            &[
                ProductionGuiEvalDimension::ModeCorrectness,
                ProductionGuiEvalDimension::AppAnchorFidelity,
                ProductionGuiEvalDimension::PromptCoverage,
            ],
            &[
                ProductionPromptShape::LaymanPrompt,
                ProductionPromptShape::SpreadsheetWorkflow,
            ],
            expectation(
                Some(ExecutionMode::VisibleAppWorkflow),
                Some(WorkflowContractId::GeneralVisibleWorkflow),
                Some(WorkflowFidelityTier::AppContextFidelity),
                Some(TaskFamily::Spreadsheet),
                Some(VisibilityExpectation::AppVisible),
                &[AppClass::Spreadsheet],
                Some(false),
                &[RequiredVerifier::AppContextVisible],
            ),
            &["spreadsheet.app_visible", "app.context_visible"],
            &["spreadsheet_backend_only"],
        ),
        case(
            "prod-gui-010-messy-deictic",
            "Messy deictic prompt must ask instead of inventing target/workflow identity.",
            "run this there and show me",
            ProductionGuiSpecTemplate::MessyDeicticRunThereShow,
            &[
                ProductionGuiEvalDimension::ClarificationQuality,
                ProductionGuiEvalDimension::VisibilitySatisfaction,
                ProductionGuiEvalDimension::PromptCoverage,
            ],
            &[ProductionPromptShape::MessyPrompt],
            expectation(
                Some(ExecutionMode::VerificationVisibleWorkflow),
                Some(WorkflowContractId::VerificationVisibleWorkflow),
                Some(WorkflowFidelityTier::WorkflowStageFidelity),
                Some(TaskFamily::General),
                Some(VisibilityExpectation::ResultVisible),
                &[],
                Some(true),
                &[RequiredVerifier::WorkflowStageVisible, RequiredVerifier::OutputSurfaced],
            ),
            &["clarification.identity", "verifier.output_surfaced"],
            &["deictic_target_invented"],
        ),
        case(
            "prod-gui-011-fallback-honesty",
            "Visible coding policy block must surface degraded fidelity as partial work.",
            "open code and write a program, run it and show output",
            ProductionGuiSpecTemplate::VisibleCodingPolicyBlocked,
            &[
                ProductionGuiEvalDimension::FallbackHonesty,
                ProductionGuiEvalDimension::RecoverySemantics,
                ProductionGuiEvalDimension::VisibilitySatisfaction,
            ],
            &[
                ProductionPromptShape::DeveloperPrompt,
                ProductionPromptShape::IdeWorkflow,
            ],
            expectation(
                Some(ExecutionMode::StructuralExecution),
                Some(WorkflowContractId::VisibleCodingWorkflow),
                Some(WorkflowFidelityTier::WorkflowStageFidelity),
                Some(TaskFamily::Coding),
                Some(VisibilityExpectation::WorkflowVisible),
                &[AppClass::Ide],
                Some(false),
                &[RequiredVerifier::OutputSurfaced, RequiredVerifier::IdeFileVisible],
            )
            .with_forbidden_degradations(&[ForbiddenDegradation::SilentVisibleToStructural])
            .with_partial_completion_required(true)
            .with_recovery_pause_or_disclosure(true),
            &["fallback.disclosure", "partial_completion"],
            &["silent_visible_to_structural"],
        ),
        case(
            "prod-gui-012-verifier-authority-conflict",
            "Surface-only account evidence must not satisfy account/session semantic truth.",
            "open browser and upload this file to my account",
            ProductionGuiSpecTemplate::BrowserAccountSurfaceConflict,
            &[
                ProductionGuiEvalDimension::VerifierAuthority,
                ProductionGuiEvalDimension::BrowserMediaContract,
            ],
            &[ProductionPromptShape::BrowserWorkflow],
            expectation(
                Some(ExecutionMode::HumanCollaborativeWorkflow),
                Some(WorkflowContractId::HumanReviewWorkflow),
                Some(WorkflowFidelityTier::HumanObservedFidelity),
                Some(TaskFamily::Browser),
                Some(VisibilityExpectation::HumanObserved),
                &[AppClass::Browser],
                Some(true),
                &[RequiredVerifier::BrowserAccountContext],
            )
            .with_verifier_verdict(VerifierAuthorityOverallVerdict::Partial),
            &["verifier.authority", "browser.account_context"],
            &["weak_surface_claimed_as_semantic_truth"],
        ),
        case(
            "prod-gui-013-hybrid-stale-state",
            "Hybrid IDE workflow must invalidate stale visible file state.",
            "open code and write a program, run it and show output",
            ProductionGuiSpecTemplate::HybridCodingStaleVisibleState,
            &[
                ProductionGuiEvalDimension::HybridSynchronization,
                ProductionGuiEvalDimension::VerifierAuthority,
            ],
            &[ProductionPromptShape::IdeWorkflow],
            expectation(
                Some(ExecutionMode::HybridWorkflow),
                Some(WorkflowContractId::VisibleCodingWorkflow),
                Some(WorkflowFidelityTier::WorkflowStageFidelity),
                Some(TaskFamily::Coding),
                Some(VisibilityExpectation::WorkflowVisible),
                &[AppClass::Ide],
                Some(false),
                &[RequiredVerifier::IdeFileVisible, RequiredVerifier::OutputSurfaced],
            )
            .with_hybrid_sync_verdict(HybridSynchronizationOverallVerdict::Invalidated),
            &["hybrid.file_hash_sync", "ide.open_file_visible"],
            &["stale_visible_state"],
        ),
    ]
}

pub fn run_production_gui_workflow_suite(run_id: impl Into<String>) -> ProductionGuiWorkflowReport {
    let run_id = run_id.into();
    let prerequisite_status = phase8_prerequisite_status(&run_id);
    let case_results = production_gui_workflow_suite()
        .into_iter()
        .map(run_production_gui_case)
        .collect::<Vec<_>>();
    let total = case_results.len();
    let passed = case_results
        .iter()
        .filter(|result| result.verdict.passed)
        .count();
    let summary = ProductionGuiSummary {
        total,
        passed,
        failed: total.saturating_sub(passed),
        prerequisites_satisfied: prerequisite_status.phase8_workflow_fidelity_passed_all,
        prompt_shape_summaries: prompt_shape_summaries(&case_results),
        dimension_summaries: dimension_summaries(&case_results),
    };

    ProductionGuiWorkflowReport {
        run_id,
        generated_at: unix_now(),
        report_version: "phase9_production_gui_workflows_v1".to_string(),
        prerequisite_status,
        summary,
        case_results,
    }
}

pub fn run_production_gui_case(case: ProductionGuiEvalCase) -> ProductionGuiCaseResult {
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
    let workflow_attempt_id = format!("phase9:{}", case.id);
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
    let verdict = judge_production_gui_case(&case, &observation);

    ProductionGuiCaseResult {
        case,
        observation,
        verdict,
    }
}

pub fn print_production_gui_workflow_report(report: &ProductionGuiWorkflowReport) {
    println!("Production GUI Workflow Eval");
    println!(
        "  prerequisites.phase8_workflow_fidelity={}/{} passed",
        report.prerequisite_status.phase8_workflow_fidelity_passed,
        report.prerequisite_status.phase8_workflow_fidelity_total
    );
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

pub fn write_production_gui_workflow_markdown(
    report: &ProductionGuiWorkflowReport,
    path: impl AsRef<std::path::Path>,
) -> std::io::Result<()> {
    let mut markdown = String::new();
    markdown.push_str("# Production GUI Workflow Eval\n\n");
    markdown.push_str(&format!("**Run ID:** `{}`\n\n", report.run_id));
    markdown.push_str(&format!(
        "**Phase 8 prerequisite:** {}/{} workflow-fidelity cases passed\n\n",
        report.prerequisite_status.phase8_workflow_fidelity_passed,
        report.prerequisite_status.phase8_workflow_fidelity_total
    ));
    markdown.push_str(&format!(
        "**Summary:** {}/{} passed\n\n",
        report.summary.passed, report.summary.total
    ));

    markdown.push_str("## Dimensions\n\n");
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

    markdown.push_str("\n## Prompt Shapes\n\n");
    markdown.push_str("| Prompt shape | Passed | Total |\n");
    markdown.push_str("|---|---:|---:|\n");
    for summary in &report.summary.prompt_shape_summaries {
        markdown.push_str(&format!(
            "| {} | {} | {} |\n",
            summary.prompt_shape.as_str(),
            summary.passed,
            summary.total
        ));
    }

    markdown.push_str("\n## Cases\n\n");
    markdown.push_str("| Case | Contract | Verdict | Explanation |\n");
    markdown.push_str("|---|---|---|---|\n");
    for result in &report.case_results {
        markdown.push_str(&format!(
            "| `{}` | `{:?}` | {} | {} |\n",
            result.case.id,
            result.case.workflow_contract_id,
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

fn phase8_prerequisite_status(run_id: &str) -> ProductionGuiPrerequisiteStatus {
    let phase8 = run_workflow_fidelity_suite(format!("{run_id}:phase8-prerequisite"));
    ProductionGuiPrerequisiteStatus {
        phase8_workflow_fidelity_total: phase8.summary.total,
        phase8_workflow_fidelity_passed: phase8.summary.passed,
        phase8_workflow_fidelity_failed: phase8.summary.failed,
        phase8_workflow_fidelity_passed_all: phase8.summary.failed == 0 && phase8.summary.total > 0,
    }
}

fn build_observation(
    case: &ProductionGuiEvalCase,
    analysis: &SemanticWorkflowAnalysis,
    decision: &ExecutionModeDecision,
    contract_check: &ContractCheck,
    verifier_authority: &kria_core::agent::verifier_authority::VerifierAuthorityAssessment,
    browser_media: &kria_core::agent::browser_media_governance::BrowserMediaGovernanceAssessment,
    verifier_verdict: Option<VerifierAuthorityOverallVerdict>,
    hybrid_sync_verdict: Option<HybridSynchronizationOverallVerdict>,
) -> ProductionGuiObservation {
    let mut trace_labels = Vec::new();
    trace_labels.extend(analysis.frame.trace.signal_labels.clone());
    trace_labels.extend(decision.trace.reason_labels.clone());
    trace_labels.extend(contract_check.trace.trace_labels.clone());
    trace_labels.extend(verifier_authority.trace.trace_labels.clone());
    trace_labels.extend(browser_media.trace.trace_labels.clone());

    let app_classes = analysis
        .frame
        .app_anchors
        .iter()
        .map(|anchor| anchor.app_class)
        .collect::<Vec<_>>();
    let recovery_pause_or_disclosure_required = verifier_authority.partial_completion_required
        || !contract_check.forbidden_degradations_triggered.is_empty()
        || decision.clarification.required;

    let mut evidence = Vec::new();
    evidence.push(format!("task_family::{:?}", analysis.frame.task_family));
    evidence.push(format!(
        "visibility::{:?}",
        analysis.frame.visibility_expectation
    ));
    evidence.push(format!("mode::{:?}", decision.mode));
    evidence.push(format!("contract::{:?}", decision.workflow_contract_id));
    evidence.push(format!(
        "fidelity::{:?}",
        analysis.fidelity.requested_fidelity
    ));
    evidence.push(format!(
        "verifier_requirements::{}",
        contract_check.verifier_requirements.len()
    ));
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

    ProductionGuiObservation {
        case_id: case.id.clone(),
        task_family: analysis.frame.task_family,
        visibility: analysis.frame.visibility_expectation,
        app_classes,
        mode: decision.mode,
        contract_id: decision.workflow_contract_id,
        requested_fidelity: analysis.fidelity.requested_fidelity,
        clarification_required: decision.clarification.required,
        required_verifiers: contract_check.verifier_requirements.clone(),
        forbidden_degradations: contract_check.forbidden_degradations_triggered.clone(),
        partial_completion_required: verifier_authority.partial_completion_required,
        recovery_pause_or_disclosure_required,
        browser_media_action: browser_media.action,
        verifier_verdict,
        hybrid_sync_verdict,
        trace_labels,
        evidence,
    }
}

fn judge_production_gui_case(
    case: &ProductionGuiEvalCase,
    observation: &ProductionGuiObservation,
) -> ProductionGuiVerdict {
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
    if let Some(expected) = expectation.expected_task_family {
        if observation.task_family != expected {
            failures.push(format!(
                "task family mismatch: expected {:?}, got {:?}",
                expected, observation.task_family
            ));
        }
    }
    if let Some(expected) = expectation.expected_visibility {
        if observation.visibility != expected {
            failures.push(format!(
                "visibility mismatch: expected {:?}, got {:?}",
                expected, observation.visibility
            ));
        }
    }
    for app_class in &expectation.required_app_classes {
        if !observation.app_classes.contains(app_class) {
            failures.push(format!("missing app class::{:?}", app_class));
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
    if let Some(expected) = expectation.recovery_pause_or_disclosure_required {
        if observation.recovery_pause_or_disclosure_required != expected {
            failures.push(format!(
                "recovery/disclosure mismatch: expected {}, got {}",
                expected, observation.recovery_pause_or_disclosure_required
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

    ProductionGuiVerdict {
        case_id: case.id.clone(),
        passed: failures.is_empty(),
        explanation: if failures.is_empty() {
            "production workflow expectation satisfied".to_string()
        } else {
            failures.join("; ")
        },
        failures,
        evidence: observation.evidence.clone(),
    }
}

fn verifier_verdict_for_template(
    template: ProductionGuiSpecTemplate,
    requirements: &[kria_core::agent::verifier_authority::VerifierAuthorityRequirement],
) -> Option<VerifierAuthorityOverallVerdict> {
    if template != ProductionGuiSpecTemplate::BrowserAccountSurfaceConflict {
        return None;
    }

    let observed = vec![ObservedVerifierEvidence {
        required_verifier: RequiredVerifier::BrowserAccountContext,
        authority_level: VerifierAuthorityLevel::SurfaceAuthority,
        evidence_time_unix_ms: Some(200),
        workflow_attempt_id: Some("phase9-verifier-conflict".to_string()),
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
    template: ProductionGuiSpecTemplate,
    assessment: &kria_core::agent::hybrid_synchronization::HybridSynchronizationAssessment,
) -> Option<HybridSynchronizationOverallVerdict> {
    if template != ProductionGuiSpecTemplate::HybridCodingStaleVisibleState {
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
        visible_observed_unix_ms: Some(300),
        browser_navigation_unix_ms: None,
        external_mutation_unix_ms: None,
        evidence_summary: "IDE visible file hash does not match latest structural write"
            .to_string(),
    }];

    Some(
        HybridSynchronizationEvaluator
            .evaluate_observed(assessment, &observations)
            .overall,
    )
}

fn spec_for_template(template: ProductionGuiSpecTemplate) -> GuiTaskSpec {
    match template {
        ProductionGuiSpecTemplate::VisibleCodingRunShow
        | ProductionGuiSpecTemplate::LongVisibleCodingRunShow
        | ProductionGuiSpecTemplate::VisibleCodingPolicyBlocked
        | ProductionGuiSpecTemplate::HybridCodingStaleVisibleState => spec(
            Verb::Open,
            vec![TargetRef::App("VS Code".to_string())],
            Some(ContentClass::Generated {
                hint: "python script".to_string(),
                language: Some("python".to_string()),
            }),
            Vec::new(),
        ),
        ProductionGuiSpecTemplate::StructuralCodingContrast => spec(
            Verb::Run,
            Vec::new(),
            Some(ContentClass::Generated {
                hint: "pascal triangle program".to_string(),
                language: Some("python".to_string()),
            }),
            Vec::new(),
        ),
        ProductionGuiSpecTemplate::BrowserVisibleCheck => spec(
            Verb::Open,
            vec![TargetRef::Url("https://example.com".to_string())],
            None,
            Vec::new(),
        ),
        ProductionGuiSpecTemplate::BrowserAccountUpload
        | ProductionGuiSpecTemplate::BrowserAccountSurfaceConflict => spec(
            Verb::Open,
            vec![TargetRef::App("Firefox".to_string())],
            None,
            Vec::new(),
        ),
        ProductionGuiSpecTemplate::PrivateMediaPlaylist => spec(
            Verb::Open,
            vec![TargetRef::App("YouTube".to_string())],
            None,
            Vec::new(),
        ),
        ProductionGuiSpecTemplate::CommunicationReviewBeforeSend => spec(
            Verb::Open,
            vec![TargetRef::App("Gmail".to_string())],
            Some(ContentClass::Generated {
                hint: "email draft".to_string(),
                language: None,
            }),
            Vec::new(),
        ),
        ProductionGuiSpecTemplate::DocumentEditorVisibleUpdate => spec(
            Verb::Open,
            vec![TargetRef::App("LibreOffice Writer".to_string())],
            Some(ContentClass::Generated {
                hint: "document summary update".to_string(),
                language: None,
            }),
            Vec::new(),
        ),
        ProductionGuiSpecTemplate::SpreadsheetLaymanTotals => spec(
            Verb::Open,
            vec![TargetRef::App("Excel".to_string())],
            Some(ContentClass::Generated {
                hint: "spreadsheet totals".to_string(),
                language: None,
            }),
            Vec::new(),
        ),
        ProductionGuiSpecTemplate::MessyDeicticRunThereShow => spec(
            Verb::Run,
            Vec::new(),
            None,
            vec![Ambiguity::FileNotSpecified],
        ),
    }
}

fn policy_for_template(template: ProductionGuiSpecTemplate) -> PolicyContext {
    if template == ProductionGuiSpecTemplate::VisibleCodingPolicyBlocked {
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
    template: ProductionGuiSpecTemplate,
    dimensions: &[ProductionGuiEvalDimension],
    prompt_shapes: &[ProductionPromptShape],
    expectation: ProductionGuiExpectation,
    capability_ids: &[&str],
    failure_mode_ids: &[&str],
) -> ProductionGuiEvalCase {
    let workflow_contract_id = expectation
        .expected_contract_id
        .unwrap_or(WorkflowContractId::GeneralVisibleWorkflow);
    ProductionGuiEvalCase {
        id: id.to_string(),
        description: description.to_string(),
        prompt: prompt.to_string(),
        template,
        dimensions: dimensions.to_vec(),
        prompt_shapes: prompt_shapes.to_vec(),
        capability_ids: strings(capability_ids),
        workflow_contract_id,
        failure_mode_ids: strings(failure_mode_ids),
        expectation,
    }
}

fn expectation(
    expected_mode: Option<ExecutionMode>,
    expected_contract_id: Option<WorkflowContractId>,
    expected_fidelity: Option<WorkflowFidelityTier>,
    expected_task_family: Option<TaskFamily>,
    expected_visibility: Option<VisibilityExpectation>,
    required_app_classes: &[AppClass],
    clarification_required: Option<bool>,
    required_verifiers: &[RequiredVerifier],
) -> ProductionGuiExpectation {
    ProductionGuiExpectation {
        expected_mode,
        expected_contract_id,
        expected_fidelity,
        expected_task_family,
        expected_visibility,
        required_app_classes: required_app_classes.to_vec(),
        clarification_required,
        required_verifiers: required_verifiers.to_vec(),
        expected_governance_action: None,
        expected_forbidden_degradations: Vec::new(),
        expected_verifier_verdict: None,
        expected_hybrid_sync_verdict: None,
        partial_completion_required: None,
        recovery_pause_or_disclosure_required: None,
    }
}

impl ProductionGuiExpectation {
    fn with_governance_action(mut self, action: BrowserMediaGovernanceAction) -> Self {
        self.expected_governance_action = Some(action);
        self
    }

    fn with_forbidden_degradations(mut self, degradations: &[ForbiddenDegradation]) -> Self {
        self.expected_forbidden_degradations = degradations.to_vec();
        self
    }

    fn with_partial_completion_required(mut self, required: bool) -> Self {
        self.partial_completion_required = Some(required);
        self
    }

    fn with_recovery_pause_or_disclosure(mut self, required: bool) -> Self {
        self.recovery_pause_or_disclosure_required = Some(required);
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

fn strings(values: &[&str]) -> Vec<String> {
    values.iter().map(|value| (*value).to_string()).collect()
}

fn prompt_shape_summaries(
    results: &[ProductionGuiCaseResult],
) -> Vec<ProductionPromptShapeSummary> {
    let mut counts = BTreeMap::<ProductionPromptShape, (usize, usize)>::new();
    for result in results {
        for shape in &result.case.prompt_shapes {
            let entry = counts.entry(*shape).or_insert((0, 0));
            entry.0 += 1;
            if result.verdict.passed {
                entry.1 += 1;
            }
        }
    }
    counts
        .into_iter()
        .map(
            |(prompt_shape, (total, passed))| ProductionPromptShapeSummary {
                prompt_shape,
                total,
                passed,
                failed: total.saturating_sub(passed),
            },
        )
        .collect()
}

fn dimension_summaries(results: &[ProductionGuiCaseResult]) -> Vec<ProductionGuiDimensionSummary> {
    let mut counts = BTreeMap::<ProductionGuiEvalDimension, (usize, usize)>::new();
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
            |(dimension, (total, passed))| ProductionGuiDimensionSummary {
                dimension,
                total,
                passed,
                failed: total.saturating_sub(passed),
            },
        )
        .collect()
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
    fn phase9_suite_checks_phase8_prerequisite_and_passes() {
        let report = run_production_gui_workflow_suite("phase9-test-run");

        assert!(
            report
                .prerequisite_status
                .phase8_workflow_fidelity_passed_all
        );
        assert_eq!(report.summary.failed, 0);
        assert_eq!(report.summary.passed, report.summary.total);
        assert!(report.summary.total >= 12);
    }

    #[test]
    fn suite_covers_required_production_prompt_shapes() {
        let report = run_production_gui_workflow_suite("phase9-test-run");
        let shapes = report
            .summary
            .prompt_shape_summaries
            .iter()
            .map(|summary| summary.prompt_shape)
            .collect::<Vec<_>>();

        for expected in [
            ProductionPromptShape::RealWorld,
            ProductionPromptShape::LongPrompt,
            ProductionPromptShape::IndirectPrompt,
            ProductionPromptShape::MessyPrompt,
            ProductionPromptShape::LaymanPrompt,
            ProductionPromptShape::DeveloperPrompt,
            ProductionPromptShape::BrowserWorkflow,
            ProductionPromptShape::MediaWorkflow,
            ProductionPromptShape::DocumentWorkflow,
            ProductionPromptShape::SpreadsheetWorkflow,
            ProductionPromptShape::CommunicationWorkflow,
            ProductionPromptShape::StructuralContrast,
        ] {
            assert!(shapes.contains(&expected), "missing shape {:?}", expected);
        }
    }

    #[test]
    fn browser_account_case_requires_hitl_pause() {
        let case = production_gui_workflow_suite()
            .into_iter()
            .find(|case| case.id == "prod-gui-005-browser-account-hitl")
            .expect("browser account case");
        let result = run_production_gui_case(case);

        assert!(result.verdict.passed, "{:?}", result.verdict.failures);
        assert!(result.observation.clarification_required);
        assert_eq!(
            result.observation.browser_media_action,
            BrowserMediaGovernanceAction::RequireHitlPause
        );
    }

    #[test]
    fn report_serializes_to_json() {
        let report = run_production_gui_workflow_suite("phase9-test-run");
        let json = serde_json::to_string_pretty(&report).expect("serialize");
        let roundtrip: ProductionGuiWorkflowReport =
            serde_json::from_str(&json).expect("deserialize");

        assert_eq!(roundtrip.summary.total, report.summary.total);
        assert!(json.contains("phase9_production_gui_workflows_v1"));
    }

    #[test]
    fn markdown_report_contains_prerequisite_and_prompt_shapes() {
        let report = run_production_gui_workflow_suite("phase9-test-run");
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("production_gui_workflows.md");
        write_production_gui_workflow_markdown(&report, &path).expect("markdown write");
        let markdown = std::fs::read_to_string(path).expect("read markdown");

        assert!(markdown.contains("Phase 8 prerequisite"));
        assert!(markdown.contains("Prompt Shapes"));
        assert!(markdown.contains("prod-gui-001-visible-coding-hybrid"));
    }
}
