//! Hybrid workflow synchronization checkpoints.
//!
//! This module owns structural-visible reconciliation metadata only. It does
//! not plan tools, execute GUI actions, verify semantic completion, or perform
//! recovery. Phase 5 wires it as trace-only runtime metadata so hybrid
//! workflows can later reject stale/divergent visible state deterministically.

use crate::agent::execution_mode_reasoner::{
    ExecutionMode, ExecutionModeDecision, RequiredVerifier, WorkflowContractId,
};
use crate::agent::semantic_workflow::SemanticWorkflowAnalysis;
use crate::agent::verifier_authority::VerifierAuthorityAssessment;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HybridSynchronizationCheckpointKind {
    FileHashSync,
    WorkspaceIdentitySync,
    TerminalExecutionFreshness,
    BrowserPageFreshness,
    AccountSessionSync,
    VisibleArtifactSync,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SynchronizationInvalidationReason {
    MissingStructuralState,
    MissingVisibleState,
    FileHashChangedAfterVisibleOpen,
    FileHashMismatch,
    WorkspaceIdentityMismatch,
    TerminalOutputMissingCurrentRunMarker,
    BrowserNavigationPredatesWorkflowAttempt,
    BrowserTargetIdentityMismatch,
    AccountSessionMismatch,
    VisibleArtifactMismatch,
    ExternalMutationAfterObservation,
    EvidencePredatesWorkflowAttempt,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SynchronizationCheckpointStatus {
    Pending,
    Synchronized,
    Invalidated,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HybridSynchronizationOverallVerdict {
    NotRequired,
    Pending,
    Synchronized,
    Invalidated,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HybridSynchronizationCheckpoint {
    pub checkpoint_id: String,
    pub kind: HybridSynchronizationCheckpointKind,
    pub required_verifiers: Vec<RequiredVerifier>,
    pub requires_structural_state: bool,
    pub requires_visible_state: bool,
    pub requires_freshness_marker: bool,
    pub invalidation_reasons: Vec<SynchronizationInvalidationReason>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HybridSynchronizationTrace {
    pub workflow_attempt_id: String,
    pub contract_id: WorkflowContractId,
    pub trace_labels: Vec<String>,
    pub explanation: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HybridSynchronizationAssessment {
    pub required: bool,
    pub checkpoints: Vec<HybridSynchronizationCheckpoint>,
    pub trace: HybridSynchronizationTrace,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HybridSynchronizationObservation {
    pub checkpoint_id: Option<String>,
    pub kind: HybridSynchronizationCheckpointKind,
    pub structural_identity: Option<String>,
    pub visible_identity: Option<String>,
    pub structural_hash: Option<String>,
    pub visible_hash: Option<String>,
    pub visible_open_hash: Option<String>,
    pub expected_workspace: Option<String>,
    pub observed_workspace: Option<String>,
    pub current_run_marker: Option<String>,
    pub observed_run_marker: Option<String>,
    pub expected_account_identity: Option<String>,
    pub observed_account_identity: Option<String>,
    pub action_started_unix_ms: Option<i64>,
    pub visible_observed_unix_ms: Option<i64>,
    pub browser_navigation_unix_ms: Option<i64>,
    pub external_mutation_unix_ms: Option<i64>,
    pub evidence_summary: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HybridSynchronizationCheckpointVerdict {
    pub checkpoint: HybridSynchronizationCheckpoint,
    pub status: SynchronizationCheckpointStatus,
    pub invalidation_reason: Option<SynchronizationInvalidationReason>,
    pub evidence_summary: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HybridSynchronizationVerdict {
    pub overall: HybridSynchronizationOverallVerdict,
    pub statuses: Vec<HybridSynchronizationCheckpointVerdict>,
    pub synchronized_count: usize,
    pub invalidated_count: usize,
    pub pending_count: usize,
}

#[derive(Debug, Default, Clone, Copy)]
pub struct HybridSynchronizationEvaluator;

impl HybridSynchronizationEvaluator {
    pub fn assess(
        &self,
        decision: &ExecutionModeDecision,
        _analysis: &SemanticWorkflowAnalysis,
        verifier_authority: &VerifierAuthorityAssessment,
        workflow_attempt_id: impl Into<String>,
    ) -> HybridSynchronizationAssessment {
        let workflow_attempt_id = workflow_attempt_id.into();
        let required = matches!(decision.mode, ExecutionMode::HybridWorkflow);
        let mut trace_labels = vec![format!("mode::{:?}", decision.mode)];

        if !required {
            trace_labels.push("hybrid_sync_not_required".to_string());
            return HybridSynchronizationAssessment {
                required,
                checkpoints: Vec::new(),
                trace: HybridSynchronizationTrace {
                    workflow_attempt_id,
                    contract_id: decision.workflow_contract_id,
                    trace_labels,
                    explanation: "phase_5_hybrid_synchronization_requirements_only".to_string(),
                },
            };
        }

        let mut checkpoints = Vec::new();
        for verifier in decision
            .required_verifiers
            .iter()
            .chain(
                verifier_authority
                    .requirements
                    .iter()
                    .map(|requirement| &requirement.required_verifier),
            )
            .copied()
        {
            push_checkpoints_for_verifier(&mut checkpoints, verifier, &workflow_attempt_id);
        }

        checkpoints.sort_by_key(|checkpoint| checkpoint.kind as u8);
        if checkpoints.is_empty() {
            trace_labels.push("hybrid_mode_without_sync_checkpoints".to_string());
        } else {
            trace_labels.push(format!("checkpoint_count::{}", checkpoints.len()));
        }

        HybridSynchronizationAssessment {
            required,
            checkpoints,
            trace: HybridSynchronizationTrace {
                workflow_attempt_id,
                contract_id: decision.workflow_contract_id,
                trace_labels,
                explanation: "phase_5_hybrid_synchronization_requirements_only".to_string(),
            },
        }
    }

    pub fn evaluate_observed(
        &self,
        assessment: &HybridSynchronizationAssessment,
        observations: &[HybridSynchronizationObservation],
    ) -> HybridSynchronizationVerdict {
        if !assessment.required || assessment.checkpoints.is_empty() {
            return HybridSynchronizationVerdict {
                overall: HybridSynchronizationOverallVerdict::NotRequired,
                statuses: Vec::new(),
                synchronized_count: 0,
                invalidated_count: 0,
                pending_count: 0,
            };
        }

        let statuses = assessment
            .checkpoints
            .iter()
            .cloned()
            .map(|checkpoint| evaluate_checkpoint(checkpoint, observations))
            .collect::<Vec<_>>();
        let synchronized_count = statuses
            .iter()
            .filter(|status| status.status == SynchronizationCheckpointStatus::Synchronized)
            .count();
        let invalidated_count = statuses
            .iter()
            .filter(|status| status.status == SynchronizationCheckpointStatus::Invalidated)
            .count();
        let pending_count = statuses
            .iter()
            .filter(|status| status.status == SynchronizationCheckpointStatus::Pending)
            .count();
        let overall = if invalidated_count > 0 {
            HybridSynchronizationOverallVerdict::Invalidated
        } else if pending_count > 0 {
            HybridSynchronizationOverallVerdict::Pending
        } else {
            HybridSynchronizationOverallVerdict::Synchronized
        };

        HybridSynchronizationVerdict {
            overall,
            statuses,
            synchronized_count,
            invalidated_count,
            pending_count,
        }
    }
}

pub fn hash_structural_bytes(bytes: &[u8]) -> String {
    blake3::hash(bytes).to_hex().to_string()
}

fn push_checkpoints_for_verifier(
    checkpoints: &mut Vec<HybridSynchronizationCheckpoint>,
    verifier: RequiredVerifier,
    workflow_attempt_id: &str,
) {
    match verifier {
        RequiredVerifier::IdeFileVisible | RequiredVerifier::DocumentContentVisible => {
            push_checkpoint(
                checkpoints,
                HybridSynchronizationCheckpointKind::FileHashSync,
                verifier,
                workflow_attempt_id,
            );
            push_checkpoint(
                checkpoints,
                HybridSynchronizationCheckpointKind::VisibleArtifactSync,
                verifier,
                workflow_attempt_id,
            );
        }
        RequiredVerifier::IdeWorkspaceVisible => push_checkpoint(
            checkpoints,
            HybridSynchronizationCheckpointKind::WorkspaceIdentitySync,
            verifier,
            workflow_attempt_id,
        ),
        RequiredVerifier::TerminalOutputVisible
        | RequiredVerifier::WorkflowStageVisible
        | RequiredVerifier::OutputSurfaced => push_checkpoint(
            checkpoints,
            HybridSynchronizationCheckpointKind::TerminalExecutionFreshness,
            verifier,
            workflow_attempt_id,
        ),
        RequiredVerifier::BrowserPageVisible => push_checkpoint(
            checkpoints,
            HybridSynchronizationCheckpointKind::BrowserPageFreshness,
            verifier,
            workflow_attempt_id,
        ),
        RequiredVerifier::BrowserAccountContext => push_checkpoint(
            checkpoints,
            HybridSynchronizationCheckpointKind::AccountSessionSync,
            verifier,
            workflow_attempt_id,
        ),
        RequiredVerifier::AppContextVisible
        | RequiredVerifier::MediaPlaybackVisible
        | RequiredVerifier::HumanReviewPending
        | RequiredVerifier::UserConfirmation => push_checkpoint(
            checkpoints,
            HybridSynchronizationCheckpointKind::VisibleArtifactSync,
            verifier,
            workflow_attempt_id,
        ),
        RequiredVerifier::StructuralResult => {}
    }
}

fn push_checkpoint(
    checkpoints: &mut Vec<HybridSynchronizationCheckpoint>,
    kind: HybridSynchronizationCheckpointKind,
    verifier: RequiredVerifier,
    workflow_attempt_id: &str,
) {
    if let Some(existing) = checkpoints
        .iter_mut()
        .find(|checkpoint| checkpoint.kind == kind)
    {
        push_unique_verifier(&mut existing.required_verifiers, verifier);
        return;
    }

    checkpoints.push(HybridSynchronizationCheckpoint {
        checkpoint_id: format!("{workflow_attempt_id}::{kind:?}"),
        kind,
        required_verifiers: vec![verifier],
        requires_structural_state: requires_structural_state(kind),
        requires_visible_state: true,
        requires_freshness_marker: requires_freshness_marker(kind),
        invalidation_reasons: invalidation_reasons(kind),
    });
}

fn push_unique_verifier(verifiers: &mut Vec<RequiredVerifier>, verifier: RequiredVerifier) {
    if !verifiers.contains(&verifier) {
        verifiers.push(verifier);
    }
}

fn requires_structural_state(kind: HybridSynchronizationCheckpointKind) -> bool {
    matches!(
        kind,
        HybridSynchronizationCheckpointKind::FileHashSync
            | HybridSynchronizationCheckpointKind::WorkspaceIdentitySync
            | HybridSynchronizationCheckpointKind::TerminalExecutionFreshness
            | HybridSynchronizationCheckpointKind::BrowserPageFreshness
            | HybridSynchronizationCheckpointKind::VisibleArtifactSync
    )
}

fn requires_freshness_marker(kind: HybridSynchronizationCheckpointKind) -> bool {
    matches!(
        kind,
        HybridSynchronizationCheckpointKind::TerminalExecutionFreshness
            | HybridSynchronizationCheckpointKind::BrowserPageFreshness
    )
}

fn invalidation_reasons(
    kind: HybridSynchronizationCheckpointKind,
) -> Vec<SynchronizationInvalidationReason> {
    match kind {
        HybridSynchronizationCheckpointKind::FileHashSync => vec![
            SynchronizationInvalidationReason::MissingStructuralState,
            SynchronizationInvalidationReason::MissingVisibleState,
            SynchronizationInvalidationReason::FileHashChangedAfterVisibleOpen,
            SynchronizationInvalidationReason::FileHashMismatch,
            SynchronizationInvalidationReason::ExternalMutationAfterObservation,
            SynchronizationInvalidationReason::EvidencePredatesWorkflowAttempt,
        ],
        HybridSynchronizationCheckpointKind::WorkspaceIdentitySync => vec![
            SynchronizationInvalidationReason::MissingStructuralState,
            SynchronizationInvalidationReason::MissingVisibleState,
            SynchronizationInvalidationReason::WorkspaceIdentityMismatch,
        ],
        HybridSynchronizationCheckpointKind::TerminalExecutionFreshness => vec![
            SynchronizationInvalidationReason::MissingStructuralState,
            SynchronizationInvalidationReason::MissingVisibleState,
            SynchronizationInvalidationReason::TerminalOutputMissingCurrentRunMarker,
            SynchronizationInvalidationReason::EvidencePredatesWorkflowAttempt,
        ],
        HybridSynchronizationCheckpointKind::BrowserPageFreshness => vec![
            SynchronizationInvalidationReason::MissingVisibleState,
            SynchronizationInvalidationReason::BrowserNavigationPredatesWorkflowAttempt,
            SynchronizationInvalidationReason::BrowserTargetIdentityMismatch,
        ],
        HybridSynchronizationCheckpointKind::AccountSessionSync => vec![
            SynchronizationInvalidationReason::MissingVisibleState,
            SynchronizationInvalidationReason::AccountSessionMismatch,
        ],
        HybridSynchronizationCheckpointKind::VisibleArtifactSync => vec![
            SynchronizationInvalidationReason::MissingStructuralState,
            SynchronizationInvalidationReason::MissingVisibleState,
            SynchronizationInvalidationReason::VisibleArtifactMismatch,
            SynchronizationInvalidationReason::EvidencePredatesWorkflowAttempt,
        ],
    }
}

fn evaluate_checkpoint(
    checkpoint: HybridSynchronizationCheckpoint,
    observations: &[HybridSynchronizationObservation],
) -> HybridSynchronizationCheckpointVerdict {
    let Some(observation) = observations.iter().find(|observation| {
        observation
            .checkpoint_id
            .as_ref()
            .map(|id| id == &checkpoint.checkpoint_id)
            .unwrap_or(false)
            || observation.kind == checkpoint.kind
    }) else {
        return HybridSynchronizationCheckpointVerdict {
            checkpoint,
            status: SynchronizationCheckpointStatus::Pending,
            invalidation_reason: None,
            evidence_summary: None,
        };
    };

    if let Some(reason) = generic_invalidation_reason(&checkpoint, observation) {
        return invalidated(checkpoint, observation, reason);
    }

    if let Some(reason) = kind_invalidation_reason(checkpoint.kind, observation) {
        return invalidated(checkpoint, observation, reason);
    }

    HybridSynchronizationCheckpointVerdict {
        checkpoint,
        status: SynchronizationCheckpointStatus::Synchronized,
        invalidation_reason: None,
        evidence_summary: Some(observation.evidence_summary.clone()),
    }
}

fn generic_invalidation_reason(
    checkpoint: &HybridSynchronizationCheckpoint,
    observation: &HybridSynchronizationObservation,
) -> Option<SynchronizationInvalidationReason> {
    if checkpoint.requires_structural_state
        && structural_state_missing(checkpoint.kind, observation)
    {
        return Some(SynchronizationInvalidationReason::MissingStructuralState);
    }
    if checkpoint.requires_visible_state && visible_state_missing(checkpoint.kind, observation) {
        return Some(SynchronizationInvalidationReason::MissingVisibleState);
    }
    if let (Some(action_started), Some(visible_observed)) = (
        observation.action_started_unix_ms,
        observation.visible_observed_unix_ms,
    ) {
        if visible_observed < action_started {
            return Some(SynchronizationInvalidationReason::EvidencePredatesWorkflowAttempt);
        }
    }
    if let (Some(external_mutation), Some(visible_observed)) = (
        observation.external_mutation_unix_ms,
        observation.visible_observed_unix_ms,
    ) {
        if external_mutation > visible_observed {
            return Some(SynchronizationInvalidationReason::ExternalMutationAfterObservation);
        }
    }
    None
}

fn kind_invalidation_reason(
    kind: HybridSynchronizationCheckpointKind,
    observation: &HybridSynchronizationObservation,
) -> Option<SynchronizationInvalidationReason> {
    match kind {
        HybridSynchronizationCheckpointKind::FileHashSync => {
            if let (Some(open_hash), Some(visible_hash)) = (
                observation.visible_open_hash.as_ref(),
                observation.visible_hash.as_ref(),
            ) {
                if open_hash != visible_hash {
                    return Some(
                        SynchronizationInvalidationReason::FileHashChangedAfterVisibleOpen,
                    );
                }
            }
            if observation.structural_hash != observation.visible_hash {
                return Some(SynchronizationInvalidationReason::FileHashMismatch);
            }
            None
        }
        HybridSynchronizationCheckpointKind::WorkspaceIdentitySync => {
            if observation.expected_workspace != observation.observed_workspace {
                return Some(SynchronizationInvalidationReason::WorkspaceIdentityMismatch);
            }
            None
        }
        HybridSynchronizationCheckpointKind::TerminalExecutionFreshness => {
            if observation.current_run_marker != observation.observed_run_marker {
                return Some(
                    SynchronizationInvalidationReason::TerminalOutputMissingCurrentRunMarker,
                );
            }
            None
        }
        HybridSynchronizationCheckpointKind::BrowserPageFreshness => {
            if let (Some(action_started), Some(navigation_time)) = (
                observation.action_started_unix_ms,
                observation.browser_navigation_unix_ms,
            ) {
                if navigation_time < action_started {
                    return Some(
                        SynchronizationInvalidationReason::BrowserNavigationPredatesWorkflowAttempt,
                    );
                }
            }
            if observation.structural_identity != observation.visible_identity {
                return Some(SynchronizationInvalidationReason::BrowserTargetIdentityMismatch);
            }
            None
        }
        HybridSynchronizationCheckpointKind::AccountSessionSync => {
            if observation.expected_account_identity != observation.observed_account_identity {
                return Some(SynchronizationInvalidationReason::AccountSessionMismatch);
            }
            None
        }
        HybridSynchronizationCheckpointKind::VisibleArtifactSync => {
            if observation.structural_identity != observation.visible_identity {
                return Some(SynchronizationInvalidationReason::VisibleArtifactMismatch);
            }
            None
        }
    }
}

fn structural_state_missing(
    kind: HybridSynchronizationCheckpointKind,
    observation: &HybridSynchronizationObservation,
) -> bool {
    match kind {
        HybridSynchronizationCheckpointKind::FileHashSync => observation.structural_hash.is_none(),
        HybridSynchronizationCheckpointKind::WorkspaceIdentitySync => {
            observation.expected_workspace.is_none()
        }
        HybridSynchronizationCheckpointKind::TerminalExecutionFreshness => {
            observation.current_run_marker.is_none()
        }
        HybridSynchronizationCheckpointKind::BrowserPageFreshness
        | HybridSynchronizationCheckpointKind::VisibleArtifactSync => {
            observation.structural_identity.is_none()
        }
        HybridSynchronizationCheckpointKind::AccountSessionSync => false,
    }
}

fn visible_state_missing(
    kind: HybridSynchronizationCheckpointKind,
    observation: &HybridSynchronizationObservation,
) -> bool {
    match kind {
        HybridSynchronizationCheckpointKind::FileHashSync => observation.visible_hash.is_none(),
        HybridSynchronizationCheckpointKind::WorkspaceIdentitySync => {
            observation.observed_workspace.is_none()
        }
        HybridSynchronizationCheckpointKind::TerminalExecutionFreshness => {
            observation.observed_run_marker.is_none()
        }
        HybridSynchronizationCheckpointKind::BrowserPageFreshness
        | HybridSynchronizationCheckpointKind::VisibleArtifactSync => {
            observation.visible_identity.is_none()
        }
        HybridSynchronizationCheckpointKind::AccountSessionSync => {
            observation.observed_account_identity.is_none()
        }
    }
}

fn invalidated(
    checkpoint: HybridSynchronizationCheckpoint,
    observation: &HybridSynchronizationObservation,
    reason: SynchronizationInvalidationReason,
) -> HybridSynchronizationCheckpointVerdict {
    HybridSynchronizationCheckpointVerdict {
        checkpoint,
        status: SynchronizationCheckpointStatus::Invalidated,
        invalidation_reason: Some(reason),
        evidence_summary: Some(observation.evidence_summary.clone()),
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
    use crate::agent::verifier_authority::VerifierAuthorityEvaluator;
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

    fn assessment_for_prompt(prompt: &str) -> HybridSynchronizationAssessment {
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
        let authority =
            VerifierAuthorityEvaluator.assess(&contract_check, &decision, &analysis, "attempt");

        HybridSynchronizationEvaluator.assess(&decision, &analysis, &authority, "attempt")
    }

    fn observation(kind: HybridSynchronizationCheckpointKind) -> HybridSynchronizationObservation {
        HybridSynchronizationObservation {
            checkpoint_id: None,
            kind,
            structural_identity: Some("target".to_string()),
            visible_identity: Some("target".to_string()),
            structural_hash: Some("hash".to_string()),
            visible_hash: Some("hash".to_string()),
            visible_open_hash: Some("hash".to_string()),
            expected_workspace: Some("/project".to_string()),
            observed_workspace: Some("/project".to_string()),
            current_run_marker: Some("run-1".to_string()),
            observed_run_marker: Some("run-1".to_string()),
            expected_account_identity: Some("user@example.com".to_string()),
            observed_account_identity: Some("user@example.com".to_string()),
            action_started_unix_ms: Some(100),
            visible_observed_unix_ms: Some(150),
            browser_navigation_unix_ms: Some(150),
            external_mutation_unix_ms: None,
            evidence_summary: "fresh synchronized evidence".to_string(),
        }
    }

    #[test]
    fn hybrid_coding_requires_file_workspace_and_output_sync() {
        let assessment = assessment_for_prompt(
            "open code and write a program to print pascal triangle and run it and show output",
        );

        assert!(assessment.required);
        assert!(assessment.checkpoints.iter().any(|checkpoint| {
            checkpoint.kind == HybridSynchronizationCheckpointKind::FileHashSync
        }));
        assert!(assessment.checkpoints.iter().any(|checkpoint| {
            checkpoint.kind == HybridSynchronizationCheckpointKind::WorkspaceIdentitySync
        }));
        assert!(assessment.checkpoints.iter().any(|checkpoint| {
            checkpoint.kind == HybridSynchronizationCheckpointKind::TerminalExecutionFreshness
        }));
    }

    #[test]
    fn structural_mode_does_not_require_hybrid_sync() {
        let spec = spec(
            Verb::Run,
            Vec::new(),
            Some(ContentClass::Generated {
                hint: "python program".to_string(),
                language: Some("python".to_string()),
            }),
        );
        let analysis = analyze_semantic_workflow(&spec, "write a python program that prints hello");
        let decision = ExecutionModeReasoner.decide(
            &spec,
            &analysis,
            &EnvironmentCapabilities::unchecked_default(),
            &PolicyContext::default(),
        );
        let contract_check = WorkflowIntentContractRegistry.evaluate(&decision, &analysis);
        let authority =
            VerifierAuthorityEvaluator.assess(&contract_check, &decision, &analysis, "attempt");

        let assessment =
            HybridSynchronizationEvaluator.assess(&decision, &analysis, &authority, "attempt");
        let verdict = HybridSynchronizationEvaluator.evaluate_observed(&assessment, &[]);

        assert!(!assessment.required);
        assert!(assessment.checkpoints.is_empty());
        assert_eq!(
            verdict.overall,
            HybridSynchronizationOverallVerdict::NotRequired
        );
    }

    #[test]
    fn file_hash_mismatch_invalidates_completion() {
        let assessment = HybridSynchronizationAssessment {
            required: true,
            checkpoints: vec![HybridSynchronizationCheckpoint {
                checkpoint_id: "attempt::FileHashSync".to_string(),
                kind: HybridSynchronizationCheckpointKind::FileHashSync,
                required_verifiers: vec![RequiredVerifier::IdeFileVisible],
                requires_structural_state: true,
                requires_visible_state: true,
                requires_freshness_marker: false,
                invalidation_reasons: invalidation_reasons(
                    HybridSynchronizationCheckpointKind::FileHashSync,
                ),
            }],
            trace: HybridSynchronizationTrace {
                workflow_attempt_id: "attempt".to_string(),
                contract_id: WorkflowContractId::VisibleCodingWorkflow,
                trace_labels: Vec::new(),
                explanation: "test".to_string(),
            },
        };
        let mut observed = observation(HybridSynchronizationCheckpointKind::FileHashSync);
        observed.visible_hash = Some("different".to_string());
        observed.visible_open_hash = Some("different".to_string());

        let verdict = HybridSynchronizationEvaluator.evaluate_observed(&assessment, &[observed]);

        assert_eq!(
            verdict.overall,
            HybridSynchronizationOverallVerdict::Invalidated
        );
        assert!(verdict.statuses.iter().any(|status| {
            status.invalidation_reason == Some(SynchronizationInvalidationReason::FileHashMismatch)
        }));
    }

    #[test]
    fn terminal_marker_mismatch_invalidates_freshness() {
        let assessment = HybridSynchronizationAssessment {
            required: true,
            checkpoints: vec![HybridSynchronizationCheckpoint {
                checkpoint_id: "attempt::TerminalExecutionFreshness".to_string(),
                kind: HybridSynchronizationCheckpointKind::TerminalExecutionFreshness,
                required_verifiers: vec![RequiredVerifier::TerminalOutputVisible],
                requires_structural_state: true,
                requires_visible_state: true,
                requires_freshness_marker: true,
                invalidation_reasons: invalidation_reasons(
                    HybridSynchronizationCheckpointKind::TerminalExecutionFreshness,
                ),
            }],
            trace: HybridSynchronizationTrace {
                workflow_attempt_id: "attempt".to_string(),
                contract_id: WorkflowContractId::VisibleCodingWorkflow,
                trace_labels: Vec::new(),
                explanation: "test".to_string(),
            },
        };
        let mut observed =
            observation(HybridSynchronizationCheckpointKind::TerminalExecutionFreshness);
        observed.observed_run_marker = Some("old-run".to_string());

        let verdict = HybridSynchronizationEvaluator.evaluate_observed(&assessment, &[observed]);

        assert_eq!(
            verdict.overall,
            HybridSynchronizationOverallVerdict::Invalidated
        );
        assert!(verdict.statuses.iter().any(|status| {
            status.invalidation_reason
                == Some(SynchronizationInvalidationReason::TerminalOutputMissingCurrentRunMarker)
        }));
    }

    #[test]
    fn browser_navigation_before_attempt_invalidates_freshness() {
        let assessment = HybridSynchronizationAssessment {
            required: true,
            checkpoints: vec![HybridSynchronizationCheckpoint {
                checkpoint_id: "attempt::BrowserPageFreshness".to_string(),
                kind: HybridSynchronizationCheckpointKind::BrowserPageFreshness,
                required_verifiers: vec![RequiredVerifier::BrowserPageVisible],
                requires_structural_state: true,
                requires_visible_state: true,
                requires_freshness_marker: true,
                invalidation_reasons: invalidation_reasons(
                    HybridSynchronizationCheckpointKind::BrowserPageFreshness,
                ),
            }],
            trace: HybridSynchronizationTrace {
                workflow_attempt_id: "attempt".to_string(),
                contract_id: WorkflowContractId::VisibleBrowserWorkflow,
                trace_labels: Vec::new(),
                explanation: "test".to_string(),
            },
        };
        let mut observed = observation(HybridSynchronizationCheckpointKind::BrowserPageFreshness);
        observed.browser_navigation_unix_ms = Some(90);

        let verdict = HybridSynchronizationEvaluator.evaluate_observed(&assessment, &[observed]);

        assert_eq!(
            verdict.overall,
            HybridSynchronizationOverallVerdict::Invalidated
        );
        assert!(verdict.statuses.iter().any(|status| {
            status.invalidation_reason
                == Some(SynchronizationInvalidationReason::BrowserNavigationPredatesWorkflowAttempt)
        }));
    }

    #[test]
    fn synchronized_observations_pass_all_checkpoints() {
        let assessment = assessment_for_prompt(
            "open code and write a program to print pascal triangle and run it and show output",
        );
        let observations = assessment
            .checkpoints
            .iter()
            .map(|checkpoint| observation(checkpoint.kind))
            .collect::<Vec<_>>();

        let verdict = HybridSynchronizationEvaluator.evaluate_observed(&assessment, &observations);

        assert_eq!(
            verdict.overall,
            HybridSynchronizationOverallVerdict::Synchronized
        );
        assert_eq!(verdict.synchronized_count, assessment.checkpoints.len());
    }

    #[test]
    fn structural_hash_is_deterministic() {
        assert_eq!(
            hash_structural_bytes(b"same"),
            hash_structural_bytes(b"same")
        );
        assert_ne!(
            hash_structural_bytes(b"same"),
            hash_structural_bytes(b"different")
        );
    }

    #[test]
    fn assessment_serializes_to_json() {
        let assessment = assessment_for_prompt(
            "open code and write a program to print pascal triangle and run it and show output",
        );
        let json = serde_json::to_string(&assessment).expect("assessment is serializable");
        let roundtrip: HybridSynchronizationAssessment =
            serde_json::from_str(&json).expect("assessment is deserializable");

        assert_eq!(roundtrip, assessment);
    }
}
