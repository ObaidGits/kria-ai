//! Verifier authority and freshness metadata for GUI workflow intelligence.
//!
//! This module does not perform live verification. It defines claim boundaries
//! and freshness requirements, then maps Phase 3 contract verifier requirements
//! into explicit authority requirements. Runtime integration is trace-only until
//! concrete verifier paths consume these structures.

use crate::agent::execution_mode_reasoner::{
    ExecutionModeDecision, RequiredVerifier, WorkflowContractId,
};
use crate::agent::semantic_workflow::{SemanticWorkflowAnalysis, VisibilityExpectation};
use crate::agent::workflow_intent_contract::ContractCheck;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VerifierAuthorityLevel {
    StructuralAuthority,
    SurfaceAuthority,
    SemanticAuthority,
    UserConfirmedAuthority,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VerifierClaim {
    StructuralTruth,
    SurfaceVisible,
    SemanticCorrectness,
    HumanConfirmation,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EvidenceFreshnessStatus {
    Fresh,
    Stale,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EvidenceFreshnessPolicy {
    FreshRequired,
    FreshOrUnknownAccepted,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AuthorityConfidenceTier {
    Strong,
    Partial,
    Weak,
    Unavailable,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ForbiddenVerifierAssumption {
    WindowFocusMeansContent,
    AppLaunchedMeansWorkflowComplete,
    BrowserOpenedMeansAuthenticated,
    TerminalVisibleMeansLatestOutput,
    FileExistsMeansIdeBufferFresh,
    OcrTextMeansSemanticSuccess,
    LlmSummaryMeansVerifiedCompletion,
    OutputFileMeansShown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VisibleVerifierKind {
    IdeFileVisible,
    IdeWorkspaceVisible,
    TerminalOutputVisible,
    BrowserPageVisible,
    BrowserAccountContext,
    DocumentContentVisible,
    MediaPlaybackVisible,
    HumanReviewPending,
    OutputSurfaced,
    AppContextVisible,
    StructuralResult,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VerifierAuthorityRequirement {
    pub required_verifier: RequiredVerifier,
    pub visible_verifier_kind: VisibleVerifierKind,
    pub minimum_authority: VerifierAuthorityLevel,
    pub freshness_policy: EvidenceFreshnessPolicy,
    pub workflow_attempt_id_required: bool,
    pub target_identity_required: bool,
    pub forbidden_assumptions: Vec<ForbiddenVerifierAssumption>,
    pub allowed_claims: Vec<VerifierClaim>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ObservedVerifierEvidence {
    pub required_verifier: RequiredVerifier,
    pub authority_level: VerifierAuthorityLevel,
    pub evidence_time_unix_ms: Option<i64>,
    pub workflow_attempt_id: Option<String>,
    pub target_identity: Option<String>,
    pub freshness_status: EvidenceFreshnessStatus,
    pub confidence_tier: AuthorityConfidenceTier,
    pub evidence_summary: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RequirementEvidenceStatus {
    Pending,
    Satisfied,
    Stale,
    InsufficientAuthority,
    UnsupportedClaim,
    WorkflowAttemptMissing,
    TargetIdentityMissing,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VerifierRequirementStatus {
    pub requirement: VerifierAuthorityRequirement,
    pub status: RequirementEvidenceStatus,
    pub evidence_summary: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VerifierAuthorityOverallVerdict {
    Pending,
    Satisfied,
    Partial,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VerifierAuthorityTrace {
    pub workflow_attempt_id: String,
    pub target_identity: String,
    pub trace_labels: Vec<String>,
    pub explanation: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VerifierAuthorityAssessment {
    pub contract_id: WorkflowContractId,
    pub requirements: Vec<VerifierAuthorityRequirement>,
    pub partial_completion_required: bool,
    pub output_file_only_forbidden: bool,
    pub weak_surface_claims_semantic_truth_forbidden: bool,
    pub trace: VerifierAuthorityTrace,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VerifierAuthorityObservedVerdict {
    pub overall: VerifierAuthorityOverallVerdict,
    pub statuses: Vec<VerifierRequirementStatus>,
    pub stale_evidence_count: usize,
    pub insufficient_authority_count: usize,
    pub pending_count: usize,
}

#[derive(Debug, Default, Clone, Copy)]
pub struct VerifierAuthorityEvaluator;

impl VerifierAuthorityEvaluator {
    pub fn assess(
        &self,
        contract_check: &ContractCheck,
        decision: &ExecutionModeDecision,
        analysis: &SemanticWorkflowAnalysis,
        workflow_attempt_id: impl Into<String>,
    ) -> VerifierAuthorityAssessment {
        let requirements = contract_check
            .verifier_requirements
            .iter()
            .copied()
            .map(requirement_for_verifier)
            .collect::<Vec<_>>();
        let partial_completion_required = !contract_check.missing_requirements.is_empty()
            || !contract_check.forbidden_degradations_triggered.is_empty();
        let output_file_only_forbidden = requirements
            .iter()
            .any(|requirement| requirement.required_verifier == RequiredVerifier::OutputSurfaced);
        let target_identity = target_identity(decision, analysis);
        let mut trace_labels = Vec::new();
        trace_labels.push(format!("contract::{:?}", contract_check.contract_id));
        trace_labels.push(format!(
            "requested_fidelity::{:?}",
            analysis.fidelity.requested_fidelity
        ));
        if output_file_only_forbidden {
            trace_labels.push("output_file_only_forbidden".to_string());
        }
        if matches!(
            analysis.frame.visibility_expectation,
            VisibilityExpectation::AppVisible
                | VisibilityExpectation::WorkflowVisible
                | VisibilityExpectation::HumanObserved
        ) {
            trace_labels.push("visible_evidence_required".to_string());
        }

        VerifierAuthorityAssessment {
            contract_id: contract_check.contract_id,
            requirements,
            partial_completion_required,
            output_file_only_forbidden,
            weak_surface_claims_semantic_truth_forbidden: true,
            trace: VerifierAuthorityTrace {
                workflow_attempt_id: workflow_attempt_id.into(),
                target_identity,
                trace_labels,
                explanation: "phase_4_verifier_authority_requirements_only".to_string(),
            },
        }
    }

    pub fn evaluate_observed(
        &self,
        requirements: &[VerifierAuthorityRequirement],
        observed: &[ObservedVerifierEvidence],
    ) -> VerifierAuthorityObservedVerdict {
        let statuses = requirements
            .iter()
            .cloned()
            .map(|requirement| evaluate_requirement(requirement, observed))
            .collect::<Vec<_>>();
        let stale_evidence_count = statuses
            .iter()
            .filter(|status| status.status == RequirementEvidenceStatus::Stale)
            .count();
        let insufficient_authority_count = statuses
            .iter()
            .filter(|status| status.status == RequirementEvidenceStatus::InsufficientAuthority)
            .count();
        let pending_count = statuses
            .iter()
            .filter(|status| status.status == RequirementEvidenceStatus::Pending)
            .count();
        let target_identity_missing = statuses
            .iter()
            .any(|status| status.status == RequirementEvidenceStatus::TargetIdentityMissing);

        let workflow_attempt_missing = statuses
            .iter()
            .any(|status| status.status == RequirementEvidenceStatus::WorkflowAttemptMissing);

        let overall = if statuses
            .iter()
            .all(|status| status.status == RequirementEvidenceStatus::Satisfied)
        {
            VerifierAuthorityOverallVerdict::Satisfied
        } else if stale_evidence_count > 0
            || insufficient_authority_count > 0
            || target_identity_missing
            || workflow_attempt_missing
        {
            VerifierAuthorityOverallVerdict::Partial
        } else if pending_count > 0 {
            VerifierAuthorityOverallVerdict::Pending
        } else {
            VerifierAuthorityOverallVerdict::Failed
        };

        VerifierAuthorityObservedVerdict {
            overall,
            statuses,
            stale_evidence_count,
            insufficient_authority_count,
            pending_count,
        }
    }
}

pub fn claim_allowed(authority: VerifierAuthorityLevel, claim: VerifierClaim) -> bool {
    match (authority, claim) {
        (VerifierAuthorityLevel::StructuralAuthority, VerifierClaim::StructuralTruth) => true,
        (VerifierAuthorityLevel::SurfaceAuthority, VerifierClaim::SurfaceVisible) => true,
        (VerifierAuthorityLevel::SemanticAuthority, VerifierClaim::StructuralTruth)
        | (VerifierAuthorityLevel::SemanticAuthority, VerifierClaim::SurfaceVisible)
        | (VerifierAuthorityLevel::SemanticAuthority, VerifierClaim::SemanticCorrectness) => true,
        (VerifierAuthorityLevel::UserConfirmedAuthority, VerifierClaim::HumanConfirmation) => true,
        _ => false,
    }
}

fn evaluate_requirement(
    requirement: VerifierAuthorityRequirement,
    observed: &[ObservedVerifierEvidence],
) -> VerifierRequirementStatus {
    let Some(evidence) = observed
        .iter()
        .find(|evidence| evidence.required_verifier == requirement.required_verifier)
    else {
        return VerifierRequirementStatus {
            requirement,
            status: RequirementEvidenceStatus::Pending,
            evidence_summary: None,
        };
    };

    if requirement.freshness_policy == EvidenceFreshnessPolicy::FreshRequired
        && evidence.freshness_status != EvidenceFreshnessStatus::Fresh
    {
        return VerifierRequirementStatus {
            requirement,
            status: RequirementEvidenceStatus::Stale,
            evidence_summary: Some(evidence.evidence_summary.clone()),
        };
    }
    if requirement.workflow_attempt_id_required && evidence.workflow_attempt_id.is_none() {
        return VerifierRequirementStatus {
            requirement,
            status: RequirementEvidenceStatus::WorkflowAttemptMissing,
            evidence_summary: Some(evidence.evidence_summary.clone()),
        };
    }
    if requirement.target_identity_required && evidence.target_identity.is_none() {
        return VerifierRequirementStatus {
            requirement,
            status: RequirementEvidenceStatus::TargetIdentityMissing,
            evidence_summary: Some(evidence.evidence_summary.clone()),
        };
    }
    if evidence.authority_level < requirement.minimum_authority {
        return VerifierRequirementStatus {
            requirement,
            status: RequirementEvidenceStatus::InsufficientAuthority,
            evidence_summary: Some(evidence.evidence_summary.clone()),
        };
    }
    if !requirement
        .allowed_claims
        .iter()
        .all(|claim| claim_allowed(evidence.authority_level, *claim))
    {
        return VerifierRequirementStatus {
            requirement,
            status: RequirementEvidenceStatus::UnsupportedClaim,
            evidence_summary: Some(evidence.evidence_summary.clone()),
        };
    }

    VerifierRequirementStatus {
        requirement,
        status: RequirementEvidenceStatus::Satisfied,
        evidence_summary: Some(evidence.evidence_summary.clone()),
    }
}

fn requirement_for_verifier(verifier: RequiredVerifier) -> VerifierAuthorityRequirement {
    match verifier {
        RequiredVerifier::StructuralResult => VerifierAuthorityRequirement {
            required_verifier: verifier,
            visible_verifier_kind: VisibleVerifierKind::StructuralResult,
            minimum_authority: VerifierAuthorityLevel::StructuralAuthority,
            freshness_policy: EvidenceFreshnessPolicy::FreshOrUnknownAccepted,
            workflow_attempt_id_required: true,
            target_identity_required: false,
            forbidden_assumptions: vec![
                ForbiddenVerifierAssumption::LlmSummaryMeansVerifiedCompletion,
            ],
            allowed_claims: vec![VerifierClaim::StructuralTruth],
        },
        RequiredVerifier::AppContextVisible => surface_requirement(
            verifier,
            VisibleVerifierKind::AppContextVisible,
            vec![
                ForbiddenVerifierAssumption::WindowFocusMeansContent,
                ForbiddenVerifierAssumption::AppLaunchedMeansWorkflowComplete,
            ],
        ),
        RequiredVerifier::WorkflowStageVisible => surface_requirement(
            verifier,
            VisibleVerifierKind::AppContextVisible,
            vec![ForbiddenVerifierAssumption::AppLaunchedMeansWorkflowComplete],
        ),
        RequiredVerifier::OutputSurfaced => surface_requirement(
            verifier,
            VisibleVerifierKind::OutputSurfaced,
            vec![ForbiddenVerifierAssumption::OutputFileMeansShown],
        ),
        RequiredVerifier::IdeFileVisible => surface_requirement(
            verifier,
            VisibleVerifierKind::IdeFileVisible,
            vec![ForbiddenVerifierAssumption::FileExistsMeansIdeBufferFresh],
        ),
        RequiredVerifier::IdeWorkspaceVisible => surface_requirement(
            verifier,
            VisibleVerifierKind::IdeWorkspaceVisible,
            vec![ForbiddenVerifierAssumption::FileExistsMeansIdeBufferFresh],
        ),
        RequiredVerifier::TerminalOutputVisible => surface_requirement(
            verifier,
            VisibleVerifierKind::TerminalOutputVisible,
            vec![ForbiddenVerifierAssumption::TerminalVisibleMeansLatestOutput],
        ),
        RequiredVerifier::BrowserPageVisible => surface_requirement(
            verifier,
            VisibleVerifierKind::BrowserPageVisible,
            vec![
                ForbiddenVerifierAssumption::WindowFocusMeansContent,
                ForbiddenVerifierAssumption::BrowserOpenedMeansAuthenticated,
            ],
        ),
        RequiredVerifier::BrowserAccountContext => VerifierAuthorityRequirement {
            required_verifier: verifier,
            visible_verifier_kind: VisibleVerifierKind::BrowserAccountContext,
            minimum_authority: VerifierAuthorityLevel::UserConfirmedAuthority,
            freshness_policy: EvidenceFreshnessPolicy::FreshRequired,
            workflow_attempt_id_required: true,
            target_identity_required: true,
            forbidden_assumptions: vec![
                ForbiddenVerifierAssumption::BrowserOpenedMeansAuthenticated,
            ],
            allowed_claims: vec![VerifierClaim::HumanConfirmation],
        },
        RequiredVerifier::DocumentContentVisible => surface_requirement(
            verifier,
            VisibleVerifierKind::DocumentContentVisible,
            vec![
                ForbiddenVerifierAssumption::WindowFocusMeansContent,
                ForbiddenVerifierAssumption::OcrTextMeansSemanticSuccess,
            ],
        ),
        RequiredVerifier::MediaPlaybackVisible => surface_requirement(
            verifier,
            VisibleVerifierKind::MediaPlaybackVisible,
            vec![ForbiddenVerifierAssumption::WindowFocusMeansContent],
        ),
        RequiredVerifier::HumanReviewPending => VerifierAuthorityRequirement {
            required_verifier: verifier,
            visible_verifier_kind: VisibleVerifierKind::HumanReviewPending,
            minimum_authority: VerifierAuthorityLevel::SurfaceAuthority,
            freshness_policy: EvidenceFreshnessPolicy::FreshRequired,
            workflow_attempt_id_required: true,
            target_identity_required: true,
            forbidden_assumptions: vec![
                ForbiddenVerifierAssumption::AppLaunchedMeansWorkflowComplete,
            ],
            allowed_claims: vec![VerifierClaim::SurfaceVisible],
        },
        RequiredVerifier::UserConfirmation => VerifierAuthorityRequirement {
            required_verifier: verifier,
            visible_verifier_kind: VisibleVerifierKind::HumanReviewPending,
            minimum_authority: VerifierAuthorityLevel::UserConfirmedAuthority,
            freshness_policy: EvidenceFreshnessPolicy::FreshRequired,
            workflow_attempt_id_required: true,
            target_identity_required: true,
            forbidden_assumptions: Vec::new(),
            allowed_claims: vec![VerifierClaim::HumanConfirmation],
        },
    }
}

fn surface_requirement(
    verifier: RequiredVerifier,
    visible_kind: VisibleVerifierKind,
    forbidden_assumptions: Vec<ForbiddenVerifierAssumption>,
) -> VerifierAuthorityRequirement {
    VerifierAuthorityRequirement {
        required_verifier: verifier,
        visible_verifier_kind: visible_kind,
        minimum_authority: VerifierAuthorityLevel::SurfaceAuthority,
        freshness_policy: EvidenceFreshnessPolicy::FreshRequired,
        workflow_attempt_id_required: true,
        target_identity_required: true,
        forbidden_assumptions,
        allowed_claims: vec![VerifierClaim::SurfaceVisible],
    }
}

fn target_identity(
    decision: &ExecutionModeDecision,
    analysis: &SemanticWorkflowAnalysis,
) -> String {
    if let Some(anchor) = decision.app_anchor_decisions.first() {
        return format!("{:?}:{}", anchor.app_class, anchor.label);
    }
    format!(
        "{:?}:{:?}",
        analysis.frame.task_family, analysis.fidelity.requested_fidelity
    )
}

pub fn freshness_from_times(
    evidence_time_unix_ms: Option<i64>,
    action_started_unix_ms: i64,
) -> EvidenceFreshnessStatus {
    match evidence_time_unix_ms {
        Some(time) if time >= action_started_unix_ms => EvidenceFreshnessStatus::Fresh,
        Some(_) => EvidenceFreshnessStatus::Stale,
        None => EvidenceFreshnessStatus::Unknown,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::execution_mode_reasoner::{
        EnvironmentCapabilities, ExecutionModeReasoner, PolicyContext,
    };
    use crate::agent::intent_compiler::{ContentClass, GuiTaskSpec, TargetRef, Verb};
    use crate::agent::semantic_workflow::analyze_semantic_workflow;
    use crate::agent::workflow_intent_contract::WorkflowIntentContractRegistry;

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

    fn assessment_for_prompt(prompt: &str) -> VerifierAuthorityAssessment {
        let spec = spec(
            Verb::Open,
            vec![TargetRef::App("VS Code".to_string())],
            Some(ContentClass::Generated {
                hint: "pascal triangle program".to_string(),
                language: Some("python".to_string()),
            }),
        );
        let analysis = analyze_semantic_workflow(&spec, prompt);
        let decision = ExecutionModeReasoner.decide(
            &spec,
            &analysis,
            &EnvironmentCapabilities::unchecked_default(),
            &PolicyContext::default(),
        );
        let contract_check = WorkflowIntentContractRegistry.evaluate(&decision, &analysis);
        VerifierAuthorityEvaluator.assess(&contract_check, &decision, &analysis, "attempt-test")
    }

    #[test]
    fn surface_authority_cannot_claim_semantic_truth() {
        assert!(!claim_allowed(
            VerifierAuthorityLevel::SurfaceAuthority,
            VerifierClaim::SemanticCorrectness
        ));
        assert!(claim_allowed(
            VerifierAuthorityLevel::SurfaceAuthority,
            VerifierClaim::SurfaceVisible
        ));
    }

    #[test]
    fn visible_coding_requires_output_surfaced_and_ide_visible_authority() {
        let assessment = assessment_for_prompt(
            "open code and write a program to print pascal triangle and run it and show output",
        );

        assert!(assessment.output_file_only_forbidden);
        assert!(assessment.requirements.iter().any(|requirement| {
            requirement.required_verifier == RequiredVerifier::OutputSurfaced
                && requirement
                    .forbidden_assumptions
                    .contains(&ForbiddenVerifierAssumption::OutputFileMeansShown)
        }));
        assert!(assessment.requirements.iter().any(|requirement| {
            requirement.required_verifier == RequiredVerifier::IdeFileVisible
                && requirement.minimum_authority == VerifierAuthorityLevel::SurfaceAuthority
        }));
    }

    #[test]
    fn output_file_only_does_not_satisfy_output_surfaced_requirement() {
        let assessment = assessment_for_prompt(
            "open code and write a program to print pascal triangle and run it and show output",
        );
        let observed = vec![ObservedVerifierEvidence {
            required_verifier: RequiredVerifier::StructuralResult,
            authority_level: VerifierAuthorityLevel::StructuralAuthority,
            evidence_time_unix_ms: Some(200),
            workflow_attempt_id: Some("attempt-test".to_string()),
            target_identity: Some("file".to_string()),
            freshness_status: EvidenceFreshnessStatus::Fresh,
            confidence_tier: AuthorityConfidenceTier::Strong,
            evidence_summary: "output file exists".to_string(),
        }];

        let verdict =
            VerifierAuthorityEvaluator.evaluate_observed(&assessment.requirements, &observed);

        assert_ne!(verdict.overall, VerifierAuthorityOverallVerdict::Satisfied);
        assert!(verdict.statuses.iter().any(|status| {
            status.requirement.required_verifier == RequiredVerifier::OutputSurfaced
                && status.status == RequirementEvidenceStatus::Pending
        }));
    }

    #[test]
    fn stale_visible_evidence_is_not_satisfied() {
        let requirement = requirement_for_verifier(RequiredVerifier::TerminalOutputVisible);
        let observed = vec![ObservedVerifierEvidence {
            required_verifier: RequiredVerifier::TerminalOutputVisible,
            authority_level: VerifierAuthorityLevel::SurfaceAuthority,
            evidence_time_unix_ms: Some(100),
            workflow_attempt_id: Some("attempt-test".to_string()),
            target_identity: Some("terminal".to_string()),
            freshness_status: EvidenceFreshnessStatus::Stale,
            confidence_tier: AuthorityConfidenceTier::Strong,
            evidence_summary: "terminal output from old run".to_string(),
        }];

        let verdict = VerifierAuthorityEvaluator.evaluate_observed(&[requirement], &observed);

        assert_eq!(verdict.stale_evidence_count, 1);
        assert_eq!(verdict.overall, VerifierAuthorityOverallVerdict::Partial);
    }

    #[test]
    fn document_content_visible_has_surface_claim_boundary() {
        let requirement = requirement_for_verifier(RequiredVerifier::DocumentContentVisible);

        assert_eq!(
            requirement.visible_verifier_kind,
            VisibleVerifierKind::DocumentContentVisible
        );
        assert_eq!(
            requirement.minimum_authority,
            VerifierAuthorityLevel::SurfaceAuthority
        );
        assert!(requirement
            .forbidden_assumptions
            .contains(&ForbiddenVerifierAssumption::OcrTextMeansSemanticSuccess));
    }

    #[test]
    fn user_confirmation_cannot_satisfy_structural_truth() {
        let requirement = requirement_for_verifier(RequiredVerifier::StructuralResult);
        let observed = vec![ObservedVerifierEvidence {
            required_verifier: RequiredVerifier::StructuralResult,
            authority_level: VerifierAuthorityLevel::UserConfirmedAuthority,
            evidence_time_unix_ms: Some(200),
            workflow_attempt_id: Some("attempt-test".to_string()),
            target_identity: None,
            freshness_status: EvidenceFreshnessStatus::Fresh,
            confidence_tier: AuthorityConfidenceTier::Strong,
            evidence_summary: "user said the file exists".to_string(),
        }];

        let verdict = VerifierAuthorityEvaluator.evaluate_observed(&[requirement], &observed);

        assert!(verdict.statuses.iter().any(|status| {
            status.requirement.required_verifier == RequiredVerifier::StructuralResult
                && status.status == RequirementEvidenceStatus::UnsupportedClaim
        }));
        assert_ne!(verdict.overall, VerifierAuthorityOverallVerdict::Satisfied);
    }

    #[test]
    fn workflow_attempt_id_is_required_for_visible_evidence() {
        let requirement = requirement_for_verifier(RequiredVerifier::BrowserPageVisible);
        let observed = vec![ObservedVerifierEvidence {
            required_verifier: RequiredVerifier::BrowserPageVisible,
            authority_level: VerifierAuthorityLevel::SurfaceAuthority,
            evidence_time_unix_ms: Some(200),
            workflow_attempt_id: None,
            target_identity: Some("browser:example".to_string()),
            freshness_status: EvidenceFreshnessStatus::Fresh,
            confidence_tier: AuthorityConfidenceTier::Strong,
            evidence_summary: "browser shows expected page".to_string(),
        }];

        let verdict = VerifierAuthorityEvaluator.evaluate_observed(&[requirement], &observed);

        assert!(verdict.statuses.iter().any(|status| {
            status.requirement.required_verifier == RequiredVerifier::BrowserPageVisible
                && status.status == RequirementEvidenceStatus::WorkflowAttemptMissing
        }));
        assert_ne!(verdict.overall, VerifierAuthorityOverallVerdict::Satisfied);
    }

    #[test]
    fn browser_account_context_requires_user_confirmed_authority() {
        let requirement = requirement_for_verifier(RequiredVerifier::BrowserAccountContext);

        assert_eq!(
            requirement.visible_verifier_kind,
            VisibleVerifierKind::BrowserAccountContext
        );
        assert_eq!(
            requirement.minimum_authority,
            VerifierAuthorityLevel::UserConfirmedAuthority
        );
        assert!(requirement
            .forbidden_assumptions
            .contains(&ForbiddenVerifierAssumption::BrowserOpenedMeansAuthenticated));
    }

    #[test]
    fn personal_media_assessment_carries_media_and_account_authority_requirements() {
        let spec = spec(
            Verb::Open,
            vec![TargetRef::App("YouTube".to_string())],
            None,
        );
        let analysis = analyze_semantic_workflow(&spec, "open youtube and play my playlist");
        let decision = ExecutionModeReasoner.decide(
            &spec,
            &analysis,
            &EnvironmentCapabilities::unchecked_default(),
            &PolicyContext::default(),
        );
        let contract_check = WorkflowIntentContractRegistry.evaluate(&decision, &analysis);
        let assessment =
            VerifierAuthorityEvaluator.assess(&contract_check, &decision, &analysis, "attempt");

        assert!(assessment.requirements.iter().any(|requirement| {
            requirement.required_verifier == RequiredVerifier::MediaPlaybackVisible
        }));
        assert!(assessment.requirements.iter().any(|requirement| {
            requirement.required_verifier == RequiredVerifier::BrowserAccountContext
                && requirement.minimum_authority == VerifierAuthorityLevel::UserConfirmedAuthority
        }));
    }

    #[test]
    fn freshness_from_times_rejects_evidence_before_action() {
        assert_eq!(
            freshness_from_times(Some(99), 100),
            EvidenceFreshnessStatus::Stale
        );
        assert_eq!(
            freshness_from_times(Some(100), 100),
            EvidenceFreshnessStatus::Fresh
        );
        assert_eq!(
            freshness_from_times(None, 100),
            EvidenceFreshnessStatus::Unknown
        );
    }

    #[test]
    fn assessment_serializes_to_json() {
        let assessment = assessment_for_prompt(
            "open code and write a program to print pascal triangle and run it and show output",
        );
        let json = serde_json::to_string(&assessment).expect("assessment is serializable");
        let roundtrip: VerifierAuthorityAssessment =
            serde_json::from_str(&json).expect("assessment is deserializable");

        assert_eq!(roundtrip, assessment);
    }
}
