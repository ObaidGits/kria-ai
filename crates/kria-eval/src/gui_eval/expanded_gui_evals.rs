//! Bounded expanded GUI workflow evals.
//!
//! This suite adds targeted coverage for high-value verifier, freshness,
//! fallback, ambiguity, document, spreadsheet, communication, browser, media,
//! and prompt-mutation cases. It is intentionally separate from the Phase 9/10
//! gates so new evals can be run independently before promotion.

use kria_core::agent::browser_media_governance::{
    BrowserMediaGovernanceAction, BrowserMediaGovernanceEvaluator,
};
use kria_core::agent::execution_mode_reasoner::{
    CapabilityAvailability, EnvironmentCapabilities, ExecutionMode, ExecutionModeDecision,
    ExecutionModeReasoner, PolicyContext, RequiredVerifier, WorkflowContractId,
};
use kria_core::agent::hybrid_synchronization::{
    HybridSynchronizationCheckpointKind, HybridSynchronizationEvaluator,
    HybridSynchronizationObservation, HybridSynchronizationOverallVerdict,
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
pub enum ExpandedGuiEvalDimension {
    NegativeVerifierFreshness,
    BrowserAccountSession,
    DocumentSpreadsheetVisibleState,
    CommunicationApproval,
    AmbiguityGovernance,
    FallbackHonesty,
    PromptMutation,
    MediaPlayback,
}

impl ExpandedGuiEvalDimension {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::NegativeVerifierFreshness => "negative_verifier_freshness",
            Self::BrowserAccountSession => "browser_account_session",
            Self::DocumentSpreadsheetVisibleState => "document_spreadsheet_visible_state",
            Self::CommunicationApproval => "communication_approval",
            Self::AmbiguityGovernance => "ambiguity_governance",
            Self::FallbackHonesty => "fallback_honesty",
            Self::PromptMutation => "prompt_mutation",
            Self::MediaPlayback => "media_playback",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExpandedGuiFailureMode {
    WrongVisibleFile,
    StaleIdeBuffer,
    OldTerminalOutput,
    OutputFileOnly,
    WrongBrowserAccount,
    StaleBrowserNavigation,
    WrongBrowserTarget,
    SearchPageNotPlayback,
    WrongSpreadsheetSheet,
    StaleDocumentContent,
    StaleApproval,
    MissingApproval,
    VisibleFallbackDowngrade,
    VisibleGuiUnavailable,
    AmbiguousObjectReference,
    PromptMutation,
}

impl ExpandedGuiFailureMode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::WrongVisibleFile => "wrong_visible_file",
            Self::StaleIdeBuffer => "stale_ide_buffer",
            Self::OldTerminalOutput => "old_terminal_output",
            Self::OutputFileOnly => "output_file_only",
            Self::WrongBrowserAccount => "wrong_browser_account",
            Self::StaleBrowserNavigation => "stale_browser_navigation",
            Self::WrongBrowserTarget => "wrong_browser_target",
            Self::SearchPageNotPlayback => "search_page_not_playback",
            Self::WrongSpreadsheetSheet => "wrong_spreadsheet_sheet",
            Self::StaleDocumentContent => "stale_document_content",
            Self::StaleApproval => "stale_approval",
            Self::MissingApproval => "missing_approval",
            Self::VisibleFallbackDowngrade => "visible_fallback_downgrade",
            Self::VisibleGuiUnavailable => "visible_gui_unavailable",
            Self::AmbiguousObjectReference => "ambiguous_object_reference",
            Self::PromptMutation => "prompt_mutation",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExpandedGuiTemplate {
    IdeWrongVisibleFile,
    IdeStaleBuffer,
    TerminalOldOutput,
    TerminalOutputFileOnly,
    BrowserWrongAccount,
    BrowserStaleNavigation,
    BrowserWrongTarget,
    MediaSearchNotPlayback,
    SpreadsheetVisibleTotals,
    SpreadsheetWrongSheet,
    DocumentVisibleContent,
    DocumentStaleContent,
    CommunicationApproval,
    CommunicationStaleApproval,
    CommunicationMissingApproval,
    VisibleFallbackPolicy,
    VisibleGuiUnavailable,
    AmbiguousUploadIt,
    DeveloperDeicticCode,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExpandedGuiExpectation {
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
    pub expected_trace_label: Option<String>,
    pub partial_completion_required: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExpandedGuiEvalCase {
    pub id: String,
    pub description: String,
    pub prompt: String,
    pub template: ExpandedGuiTemplate,
    pub dimensions: Vec<ExpandedGuiEvalDimension>,
    pub capability_ids: Vec<String>,
    pub failure_modes: Vec<ExpandedGuiFailureMode>,
    pub expectation: ExpandedGuiExpectation,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExpandedGuiObservation {
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
    pub browser_media_action: BrowserMediaGovernanceAction,
    pub verifier_verdict: Option<VerifierAuthorityOverallVerdict>,
    pub hybrid_sync_verdict: Option<HybridSynchronizationOverallVerdict>,
    pub trace_labels: Vec<String>,
    pub evidence: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExpandedGuiVerdict {
    pub case_id: String,
    pub passed: bool,
    pub explanation: String,
    pub failures: Vec<String>,
    pub evidence: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExpandedGuiCaseResult {
    pub case: ExpandedGuiEvalCase,
    pub observation: ExpandedGuiObservation,
    pub verdict: ExpandedGuiVerdict,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExpandedGuiDimensionSummary {
    pub dimension: ExpandedGuiEvalDimension,
    pub total: usize,
    pub passed: usize,
    pub failed: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExpandedGuiFailureModeSummary {
    pub failure_mode: ExpandedGuiFailureMode,
    pub total: usize,
    pub passed: usize,
    pub failed: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExpandedGuiSummary {
    pub total: usize,
    pub passed: usize,
    pub failed: usize,
    pub dimension_summaries: Vec<ExpandedGuiDimensionSummary>,
    pub failure_mode_summaries: Vec<ExpandedGuiFailureModeSummary>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExpandedGuiEvalReport {
    pub run_id: String,
    pub generated_at: String,
    pub report_version: String,
    pub summary: ExpandedGuiSummary,
    pub case_results: Vec<ExpandedGuiCaseResult>,
}

pub fn expanded_gui_eval_suite() -> Vec<ExpandedGuiEvalCase> {
    vec![
        case(
            "expanded-gui-001-ide-wrong-visible-file",
            "IDE visible surface must not count if it shows the wrong file.",
            "open code and run the script and show output",
            ExpandedGuiTemplate::IdeWrongVisibleFile,
            &[ExpandedGuiEvalDimension::NegativeVerifierFreshness],
            &["ide.open_file_visible", "hybrid.visible_artifact_sync"],
            &[ExpandedGuiFailureMode::WrongVisibleFile],
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
        ),
        case(
            "expanded-gui-002-ide-stale-buffer",
            "IDE visible file hash mismatch must invalidate hybrid completion.",
            "open code and write a python script, run it, and show me output",
            ExpandedGuiTemplate::IdeStaleBuffer,
            &[ExpandedGuiEvalDimension::NegativeVerifierFreshness],
            &["ide.open_file_visible", "hybrid.file_hash_sync"],
            &[ExpandedGuiFailureMode::StaleIdeBuffer],
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
        ),
        case(
            "expanded-gui-003-terminal-old-output",
            "Visible terminal output must carry the current run marker.",
            "open terminal and run df -h and show output",
            ExpandedGuiTemplate::TerminalOldOutput,
            &[ExpandedGuiEvalDimension::NegativeVerifierFreshness],
            &["terminal.output_visible", "hybrid.terminal_freshness"],
            &[ExpandedGuiFailureMode::OldTerminalOutput],
            expectation(
                Some(ExecutionMode::HybridWorkflow),
                Some(WorkflowContractId::GeneralVisibleWorkflow),
                Some(WorkflowFidelityTier::WorkflowStageFidelity),
                Some(TaskFamily::SystemTerminal),
                Some(VisibilityExpectation::WorkflowVisible),
                &[AppClass::Terminal],
                Some(false),
                &[RequiredVerifier::TerminalOutputVisible],
            )
            .with_hybrid_sync_verdict(HybridSynchronizationOverallVerdict::Invalidated),
        ),
        case(
            "expanded-gui-004-output-file-only",
            "Captured output file alone must not satisfy output-surfaced verifier.",
            "open terminal and run uptime and show output",
            ExpandedGuiTemplate::TerminalOutputFileOnly,
            &[ExpandedGuiEvalDimension::NegativeVerifierFreshness],
            &["terminal.output_visible", "verifier.output_surfaced"],
            &[ExpandedGuiFailureMode::OutputFileOnly],
            expectation(
                Some(ExecutionMode::HybridWorkflow),
                Some(WorkflowContractId::GeneralVisibleWorkflow),
                Some(WorkflowFidelityTier::WorkflowStageFidelity),
                Some(TaskFamily::SystemTerminal),
                Some(VisibilityExpectation::WorkflowVisible),
                &[AppClass::Terminal],
                Some(false),
                &[RequiredVerifier::OutputSurfaced],
            )
            .with_verifier_verdict(VerifierAuthorityOverallVerdict::Partial),
        ),
        case(
            "expanded-gui-005-browser-wrong-account",
            "Browser account context requires user-confirmed authority.",
            "open browser and upload this file to my account",
            ExpandedGuiTemplate::BrowserWrongAccount,
            &[
                ExpandedGuiEvalDimension::BrowserAccountSession,
                ExpandedGuiEvalDimension::NegativeVerifierFreshness,
            ],
            &["browser.account_context", "hitl.pause"],
            &[ExpandedGuiFailureMode::WrongBrowserAccount],
            expectation(
                Some(ExecutionMode::HumanCollaborativeWorkflow),
                Some(WorkflowContractId::HumanReviewWorkflow),
                Some(WorkflowFidelityTier::HumanObservedFidelity),
                Some(TaskFamily::Browser),
                Some(VisibilityExpectation::HumanObserved),
                &[AppClass::Browser],
                Some(true),
                &[RequiredVerifier::BrowserAccountContext, RequiredVerifier::UserConfirmation],
            )
            .with_governance_action(BrowserMediaGovernanceAction::RequireHitlPause)
            .with_verifier_verdict(VerifierAuthorityOverallVerdict::Partial),
        ),
        case(
            "expanded-gui-006-browser-stale-navigation",
            "Browser page navigation predating the workflow attempt is stale evidence.",
            "open browser at https://example.com and see if it works",
            ExpandedGuiTemplate::BrowserStaleNavigation,
            &[
                ExpandedGuiEvalDimension::BrowserAccountSession,
                ExpandedGuiEvalDimension::NegativeVerifierFreshness,
            ],
            &["browser.navigate_visible", "hybrid.browser_freshness"],
            &[ExpandedGuiFailureMode::StaleBrowserNavigation],
            expectation(
                Some(ExecutionMode::VisibleAppWorkflow),
                Some(WorkflowContractId::VisibleBrowserWorkflow),
                Some(WorkflowFidelityTier::WorkflowStageFidelity),
                Some(TaskFamily::Browser),
                Some(VisibilityExpectation::WorkflowVisible),
                &[AppClass::Browser],
                Some(false),
                &[RequiredVerifier::BrowserPageVisible],
            )
            .with_verifier_verdict(VerifierAuthorityOverallVerdict::Partial),
        ),
        case(
            "expanded-gui-007-browser-wrong-target",
            "Browser visible page must match the requested target identity.",
            "open browser at https://example.com and see if it works",
            ExpandedGuiTemplate::BrowserWrongTarget,
            &[ExpandedGuiEvalDimension::BrowserAccountSession],
            &["browser.page_visible", "hybrid.browser_target_identity"],
            &[ExpandedGuiFailureMode::WrongBrowserTarget],
            expectation(
                Some(ExecutionMode::VisibleAppWorkflow),
                Some(WorkflowContractId::VisibleBrowserWorkflow),
                Some(WorkflowFidelityTier::WorkflowStageFidelity),
                Some(TaskFamily::Browser),
                Some(VisibilityExpectation::WorkflowVisible),
                &[AppClass::Browser],
                Some(false),
                &[RequiredVerifier::BrowserPageVisible],
            )
            .with_verifier_verdict(VerifierAuthorityOverallVerdict::Partial),
        ),
        case(
            "expanded-gui-008-media-search-not-playback",
            "Media search/listing page must not count as playback.",
            "open youtube and play relaxing music",
            ExpandedGuiTemplate::MediaSearchNotPlayback,
            &[ExpandedGuiEvalDimension::MediaPlayback],
            &["media.playback_visible"],
            &[ExpandedGuiFailureMode::SearchPageNotPlayback],
            expectation(
                Some(ExecutionMode::VisibleAppWorkflow),
                Some(WorkflowContractId::MediaInteractionWorkflow),
                Some(WorkflowFidelityTier::AppContextFidelity),
                Some(TaskFamily::Media),
                Some(VisibilityExpectation::AppVisible),
                &[AppClass::Media],
                Some(false),
                &[RequiredVerifier::MediaPlaybackVisible],
            )
            .with_verifier_verdict(VerifierAuthorityOverallVerdict::Pending),
        ),
        case(
            "expanded-gui-009-spreadsheet-visible-totals",
            "Spreadsheet correction must preserve visible spreadsheet app context.",
            "open excel and fix the total formulas",
            ExpandedGuiTemplate::SpreadsheetVisibleTotals,
            &[ExpandedGuiEvalDimension::DocumentSpreadsheetVisibleState],
            &["spreadsheet.app_visible"],
            &[],
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
        ),
        case(
            "expanded-gui-010-spreadsheet-wrong-sheet",
            "Spreadsheet visible context without target identity is only partial evidence.",
            "open excel and fix the total formulas",
            ExpandedGuiTemplate::SpreadsheetWrongSheet,
            &[
                ExpandedGuiEvalDimension::DocumentSpreadsheetVisibleState,
                ExpandedGuiEvalDimension::NegativeVerifierFreshness,
            ],
            &["spreadsheet.app_visible", "verifier.target_identity"],
            &[ExpandedGuiFailureMode::WrongSpreadsheetSheet],
            expectation(
                Some(ExecutionMode::VisibleAppWorkflow),
                Some(WorkflowContractId::GeneralVisibleWorkflow),
                Some(WorkflowFidelityTier::AppContextFidelity),
                Some(TaskFamily::Spreadsheet),
                Some(VisibilityExpectation::AppVisible),
                &[AppClass::Spreadsheet],
                Some(false),
                &[RequiredVerifier::AppContextVisible],
            )
            .with_verifier_verdict(VerifierAuthorityOverallVerdict::Partial),
        ),
        case(
            "expanded-gui-011-document-visible-content",
            "Document update must require visible document content evidence.",
            "open writer and update the summary then show me result",
            ExpandedGuiTemplate::DocumentVisibleContent,
            &[ExpandedGuiEvalDimension::DocumentSpreadsheetVisibleState],
            &["document.content_visible"],
            &[],
            expectation(
                Some(ExecutionMode::VisibleAppWorkflow),
                Some(WorkflowContractId::GeneralVisibleWorkflow),
                Some(WorkflowFidelityTier::AppContextFidelity),
                Some(TaskFamily::DocumentEditing),
                Some(VisibilityExpectation::AppVisible),
                &[AppClass::DocumentEditor],
                Some(false),
                &[RequiredVerifier::DocumentContentVisible],
            ),
        ),
        case(
            "expanded-gui-012-document-stale-content",
            "Document visible content evidence must be fresh.",
            "open writer and update the summary then show me result",
            ExpandedGuiTemplate::DocumentStaleContent,
            &[
                ExpandedGuiEvalDimension::DocumentSpreadsheetVisibleState,
                ExpandedGuiEvalDimension::NegativeVerifierFreshness,
            ],
            &["document.content_visible", "verifier.freshness"],
            &[ExpandedGuiFailureMode::StaleDocumentContent],
            expectation(
                Some(ExecutionMode::VisibleAppWorkflow),
                Some(WorkflowContractId::GeneralVisibleWorkflow),
                Some(WorkflowFidelityTier::AppContextFidelity),
                Some(TaskFamily::DocumentEditing),
                Some(VisibilityExpectation::AppVisible),
                &[AppClass::DocumentEditor],
                Some(false),
                &[RequiredVerifier::DocumentContentVisible],
            )
            .with_verifier_verdict(VerifierAuthorityOverallVerdict::Partial),
        ),
        case(
            "expanded-gui-013-communication-approval",
            "Email approval wording must require human collaborative workflow.",
            "draft an email in gmail but do not send until i approve",
            ExpandedGuiTemplate::CommunicationApproval,
            &[ExpandedGuiEvalDimension::CommunicationApproval],
            &["communication.review_surface", "hitl.approval"],
            &[],
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
            .with_partial_completion_required(false),
        ),
        case(
            "expanded-gui-014-communication-stale-approval",
            "Stale approval cannot authorize a changed communication action.",
            "draft an email in gmail but do not send until i approve",
            ExpandedGuiTemplate::CommunicationStaleApproval,
            &[
                ExpandedGuiEvalDimension::CommunicationApproval,
                ExpandedGuiEvalDimension::NegativeVerifierFreshness,
            ],
            &["communication.review_surface", "hitl.approval_freshness"],
            &[ExpandedGuiFailureMode::StaleApproval],
            expectation(
                Some(ExecutionMode::HumanCollaborativeWorkflow),
                Some(WorkflowContractId::HumanReviewWorkflow),
                Some(WorkflowFidelityTier::HumanObservedFidelity),
                Some(TaskFamily::Communication),
                Some(VisibilityExpectation::HumanObserved),
                &[AppClass::Communication],
                Some(true),
                &[RequiredVerifier::UserConfirmation],
            )
            .with_verifier_verdict(VerifierAuthorityOverallVerdict::Partial),
        ),
        case(
            "expanded-gui-015-communication-missing-approval",
            "Prepared communication draft without user confirmation remains pending.",
            "draft an email in gmail and send it after i approve",
            ExpandedGuiTemplate::CommunicationMissingApproval,
            &[ExpandedGuiEvalDimension::CommunicationApproval],
            &["communication.review_surface", "hitl.approval"],
            &[ExpandedGuiFailureMode::MissingApproval],
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
            .with_verifier_verdict(VerifierAuthorityOverallVerdict::Pending),
        ),
        case(
            "expanded-gui-016-visible-fallback-policy",
            "Policy-blocked visible workflow must report forbidden visible-to-structural downgrade.",
            "open code and write a script, run it and show output",
            ExpandedGuiTemplate::VisibleFallbackPolicy,
            &[ExpandedGuiEvalDimension::FallbackHonesty],
            &["fallback.disclosure", "partial_completion"],
            &[ExpandedGuiFailureMode::VisibleFallbackDowngrade],
            expectation(
                Some(ExecutionMode::StructuralExecution),
                Some(WorkflowContractId::VisibleCodingWorkflow),
                Some(WorkflowFidelityTier::WorkflowStageFidelity),
                Some(TaskFamily::Coding),
                Some(VisibilityExpectation::WorkflowVisible),
                &[AppClass::Ide],
                Some(false),
                &[RequiredVerifier::OutputSurfaced],
            )
            .with_forbidden_degradations(&[ForbiddenDegradation::SilentVisibleToStructural])
            .with_partial_completion_required(true),
        ),
        case(
            "expanded-gui-017-visible-gui-unavailable",
            "Unavailable visible GUI capability must be trace-visible before planning.",
            "open code and write a script, run it and show output",
            ExpandedGuiTemplate::VisibleGuiUnavailable,
            &[
                ExpandedGuiEvalDimension::FallbackHonesty,
                ExpandedGuiEvalDimension::PromptMutation,
            ],
            &["environment.visible_gui", "fallback.disclosure"],
            &[ExpandedGuiFailureMode::VisibleGuiUnavailable],
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
            .with_trace_label("environment_visible_gui_unavailable"),
        ),
        case(
            "expanded-gui-018-ambiguous-upload-it",
            "Account upload with deictic target must require clarification/HITL.",
            "upload it to my account",
            ExpandedGuiTemplate::AmbiguousUploadIt,
            &[
                ExpandedGuiEvalDimension::AmbiguityGovernance,
                ExpandedGuiEvalDimension::BrowserAccountSession,
            ],
            &["clarification.identity", "browser.account_context"],
            &[ExpandedGuiFailureMode::AmbiguousObjectReference],
            expectation(
                Some(ExecutionMode::HumanCollaborativeWorkflow),
                Some(WorkflowContractId::HumanReviewWorkflow),
                Some(WorkflowFidelityTier::HumanObservedFidelity),
                Some(TaskFamily::Browser),
                Some(VisibilityExpectation::HumanObserved),
                &[AppClass::Browser],
                Some(true),
                &[RequiredVerifier::BrowserAccountContext, RequiredVerifier::UserConfirmation],
            )
            .with_governance_action(BrowserMediaGovernanceAction::RequireHitlPause),
        ),
        case(
            "expanded-gui-019-developer-deictic-code",
            "Developer shorthand with unresolved 'this' must preserve IDE context and ask.",
            "can you open code and try this quickly",
            ExpandedGuiTemplate::DeveloperDeicticCode,
            &[
                ExpandedGuiEvalDimension::AmbiguityGovernance,
                ExpandedGuiEvalDimension::PromptMutation,
            ],
            &["ide.open_file_visible", "clarification.identity"],
            &[ExpandedGuiFailureMode::PromptMutation],
            expectation(
                Some(ExecutionMode::VisibleAppWorkflow),
                Some(WorkflowContractId::IdeCollaborativeWorkflow),
                Some(WorkflowFidelityTier::AppContextFidelity),
                Some(TaskFamily::Coding),
                Some(VisibilityExpectation::AppVisible),
                &[AppClass::Ide],
                Some(true),
                &[RequiredVerifier::IdeFileVisible, RequiredVerifier::AppContextVisible],
            ),
        ),
    ]
}

pub fn run_expanded_gui_eval_suite(run_id: impl Into<String>) -> ExpandedGuiEvalReport {
    let case_results = expanded_gui_eval_suite()
        .into_iter()
        .map(run_expanded_gui_eval_case)
        .collect::<Vec<_>>();
    let total = case_results.len();
    let passed = case_results
        .iter()
        .filter(|result| result.verdict.passed)
        .count();
    let summary = ExpandedGuiSummary {
        total,
        passed,
        failed: total.saturating_sub(passed),
        dimension_summaries: dimension_summaries(&case_results),
        failure_mode_summaries: failure_mode_summaries(&case_results),
    };

    ExpandedGuiEvalReport {
        run_id: run_id.into(),
        generated_at: unix_now(),
        report_version: "expanded_gui_evals_v1".to_string(),
        summary,
        case_results,
    }
}

pub fn run_expanded_gui_eval_case(case: ExpandedGuiEvalCase) -> ExpandedGuiCaseResult {
    let spec = spec_for_template(case.template);
    let analysis = analyze_semantic_workflow(&spec, &case.prompt);
    let policy = policy_for_template(case.template);
    let environment = environment_for_template(case.template);
    let decision = ExecutionModeReasoner.decide(&spec, &analysis, &environment, &policy);
    let contract_check = WorkflowIntentContractRegistry.evaluate(&decision, &analysis);
    let attempt_id = format!("expanded:{}", case.id);
    let verifier_authority =
        VerifierAuthorityEvaluator.assess(&contract_check, &decision, &analysis, attempt_id);
    let hybrid_assessment = HybridSynchronizationEvaluator.assess(
        &decision,
        &analysis,
        &verifier_authority,
        format!("expanded:{}", case.id),
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
        &browser_media,
        verifier_authority.partial_completion_required,
        verifier_verdict,
        hybrid_sync_verdict,
    );
    let verdict = judge_case(&case, &observation);

    ExpandedGuiCaseResult {
        case,
        observation,
        verdict,
    }
}

pub fn print_expanded_gui_eval_report(report: &ExpandedGuiEvalReport) {
    println!("Expanded GUI Eval");
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

pub fn write_expanded_gui_eval_markdown(
    report: &ExpandedGuiEvalReport,
    path: impl AsRef<std::path::Path>,
) -> std::io::Result<()> {
    let mut markdown = String::new();
    markdown.push_str("# Expanded GUI Evals\n\n");
    markdown.push_str(&format!("**Run ID:** `{}`\n\n", report.run_id));
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
    markdown.push_str("\n## Failure Modes\n\n");
    markdown.push_str("| Failure mode | Passed | Total |\n");
    markdown.push_str("|---|---:|---:|\n");
    for summary in &report.summary.failure_mode_summaries {
        markdown.push_str(&format!(
            "| {} | {} | {} |\n",
            summary.failure_mode.as_str(),
            summary.passed,
            summary.total
        ));
    }
    markdown.push_str("\n## Cases\n\n");
    markdown.push_str("| Case | Verdict | Explanation |\n");
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
    case: &ExpandedGuiEvalCase,
    analysis: &SemanticWorkflowAnalysis,
    decision: &ExecutionModeDecision,
    contract_check: &ContractCheck,
    browser_media: &kria_core::agent::browser_media_governance::BrowserMediaGovernanceAssessment,
    partial_completion_required: bool,
    verifier_verdict: Option<VerifierAuthorityOverallVerdict>,
    hybrid_sync_verdict: Option<HybridSynchronizationOverallVerdict>,
) -> ExpandedGuiObservation {
    let mut trace_labels = Vec::new();
    trace_labels.extend(analysis.frame.trace.signal_labels.clone());
    trace_labels.extend(decision.trace.reason_labels.clone());
    trace_labels.extend(contract_check.trace.trace_labels.clone());
    trace_labels.extend(browser_media.trace.trace_labels.clone());

    let app_classes = analysis
        .frame
        .app_anchors
        .iter()
        .map(|anchor| anchor.app_class)
        .collect::<Vec<_>>();

    let mut evidence = Vec::new();
    evidence.push(format!("mode::{:?}", decision.mode));
    evidence.push(format!("contract::{:?}", decision.workflow_contract_id));
    evidence.push(format!(
        "fidelity::{:?}",
        analysis.fidelity.requested_fidelity
    ));
    evidence.push(format!(
        "verifiers::{}",
        contract_check.verifier_requirements.len()
    ));
    if let Some(verdict) = verifier_verdict {
        evidence.push(format!("verifier_verdict::{verdict:?}"));
    }
    if let Some(verdict) = hybrid_sync_verdict {
        evidence.push(format!("hybrid_sync_verdict::{verdict:?}"));
    }

    ExpandedGuiObservation {
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
        partial_completion_required,
        browser_media_action: browser_media.action,
        verifier_verdict,
        hybrid_sync_verdict,
        trace_labels,
        evidence,
    }
}

fn judge_case(
    case: &ExpandedGuiEvalCase,
    observation: &ExpandedGuiObservation,
) -> ExpandedGuiVerdict {
    let mut failures = Vec::new();
    let expectation = &case.expectation;

    if let Some(expected) = expectation.expected_mode {
        if observation.mode != expected {
            failures.push(format!(
                "mode mismatch: expected {expected:?}, got {:?}",
                observation.mode
            ));
        }
    }
    if let Some(expected) = expectation.expected_contract_id {
        if observation.contract_id != expected {
            failures.push(format!(
                "contract mismatch: expected {expected:?}, got {:?}",
                observation.contract_id
            ));
        }
    }
    if let Some(expected) = expectation.expected_fidelity {
        if observation.requested_fidelity != expected {
            failures.push(format!(
                "fidelity mismatch: expected {expected:?}, got {:?}",
                observation.requested_fidelity
            ));
        }
    }
    if let Some(expected) = expectation.expected_task_family {
        if observation.task_family != expected {
            failures.push(format!(
                "task family mismatch: expected {expected:?}, got {:?}",
                observation.task_family
            ));
        }
    }
    if let Some(expected) = expectation.expected_visibility {
        if observation.visibility != expected {
            failures.push(format!(
                "visibility mismatch: expected {expected:?}, got {:?}",
                observation.visibility
            ));
        }
    }
    for app_class in &expectation.required_app_classes {
        if !observation.app_classes.contains(app_class) {
            failures.push(format!("missing app class::{app_class:?}"));
        }
    }
    if let Some(expected) = expectation.clarification_required {
        if observation.clarification_required != expected {
            failures.push(format!(
                "clarification mismatch: expected {expected}, got {}",
                observation.clarification_required
            ));
        }
    }
    for verifier in &expectation.required_verifiers {
        if !observation.required_verifiers.contains(verifier) {
            failures.push(format!("missing verifier::{verifier:?}"));
        }
    }
    if let Some(expected) = expectation.expected_governance_action {
        if observation.browser_media_action != expected {
            failures.push(format!(
                "governance mismatch: expected {expected:?}, got {:?}",
                observation.browser_media_action
            ));
        }
    }
    for degradation in &expectation.expected_forbidden_degradations {
        if !observation.forbidden_degradations.contains(degradation) {
            failures.push(format!("missing forbidden degradation::{degradation:?}"));
        }
    }
    if let Some(expected) = expectation.expected_verifier_verdict {
        if observation.verifier_verdict != Some(expected) {
            failures.push(format!(
                "verifier verdict mismatch: expected {expected:?}, got {:?}",
                observation.verifier_verdict
            ));
        }
    }
    if let Some(expected) = expectation.expected_hybrid_sync_verdict {
        if observation.hybrid_sync_verdict != Some(expected) {
            failures.push(format!(
                "hybrid sync mismatch: expected {expected:?}, got {:?}",
                observation.hybrid_sync_verdict
            ));
        }
    }
    if let Some(expected) = expectation.expected_trace_label.as_ref() {
        if !observation
            .trace_labels
            .iter()
            .any(|label| label == expected)
        {
            failures.push(format!("missing trace label::{expected}"));
        }
    }
    if let Some(expected) = expectation.partial_completion_required {
        if observation.partial_completion_required != expected {
            failures.push(format!(
                "partial completion mismatch: expected {expected}, got {}",
                observation.partial_completion_required
            ));
        }
    }

    ExpandedGuiVerdict {
        case_id: case.id.clone(),
        passed: failures.is_empty(),
        explanation: if failures.is_empty() {
            "expanded GUI expectation satisfied".to_string()
        } else {
            failures.join("; ")
        },
        failures,
        evidence: observation.evidence.clone(),
    }
}

fn verifier_verdict_for_template(
    template: ExpandedGuiTemplate,
    requirements: &[kria_core::agent::verifier_authority::VerifierAuthorityRequirement],
) -> Option<VerifierAuthorityOverallVerdict> {
    let observed = match template {
        ExpandedGuiTemplate::TerminalOutputFileOnly => vec![observed(
            RequiredVerifier::OutputSurfaced,
            VerifierAuthorityLevel::StructuralAuthority,
            EvidenceFreshnessStatus::Fresh,
            Some("attempt-output".to_string()),
            Some("terminal:uptime".to_string()),
            "output exists only as structural file evidence",
        )],
        ExpandedGuiTemplate::BrowserWrongAccount => vec![observed(
            RequiredVerifier::BrowserAccountContext,
            VerifierAuthorityLevel::SurfaceAuthority,
            EvidenceFreshnessStatus::Fresh,
            Some("attempt-account".to_string()),
            Some("browser:visible-profile".to_string()),
            "visible account cue is not user-confirmed",
        )],
        ExpandedGuiTemplate::BrowserStaleNavigation => vec![observed(
            RequiredVerifier::BrowserPageVisible,
            VerifierAuthorityLevel::SurfaceAuthority,
            EvidenceFreshnessStatus::Stale,
            Some("attempt-browser".to_string()),
            Some("browser:https://example.com".to_string()),
            "browser page evidence predates the workflow attempt",
        )],
        ExpandedGuiTemplate::BrowserWrongTarget => vec![observed(
            RequiredVerifier::BrowserPageVisible,
            VerifierAuthorityLevel::SurfaceAuthority,
            EvidenceFreshnessStatus::Fresh,
            Some("attempt-browser".to_string()),
            None,
            "browser page visible but target identity is not confirmed",
        )],
        ExpandedGuiTemplate::MediaSearchNotPlayback => vec![observed(
            RequiredVerifier::BrowserPageVisible,
            VerifierAuthorityLevel::SurfaceAuthority,
            EvidenceFreshnessStatus::Fresh,
            Some("attempt-media".to_string()),
            Some("browser:youtube-search".to_string()),
            "search results page visible, playback not observed",
        )],
        ExpandedGuiTemplate::SpreadsheetWrongSheet => vec![observed(
            RequiredVerifier::AppContextVisible,
            VerifierAuthorityLevel::SurfaceAuthority,
            EvidenceFreshnessStatus::Fresh,
            Some("attempt-sheet".to_string()),
            None,
            "spreadsheet visible but sheet identity is unknown",
        )],
        ExpandedGuiTemplate::DocumentStaleContent => vec![observed(
            RequiredVerifier::DocumentContentVisible,
            VerifierAuthorityLevel::SurfaceAuthority,
            EvidenceFreshnessStatus::Stale,
            Some("attempt-doc".to_string()),
            Some("document:summary".to_string()),
            "document content evidence predates edit",
        )],
        ExpandedGuiTemplate::CommunicationStaleApproval => vec![observed(
            RequiredVerifier::UserConfirmation,
            VerifierAuthorityLevel::UserConfirmedAuthority,
            EvidenceFreshnessStatus::Stale,
            Some("attempt-email".to_string()),
            Some("draft:email".to_string()),
            "approval predates changed draft",
        )],
        ExpandedGuiTemplate::CommunicationMissingApproval => vec![observed(
            RequiredVerifier::HumanReviewPending,
            VerifierAuthorityLevel::SurfaceAuthority,
            EvidenceFreshnessStatus::Fresh,
            Some("attempt-email".to_string()),
            Some("draft:email".to_string()),
            "draft visible and pending approval",
        )],
        _ => return None,
    };

    Some(
        VerifierAuthorityEvaluator
            .evaluate_observed(requirements, &observed)
            .overall,
    )
}

fn hybrid_verdict_for_template(
    template: ExpandedGuiTemplate,
    assessment: &kria_core::agent::hybrid_synchronization::HybridSynchronizationAssessment,
) -> Option<HybridSynchronizationOverallVerdict> {
    let observation = match template {
        ExpandedGuiTemplate::IdeWrongVisibleFile => HybridSynchronizationObservation {
            kind: HybridSynchronizationCheckpointKind::VisibleArtifactSync,
            structural_identity: Some("file:expected.py".to_string()),
            visible_identity: Some("file:other.py".to_string()),
            action_started_unix_ms: Some(100),
            visible_observed_unix_ms: Some(200),
            evidence_summary: "IDE shows a different file than the generated artifact".to_string(),
            ..blank_sync_observation()
        },
        ExpandedGuiTemplate::IdeStaleBuffer => HybridSynchronizationObservation {
            kind: HybridSynchronizationCheckpointKind::FileHashSync,
            structural_hash: Some("current-structural-hash".to_string()),
            visible_hash: Some("old-visible-hash".to_string()),
            visible_open_hash: Some("old-visible-hash".to_string()),
            action_started_unix_ms: Some(100),
            visible_observed_unix_ms: Some(200),
            evidence_summary: "IDE buffer hash does not match latest structural content"
                .to_string(),
            ..blank_sync_observation()
        },
        ExpandedGuiTemplate::TerminalOldOutput => HybridSynchronizationObservation {
            kind: HybridSynchronizationCheckpointKind::TerminalExecutionFreshness,
            current_run_marker: Some("run-current".to_string()),
            observed_run_marker: Some("run-old".to_string()),
            action_started_unix_ms: Some(100),
            visible_observed_unix_ms: Some(200),
            evidence_summary: "terminal output marker belongs to an older command".to_string(),
            ..blank_sync_observation()
        },
        ExpandedGuiTemplate::BrowserStaleNavigation => HybridSynchronizationObservation {
            kind: HybridSynchronizationCheckpointKind::BrowserPageFreshness,
            structural_identity: Some("https://example.com".to_string()),
            visible_identity: Some("https://example.com".to_string()),
            action_started_unix_ms: Some(200),
            visible_observed_unix_ms: Some(250),
            browser_navigation_unix_ms: Some(100),
            evidence_summary: "browser navigation predates current workflow attempt".to_string(),
            ..blank_sync_observation()
        },
        ExpandedGuiTemplate::BrowserWrongTarget => HybridSynchronizationObservation {
            kind: HybridSynchronizationCheckpointKind::BrowserPageFreshness,
            structural_identity: Some("https://example.com".to_string()),
            visible_identity: Some("https://wrong.example".to_string()),
            action_started_unix_ms: Some(100),
            visible_observed_unix_ms: Some(200),
            browser_navigation_unix_ms: Some(200),
            evidence_summary: "browser visible target differs from requested URL".to_string(),
            ..blank_sync_observation()
        },
        _ => return None,
    };

    Some(
        HybridSynchronizationEvaluator
            .evaluate_observed(assessment, &[observation])
            .overall,
    )
}

fn observed(
    required_verifier: RequiredVerifier,
    authority_level: VerifierAuthorityLevel,
    freshness_status: EvidenceFreshnessStatus,
    workflow_attempt_id: Option<String>,
    target_identity: Option<String>,
    evidence_summary: &str,
) -> ObservedVerifierEvidence {
    ObservedVerifierEvidence {
        required_verifier,
        authority_level,
        evidence_time_unix_ms: Some(200),
        workflow_attempt_id,
        target_identity,
        freshness_status,
        confidence_tier: AuthorityConfidenceTier::Strong,
        evidence_summary: evidence_summary.to_string(),
    }
}

fn blank_sync_observation() -> HybridSynchronizationObservation {
    HybridSynchronizationObservation {
        checkpoint_id: None,
        kind: HybridSynchronizationCheckpointKind::VisibleArtifactSync,
        structural_identity: None,
        visible_identity: None,
        structural_hash: None,
        visible_hash: None,
        visible_open_hash: None,
        expected_workspace: None,
        observed_workspace: None,
        current_run_marker: None,
        observed_run_marker: None,
        expected_account_identity: None,
        observed_account_identity: None,
        action_started_unix_ms: None,
        visible_observed_unix_ms: None,
        browser_navigation_unix_ms: None,
        external_mutation_unix_ms: None,
        evidence_summary: String::new(),
    }
}

fn spec_for_template(template: ExpandedGuiTemplate) -> GuiTaskSpec {
    match template {
        ExpandedGuiTemplate::IdeWrongVisibleFile
        | ExpandedGuiTemplate::IdeStaleBuffer
        | ExpandedGuiTemplate::VisibleFallbackPolicy
        | ExpandedGuiTemplate::VisibleGuiUnavailable => spec(
            Verb::Open,
            vec![TargetRef::App("VS Code".to_string())],
            Some(ContentClass::Generated {
                hint: "python script".to_string(),
                language: Some("python".to_string()),
            }),
            Vec::new(),
        ),
        ExpandedGuiTemplate::DeveloperDeicticCode => spec(
            Verb::Open,
            vec![TargetRef::App("VS Code".to_string())],
            None,
            vec![Ambiguity::FileNotSpecified],
        ),
        ExpandedGuiTemplate::TerminalOldOutput | ExpandedGuiTemplate::TerminalOutputFileOnly => {
            spec(
                Verb::Open,
                vec![TargetRef::App("Terminal".to_string())],
                None,
                Vec::new(),
            )
        }
        ExpandedGuiTemplate::BrowserWrongAccount | ExpandedGuiTemplate::AmbiguousUploadIt => spec(
            Verb::Open,
            vec![TargetRef::App("Firefox".to_string())],
            None,
            vec![Ambiguity::FileNotSpecified],
        ),
        ExpandedGuiTemplate::BrowserStaleNavigation | ExpandedGuiTemplate::BrowserWrongTarget => {
            spec(
                Verb::Open,
                vec![TargetRef::Url("https://example.com".to_string())],
                None,
                Vec::new(),
            )
        }
        ExpandedGuiTemplate::MediaSearchNotPlayback => spec(
            Verb::Open,
            vec![TargetRef::App("YouTube".to_string())],
            None,
            Vec::new(),
        ),
        ExpandedGuiTemplate::SpreadsheetVisibleTotals
        | ExpandedGuiTemplate::SpreadsheetWrongSheet => spec(
            Verb::Open,
            vec![TargetRef::App("Excel".to_string())],
            Some(ContentClass::Generated {
                hint: "spreadsheet totals".to_string(),
                language: None,
            }),
            Vec::new(),
        ),
        ExpandedGuiTemplate::DocumentVisibleContent | ExpandedGuiTemplate::DocumentStaleContent => {
            spec(
                Verb::Open,
                vec![TargetRef::App("LibreOffice Writer".to_string())],
                Some(ContentClass::Generated {
                    hint: "document summary".to_string(),
                    language: None,
                }),
                Vec::new(),
            )
        }
        ExpandedGuiTemplate::CommunicationApproval
        | ExpandedGuiTemplate::CommunicationStaleApproval
        | ExpandedGuiTemplate::CommunicationMissingApproval => spec(
            Verb::Open,
            vec![TargetRef::App("Gmail".to_string())],
            Some(ContentClass::Generated {
                hint: "email draft".to_string(),
                language: None,
            }),
            Vec::new(),
        ),
    }
}

fn policy_for_template(template: ExpandedGuiTemplate) -> PolicyContext {
    if template == ExpandedGuiTemplate::VisibleFallbackPolicy {
        PolicyContext {
            allow_structural_execution: true,
            allow_visible_workflows: false,
            allow_human_collaboration: true,
        }
    } else {
        PolicyContext::default()
    }
}

fn environment_for_template(template: ExpandedGuiTemplate) -> EnvironmentCapabilities {
    if template == ExpandedGuiTemplate::VisibleGuiUnavailable {
        EnvironmentCapabilities {
            visible_gui: CapabilityAvailability::Unavailable,
            app_launch: CapabilityAvailability::Unavailable,
            structural_execution: CapabilityAvailability::Available,
            human_interaction: CapabilityAvailability::Unknown,
        }
    } else {
        EnvironmentCapabilities::unchecked_default()
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
    template: ExpandedGuiTemplate,
    dimensions: &[ExpandedGuiEvalDimension],
    capability_ids: &[&str],
    failure_modes: &[ExpandedGuiFailureMode],
    expectation: ExpandedGuiExpectation,
) -> ExpandedGuiEvalCase {
    ExpandedGuiEvalCase {
        id: id.to_string(),
        description: description.to_string(),
        prompt: prompt.to_string(),
        template,
        dimensions: dimensions.to_vec(),
        capability_ids: capability_ids
            .iter()
            .map(|value| (*value).to_string())
            .collect(),
        failure_modes: failure_modes.to_vec(),
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
) -> ExpandedGuiExpectation {
    ExpandedGuiExpectation {
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
        expected_trace_label: None,
        partial_completion_required: None,
    }
}

impl ExpandedGuiExpectation {
    fn with_governance_action(mut self, action: BrowserMediaGovernanceAction) -> Self {
        self.expected_governance_action = Some(action);
        self
    }

    fn with_forbidden_degradations(mut self, degradations: &[ForbiddenDegradation]) -> Self {
        self.expected_forbidden_degradations = degradations.to_vec();
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

    fn with_trace_label(mut self, label: &str) -> Self {
        self.expected_trace_label = Some(label.to_string());
        self
    }

    fn with_partial_completion_required(mut self, required: bool) -> Self {
        self.partial_completion_required = Some(required);
        self
    }
}

fn dimension_summaries(results: &[ExpandedGuiCaseResult]) -> Vec<ExpandedGuiDimensionSummary> {
    let mut counts = BTreeMap::<ExpandedGuiEvalDimension, (usize, usize)>::new();
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
        .map(|(dimension, (total, passed))| ExpandedGuiDimensionSummary {
            dimension,
            total,
            passed,
            failed: total.saturating_sub(passed),
        })
        .collect()
}

fn failure_mode_summaries(results: &[ExpandedGuiCaseResult]) -> Vec<ExpandedGuiFailureModeSummary> {
    let mut counts = BTreeMap::<ExpandedGuiFailureMode, (usize, usize)>::new();
    for result in results {
        for failure_mode in &result.case.failure_modes {
            let entry = counts.entry(*failure_mode).or_insert((0, 0));
            entry.0 += 1;
            if result.verdict.passed {
                entry.1 += 1;
            }
        }
    }
    counts
        .into_iter()
        .map(
            |(failure_mode, (total, passed))| ExpandedGuiFailureModeSummary {
                failure_mode,
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
    fn expanded_gui_suite_passes_all_new_cases() {
        let report = run_expanded_gui_eval_suite("expanded-test-run");

        assert_eq!(report.summary.total, 19);
        assert_eq!(report.summary.failed, 0);
        assert_eq!(report.summary.passed, report.summary.total);
    }

    #[test]
    fn expanded_suite_covers_targeted_failure_modes() {
        let report = run_expanded_gui_eval_suite("expanded-test-run");
        let failure_modes = report
            .summary
            .failure_mode_summaries
            .iter()
            .map(|summary| summary.failure_mode)
            .collect::<Vec<_>>();

        for expected in [
            ExpandedGuiFailureMode::WrongVisibleFile,
            ExpandedGuiFailureMode::StaleIdeBuffer,
            ExpandedGuiFailureMode::OldTerminalOutput,
            ExpandedGuiFailureMode::OutputFileOnly,
            ExpandedGuiFailureMode::WrongBrowserAccount,
            ExpandedGuiFailureMode::StaleBrowserNavigation,
            ExpandedGuiFailureMode::WrongBrowserTarget,
            ExpandedGuiFailureMode::SearchPageNotPlayback,
            ExpandedGuiFailureMode::WrongSpreadsheetSheet,
            ExpandedGuiFailureMode::StaleDocumentContent,
            ExpandedGuiFailureMode::StaleApproval,
            ExpandedGuiFailureMode::MissingApproval,
            ExpandedGuiFailureMode::VisibleFallbackDowngrade,
            ExpandedGuiFailureMode::VisibleGuiUnavailable,
            ExpandedGuiFailureMode::AmbiguousObjectReference,
            ExpandedGuiFailureMode::PromptMutation,
        ] {
            assert!(failure_modes.contains(&expected), "missing {expected:?}");
        }
    }

    #[test]
    fn expanded_cli_report_serializes() {
        let report = run_expanded_gui_eval_suite("expanded-test-run");
        let json = serde_json::to_string_pretty(&report).expect("serialize");
        let roundtrip: ExpandedGuiEvalReport = serde_json::from_str(&json).expect("deserialize");

        assert_eq!(roundtrip.summary.total, report.summary.total);
        assert!(json.contains("expanded_gui_evals_v1"));
    }

    #[test]
    fn expanded_markdown_contains_new_failure_modes() {
        let report = run_expanded_gui_eval_suite("expanded-test-run");
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("expanded_gui_evals.md");
        write_expanded_gui_eval_markdown(&report, &path).expect("write markdown");
        let markdown = std::fs::read_to_string(path).expect("read markdown");

        assert!(markdown.contains("wrong_visible_file"));
        assert!(markdown.contains("old_terminal_output"));
        assert!(markdown.contains("expanded-gui-019-developer-deictic-code"));
    }
}
