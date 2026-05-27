//! Deterministic execution-mode selection for semantic GUI workflows.
//!
//! This module owns mode selection only. It does not generate tool steps,
//! inspect live desktop state, verify completion, perform recovery, or call an
//! LLM. Phase 2 wires this as trace-only metadata so existing execution remains
//! stable while downstream phases learn to consume the decision.

use crate::agent::intent_compiler::{ContentClass, GuiTaskSpec, Verb};
use crate::agent::semantic_workflow::{
    AmbiguitySeverity, AppAnchor, AppAnchorStrength, AppClass, CollaborationRequirement,
    FidelityDegradationPolicy, SemanticWorkflowAnalysis, TaskFamily, VisibilityExpectation,
    WorkflowFidelityTier, WorkflowSafetyClass,
};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExecutionMode {
    StructuralExecution,
    VisibleAppWorkflow,
    HybridWorkflow,
    HumanCollaborativeWorkflow,
    VerificationVisibleWorkflow,
    SilentAutomationWorkflow,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkflowContractId {
    VisibleCodingWorkflow,
    VisibleBrowserWorkflow,
    SilentExecutionWorkflow,
    VerificationVisibleWorkflow,
    HumanReviewWorkflow,
    IdeCollaborativeWorkflow,
    MediaInteractionWorkflow,
    GeneralVisibleWorkflow,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StructuralAcceptability {
    Allowed,
    AllowedAsInternalStep,
    RequiresVisibleContext,
    NotAcceptable,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RequiredVerifier {
    StructuralResult,
    AppContextVisible,
    WorkflowStageVisible,
    OutputSurfaced,
    IdeFileVisible,
    IdeWorkspaceVisible,
    TerminalOutputVisible,
    BrowserPageVisible,
    BrowserAccountContext,
    DocumentContentVisible,
    MediaPlaybackVisible,
    HumanReviewPending,
    UserConfirmation,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CapabilityAvailability {
    Available,
    Unavailable,
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EnvironmentCapabilities {
    pub visible_gui: CapabilityAvailability,
    pub app_launch: CapabilityAvailability,
    pub structural_execution: CapabilityAvailability,
    pub human_interaction: CapabilityAvailability,
}

impl EnvironmentCapabilities {
    pub fn unchecked_default() -> Self {
        Self {
            visible_gui: CapabilityAvailability::Unknown,
            app_launch: CapabilityAvailability::Unknown,
            structural_execution: CapabilityAvailability::Available,
            human_interaction: CapabilityAvailability::Unknown,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PolicyContext {
    pub allow_structural_execution: bool,
    pub allow_visible_workflows: bool,
    pub allow_human_collaboration: bool,
}

impl Default for PolicyContext {
    fn default() -> Self {
        Self {
            allow_structural_execution: true,
            allow_visible_workflows: true,
            allow_human_collaboration: true,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AppAnchorDecision {
    pub label: String,
    pub app_class: AppClass,
    pub anchor_strength: AppAnchorStrength,
    pub required_for_completion: bool,
    pub requires_visible_context: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClarificationDecision {
    pub required: bool,
    pub ambiguity: AmbiguitySeverity,
    pub reason: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuntimeTruthOwnership {
    pub workflow_semantics_owner: String,
    pub execution_style_owner: String,
    pub invariants_owner: String,
    pub execution_steps_owner: String,
    pub runtime_state_owner: String,
    pub completion_truth_owner: String,
    pub recovery_state_owner: String,
}

impl RuntimeTruthOwnership {
    pub fn canonical() -> Self {
        Self {
            workflow_semantics_owner: "SemanticWorkflowFrame".to_string(),
            execution_style_owner: "ExecutionModeDecision".to_string(),
            invariants_owner: "WorkflowIntentContract".to_string(),
            execution_steps_owner: "SubstratePlanner".to_string(),
            runtime_state_owner: "WorkflowExecutor".to_string(),
            completion_truth_owner: "Verifier".to_string(),
            recovery_state_owner: "RecoveryManager".to_string(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExecutionModeTrace {
    pub rule_id: String,
    pub reason_labels: Vec<String>,
    pub explanation: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExecutionModeDecision {
    pub mode: ExecutionMode,
    pub workflow_contract_id: WorkflowContractId,
    pub visibility_level: VisibilityExpectation,
    pub app_anchor_decisions: Vec<AppAnchorDecision>,
    pub structural_acceptability: StructuralAcceptability,
    pub required_verifiers: Vec<RequiredVerifier>,
    pub clarification: ClarificationDecision,
    pub fallback_policy: FidelityDegradationPolicy,
    pub ownership: RuntimeTruthOwnership,
    pub trace: ExecutionModeTrace,
}

#[derive(Debug, Default, Clone, Copy)]
pub struct ExecutionModeReasoner;

impl ExecutionModeReasoner {
    pub fn decide(
        &self,
        spec: &GuiTaskSpec,
        analysis: &SemanticWorkflowAnalysis,
        environment: &EnvironmentCapabilities,
        policy: &PolicyContext,
    ) -> ExecutionModeDecision {
        let frame = &analysis.frame;
        let fidelity = &analysis.fidelity;
        let app_anchor_decisions = frame
            .app_anchors
            .iter()
            .map(|anchor| app_anchor_decision(anchor, frame.visibility_expectation))
            .collect::<Vec<_>>();
        let clarification = clarification_decision(frame.ambiguity_level, frame.safety_class);
        let structural_acceptability =
            structural_acceptability(frame, fidelity.requested_fidelity, policy);
        let required_verifiers =
            required_verifiers(frame, fidelity.requested_fidelity, &app_anchor_decisions);
        let mut reason_labels = Vec::new();

        let (mut mode, contract, rule_id) = select_mode_and_contract(
            spec,
            analysis,
            &app_anchor_decisions,
            &clarification,
            &mut reason_labels,
        );

        if !policy.allow_visible_workflows
            && matches!(
                mode,
                ExecutionMode::VisibleAppWorkflow
                    | ExecutionMode::HybridWorkflow
                    | ExecutionMode::VerificationVisibleWorkflow
            )
        {
            reason_labels.push("policy_blocks_visible_workflow".to_string());
            mode = ExecutionMode::StructuralExecution;
        }
        if !policy.allow_human_collaboration
            && matches!(mode, ExecutionMode::HumanCollaborativeWorkflow)
        {
            reason_labels.push("policy_blocks_human_collaboration".to_string());
            mode = ExecutionMode::VerificationVisibleWorkflow;
        }
        if visible_mode(mode) && environment.visible_gui == CapabilityAvailability::Unavailable {
            reason_labels.push("environment_visible_gui_unavailable".to_string());
        }
        if matches!(mode, ExecutionMode::StructuralExecution)
            && environment.structural_execution == CapabilityAvailability::Unavailable
        {
            reason_labels.push("environment_structural_execution_unavailable".to_string());
        }

        ExecutionModeDecision {
            mode,
            workflow_contract_id: contract,
            visibility_level: frame.visibility_expectation,
            app_anchor_decisions,
            structural_acceptability,
            required_verifiers,
            clarification,
            fallback_policy: fidelity.degradation_policy,
            ownership: RuntimeTruthOwnership::canonical(),
            trace: ExecutionModeTrace {
                rule_id,
                reason_labels,
                explanation: "deterministic_phase_2_mode_selection_only".to_string(),
            },
        }
    }
}

fn select_mode_and_contract(
    spec: &GuiTaskSpec,
    analysis: &SemanticWorkflowAnalysis,
    app_anchor_decisions: &[AppAnchorDecision],
    clarification: &ClarificationDecision,
    reason_labels: &mut Vec<String>,
) -> (ExecutionMode, WorkflowContractId, String) {
    let frame = &analysis.frame;
    let requested_fidelity = analysis.fidelity.requested_fidelity;

    if clarification.required
        && matches!(
            frame.ambiguity_level,
            AmbiguitySeverity::AccountSession | AmbiguitySeverity::Destructive
        )
    {
        reason_labels.push("critical_or_account_ambiguity_requires_human".to_string());
        return (
            ExecutionMode::HumanCollaborativeWorkflow,
            WorkflowContractId::HumanReviewWorkflow,
            "human_observed_ambiguity".to_string(),
        );
    }

    if matches!(
        frame.collaboration_requirement,
        CollaborationRequirement::Required
    ) || requested_fidelity == WorkflowFidelityTier::HumanObservedFidelity
    {
        reason_labels.push("human_observed_fidelity_or_collaboration".to_string());
        return (
            ExecutionMode::HumanCollaborativeWorkflow,
            WorkflowContractId::HumanReviewWorkflow,
            "human_collaborative_requirement".to_string(),
        );
    }

    if frame.task_family == TaskFamily::Media {
        reason_labels.push("media_requires_visible_surface".to_string());
        return (
            ExecutionMode::VisibleAppWorkflow,
            WorkflowContractId::MediaInteractionWorkflow,
            "media_visible_workflow".to_string(),
        );
    }

    if frame.task_family == TaskFamily::Browser
        && !matches!(frame.visibility_expectation, VisibilityExpectation::None)
    {
        reason_labels.push("browser_visible_context".to_string());
        return (
            ExecutionMode::VisibleAppWorkflow,
            WorkflowContractId::VisibleBrowserWorkflow,
            "browser_visible_workflow".to_string(),
        );
    }

    let required_app_anchor = app_anchor_decisions
        .iter()
        .any(|anchor| anchor.required_for_completion);
    let required_ide_anchor = app_anchor_decisions
        .iter()
        .any(|anchor| anchor.required_for_completion && anchor.app_class == AppClass::Ide);
    let run_or_show_intent = contains_generated_runnable_content(spec)
        || requested_fidelity >= WorkflowFidelityTier::WorkflowStageFidelity;

    if required_ide_anchor && run_or_show_intent {
        reason_labels.push("required_ide_anchor_with_run_or_show_intent".to_string());
        return (
            ExecutionMode::HybridWorkflow,
            WorkflowContractId::VisibleCodingWorkflow,
            "required_ide_hybrid_coding".to_string(),
        );
    }

    if required_app_anchor && requested_fidelity >= WorkflowFidelityTier::WorkflowStageFidelity {
        reason_labels.push("required_app_anchor_with_workflow_stage_fidelity".to_string());
        return (
            ExecutionMode::HybridWorkflow,
            contract_for_task_family(frame.task_family, true),
            "required_app_hybrid_workflow".to_string(),
        );
    }

    if required_app_anchor {
        reason_labels.push("required_app_anchor_visible_context".to_string());
        return (
            ExecutionMode::VisibleAppWorkflow,
            contract_for_task_family(frame.task_family, false),
            "required_app_visible_workflow".to_string(),
        );
    }

    if requested_fidelity >= WorkflowFidelityTier::WorkflowStageFidelity
        || matches!(
            frame.visibility_expectation,
            VisibilityExpectation::ResultVisible | VisibilityExpectation::WorkflowVisible
        )
    {
        reason_labels.push("visible_result_or_workflow_stage_required".to_string());
        return (
            ExecutionMode::VerificationVisibleWorkflow,
            WorkflowContractId::VerificationVisibleWorkflow,
            "verification_visible_result".to_string(),
        );
    }

    if matches!(spec.primary_verb, Verb::Other(_))
        && frame.visibility_expectation == VisibilityExpectation::None
        && frame.app_anchors.is_empty()
    {
        reason_labels.push("silent_background_style_no_visible_anchor".to_string());
        return (
            ExecutionMode::SilentAutomationWorkflow,
            WorkflowContractId::SilentExecutionWorkflow,
            "silent_automation_no_visible_anchor".to_string(),
        );
    }

    reason_labels.push("minimal_structural_fidelity".to_string());
    (
        ExecutionMode::StructuralExecution,
        WorkflowContractId::SilentExecutionWorkflow,
        "structural_minimal_result".to_string(),
    )
}

fn app_anchor_decision(anchor: &AppAnchor, visibility: VisibilityExpectation) -> AppAnchorDecision {
    let required_for_completion = matches!(
        anchor.strength,
        AppAnchorStrength::Required | AppAnchorStrength::SafetyCredential
    );
    let requires_visible_context = required_for_completion
        || matches!(
            visibility,
            VisibilityExpectation::AppVisible
                | VisibilityExpectation::WorkflowVisible
                | VisibilityExpectation::HumanObserved
        );
    AppAnchorDecision {
        label: anchor.label.clone(),
        app_class: anchor.app_class,
        anchor_strength: anchor.strength,
        required_for_completion,
        requires_visible_context,
    }
}

fn clarification_decision(
    ambiguity: AmbiguitySeverity,
    safety: WorkflowSafetyClass,
) -> ClarificationDecision {
    let required = matches!(
        ambiguity,
        AmbiguitySeverity::Identity
            | AmbiguitySeverity::AccountSession
            | AmbiguitySeverity::Destructive
    ) || matches!(
        safety,
        WorkflowSafetyClass::ReviewRequired | WorkflowSafetyClass::DestructiveOrExternal
    );
    let reason = if required {
        Some(match ambiguity {
            AmbiguitySeverity::Identity => "target identity ambiguity".to_string(),
            AmbiguitySeverity::AccountSession => "account or session ambiguity".to_string(),
            AmbiguitySeverity::Destructive => {
                "destructive or external side-effect ambiguity".to_string()
            }
            _ => "safety review required".to_string(),
        })
    } else {
        None
    };
    ClarificationDecision {
        required,
        ambiguity,
        reason,
    }
}

fn structural_acceptability(
    frame: &crate::agent::semantic_workflow::SemanticWorkflowFrame,
    requested_fidelity: WorkflowFidelityTier,
    policy: &PolicyContext,
) -> StructuralAcceptability {
    if !policy.allow_structural_execution {
        return StructuralAcceptability::NotAcceptable;
    }
    if requested_fidelity == WorkflowFidelityTier::MinimalResultFidelity
        && frame.app_anchors.is_empty()
        && !matches!(
            frame.visibility_expectation,
            VisibilityExpectation::ResultVisible
                | VisibilityExpectation::AppVisible
                | VisibilityExpectation::WorkflowVisible
                | VisibilityExpectation::HumanObserved
        )
    {
        StructuralAcceptability::Allowed
    } else if requested_fidelity >= WorkflowFidelityTier::WorkflowStageFidelity
        || !frame.app_anchors.is_empty()
    {
        StructuralAcceptability::AllowedAsInternalStep
    } else {
        StructuralAcceptability::RequiresVisibleContext
    }
}

fn required_verifiers(
    frame: &crate::agent::semantic_workflow::SemanticWorkflowFrame,
    requested_fidelity: WorkflowFidelityTier,
    app_anchor_decisions: &[AppAnchorDecision],
) -> Vec<RequiredVerifier> {
    let mut verifiers = vec![RequiredVerifier::StructuralResult];

    if requested_fidelity >= WorkflowFidelityTier::AppContextFidelity {
        push_unique(&mut verifiers, RequiredVerifier::AppContextVisible);
    }
    if requested_fidelity >= WorkflowFidelityTier::WorkflowStageFidelity {
        push_unique(&mut verifiers, RequiredVerifier::WorkflowStageVisible);
        push_unique(&mut verifiers, RequiredVerifier::OutputSurfaced);
    }
    if requested_fidelity >= WorkflowFidelityTier::HumanObservedFidelity {
        push_unique(&mut verifiers, RequiredVerifier::HumanReviewPending);
        push_unique(&mut verifiers, RequiredVerifier::UserConfirmation);
    }

    for anchor in app_anchor_decisions {
        match anchor.app_class {
            AppClass::Ide => {
                push_unique(&mut verifiers, RequiredVerifier::IdeFileVisible);
                if requested_fidelity >= WorkflowFidelityTier::WorkflowStageFidelity {
                    push_unique(&mut verifiers, RequiredVerifier::IdeWorkspaceVisible);
                }
            }
            AppClass::Terminal => {
                if requested_fidelity >= WorkflowFidelityTier::WorkflowStageFidelity {
                    push_unique(&mut verifiers, RequiredVerifier::TerminalOutputVisible);
                }
            }
            AppClass::Browser => {
                push_unique(&mut verifiers, RequiredVerifier::BrowserPageVisible);
                if matches!(frame.ambiguity_level, AmbiguitySeverity::AccountSession) {
                    push_unique(&mut verifiers, RequiredVerifier::BrowserAccountContext);
                }
            }
            AppClass::Media => {
                push_unique(&mut verifiers, RequiredVerifier::MediaPlaybackVisible);
                if matches!(frame.ambiguity_level, AmbiguitySeverity::AccountSession) {
                    push_unique(&mut verifiers, RequiredVerifier::BrowserAccountContext);
                }
            }
            AppClass::DocumentEditor => {
                push_unique(&mut verifiers, RequiredVerifier::DocumentContentVisible);
            }
            AppClass::Spreadsheet
            | AppClass::Communication
            | AppClass::FileManager
            | AppClass::Unknown => {}
        }
    }

    if matches!(frame.ambiguity_level, AmbiguitySeverity::AccountSession)
        && matches!(frame.task_family, TaskFamily::Browser | TaskFamily::Media)
    {
        push_unique(&mut verifiers, RequiredVerifier::BrowserAccountContext);
    }
    if frame.task_family == TaskFamily::Browser {
        push_unique(&mut verifiers, RequiredVerifier::BrowserPageVisible);
    }
    if frame.task_family == TaskFamily::Media {
        push_unique(&mut verifiers, RequiredVerifier::MediaPlaybackVisible);
    }

    verifiers
}

fn contract_for_task_family(task_family: TaskFamily, hybrid: bool) -> WorkflowContractId {
    match task_family {
        TaskFamily::Coding if hybrid => WorkflowContractId::VisibleCodingWorkflow,
        TaskFamily::Coding => WorkflowContractId::IdeCollaborativeWorkflow,
        TaskFamily::Browser => WorkflowContractId::VisibleBrowserWorkflow,
        TaskFamily::Media => WorkflowContractId::MediaInteractionWorkflow,
        TaskFamily::Communication => WorkflowContractId::HumanReviewWorkflow,
        _ => WorkflowContractId::GeneralVisibleWorkflow,
    }
}

fn contains_generated_runnable_content(spec: &GuiTaskSpec) -> bool {
    matches!(
        spec.content.as_ref(),
        Some(ContentClass::Generated {
            language: Some(_),
            ..
        })
    )
}

fn visible_mode(mode: ExecutionMode) -> bool {
    matches!(
        mode,
        ExecutionMode::VisibleAppWorkflow
            | ExecutionMode::HybridWorkflow
            | ExecutionMode::HumanCollaborativeWorkflow
            | ExecutionMode::VerificationVisibleWorkflow
    )
}

fn push_unique(verifiers: &mut Vec<RequiredVerifier>, verifier: RequiredVerifier) {
    if !verifiers.contains(&verifier) {
        verifiers.push(verifier);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::intent_compiler::{ContentClass, TargetRef};

    fn spec(
        primary_verb: Verb,
        targets: Vec<TargetRef>,
        content: Option<ContentClass>,
    ) -> GuiTaskSpec {
        GuiTaskSpec {
            primary_verb,
            targets,
            content,
            declared_preconditions: Vec::new(),
            declared_success_criteria: Vec::new(),
            ambiguities: Vec::new(),
        }
    }

    fn decide(spec: &GuiTaskSpec, prompt: &str) -> ExecutionModeDecision {
        let analysis = crate::agent::semantic_workflow::analyze_semantic_workflow(spec, prompt);
        ExecutionModeReasoner.decide(
            spec,
            &analysis,
            &EnvironmentCapabilities::unchecked_default(),
            &PolicyContext::default(),
        )
    }

    #[test]
    fn open_code_run_show_output_selects_hybrid_coding() {
        let spec = spec(
            Verb::Open,
            vec![TargetRef::App("VS Code".to_string())],
            Some(ContentClass::Generated {
                hint: "pascal triangle program".to_string(),
                language: Some("python".to_string()),
            }),
        );

        let decision = decide(
            &spec,
            "open code and write a program to print pascal triangle and run it and show output",
        );

        assert_eq!(decision.mode, ExecutionMode::HybridWorkflow);
        assert_eq!(
            decision.workflow_contract_id,
            WorkflowContractId::VisibleCodingWorkflow
        );
        assert_eq!(
            decision.structural_acceptability,
            StructuralAcceptability::AllowedAsInternalStep
        );
        assert!(decision
            .required_verifiers
            .contains(&RequiredVerifier::OutputSurfaced));
    }

    #[test]
    fn simple_program_generation_selects_structural_execution() {
        let spec = spec(
            Verb::Run,
            Vec::new(),
            Some(ContentClass::Generated {
                hint: "python program".to_string(),
                language: Some("python".to_string()),
            }),
        );

        let decision = decide(&spec, "write a python program that prints hello");

        assert_eq!(decision.mode, ExecutionMode::StructuralExecution);
        assert_eq!(
            decision.workflow_contract_id,
            WorkflowContractId::SilentExecutionWorkflow
        );
        assert_eq!(
            decision.structural_acceptability,
            StructuralAcceptability::Allowed
        );
    }

    #[test]
    fn browser_account_workflow_requires_human_collaboration() {
        let spec = spec(
            Verb::Open,
            vec![TargetRef::App("Firefox".to_string())],
            None,
        );

        let decision = decide(
            &spec,
            "open browser and login to my account and upload this file",
        );

        assert_eq!(decision.mode, ExecutionMode::HumanCollaborativeWorkflow);
        assert_eq!(
            decision.workflow_contract_id,
            WorkflowContractId::HumanReviewWorkflow
        );
        assert!(decision.clarification.required);
        assert!(decision
            .required_verifiers
            .contains(&RequiredVerifier::UserConfirmation));
    }

    #[test]
    fn visible_browser_prompt_selects_visible_browser_workflow() {
        let spec = spec(
            Verb::Open,
            vec![TargetRef::App("Firefox".to_string())],
            None,
        );

        let decision = decide(&spec, "open firefox and show me the website");

        assert_eq!(decision.mode, ExecutionMode::VisibleAppWorkflow);
        assert_eq!(
            decision.workflow_contract_id,
            WorkflowContractId::VisibleBrowserWorkflow
        );
        assert!(decision
            .required_verifiers
            .contains(&RequiredVerifier::BrowserPageVisible));
    }

    #[test]
    fn personal_media_prompt_requires_human_collaboration_and_account_context() {
        let spec = spec(
            Verb::Open,
            vec![TargetRef::App("YouTube".to_string())],
            None,
        );

        let decision = decide(&spec, "open youtube and play my playlist");

        assert_eq!(decision.mode, ExecutionMode::HumanCollaborativeWorkflow);
        assert_eq!(
            decision.workflow_contract_id,
            WorkflowContractId::HumanReviewWorkflow
        );
        assert!(decision
            .required_verifiers
            .contains(&RequiredVerifier::MediaPlaybackVisible));
        assert!(decision
            .required_verifiers
            .contains(&RequiredVerifier::BrowserAccountContext));
        assert!(decision
            .required_verifiers
            .contains(&RequiredVerifier::HumanReviewPending));
    }

    #[test]
    fn decision_serializes_to_json() {
        let spec = spec(
            Verb::Open,
            vec![TargetRef::App("Terminal".to_string())],
            None,
        );
        let decision = decide(&spec, "open terminal and run df -h and show output");
        let json = serde_json::to_string(&decision).expect("decision is serializable");
        let roundtrip: ExecutionModeDecision =
            serde_json::from_str(&json).expect("decision is deserializable");

        assert_eq!(roundtrip, decision);
    }
}
