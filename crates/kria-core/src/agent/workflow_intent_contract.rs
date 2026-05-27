//! Declarative workflow intent contracts.
//!
//! Contracts define invariants, verifier requirements, fallback policy, and
//! completion response requirements. They do not infer meaning, choose tools,
//! generate plans, execute, verify, or recover.

use crate::agent::execution_mode_reasoner::{
    ExecutionMode, ExecutionModeDecision, RequiredVerifier, WorkflowContractId,
};
use crate::agent::semantic_workflow::{
    AppClass, FidelityDegradationPolicy, SemanticWorkflowAnalysis, TaskFamily, WorkflowFidelityTier,
};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VisibilityRequirement {
    None,
    ResultSurfaced,
    AppContextVisible,
    WorkflowStagesVisible,
    HumanReviewVisible,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StructuralActionPolicy {
    Allowed,
    AllowedAsInternalStep,
    RequiresVisibleContext,
    Forbidden,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ForbiddenDegradation {
    SilentVisibleToStructural,
    OutputFileOnlyWhenOutputRequested,
    AppOpenedAfterHiddenCompletion,
    AccountSessionAssumed,
    HumanReviewSkipped,
    WeakSurfaceClaimedAsSemanticTruth,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ContractRecoveryPolicy {
    ReportPartialAndPause,
    AskBeforeFallback,
    ExplainFallback,
    FailClosed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ContractHitlPolicy {
    None,
    OptionalReview,
    RequiredReview,
    RequiredApproval,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CompletionResponseRequirement {
    StateCompletedWork,
    StateIncompleteWork,
    SurfaceOutput,
    ListArtifacts,
    ExplainFallback,
    RequestUserDecision,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContractBoundaryRules {
    pub declarative_only: bool,
    pub may_infer_meaning: bool,
    pub may_generate_plan: bool,
    pub may_choose_tools: bool,
    pub may_execute: bool,
    pub may_verify_completion: bool,
    pub may_perform_recovery: bool,
}

impl ContractBoundaryRules {
    pub fn declarative() -> Self {
        Self {
            declarative_only: true,
            may_infer_meaning: false,
            may_generate_plan: false,
            may_choose_tools: false,
            may_execute: false,
            may_verify_completion: false,
            may_perform_recovery: false,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkflowIntentContract {
    pub contract_id: WorkflowContractId,
    pub task_family: Option<TaskFamily>,
    pub allowed_modes: Vec<ExecutionMode>,
    pub minimum_fidelity: WorkflowFidelityTier,
    pub required_app_classes: Vec<AppClass>,
    pub visibility_requirements: Vec<VisibilityRequirement>,
    pub structural_action_policy: StructuralActionPolicy,
    pub required_verifiers: Vec<RequiredVerifier>,
    pub forbidden_degradations: Vec<ForbiddenDegradation>,
    pub fallback_policy: FidelityDegradationPolicy,
    pub recovery_policy: ContractRecoveryPolicy,
    pub hitl_policy: ContractHitlPolicy,
    pub completion_response_requirements: Vec<CompletionResponseRequirement>,
    pub boundary_rules: ContractBoundaryRules,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContractRequirement {
    pub kind: ContractRequirementKind,
    pub label: String,
}

impl ContractRequirement {
    fn new(kind: ContractRequirementKind, label: impl Into<String>) -> Self {
        Self {
            kind,
            label: label.into(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ContractRequirementKind {
    ExecutionMode,
    Fidelity,
    AppClass,
    Verifier,
    Boundary,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContractEvaluationTrace {
    pub contract_found: bool,
    pub trace_labels: Vec<String>,
    pub explanation: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContractCheck {
    pub contract_id: WorkflowContractId,
    pub satisfied_requirements: Vec<ContractRequirement>,
    pub missing_requirements: Vec<ContractRequirement>,
    pub forbidden_degradations_triggered: Vec<ForbiddenDegradation>,
    pub verifier_requirements: Vec<RequiredVerifier>,
    pub fallback_policy: FidelityDegradationPolicy,
    pub hitl_policy: ContractHitlPolicy,
    pub trace: ContractEvaluationTrace,
}

#[derive(Debug, Default, Clone, Copy)]
pub struct WorkflowIntentContractRegistry;

impl WorkflowIntentContractRegistry {
    pub fn get(&self, contract_id: WorkflowContractId) -> Option<WorkflowIntentContract> {
        Some(match contract_id {
            WorkflowContractId::VisibleCodingWorkflow => visible_coding_contract(),
            WorkflowContractId::VisibleBrowserWorkflow => visible_browser_contract(),
            WorkflowContractId::SilentExecutionWorkflow => silent_execution_contract(),
            WorkflowContractId::VerificationVisibleWorkflow => verification_visible_contract(),
            WorkflowContractId::HumanReviewWorkflow => human_review_contract(),
            WorkflowContractId::IdeCollaborativeWorkflow => ide_collaborative_contract(),
            WorkflowContractId::MediaInteractionWorkflow => media_interaction_contract(),
            WorkflowContractId::GeneralVisibleWorkflow => general_visible_contract(),
        })
    }

    pub fn all(&self) -> Vec<WorkflowIntentContract> {
        [
            WorkflowContractId::VisibleCodingWorkflow,
            WorkflowContractId::VisibleBrowserWorkflow,
            WorkflowContractId::SilentExecutionWorkflow,
            WorkflowContractId::VerificationVisibleWorkflow,
            WorkflowContractId::HumanReviewWorkflow,
            WorkflowContractId::IdeCollaborativeWorkflow,
            WorkflowContractId::MediaInteractionWorkflow,
            WorkflowContractId::GeneralVisibleWorkflow,
        ]
        .into_iter()
        .filter_map(|id| self.get(id))
        .collect()
    }

    pub fn evaluate(
        &self,
        decision: &ExecutionModeDecision,
        analysis: &SemanticWorkflowAnalysis,
    ) -> ContractCheck {
        let Some(contract) = self.get(decision.workflow_contract_id) else {
            return ContractCheck {
                contract_id: decision.workflow_contract_id,
                satisfied_requirements: Vec::new(),
                missing_requirements: vec![ContractRequirement::new(
                    ContractRequirementKind::Boundary,
                    "selected contract is not registered",
                )],
                forbidden_degradations_triggered: Vec::new(),
                verifier_requirements: Vec::new(),
                fallback_policy: decision.fallback_policy,
                hitl_policy: ContractHitlPolicy::None,
                trace: ContractEvaluationTrace {
                    contract_found: false,
                    trace_labels: vec!["contract_missing".to_string()],
                    explanation: "declarative_phase_3_contract_check_only".to_string(),
                },
            };
        };

        let mut satisfied = Vec::new();
        let mut missing = Vec::new();
        let mut trace_labels = Vec::new();

        if contract.allowed_modes.contains(&decision.mode) {
            satisfied.push(ContractRequirement::new(
                ContractRequirementKind::ExecutionMode,
                format!("mode::{:?}", decision.mode),
            ));
        } else {
            missing.push(ContractRequirement::new(
                ContractRequirementKind::ExecutionMode,
                format!("mode::{:?}", decision.mode),
            ));
        }

        if analysis.fidelity.requested_fidelity >= contract.minimum_fidelity {
            satisfied.push(ContractRequirement::new(
                ContractRequirementKind::Fidelity,
                format!("minimum::{:?}", contract.minimum_fidelity),
            ));
        } else {
            missing.push(ContractRequirement::new(
                ContractRequirementKind::Fidelity,
                format!("minimum::{:?}", contract.minimum_fidelity),
            ));
        }

        for app_class in &contract.required_app_classes {
            if decision
                .app_anchor_decisions
                .iter()
                .any(|anchor| anchor.app_class == *app_class && anchor.required_for_completion)
            {
                satisfied.push(ContractRequirement::new(
                    ContractRequirementKind::AppClass,
                    format!("required_app::{:?}", app_class),
                ));
            } else {
                missing.push(ContractRequirement::new(
                    ContractRequirementKind::AppClass,
                    format!("required_app::{:?}", app_class),
                ));
            }
        }

        for verifier in &contract.required_verifiers {
            if decision.required_verifiers.contains(verifier) {
                satisfied.push(ContractRequirement::new(
                    ContractRequirementKind::Verifier,
                    format!("verifier::{:?}", verifier),
                ));
            } else {
                missing.push(ContractRequirement::new(
                    ContractRequirementKind::Verifier,
                    format!("verifier::{:?}", verifier),
                ));
            }
        }

        if contract.boundary_rules.declarative_only
            && !contract.boundary_rules.may_infer_meaning
            && !contract.boundary_rules.may_generate_plan
            && !contract.boundary_rules.may_choose_tools
            && !contract.boundary_rules.may_execute
            && !contract.boundary_rules.may_verify_completion
            && !contract.boundary_rules.may_perform_recovery
        {
            satisfied.push(ContractRequirement::new(
                ContractRequirementKind::Boundary,
                "contract_boundary::declarative_only",
            ));
        } else {
            missing.push(ContractRequirement::new(
                ContractRequirementKind::Boundary,
                "contract_boundary::declarative_only",
            ));
        }

        let forbidden_degradations_triggered =
            triggered_forbidden_degradations(&contract, decision, analysis);
        if missing.is_empty() {
            trace_labels.push("contract_requirements_satisfied".to_string());
        } else {
            trace_labels.push("contract_requirements_missing".to_string());
        }
        if !forbidden_degradations_triggered.is_empty() {
            trace_labels.push("forbidden_degradation_triggered".to_string());
        }
        let verifier_requirements = merged_verifier_requirements(
            &contract.required_verifiers,
            &decision.required_verifiers,
        );
        if verifier_requirements.len() > contract.required_verifiers.len() {
            trace_labels.push("decision_verifier_requirements_merged".to_string());
        }

        ContractCheck {
            contract_id: contract.contract_id,
            satisfied_requirements: satisfied,
            missing_requirements: missing,
            forbidden_degradations_triggered,
            verifier_requirements,
            fallback_policy: contract.fallback_policy,
            hitl_policy: contract.hitl_policy,
            trace: ContractEvaluationTrace {
                contract_found: true,
                trace_labels,
                explanation: "declarative_phase_3_contract_check_only".to_string(),
            },
        }
    }
}

fn triggered_forbidden_degradations(
    contract: &WorkflowIntentContract,
    decision: &ExecutionModeDecision,
    analysis: &SemanticWorkflowAnalysis,
) -> Vec<ForbiddenDegradation> {
    let mut triggered = Vec::new();
    if contract
        .forbidden_degradations
        .contains(&ForbiddenDegradation::SilentVisibleToStructural)
        && matches!(decision.mode, ExecutionMode::StructuralExecution)
        && analysis.fidelity.requested_fidelity >= WorkflowFidelityTier::AppContextFidelity
    {
        triggered.push(ForbiddenDegradation::SilentVisibleToStructural);
    }
    if contract
        .forbidden_degradations
        .contains(&ForbiddenDegradation::AccountSessionAssumed)
        && decision.clarification.required
        && analysis.frame.ambiguity_level
            == crate::agent::semantic_workflow::AmbiguitySeverity::AccountSession
        && !matches!(decision.mode, ExecutionMode::HumanCollaborativeWorkflow)
    {
        triggered.push(ForbiddenDegradation::AccountSessionAssumed);
    }
    if contract
        .forbidden_degradations
        .contains(&ForbiddenDegradation::HumanReviewSkipped)
        && matches!(
            contract.hitl_policy,
            ContractHitlPolicy::RequiredReview | ContractHitlPolicy::RequiredApproval
        )
        && !matches!(decision.mode, ExecutionMode::HumanCollaborativeWorkflow)
    {
        triggered.push(ForbiddenDegradation::HumanReviewSkipped);
    }
    triggered
}

fn merged_verifier_requirements(
    contract_requirements: &[RequiredVerifier],
    decision_requirements: &[RequiredVerifier],
) -> Vec<RequiredVerifier> {
    let mut merged = contract_requirements.to_vec();
    for verifier in decision_requirements {
        if !merged.contains(verifier) {
            merged.push(*verifier);
        }
    }
    merged
}

fn base_contract(
    contract_id: WorkflowContractId,
    task_family: Option<TaskFamily>,
    allowed_modes: Vec<ExecutionMode>,
    minimum_fidelity: WorkflowFidelityTier,
) -> WorkflowIntentContract {
    WorkflowIntentContract {
        contract_id,
        task_family,
        allowed_modes,
        minimum_fidelity,
        required_app_classes: Vec::new(),
        visibility_requirements: Vec::new(),
        structural_action_policy: StructuralActionPolicy::Allowed,
        required_verifiers: vec![RequiredVerifier::StructuralResult],
        forbidden_degradations: Vec::new(),
        fallback_policy: FidelityDegradationPolicy::ExplainFallback,
        recovery_policy: ContractRecoveryPolicy::ReportPartialAndPause,
        hitl_policy: ContractHitlPolicy::None,
        completion_response_requirements: vec![CompletionResponseRequirement::StateCompletedWork],
        boundary_rules: ContractBoundaryRules::declarative(),
    }
}

fn visible_coding_contract() -> WorkflowIntentContract {
    let mut contract = base_contract(
        WorkflowContractId::VisibleCodingWorkflow,
        Some(TaskFamily::Coding),
        vec![
            ExecutionMode::HybridWorkflow,
            ExecutionMode::VisibleAppWorkflow,
        ],
        WorkflowFidelityTier::WorkflowStageFidelity,
    );
    contract.required_app_classes = vec![AppClass::Ide];
    contract.visibility_requirements = vec![
        VisibilityRequirement::AppContextVisible,
        VisibilityRequirement::WorkflowStagesVisible,
        VisibilityRequirement::ResultSurfaced,
    ];
    contract.structural_action_policy = StructuralActionPolicy::AllowedAsInternalStep;
    contract.required_verifiers = vec![
        RequiredVerifier::StructuralResult,
        RequiredVerifier::AppContextVisible,
        RequiredVerifier::WorkflowStageVisible,
        RequiredVerifier::OutputSurfaced,
        RequiredVerifier::IdeFileVisible,
    ];
    contract.forbidden_degradations = vec![
        ForbiddenDegradation::SilentVisibleToStructural,
        ForbiddenDegradation::OutputFileOnlyWhenOutputRequested,
        ForbiddenDegradation::AppOpenedAfterHiddenCompletion,
    ];
    contract.completion_response_requirements = vec![
        CompletionResponseRequirement::StateCompletedWork,
        CompletionResponseRequirement::SurfaceOutput,
        CompletionResponseRequirement::ListArtifacts,
        CompletionResponseRequirement::ExplainFallback,
    ];
    contract
}

fn visible_browser_contract() -> WorkflowIntentContract {
    let mut contract = base_contract(
        WorkflowContractId::VisibleBrowserWorkflow,
        Some(TaskFamily::Browser),
        vec![
            ExecutionMode::VisibleAppWorkflow,
            ExecutionMode::HybridWorkflow,
        ],
        WorkflowFidelityTier::AppContextFidelity,
    );
    contract.required_app_classes = vec![AppClass::Browser];
    contract.visibility_requirements = vec![VisibilityRequirement::AppContextVisible];
    contract.structural_action_policy = StructuralActionPolicy::RequiresVisibleContext;
    contract.required_verifiers = vec![
        RequiredVerifier::StructuralResult,
        RequiredVerifier::AppContextVisible,
        RequiredVerifier::BrowserPageVisible,
    ];
    contract.forbidden_degradations = vec![
        ForbiddenDegradation::SilentVisibleToStructural,
        ForbiddenDegradation::AccountSessionAssumed,
    ];
    contract.completion_response_requirements = vec![
        CompletionResponseRequirement::StateCompletedWork,
        CompletionResponseRequirement::ExplainFallback,
    ];
    contract
}

fn silent_execution_contract() -> WorkflowIntentContract {
    let mut contract = base_contract(
        WorkflowContractId::SilentExecutionWorkflow,
        None,
        vec![
            ExecutionMode::StructuralExecution,
            ExecutionMode::SilentAutomationWorkflow,
        ],
        WorkflowFidelityTier::MinimalResultFidelity,
    );
    contract.fallback_policy = FidelityDegradationPolicy::SilentFallbackAllowed;
    contract.recovery_policy = ContractRecoveryPolicy::ExplainFallback;
    contract.completion_response_requirements = vec![
        CompletionResponseRequirement::StateCompletedWork,
        CompletionResponseRequirement::ListArtifacts,
    ];
    contract
}

fn verification_visible_contract() -> WorkflowIntentContract {
    let mut contract = base_contract(
        WorkflowContractId::VerificationVisibleWorkflow,
        None,
        vec![
            ExecutionMode::VerificationVisibleWorkflow,
            ExecutionMode::HybridWorkflow,
        ],
        WorkflowFidelityTier::WorkflowStageFidelity,
    );
    contract.visibility_requirements = vec![
        VisibilityRequirement::ResultSurfaced,
        VisibilityRequirement::WorkflowStagesVisible,
    ];
    contract.structural_action_policy = StructuralActionPolicy::AllowedAsInternalStep;
    contract.required_verifiers = vec![
        RequiredVerifier::StructuralResult,
        RequiredVerifier::WorkflowStageVisible,
        RequiredVerifier::OutputSurfaced,
    ];
    contract.forbidden_degradations = vec![
        ForbiddenDegradation::SilentVisibleToStructural,
        ForbiddenDegradation::OutputFileOnlyWhenOutputRequested,
    ];
    contract.completion_response_requirements = vec![
        CompletionResponseRequirement::StateCompletedWork,
        CompletionResponseRequirement::SurfaceOutput,
        CompletionResponseRequirement::ExplainFallback,
    ];
    contract
}

fn human_review_contract() -> WorkflowIntentContract {
    let mut contract = base_contract(
        WorkflowContractId::HumanReviewWorkflow,
        None,
        vec![ExecutionMode::HumanCollaborativeWorkflow],
        WorkflowFidelityTier::HumanObservedFidelity,
    );
    contract.visibility_requirements = vec![VisibilityRequirement::HumanReviewVisible];
    contract.structural_action_policy = StructuralActionPolicy::RequiresVisibleContext;
    contract.required_verifiers = vec![
        RequiredVerifier::StructuralResult,
        RequiredVerifier::HumanReviewPending,
        RequiredVerifier::UserConfirmation,
    ];
    contract.forbidden_degradations = vec![
        ForbiddenDegradation::AccountSessionAssumed,
        ForbiddenDegradation::HumanReviewSkipped,
    ];
    contract.fallback_policy = FidelityDegradationPolicy::AskBeforeFallback;
    contract.recovery_policy = ContractRecoveryPolicy::AskBeforeFallback;
    contract.hitl_policy = ContractHitlPolicy::RequiredReview;
    contract.completion_response_requirements = vec![
        CompletionResponseRequirement::StateCompletedWork,
        CompletionResponseRequirement::RequestUserDecision,
    ];
    contract
}

fn ide_collaborative_contract() -> WorkflowIntentContract {
    let mut contract = base_contract(
        WorkflowContractId::IdeCollaborativeWorkflow,
        Some(TaskFamily::Coding),
        vec![
            ExecutionMode::VisibleAppWorkflow,
            ExecutionMode::HybridWorkflow,
            ExecutionMode::HumanCollaborativeWorkflow,
        ],
        WorkflowFidelityTier::AppContextFidelity,
    );
    contract.required_app_classes = vec![AppClass::Ide];
    contract.visibility_requirements = vec![VisibilityRequirement::AppContextVisible];
    contract.structural_action_policy = StructuralActionPolicy::AllowedAsInternalStep;
    contract.required_verifiers = vec![
        RequiredVerifier::StructuralResult,
        RequiredVerifier::AppContextVisible,
        RequiredVerifier::IdeFileVisible,
    ];
    contract.forbidden_degradations = vec![ForbiddenDegradation::SilentVisibleToStructural];
    contract.hitl_policy = ContractHitlPolicy::OptionalReview;
    contract
}

fn media_interaction_contract() -> WorkflowIntentContract {
    let mut contract = base_contract(
        WorkflowContractId::MediaInteractionWorkflow,
        Some(TaskFamily::Media),
        vec![ExecutionMode::VisibleAppWorkflow],
        WorkflowFidelityTier::AppContextFidelity,
    );
    contract.required_app_classes = vec![AppClass::Media];
    contract.visibility_requirements = vec![VisibilityRequirement::AppContextVisible];
    contract.structural_action_policy = StructuralActionPolicy::RequiresVisibleContext;
    contract.required_verifiers = vec![
        RequiredVerifier::StructuralResult,
        RequiredVerifier::AppContextVisible,
        RequiredVerifier::MediaPlaybackVisible,
    ];
    contract.forbidden_degradations = vec![
        ForbiddenDegradation::SilentVisibleToStructural,
        ForbiddenDegradation::AccountSessionAssumed,
    ];
    contract
}

fn general_visible_contract() -> WorkflowIntentContract {
    let mut contract = base_contract(
        WorkflowContractId::GeneralVisibleWorkflow,
        None,
        vec![
            ExecutionMode::VisibleAppWorkflow,
            ExecutionMode::HybridWorkflow,
        ],
        WorkflowFidelityTier::AppContextFidelity,
    );
    contract.visibility_requirements = vec![VisibilityRequirement::AppContextVisible];
    contract.structural_action_policy = StructuralActionPolicy::RequiresVisibleContext;
    contract.required_verifiers = vec![
        RequiredVerifier::StructuralResult,
        RequiredVerifier::AppContextVisible,
    ];
    contract.forbidden_degradations = vec![ForbiddenDegradation::SilentVisibleToStructural];
    contract
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::execution_mode_reasoner::{
        EnvironmentCapabilities, ExecutionModeReasoner, PolicyContext,
    };
    use crate::agent::intent_compiler::{ContentClass, GuiTaskSpec, TargetRef, Verb};
    use crate::agent::semantic_workflow::analyze_semantic_workflow;

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

    fn decision_and_analysis(
        spec: &GuiTaskSpec,
        prompt: &str,
    ) -> (
        crate::agent::execution_mode_reasoner::ExecutionModeDecision,
        SemanticWorkflowAnalysis,
    ) {
        let analysis = analyze_semantic_workflow(spec, prompt);
        let decision = ExecutionModeReasoner.decide(
            spec,
            &analysis,
            &EnvironmentCapabilities::unchecked_default(),
            &PolicyContext::default(),
        );
        (decision, analysis)
    }

    #[test]
    fn registry_contains_all_phase_two_contract_ids() {
        let registry = WorkflowIntentContractRegistry;

        for id in [
            WorkflowContractId::VisibleCodingWorkflow,
            WorkflowContractId::VisibleBrowserWorkflow,
            WorkflowContractId::SilentExecutionWorkflow,
            WorkflowContractId::VerificationVisibleWorkflow,
            WorkflowContractId::HumanReviewWorkflow,
            WorkflowContractId::IdeCollaborativeWorkflow,
            WorkflowContractId::MediaInteractionWorkflow,
            WorkflowContractId::GeneralVisibleWorkflow,
        ] {
            assert!(registry.get(id).is_some(), "{id:?} should be registered");
        }
    }

    #[test]
    fn all_contracts_are_declarative_only() {
        let registry = WorkflowIntentContractRegistry;

        for contract in registry.all() {
            assert!(contract.boundary_rules.declarative_only);
            assert!(!contract.boundary_rules.may_infer_meaning);
            assert!(!contract.boundary_rules.may_generate_plan);
            assert!(!contract.boundary_rules.may_choose_tools);
            assert!(!contract.boundary_rules.may_execute);
            assert!(!contract.boundary_rules.may_verify_completion);
            assert!(!contract.boundary_rules.may_perform_recovery);
        }
    }

    #[test]
    fn visible_coding_contract_is_satisfied_by_hybrid_coding_decision() {
        let spec = spec(
            Verb::Open,
            vec![TargetRef::App("VS Code".to_string())],
            Some(ContentClass::Generated {
                hint: "pascal triangle program".to_string(),
                language: Some("python".to_string()),
            }),
        );
        let (decision, analysis) = decision_and_analysis(
            &spec,
            "open code and write a program to print pascal triangle and run it and show output",
        );

        let check = WorkflowIntentContractRegistry.evaluate(&decision, &analysis);

        assert_eq!(check.contract_id, WorkflowContractId::VisibleCodingWorkflow);
        assert!(check.missing_requirements.is_empty());
        assert!(check.forbidden_degradations_triggered.is_empty());
        assert!(check
            .verifier_requirements
            .contains(&RequiredVerifier::OutputSurfaced));
    }

    #[test]
    fn silent_contract_is_satisfied_by_structural_decision() {
        let spec = spec(
            Verb::Run,
            Vec::new(),
            Some(ContentClass::Generated {
                hint: "python program".to_string(),
                language: Some("python".to_string()),
            }),
        );
        let (decision, analysis) =
            decision_and_analysis(&spec, "write a python program that prints hello");

        let check = WorkflowIntentContractRegistry.evaluate(&decision, &analysis);

        assert_eq!(
            check.contract_id,
            WorkflowContractId::SilentExecutionWorkflow
        );
        assert!(check.missing_requirements.is_empty());
        assert_eq!(
            check.fallback_policy,
            FidelityDegradationPolicy::SilentFallbackAllowed
        );
    }

    #[test]
    fn human_review_contract_is_satisfied_by_account_workflow_decision() {
        let spec = spec(
            Verb::Open,
            vec![TargetRef::App("Firefox".to_string())],
            None,
        );
        let (decision, analysis) = decision_and_analysis(
            &spec,
            "open browser and login to my account and upload this file",
        );

        let check = WorkflowIntentContractRegistry.evaluate(&decision, &analysis);

        assert_eq!(check.contract_id, WorkflowContractId::HumanReviewWorkflow);
        assert!(check.missing_requirements.is_empty());
        assert_eq!(check.hitl_policy, ContractHitlPolicy::RequiredReview);
        assert!(check
            .verifier_requirements
            .contains(&RequiredVerifier::BrowserAccountContext));
    }

    #[test]
    fn human_review_contract_preserves_personal_media_verifier_requirements() {
        let spec = spec(
            Verb::Open,
            vec![TargetRef::App("YouTube".to_string())],
            None,
        );
        let (decision, analysis) =
            decision_and_analysis(&spec, "open youtube and play my playlist");

        let check = WorkflowIntentContractRegistry.evaluate(&decision, &analysis);

        assert_eq!(check.contract_id, WorkflowContractId::HumanReviewWorkflow);
        assert!(check.missing_requirements.is_empty());
        assert!(check
            .verifier_requirements
            .contains(&RequiredVerifier::MediaPlaybackVisible));
        assert!(check
            .verifier_requirements
            .contains(&RequiredVerifier::BrowserAccountContext));
    }

    #[test]
    fn contract_check_serializes_to_json() {
        let spec = spec(
            Verb::Open,
            vec![TargetRef::App("Firefox".to_string())],
            None,
        );
        let (decision, analysis) = decision_and_analysis(&spec, "open firefox and show me website");
        let check = WorkflowIntentContractRegistry.evaluate(&decision, &analysis);

        let json = serde_json::to_string(&check).expect("contract check is serializable");
        let roundtrip: ContractCheck =
            serde_json::from_str(&json).expect("contract check is deserializable");

        assert_eq!(roundtrip, check);
    }
}
