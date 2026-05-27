//! Collaborative decision runtime primitives.
//!
//! This module is intentionally small. It does not replace HITL, policy,
//! verifier, or workflow continuation. It gives those systems a shared,
//! durable decision envelope so ambiguity and recoverable HITL waits can pause
//! workflows instead of collapsing into opaque tool errors.

use std::collections::HashMap;
use std::fs::{File, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use serde::{Deserialize, Serialize};

use crate::safety::RiskLevel;

pub type WorkflowId = String;
pub type AttemptId = String;
pub type StageId = String;
pub type DecisionId = String;
pub type OptionId = String;
pub type ActionHash = String;
pub type TargetHash = String;
pub type CheckpointId = String;
pub type ActionId = String;

const DEFAULT_DECISION_TTL_HOURS: i64 = 24;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Actor {
    Runtime,
    User,
    PolicyEngine,
    ExecutionAuthority,
    Verifier,
    Tool(String),
    External(String),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TargetBinding {
    pub kind: String,
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub execution_boundary: Option<String>,
    #[serde(default)]
    pub metadata: serde_json::Value,
}

impl TargetBinding {
    pub fn new(kind: impl Into<String>, id: impl Into<String>) -> Self {
        Self {
            kind: kind.into(),
            id: id.into(),
            workspace_id: None,
            session_id: None,
            execution_boundary: None,
            metadata: serde_json::json!({}),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ActionProposal {
    pub workflow_id: WorkflowId,
    pub attempt_id: AttemptId,
    pub stage_id: StageId,
    pub tool_name: String,
    pub parameters: serde_json::Value,
    pub target: TargetBinding,
    #[serde(default = "default_tool_schema_version")]
    pub tool_schema_version: String,
    #[serde(default = "default_tool_registry_version")]
    pub tool_registry_version: String,
    pub action_hash: ActionHash,
    pub target_hash: TargetHash,
    pub requested_by: Actor,
    pub created_at: String,
}

impl ActionProposal {
    pub fn new(
        workflow_id: impl Into<String>,
        attempt_id: impl Into<String>,
        stage_id: impl Into<String>,
        tool_name: impl Into<String>,
        parameters: serde_json::Value,
        target: TargetBinding,
        requested_by: Actor,
    ) -> Self {
        let workflow_id = workflow_id.into();
        let attempt_id = attempt_id.into();
        let stage_id = stage_id.into();
        let tool_name = tool_name.into();
        let target_hash = compute_target_hash(&target);
        let tool_schema_version = default_tool_schema_version();
        let tool_registry_version = default_tool_registry_version();
        let action_hash = compute_action_hash(
            &workflow_id,
            &attempt_id,
            &stage_id,
            &tool_name,
            &parameters,
            &target_hash,
            &tool_schema_version,
            &tool_registry_version,
        );
        Self {
            workflow_id,
            attempt_id,
            stage_id,
            tool_name,
            parameters,
            target,
            tool_schema_version,
            tool_registry_version,
            action_hash,
            target_hash,
            requested_by,
            created_at: now_rfc3339(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum GateOutcome {
    Proceed,
    Block { reason: String },
    PauseForDecision { decision_id: DecisionId },
    NeedReobserve { reason: String },
    NeedLease { resources: Vec<String> },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum DecisionType {
    Approval,
    TargetSelection,
    ScopeClarification,
    RecoveryChoice,
    CredentialRequired,
    VerifierConflict,
    UnsafeUncertainty,
}

impl DecisionType {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Approval => "approval",
            Self::TargetSelection => "target_selection",
            Self::ScopeClarification => "scope_clarification",
            Self::RecoveryChoice => "recovery_choice",
            Self::CredentialRequired => "credential_required",
            Self::VerifierConflict => "verifier_conflict",
            Self::UnsafeUncertainty => "unsafe_uncertainty",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum DecisionStatus {
    Pending,
    Resolved,
    Deferred,
    Expired,
    Invalidated,
    Denied,
    Cancelled,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum AuthorityLevel {
    PolicyBlock,
    PolicyRisk,
    VerifierTruth,
    ExecutionAuthority,
    RecoveryFeasibility,
    WorkflowSemantics,
    UserInstruction,
    Preference,
    PlannerRecommendation,
    ModelSuggestion,
}

impl AuthorityLevel {
    pub fn rank(self) -> u8 {
        match self {
            Self::PolicyBlock => 100,
            Self::PolicyRisk => 90,
            Self::VerifierTruth => 80,
            Self::ExecutionAuthority => 70,
            Self::RecoveryFeasibility => 60,
            Self::WorkflowSemantics => 50,
            Self::UserInstruction => 40,
            Self::Preference => 30,
            Self::PlannerRecommendation => 20,
            Self::ModelSuggestion => 10,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ConfidenceBand {
    High,
    Medium,
    Low,
    Conflicted,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Rollbackability {
    Reversible,
    Compensatable,
    PartiallyIrreversible,
    Irreversible,
    Unknown,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EvidenceSummary {
    pub source: String,
    pub confidence: ConfidenceBand,
    pub freshness: String,
    pub reliability: String,
    pub summary: String,
}

impl EvidenceSummary {
    pub fn deterministic(source: impl Into<String>, summary: impl Into<String>) -> Self {
        Self {
            source: source.into(),
            confidence: ConfidenceBand::High,
            freshness: "current".to_string(),
            reliability: "deterministic".to_string(),
            summary: summary.into(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DecisionOption {
    pub id: String,
    pub label: String,
    pub impact: String,
    pub risk: RiskLevel,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DecisionCandidate {
    pub decision_type: DecisionType,
    pub authority: AuthorityLevel,
    pub risk_level: RiskLevel,
    pub reason: String,
    pub options: Vec<DecisionOption>,
    pub recommended_option: Option<String>,
    pub rollbackability: Rollbackability,
    pub confidence: ConfidenceBand,
    pub affected_resources: Vec<String>,
    pub rule_id: Option<String>,
    pub evidence: Vec<EvidenceSummary>,
    pub invalidation_rules: Vec<String>,
}

impl DecisionCandidate {
    pub fn target_selection(
        reason: impl Into<String>,
        options: Vec<String>,
        affected_resource: impl Into<String>,
    ) -> Self {
        Self {
            decision_type: DecisionType::TargetSelection,
            authority: AuthorityLevel::ExecutionAuthority,
            risk_level: RiskLevel::Yellow,
            reason: reason.into(),
            options: options
                .into_iter()
                .map(|option| DecisionOption {
                    id: option.clone(),
                    label: option.clone(),
                    impact: format!("Run this step on {}", option),
                    risk: RiskLevel::Yellow,
                })
                .collect(),
            recommended_option: None,
            rollbackability: Rollbackability::Unknown,
            confidence: ConfidenceBand::Unknown,
            affected_resources: vec![affected_resource.into()],
            rule_id: Some("execution_authority.target_ambiguity".to_string()),
            evidence: vec![EvidenceSummary::deterministic(
                "ExecutionAuthority",
                "Target ambiguity requires human selection before execution",
            )],
            invalidation_rules: vec![
                "target_changed".to_string(),
                "tool_parameters_changed".to_string(),
                "workflow_owner_changed".to_string(),
            ],
        }
    }

    pub fn approval(
        action: impl Into<String>,
        reason: impl Into<String>,
        risk_level: RiskLevel,
        rollbackability: Rollbackability,
        affected_resources: Vec<String>,
        rule_id: Option<String>,
    ) -> Self {
        let action = action.into();
        Self {
            decision_type: DecisionType::Approval,
            authority: AuthorityLevel::PolicyRisk,
            risk_level,
            reason: reason.into(),
            options: vec![
                DecisionOption {
                    id: "approve".to_string(),
                    label: "Approve".to_string(),
                    impact: format!("Allow {}", action),
                    risk: risk_level,
                },
                DecisionOption {
                    id: "deny".to_string(),
                    label: "Deny".to_string(),
                    impact: "Do not execute this action".to_string(),
                    risk: RiskLevel::Green,
                },
            ],
            recommended_option: None,
            rollbackability,
            confidence: ConfidenceBand::High,
            affected_resources,
            rule_id,
            evidence: vec![EvidenceSummary::deterministic(
                "PolicyEngine",
                "Policy requires explicit approval before execution",
            )],
            invalidation_rules: vec![
                "policy_risk_changed".to_string(),
                "tool_parameters_changed".to_string(),
                "target_changed".to_string(),
            ],
        }
    }

    pub fn dedupe_key(&self) -> String {
        format!(
            "{}:{:?}:{}",
            self.decision_type.as_str(),
            self.risk_level,
            self.affected_resources.join(",")
        )
    }
}

pub trait DecisionProducer: Send + Sync {
    fn observe(&self, context: &serde_json::Value) -> Vec<DecisionCandidate>;
}

#[derive(Debug, Default, Clone)]
pub struct DecisionAggregator;

impl DecisionAggregator {
    pub fn aggregate(&self, candidates: Vec<DecisionCandidate>) -> Vec<DecisionCandidate> {
        let mut deduped: HashMap<String, DecisionCandidate> = HashMap::new();
        for candidate in candidates {
            let key = candidate.dedupe_key();
            match deduped.get(&key) {
                Some(existing) if existing.authority.rank() >= candidate.authority.rank() => {}
                _ => {
                    deduped.insert(key, candidate);
                }
            }
        }

        let mut values: Vec<_> = deduped.into_values().collect();
        values.sort_by(|a, b| {
            b.authority
                .rank()
                .cmp(&a.authority.rank())
                .then_with(|| b.risk_level.cmp(&a.risk_level))
        });
        values
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InteractionDecision {
    pub id: String,
    pub workflow_id: String,
    #[serde(default)]
    pub attempt_id: String,
    pub stage_id: Option<String>,
    #[serde(default)]
    pub action_hash: String,
    #[serde(default)]
    pub target_hash: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub action_proposal: Option<ActionProposal>,
    pub decision_type: DecisionType,
    pub status: DecisionStatus,
    #[serde(default = "default_decision_version")]
    pub version: u64,
    pub reason: String,
    pub risk_level: RiskLevel,
    pub options: Vec<DecisionOption>,
    pub recommended_option: Option<String>,
    pub rollbackability: Rollbackability,
    pub confidence: ConfidenceBand,
    pub affected_resources: Vec<String>,
    pub rule_id: Option<String>,
    pub evidence: Vec<EvidenceSummary>,
    pub invalidation_rules: Vec<String>,
    pub created_at: String,
    pub updated_at: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expires_at: Option<String>,
    pub resolution: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub execution: Option<DecisionExecutionRecord>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stage_binding: Option<DecisionStageBinding>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub checkpoint_summary: Option<CheckpointSummary>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub continuation: Option<ContinuationClaim>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub verification: Option<PostDecisionVerification>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum DecisionExecutionState {
    NotStarted,
    Preparing,
    BlockedByLease,
    Executing,
    Executed,
    Failed,
    Cancelled,
    Invalidated,
    UnknownAfterCrash,
}

impl DecisionExecutionState {
    pub fn terminal(self) -> bool {
        matches!(
            self,
            Self::Executed
                | Self::Failed
                | Self::Cancelled
                | Self::Invalidated
                | Self::UnknownAfterCrash
        )
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DecisionExecutionRecord {
    pub execution_id: String,
    pub decision_id: String,
    pub workflow_id: String,
    pub action_hash: String,
    pub target_hash: String,
    pub state: DecisionExecutionState,
    pub sequence: u64,
    pub execution_actor: String,
    pub source_command: String,
    pub session_id: Option<String>,
    pub workspace_id: Option<String>,
    pub tool_name: String,
    pub tool_schema_version: String,
    pub tool_registry_version: String,
    pub policy_version: String,
    pub started_at: String,
    pub side_effect_started_at: Option<String>,
    pub completed_at: Option<String>,
    pub gate_summary: Option<serde_json::Value>,
    pub grounding_summary: Option<serde_json::Value>,
    pub lease_refs: Vec<serde_json::Value>,
    pub redacted_tool_result: Option<serde_json::Value>,
    pub error_class: Option<String>,
    pub error_message: Option<String>,
}

#[derive(Debug, Clone)]
pub struct DecisionExecutionContext {
    pub execution_actor: String,
    pub source_command: String,
    pub session_id: Option<String>,
    pub workspace_id: Option<String>,
}

impl DecisionExecutionContext {
    pub fn user_action_center(session_id: Option<String>, workspace_id: Option<String>) -> Self {
        Self {
            execution_actor: "user".to_string(),
            source_command: "execute_resolved_interaction_decision".to_string(),
            session_id,
            workspace_id,
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct DecisionExecutionUpdate {
    pub gate_summary: Option<serde_json::Value>,
    pub grounding_summary: Option<serde_json::Value>,
    pub lease_refs: Option<Vec<serde_json::Value>>,
    pub redacted_tool_result: Option<serde_json::Value>,
    pub error_class: Option<String>,
    pub error_message: Option<String>,
    pub side_effect_started: bool,
    pub completed: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DecisionStageBinding {
    pub decision_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub execution_id: Option<String>,
    pub workflow_id: String,
    pub attempt_id: String,
    pub stage_id: String,
    pub action_group_id: String,
    pub action_id: ActionId,
    pub action_hash: ActionHash,
    pub target_hash: TargetHash,
    pub checkpoint_id: CheckpointId,
    #[serde(default = "default_workflow_schema_version")]
    pub workflow_schema_version: String,
    #[serde(default = "default_max_side_effect_count")]
    pub max_side_effect_count: u8,
    #[serde(default = "default_true")]
    pub local_deterministic_only: bool,
    pub created_at: String,
}

impl DecisionStageBinding {
    pub fn for_action(decision_id: &str, action: &ActionProposal) -> Self {
        Self {
            decision_id: decision_id.to_string(),
            execution_id: None,
            workflow_id: action.workflow_id.clone(),
            attempt_id: action.attempt_id.clone(),
            stage_id: action.stage_id.clone(),
            action_group_id: action.stage_id.clone(),
            action_id: format!("{}:{}", action.stage_id, action.action_hash),
            action_hash: action.action_hash.clone(),
            target_hash: action.target_hash.clone(),
            checkpoint_id: format!(
                "{}:{}:{}",
                action.workflow_id, action.stage_id, action.action_hash
            ),
            workflow_schema_version: default_workflow_schema_version(),
            max_side_effect_count: default_max_side_effect_count(),
            local_deterministic_only: true,
            created_at: now_rfc3339(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CheckpointSummary {
    #[serde(default)]
    pub completed_action_ids: Vec<ActionId>,
    pub blocked_action_id: ActionId,
    #[serde(default)]
    pub expected_artifacts: Vec<String>,
    #[serde(default)]
    pub active_assumptions: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next_safe_action_preview: Option<serde_json::Value>,
    #[serde(default)]
    pub verifier_requirements: Vec<String>,
    pub rollbackability: Rollbackability,
    #[serde(default)]
    pub invalidation_rules: Vec<String>,
}

impl CheckpointSummary {
    pub fn for_action(action: &ActionProposal, rollbackability: Rollbackability) -> Self {
        let action_id = format!("{}:{}", action.stage_id, action.action_hash);
        Self {
            completed_action_ids: Vec::new(),
            blocked_action_id: action_id,
            expected_artifacts: expected_artifacts_from_action(action),
            active_assumptions: vec![format!("target_hash:{}", action.target_hash)],
            next_safe_action_preview: None,
            verifier_requirements: verifier_requirements_from_action(action),
            rollbackability,
            invalidation_rules: vec![
                "action_hash_changed".to_string(),
                "target_hash_changed".to_string(),
                "policy_risk_increased".to_string(),
                "tool_schema_changed".to_string(),
            ],
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ContinuationState {
    NotStarted,
    VerifyingPriorAction,
    VerifiedPriorAction,
    AdvancingActionState,
    ReadyForNextSafeStep,
    ExecutingNextSafeStep,
    PausedAgain,
    CompletedOneStep,
    Failed,
    Cancelled,
    UnknownAfterCrash,
    Invalidated,
}

impl ContinuationState {
    pub fn terminal(self) -> bool {
        matches!(
            self,
            Self::PausedAgain
                | Self::CompletedOneStep
                | Self::Failed
                | Self::Cancelled
                | Self::UnknownAfterCrash
                | Self::Invalidated
        )
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContinuationClaim {
    pub claim_id: String,
    pub decision_id: String,
    pub execution_id: String,
    pub workflow_id: String,
    pub checkpoint_id: CheckpointId,
    pub action_hash: ActionHash,
    pub target_hash: TargetHash,
    pub state: ContinuationState,
    pub sequence: u64,
    pub actor: String,
    pub started_at: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub side_effect_started_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub completed_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub verification_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error_class: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error_message: Option<String>,
}

#[derive(Debug, Clone, Default)]
pub struct ContinuationUpdate {
    pub side_effect_started: bool,
    pub completed: bool,
    pub verification_id: Option<String>,
    pub error_class: Option<String>,
    pub error_message: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PostDecisionVerification {
    pub verification_id: String,
    pub decision_id: String,
    pub execution_id: String,
    pub workflow_id: String,
    pub action_hash: ActionHash,
    pub target_hash: TargetHash,
    pub verifier_kind: String,
    #[serde(default)]
    pub evidence: Vec<EvidenceSummary>,
    pub confidence: ConfidenceBand,
    pub deterministic: bool,
    pub passed: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub failure_reason: Option<String>,
    #[serde(default)]
    pub sensitivity_tags: Vec<String>,
    pub created_at: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expires_at: Option<String>,
}

#[derive(Debug, Clone, Default)]
pub struct DecisionResolutionContext {
    pub expected_version: Option<u64>,
    pub expected_action_hash: Option<String>,
    pub expected_target_hash: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum DecisionEventType {
    DecisionCreated,
    DecisionResolved,
    DecisionExpired,
    DecisionInvalidated,
    DecisionDenied,
    DecisionCancelled,
    DecisionExecutionClaimed,
    DecisionExecutionStarted,
    DecisionExecutionCompleted,
    DecisionExecutionFailed,
    DecisionExecutionCancelled,
    DecisionExecutionInvalidated,
    DecisionExecutionUnknownAfterCrash,
    DecisionExecutionBlockedByLease,
    ContinuationClaimed,
    ContinuationVerificationStarted,
    ContinuationVerificationCompleted,
    ContinuationActionMarkedComplete,
    ContinuationNextStepReady,
    ContinuationNextStepStarted,
    ContinuationPausedAgain,
    ContinuationCompletedOneStep,
    ContinuationFailed,
    ContinuationCancelled,
    ContinuationUnknownAfterCrash,
    ContinuationInvalidated,
    EvidenceObserved,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DecisionEvent {
    pub event_id: String,
    pub decision_id: String,
    pub workflow_id: String,
    pub stage_id: Option<String>,
    pub event_type: DecisionEventType,
    pub actor: String,
    pub authority: AuthorityLevel,
    pub payload: serde_json::Value,
    pub created_at: String,
    pub policy_version: String,
    pub runtime_version: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct DecisionMetrics {
    pub total_events: usize,
    pub pending_decisions: usize,
    pub resolved_decisions: usize,
    pub expired_decisions: usize,
    pub invalidated_decisions: usize,
    pub approval_decisions: usize,
    pub target_selection_decisions: usize,
    pub unsafe_abstentions: usize,
}

#[derive(Debug, thiserror::Error)]
pub enum DecisionStoreError {
    #[error("decision store IO error: {0}")]
    Io(#[from] std::io::Error),
    #[error("decision store serialization error: {0}")]
    Serde(#[from] serde_json::Error),
    #[error("decision {decision_id} is not pending; current status is {status:?}")]
    NotPending {
        decision_id: String,
        status: DecisionStatus,
    },
    #[error("decision {decision_id} is not resolved; current status is {status:?}")]
    NotResolved {
        decision_id: String,
        status: DecisionStatus,
    },
    #[error("stale decision version for {decision_id}: expected {expected}, got {actual}")]
    VersionMismatch {
        decision_id: String,
        expected: u64,
        actual: u64,
    },
    #[error("stale decision action hash for {decision_id}: expected {expected}, got {actual}")]
    ActionHashMismatch {
        decision_id: String,
        expected: String,
        actual: String,
    },
    #[error("stale decision target hash for {decision_id}: expected {expected}, got {actual}")]
    TargetHashMismatch {
        decision_id: String,
        expected: String,
        actual: String,
    },
    #[error("decision {decision_id} is stale; expired at {expires_at}")]
    DecisionExpired {
        decision_id: String,
        expires_at: String,
    },
    #[error("option '{option_id}' is not valid for decision {decision_id}")]
    InvalidOption {
        decision_id: String,
        option_id: String,
    },
    #[error("decision {decision_id} already has execution state {state:?}")]
    ExecutionAlreadyExists {
        decision_id: String,
        state: DecisionExecutionState,
    },
    #[error("decision execution record missing for {decision_id}")]
    ExecutionMissing { decision_id: String },
    #[error("decision {decision_id} is missing a persisted action proposal")]
    MissingActionProposal { decision_id: String },
    #[error("continuation already exists for decision {decision_id} with state {state:?}")]
    ContinuationAlreadyExists {
        decision_id: String,
        state: ContinuationState,
    },
    #[error("continuation claim missing for decision {decision_id}")]
    ContinuationMissing { decision_id: String },
}

#[derive(Debug, Default)]
struct DecisionStoreState {
    decisions: HashMap<String, InteractionDecision>,
    executions: HashMap<String, DecisionExecutionRecord>,
    continuations: HashMap<String, ContinuationClaim>,
    verifications: HashMap<String, PostDecisionVerification>,
    events: Vec<DecisionEvent>,
}

#[derive(Debug, Clone)]
pub struct DecisionStore {
    state: Arc<Mutex<DecisionStoreState>>,
    path: Option<PathBuf>,
}

impl DecisionStore {
    pub fn in_memory() -> Self {
        Self {
            state: Arc::new(Mutex::new(DecisionStoreState::default())),
            path: None,
        }
    }

    pub fn default_persistent() -> Self {
        let path = default_decision_log_path();
        match Self::persistent(path) {
            Ok(store) => store,
            Err(error) => {
                tracing::warn!(error = %error, "falling back to in-memory decision store");
                Self::in_memory()
            }
        }
    }

    pub fn persistent(path: impl Into<PathBuf>) -> Result<Self, DecisionStoreError> {
        let path = path.into();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let store = Self {
            state: Arc::new(Mutex::new(DecisionStoreState::default())),
            path: Some(path),
        };
        store.replay_from_disk()?;
        Ok(store)
    }

    pub fn create_decision(
        &self,
        workflow_id: impl Into<String>,
        stage_id: Option<String>,
        candidate: DecisionCandidate,
    ) -> Result<InteractionDecision, DecisionStoreError> {
        let now = now_rfc3339();
        let expires_at = default_expires_at_rfc3339();
        let decision = InteractionDecision {
            id: uuid::Uuid::new_v4().to_string(),
            workflow_id: workflow_id.into(),
            attempt_id: String::new(),
            stage_id,
            action_hash: String::new(),
            target_hash: String::new(),
            action_proposal: None,
            decision_type: candidate.decision_type,
            status: DecisionStatus::Pending,
            version: 1,
            reason: candidate.reason,
            risk_level: candidate.risk_level,
            options: candidate.options,
            recommended_option: candidate.recommended_option,
            rollbackability: candidate.rollbackability,
            confidence: candidate.confidence,
            affected_resources: candidate.affected_resources,
            rule_id: candidate.rule_id,
            evidence: candidate.evidence,
            invalidation_rules: candidate.invalidation_rules,
            created_at: now.clone(),
            updated_at: now,
            expires_at: Some(expires_at),
            resolution: None,
            execution: None,
            stage_binding: None,
            checkpoint_summary: None,
            continuation: None,
            verification: None,
        };
        let event = DecisionEvent {
            event_id: uuid::Uuid::new_v4().to_string(),
            decision_id: decision.id.clone(),
            workflow_id: decision.workflow_id.clone(),
            stage_id: decision.stage_id.clone(),
            event_type: DecisionEventType::DecisionCreated,
            actor: "runtime".to_string(),
            authority: candidate.authority,
            payload: serde_json::to_value(&decision)?,
            created_at: decision.created_at.clone(),
            policy_version: "policy.v1".to_string(),
            runtime_version: env!("CARGO_PKG_VERSION").to_string(),
        };
        self.append_event(event)?;
        Ok(decision)
    }

    pub fn create_decision_for_action(
        &self,
        action: &ActionProposal,
        candidate: DecisionCandidate,
    ) -> Result<InteractionDecision, DecisionStoreError> {
        let now = now_rfc3339();
        let expires_at = default_expires_at_rfc3339();
        let authority = candidate.authority;
        let decision_id = uuid::Uuid::new_v4().to_string();
        let stage_binding = DecisionStageBinding::for_action(&decision_id, action);
        let checkpoint_summary = CheckpointSummary::for_action(action, candidate.rollbackability);
        let decision = InteractionDecision {
            id: decision_id,
            workflow_id: action.workflow_id.clone(),
            attempt_id: action.attempt_id.clone(),
            stage_id: Some(action.stage_id.clone()),
            action_hash: action.action_hash.clone(),
            target_hash: action.target_hash.clone(),
            action_proposal: Some(action.clone()),
            decision_type: candidate.decision_type,
            status: DecisionStatus::Pending,
            version: 1,
            reason: candidate.reason,
            risk_level: candidate.risk_level,
            options: candidate.options,
            recommended_option: candidate.recommended_option,
            rollbackability: candidate.rollbackability,
            confidence: candidate.confidence,
            affected_resources: candidate.affected_resources,
            rule_id: candidate.rule_id,
            evidence: candidate.evidence,
            invalidation_rules: candidate.invalidation_rules,
            created_at: now.clone(),
            updated_at: now,
            expires_at: Some(expires_at),
            resolution: None,
            execution: None,
            stage_binding: Some(stage_binding),
            checkpoint_summary: Some(checkpoint_summary),
            continuation: None,
            verification: None,
        };
        let event = DecisionEvent {
            event_id: uuid::Uuid::new_v4().to_string(),
            decision_id: decision.id.clone(),
            workflow_id: decision.workflow_id.clone(),
            stage_id: decision.stage_id.clone(),
            event_type: DecisionEventType::DecisionCreated,
            actor: "runtime".to_string(),
            authority,
            payload: serde_json::to_value(&decision)?,
            created_at: decision.created_at.clone(),
            policy_version: "policy.v1".to_string(),
            runtime_version: env!("CARGO_PKG_VERSION").to_string(),
        };
        self.append_event(event)?;
        Ok(decision)
    }

    pub fn resolve(
        &self,
        decision_id: &str,
        resolution: impl Into<String>,
        actor: impl Into<String>,
    ) -> Result<Option<InteractionDecision>, DecisionStoreError> {
        self.update_status(
            decision_id,
            DecisionStatus::Resolved,
            Some(resolution.into()),
            actor.into(),
            DecisionEventType::DecisionResolved,
        )
    }

    pub fn resolve_with_version(
        &self,
        decision_id: &str,
        expected_version: u64,
        option_id: &str,
        actor: impl Into<String>,
    ) -> Result<Option<InteractionDecision>, DecisionStoreError> {
        self.resolve_with_context(
            decision_id,
            DecisionResolutionContext {
                expected_version: Some(expected_version),
                ..DecisionResolutionContext::default()
            },
            option_id,
            actor,
        )
    }

    pub fn resolve_with_context(
        &self,
        decision_id: &str,
        context: DecisionResolutionContext,
        option_id: &str,
        actor: impl Into<String>,
    ) -> Result<Option<InteractionDecision>, DecisionStoreError> {
        self.transition_pending_with_context(
            decision_id,
            context,
            Some(option_id),
            DecisionStatus::Resolved,
            Some(option_id.to_string()),
            actor,
            DecisionEventType::DecisionResolved,
        )
    }

    pub fn deny_with_context(
        &self,
        decision_id: &str,
        context: DecisionResolutionContext,
        option_id: &str,
        actor: impl Into<String>,
    ) -> Result<Option<InteractionDecision>, DecisionStoreError> {
        self.transition_pending_with_context(
            decision_id,
            context,
            Some(option_id),
            DecisionStatus::Denied,
            Some(option_id.to_string()),
            actor,
            DecisionEventType::DecisionDenied,
        )
    }

    pub fn cancel_with_context(
        &self,
        decision_id: &str,
        context: DecisionResolutionContext,
        actor: impl Into<String>,
    ) -> Result<Option<InteractionDecision>, DecisionStoreError> {
        self.transition_pending_with_context(
            decision_id,
            context,
            None,
            DecisionStatus::Cancelled,
            Some("cancelled".to_string()),
            actor,
            DecisionEventType::DecisionCancelled,
        )
    }

    fn transition_pending_with_context(
        &self,
        decision_id: &str,
        context: DecisionResolutionContext,
        option_id: Option<&str>,
        status: DecisionStatus,
        resolution: Option<String>,
        actor: impl Into<String>,
        event_type: DecisionEventType,
    ) -> Result<Option<InteractionDecision>, DecisionStoreError> {
        let actor = actor.into();
        let decision = {
            let state = self.state.lock().unwrap();
            let Some(decision) = state.decisions.get(decision_id) else {
                return Ok(None);
            };
            decision.clone()
        };

        if decision.status != DecisionStatus::Pending {
            return Err(DecisionStoreError::NotPending {
                decision_id: decision_id.to_string(),
                status: decision.status,
            });
        }
        if is_decision_expired(&decision) {
            if let Some(expires_at) = decision.expires_at.clone() {
                let _ = self.update_status(
                    decision_id,
                    DecisionStatus::Invalidated,
                    Some("expired_before_resolution".to_string()),
                    actor.clone(),
                    DecisionEventType::DecisionInvalidated,
                );
                return Err(DecisionStoreError::DecisionExpired {
                    decision_id: decision_id.to_string(),
                    expires_at,
                });
            }
        }
        if let Some(expected_version) = context.expected_version {
            if decision.version != expected_version {
                return Err(DecisionStoreError::VersionMismatch {
                    decision_id: decision_id.to_string(),
                    expected: decision.version,
                    actual: expected_version,
                });
            }
        }
        if let Some(expected_action_hash) = context.expected_action_hash {
            if !decision.action_hash.is_empty() && decision.action_hash != expected_action_hash {
                return Err(DecisionStoreError::ActionHashMismatch {
                    decision_id: decision_id.to_string(),
                    expected: decision.action_hash,
                    actual: expected_action_hash,
                });
            }
        }
        if let Some(expected_target_hash) = context.expected_target_hash {
            if !decision.target_hash.is_empty() && decision.target_hash != expected_target_hash {
                return Err(DecisionStoreError::TargetHashMismatch {
                    decision_id: decision_id.to_string(),
                    expected: decision.target_hash,
                    actual: expected_target_hash,
                });
            }
        }
        if let Some(option_id) = option_id {
            if !decision.options.iter().any(|option| option.id == option_id) {
                return Err(DecisionStoreError::InvalidOption {
                    decision_id: decision_id.to_string(),
                    option_id: option_id.to_string(),
                });
            }
        }

        self.update_status(decision_id, status, resolution, actor, event_type)
    }

    pub fn validate_resume_context(
        &self,
        decision_id: &str,
        context: DecisionResolutionContext,
        actor: impl Into<String>,
    ) -> Result<Option<InteractionDecision>, DecisionStoreError> {
        let actor = actor.into();
        let decision = {
            let state = self.state.lock().unwrap();
            let Some(decision) = state.decisions.get(decision_id) else {
                return Ok(None);
            };
            decision.clone()
        };

        if decision.status != DecisionStatus::Resolved {
            return Err(DecisionStoreError::NotResolved {
                decision_id: decision_id.to_string(),
                status: decision.status,
            });
        }
        if is_decision_expired(&decision) {
            if let Some(expires_at) = decision.expires_at.clone() {
                let _ = self.update_status(
                    decision_id,
                    DecisionStatus::Invalidated,
                    Some("expired_before_resume".to_string()),
                    actor,
                    DecisionEventType::DecisionInvalidated,
                );
                return Err(DecisionStoreError::DecisionExpired {
                    decision_id: decision_id.to_string(),
                    expires_at,
                });
            }
        }
        if let Some(expected_version) = context.expected_version {
            if decision.version != expected_version {
                return Err(DecisionStoreError::VersionMismatch {
                    decision_id: decision_id.to_string(),
                    expected: decision.version,
                    actual: expected_version,
                });
            }
        }
        if let Some(expected_action_hash) = context.expected_action_hash {
            if !decision.action_hash.is_empty() && decision.action_hash != expected_action_hash {
                return Err(DecisionStoreError::ActionHashMismatch {
                    decision_id: decision_id.to_string(),
                    expected: decision.action_hash,
                    actual: expected_action_hash,
                });
            }
        }
        if let Some(expected_target_hash) = context.expected_target_hash {
            if !decision.target_hash.is_empty() && decision.target_hash != expected_target_hash {
                return Err(DecisionStoreError::TargetHashMismatch {
                    decision_id: decision_id.to_string(),
                    expected: decision.target_hash,
                    actual: expected_target_hash,
                });
            }
        }

        Ok(Some(decision))
    }

    pub fn claim_execution(
        &self,
        decision: &InteractionDecision,
        context: DecisionExecutionContext,
    ) -> Result<DecisionExecutionRecord, DecisionStoreError> {
        let Some(action) = decision.action_proposal.as_ref() else {
            return Err(DecisionStoreError::MissingActionProposal {
                decision_id: decision.id.clone(),
            });
        };
        let key = execution_key(&decision.id, &decision.action_hash);
        let mut state = self.state.lock().unwrap();
        if let Some(existing) = state.executions.get(&key) {
            if existing.state != DecisionExecutionState::BlockedByLease {
                return Err(DecisionStoreError::ExecutionAlreadyExists {
                    decision_id: decision.id.clone(),
                    state: existing.state,
                });
            }
        }

        let now = now_rfc3339();
        let record = DecisionExecutionRecord {
            execution_id: uuid::Uuid::new_v4().to_string(),
            decision_id: decision.id.clone(),
            workflow_id: decision.workflow_id.clone(),
            action_hash: decision.action_hash.clone(),
            target_hash: decision.target_hash.clone(),
            state: DecisionExecutionState::Preparing,
            sequence: next_execution_sequence(&state, &decision.id),
            execution_actor: context.execution_actor,
            source_command: context.source_command,
            session_id: context.session_id,
            workspace_id: context.workspace_id,
            tool_name: action.tool_name.clone(),
            tool_schema_version: action.tool_schema_version.clone(),
            tool_registry_version: action.tool_registry_version.clone(),
            policy_version: "policy.v1".to_string(),
            started_at: now.clone(),
            side_effect_started_at: None,
            completed_at: None,
            gate_summary: None,
            grounding_summary: None,
            lease_refs: Vec::new(),
            redacted_tool_result: None,
            error_class: None,
            error_message: None,
        };
        let event = DecisionEvent {
            event_id: uuid::Uuid::new_v4().to_string(),
            decision_id: decision.id.clone(),
            workflow_id: decision.workflow_id.clone(),
            stage_id: decision.stage_id.clone(),
            event_type: DecisionEventType::DecisionExecutionClaimed,
            actor: "resume_executor".to_string(),
            authority: AuthorityLevel::WorkflowSemantics,
            payload: serde_json::to_value(&record)?,
            created_at: now,
            policy_version: "policy.v1".to_string(),
            runtime_version: env!("CARGO_PKG_VERSION").to_string(),
        };
        self.append_event_locked(&mut state, event)?;
        Ok(record)
    }

    pub fn execution_record(
        &self,
        decision_id: &str,
        action_hash: &str,
    ) -> Option<DecisionExecutionRecord> {
        let key = execution_key(decision_id, action_hash);
        self.state.lock().unwrap().executions.get(&key).cloned()
    }

    pub fn update_execution_state(
        &self,
        decision_id: &str,
        action_hash: &str,
        new_state: DecisionExecutionState,
        update: DecisionExecutionUpdate,
        actor: impl Into<String>,
    ) -> Result<DecisionExecutionRecord, DecisionStoreError> {
        let actor = actor.into();
        let key = execution_key(decision_id, action_hash);
        let mut state = self.state.lock().unwrap();
        let Some(existing) = state.executions.get(&key).cloned() else {
            return Err(DecisionStoreError::ExecutionMissing {
                decision_id: decision_id.to_string(),
            });
        };

        let now = now_rfc3339();
        let mut record = existing;
        record.state = new_state;
        if update.side_effect_started && record.side_effect_started_at.is_none() {
            record.side_effect_started_at = Some(now.clone());
        }
        if update.completed {
            record.completed_at = Some(now.clone());
        }
        if let Some(gate_summary) = update.gate_summary {
            record.gate_summary = Some(gate_summary);
        }
        if let Some(grounding_summary) = update.grounding_summary {
            record.grounding_summary = Some(grounding_summary);
        }
        if let Some(lease_refs) = update.lease_refs {
            record.lease_refs = lease_refs;
        }
        if let Some(redacted_tool_result) = update.redacted_tool_result {
            record.redacted_tool_result = Some(redacted_tool_result);
        }
        if let Some(error_class) = update.error_class {
            record.error_class = Some(error_class);
        }
        if let Some(error_message) = update.error_message {
            record.error_message = Some(error_message);
        }

        let event_type = match new_state {
            DecisionExecutionState::Executing => DecisionEventType::DecisionExecutionStarted,
            DecisionExecutionState::Executed => DecisionEventType::DecisionExecutionCompleted,
            DecisionExecutionState::Cancelled => DecisionEventType::DecisionExecutionCancelled,
            DecisionExecutionState::Invalidated => DecisionEventType::DecisionExecutionInvalidated,
            DecisionExecutionState::UnknownAfterCrash => {
                DecisionEventType::DecisionExecutionUnknownAfterCrash
            }
            DecisionExecutionState::BlockedByLease => {
                DecisionEventType::DecisionExecutionBlockedByLease
            }
            DecisionExecutionState::Failed
            | DecisionExecutionState::Preparing
            | DecisionExecutionState::NotStarted => DecisionEventType::DecisionExecutionFailed,
        };
        let event = DecisionEvent {
            event_id: uuid::Uuid::new_v4().to_string(),
            decision_id: decision_id.to_string(),
            workflow_id: record.workflow_id.clone(),
            stage_id: None,
            event_type,
            actor,
            authority: AuthorityLevel::WorkflowSemantics,
            payload: serde_json::to_value(&record)?,
            created_at: now,
            policy_version: "policy.v1".to_string(),
            runtime_version: env!("CARGO_PKG_VERSION").to_string(),
        };
        self.append_event_locked(&mut state, event)?;
        Ok(record)
    }

    pub fn claim_continuation(
        &self,
        decision: &InteractionDecision,
        execution: &DecisionExecutionRecord,
        binding: &DecisionStageBinding,
        actor: impl Into<String>,
    ) -> Result<ContinuationClaim, DecisionStoreError> {
        let actor = actor.into();
        let key = continuation_key(
            &decision.id,
            &execution.execution_id,
            &binding.checkpoint_id,
            &binding.action_hash,
        );
        let mut state = self.state.lock().unwrap();
        if let Some(existing) = state.continuations.get(&key) {
            return Err(DecisionStoreError::ContinuationAlreadyExists {
                decision_id: decision.id.clone(),
                state: existing.state,
            });
        }

        let now = now_rfc3339();
        let claim = ContinuationClaim {
            claim_id: uuid::Uuid::new_v4().to_string(),
            decision_id: decision.id.clone(),
            execution_id: execution.execution_id.clone(),
            workflow_id: decision.workflow_id.clone(),
            checkpoint_id: binding.checkpoint_id.clone(),
            action_hash: binding.action_hash.clone(),
            target_hash: binding.target_hash.clone(),
            state: ContinuationState::VerifyingPriorAction,
            sequence: next_continuation_sequence(&state, &decision.id),
            actor,
            started_at: now.clone(),
            side_effect_started_at: None,
            completed_at: None,
            verification_id: None,
            error_class: None,
            error_message: None,
        };
        let event = DecisionEvent {
            event_id: uuid::Uuid::new_v4().to_string(),
            decision_id: decision.id.clone(),
            workflow_id: decision.workflow_id.clone(),
            stage_id: decision.stage_id.clone(),
            event_type: DecisionEventType::ContinuationClaimed,
            actor: "continuation_reentry".to_string(),
            authority: AuthorityLevel::WorkflowSemantics,
            payload: serde_json::to_value(&claim)?,
            created_at: now,
            policy_version: "policy.v1".to_string(),
            runtime_version: env!("CARGO_PKG_VERSION").to_string(),
        };
        self.append_event_locked(&mut state, event)?;
        Ok(claim)
    }

    pub fn continuation_record(
        &self,
        decision_id: &str,
        execution_id: &str,
        checkpoint_id: &str,
        action_hash: &str,
    ) -> Option<ContinuationClaim> {
        let key = continuation_key(decision_id, execution_id, checkpoint_id, action_hash);
        self.state.lock().unwrap().continuations.get(&key).cloned()
    }

    pub fn update_continuation_state(
        &self,
        claim: &ContinuationClaim,
        new_state: ContinuationState,
        update: ContinuationUpdate,
        actor: impl Into<String>,
    ) -> Result<ContinuationClaim, DecisionStoreError> {
        let actor = actor.into();
        let key = continuation_key(
            &claim.decision_id,
            &claim.execution_id,
            &claim.checkpoint_id,
            &claim.action_hash,
        );
        let mut state = self.state.lock().unwrap();
        let Some(existing) = state.continuations.get(&key).cloned() else {
            return Err(DecisionStoreError::ContinuationMissing {
                decision_id: claim.decision_id.clone(),
            });
        };

        let now = now_rfc3339();
        let mut record = existing;
        record.state = new_state;
        if update.side_effect_started && record.side_effect_started_at.is_none() {
            record.side_effect_started_at = Some(now.clone());
        }
        if update.completed {
            record.completed_at = Some(now.clone());
        }
        if let Some(verification_id) = update.verification_id {
            record.verification_id = Some(verification_id);
        }
        if let Some(error_class) = update.error_class {
            record.error_class = Some(error_class);
        }
        if let Some(error_message) = update.error_message {
            record.error_message = Some(error_message);
        }

        let event = DecisionEvent {
            event_id: uuid::Uuid::new_v4().to_string(),
            decision_id: record.decision_id.clone(),
            workflow_id: record.workflow_id.clone(),
            stage_id: None,
            event_type: continuation_event_type(new_state),
            actor,
            authority: AuthorityLevel::WorkflowSemantics,
            payload: serde_json::to_value(&record)?,
            created_at: now,
            policy_version: "policy.v1".to_string(),
            runtime_version: env!("CARGO_PKG_VERSION").to_string(),
        };
        self.append_event_locked(&mut state, event)?;
        Ok(record)
    }

    pub fn record_post_decision_verification(
        &self,
        verification: PostDecisionVerification,
        actor: impl Into<String>,
    ) -> Result<PostDecisionVerification, DecisionStoreError> {
        let event = DecisionEvent {
            event_id: uuid::Uuid::new_v4().to_string(),
            decision_id: verification.decision_id.clone(),
            workflow_id: verification.workflow_id.clone(),
            stage_id: None,
            event_type: DecisionEventType::EvidenceObserved,
            actor: actor.into(),
            authority: AuthorityLevel::VerifierTruth,
            payload: serde_json::to_value(&verification)?,
            created_at: verification.created_at.clone(),
            policy_version: "policy.v1".to_string(),
            runtime_version: env!("CARGO_PKG_VERSION").to_string(),
        };
        self.append_event(event)?;
        Ok(verification)
    }

    pub fn expire(
        &self,
        decision_id: &str,
        actor: impl Into<String>,
    ) -> Result<Option<InteractionDecision>, DecisionStoreError> {
        self.update_status(
            decision_id,
            DecisionStatus::Expired,
            None,
            actor.into(),
            DecisionEventType::DecisionExpired,
        )
    }

    pub fn invalidate(
        &self,
        decision_id: &str,
        reason: impl Into<String>,
        actor: impl Into<String>,
    ) -> Result<Option<InteractionDecision>, DecisionStoreError> {
        self.update_status(
            decision_id,
            DecisionStatus::Invalidated,
            Some(reason.into()),
            actor.into(),
            DecisionEventType::DecisionInvalidated,
        )
    }

    pub fn pending_decisions(&self) -> Vec<InteractionDecision> {
        let state = self.state.lock().unwrap();
        state
            .decisions
            .values()
            .filter(|decision| decision.status == DecisionStatus::Pending)
            .cloned()
            .collect()
    }

    pub fn decision(&self, decision_id: &str) -> Option<InteractionDecision> {
        let state = self.state.lock().unwrap();
        state
            .decisions
            .get(decision_id)
            .cloned()
            .map(|mut decision| {
                decision.execution = state
                    .executions
                    .get(&execution_key(&decision.id, &decision.action_hash))
                    .cloned();
                decision.continuation = latest_continuation_for_decision(&state, &decision.id);
                decision.verification = latest_verification_for_decision(&state, &decision.id);
                decision
            })
    }

    pub fn all_decisions(&self) -> Vec<InteractionDecision> {
        let state = self.state.lock().unwrap();
        let mut decisions: Vec<_> = state
            .decisions
            .values()
            .cloned()
            .map(|mut decision| {
                decision.execution = state
                    .executions
                    .get(&execution_key(&decision.id, &decision.action_hash))
                    .cloned();
                decision.continuation = latest_continuation_for_decision(&state, &decision.id);
                decision.verification = latest_verification_for_decision(&state, &decision.id);
                decision
            })
            .collect();
        decisions.sort_by(|a, b| b.updated_at.cmp(&a.updated_at));
        decisions
    }

    pub fn events(&self) -> Vec<DecisionEvent> {
        self.state.lock().unwrap().events.clone()
    }

    pub fn refresh_from_disk(&self) -> Result<(), DecisionStoreError> {
        let Some(_) = &self.path else {
            return Ok(());
        };
        {
            let mut state = self.state.lock().unwrap();
            state.decisions.clear();
            state.executions.clear();
            state.continuations.clear();
            state.verifications.clear();
            state.events.clear();
        }
        self.replay_from_disk()
    }

    pub fn metrics(&self) -> DecisionMetrics {
        let state = self.state.lock().unwrap();
        let mut metrics = DecisionMetrics {
            total_events: state.events.len(),
            ..DecisionMetrics::default()
        };

        for decision in state.decisions.values() {
            match decision.status {
                DecisionStatus::Pending => metrics.pending_decisions += 1,
                DecisionStatus::Resolved => metrics.resolved_decisions += 1,
                DecisionStatus::Deferred => {}
                DecisionStatus::Expired => metrics.expired_decisions += 1,
                DecisionStatus::Invalidated => metrics.invalidated_decisions += 1,
                DecisionStatus::Denied | DecisionStatus::Cancelled => {}
            }

            match decision.decision_type {
                DecisionType::Approval => metrics.approval_decisions += 1,
                DecisionType::TargetSelection => metrics.target_selection_decisions += 1,
                DecisionType::ScopeClarification
                | DecisionType::RecoveryChoice
                | DecisionType::CredentialRequired
                | DecisionType::VerifierConflict
                | DecisionType::UnsafeUncertainty => {}
            }

            if decision.recommended_option.is_none()
                && matches!(
                    decision.confidence,
                    ConfidenceBand::Low | ConfidenceBand::Conflicted | ConfidenceBand::Unknown
                )
            {
                metrics.unsafe_abstentions += 1;
            }
        }

        metrics
    }

    fn update_status(
        &self,
        decision_id: &str,
        status: DecisionStatus,
        resolution: Option<String>,
        actor: String,
        event_type: DecisionEventType,
    ) -> Result<Option<InteractionDecision>, DecisionStoreError> {
        let maybe_decision = {
            let mut state = self.state.lock().unwrap();
            let Some(decision) = state.decisions.get_mut(decision_id) else {
                return Ok(None);
            };
            decision.status = status;
            decision.version = decision.version.saturating_add(1);
            decision.updated_at = now_rfc3339();
            decision.resolution = resolution;
            decision.clone()
        };

        let event = DecisionEvent {
            event_id: uuid::Uuid::new_v4().to_string(),
            decision_id: maybe_decision.id.clone(),
            workflow_id: maybe_decision.workflow_id.clone(),
            stage_id: maybe_decision.stage_id.clone(),
            event_type,
            actor,
            authority: AuthorityLevel::UserInstruction,
            payload: serde_json::to_value(&maybe_decision)?,
            created_at: maybe_decision.updated_at.clone(),
            policy_version: "policy.v1".to_string(),
            runtime_version: env!("CARGO_PKG_VERSION").to_string(),
        };
        self.append_event(event)?;
        Ok(Some(maybe_decision))
    }

    fn append_event(&self, event: DecisionEvent) -> Result<(), DecisionStoreError> {
        let mut state = self.state.lock().unwrap();
        self.append_event_locked(&mut state, event)
    }

    fn append_event_locked(
        &self,
        state: &mut DecisionStoreState,
        event: DecisionEvent,
    ) -> Result<(), DecisionStoreError> {
        if let Some(path) = &self.path {
            append_jsonl(path, &event)?;
        }

        apply_event(state, &event);
        state.events.push(event);
        Ok(())
    }

    fn replay_from_disk(&self) -> Result<(), DecisionStoreError> {
        let Some(path) = &self.path else {
            return Ok(());
        };
        if !path.exists() {
            return Ok(());
        }

        let file = File::open(path)?;
        let reader = BufReader::new(file);
        let mut state = self.state.lock().unwrap();
        for line in reader.lines() {
            let line = line?;
            if line.trim().is_empty() {
                continue;
            }
            let event: DecisionEvent = serde_json::from_str(&line)?;
            apply_event(&mut state, &event);
            state.events.push(event);
        }
        let unknown_after_crash = state
            .executions
            .values()
            .filter(|record| {
                matches!(
                    record.state,
                    DecisionExecutionState::Preparing | DecisionExecutionState::Executing
                )
            })
            .cloned()
            .collect::<Vec<_>>();
        let continuation_unknown_after_crash = state
            .continuations
            .values()
            .filter(|claim| !claim.state.terminal())
            .cloned()
            .collect::<Vec<_>>();
        drop(state);

        for mut record in unknown_after_crash {
            record.state = DecisionExecutionState::UnknownAfterCrash;
            record.completed_at = Some(now_rfc3339());
            record.error_class = Some("UnknownAfterCrash".to_string());
            record.error_message =
                Some("runtime restarted while decision execution was in-flight".to_string());
            let event = DecisionEvent {
                event_id: uuid::Uuid::new_v4().to_string(),
                decision_id: record.decision_id.clone(),
                workflow_id: record.workflow_id.clone(),
                stage_id: None,
                event_type: DecisionEventType::DecisionExecutionUnknownAfterCrash,
                actor: "decision_store_replay".to_string(),
                authority: AuthorityLevel::WorkflowSemantics,
                payload: serde_json::to_value(&record)?,
                created_at: now_rfc3339(),
                policy_version: "policy.v1".to_string(),
                runtime_version: env!("CARGO_PKG_VERSION").to_string(),
            };
            self.append_event(event)?;
        }
        for mut claim in continuation_unknown_after_crash {
            claim.state = ContinuationState::UnknownAfterCrash;
            claim.completed_at = Some(now_rfc3339());
            claim.error_class = Some("UnknownAfterCrash".to_string());
            claim.error_message =
                Some("runtime restarted while continuation was in-flight".to_string());
            let event = DecisionEvent {
                event_id: uuid::Uuid::new_v4().to_string(),
                decision_id: claim.decision_id.clone(),
                workflow_id: claim.workflow_id.clone(),
                stage_id: None,
                event_type: DecisionEventType::ContinuationUnknownAfterCrash,
                actor: "decision_store_replay".to_string(),
                authority: AuthorityLevel::WorkflowSemantics,
                payload: serde_json::to_value(&claim)?,
                created_at: now_rfc3339(),
                policy_version: "policy.v1".to_string(),
                runtime_version: env!("CARGO_PKG_VERSION").to_string(),
            };
            self.append_event(event)?;
        }
        Ok(())
    }
}

fn apply_event(state: &mut DecisionStoreState, event: &DecisionEvent) {
    match event.event_type {
        DecisionEventType::DecisionCreated
        | DecisionEventType::DecisionResolved
        | DecisionEventType::DecisionExpired
        | DecisionEventType::DecisionInvalidated
        | DecisionEventType::DecisionDenied
        | DecisionEventType::DecisionCancelled => {
            if let Ok(decision) =
                serde_json::from_value::<InteractionDecision>(event.payload.clone())
            {
                state.decisions.insert(decision.id.clone(), decision);
            }
        }
        DecisionEventType::DecisionExecutionClaimed
        | DecisionEventType::DecisionExecutionStarted
        | DecisionEventType::DecisionExecutionCompleted
        | DecisionEventType::DecisionExecutionFailed
        | DecisionEventType::DecisionExecutionCancelled
        | DecisionEventType::DecisionExecutionInvalidated
        | DecisionEventType::DecisionExecutionUnknownAfterCrash
        | DecisionEventType::DecisionExecutionBlockedByLease => {
            if let Ok(record) =
                serde_json::from_value::<DecisionExecutionRecord>(event.payload.clone())
            {
                state.executions.insert(
                    execution_key(&record.decision_id, &record.action_hash),
                    record,
                );
            }
        }
        DecisionEventType::ContinuationClaimed
        | DecisionEventType::ContinuationVerificationStarted
        | DecisionEventType::ContinuationVerificationCompleted
        | DecisionEventType::ContinuationActionMarkedComplete
        | DecisionEventType::ContinuationNextStepReady
        | DecisionEventType::ContinuationNextStepStarted
        | DecisionEventType::ContinuationPausedAgain
        | DecisionEventType::ContinuationCompletedOneStep
        | DecisionEventType::ContinuationFailed
        | DecisionEventType::ContinuationCancelled
        | DecisionEventType::ContinuationUnknownAfterCrash
        | DecisionEventType::ContinuationInvalidated => {
            if let Ok(claim) = serde_json::from_value::<ContinuationClaim>(event.payload.clone()) {
                state.continuations.insert(
                    continuation_key(
                        &claim.decision_id,
                        &claim.execution_id,
                        &claim.checkpoint_id,
                        &claim.action_hash,
                    ),
                    claim,
                );
            }
        }
        DecisionEventType::EvidenceObserved => {
            if let Ok(verification) =
                serde_json::from_value::<PostDecisionVerification>(event.payload.clone())
            {
                state
                    .verifications
                    .insert(verification.verification_id.clone(), verification);
            }
        }
    }
}

fn append_jsonl(path: &Path, event: &DecisionEvent) -> Result<(), DecisionStoreError> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut file = OpenOptions::new().create(true).append(true).open(path)?;
    serde_json::to_writer(&mut file, event)?;
    file.write_all(b"\n")?;
    Ok(())
}

fn default_decision_log_path() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".kria")
        .join("decision_events.jsonl")
}

fn now_rfc3339() -> String {
    chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
}

fn default_expires_at_rfc3339() -> String {
    (chrono::Utc::now() + chrono::Duration::hours(DEFAULT_DECISION_TTL_HOURS))
        .to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
}

fn is_decision_expired(decision: &InteractionDecision) -> bool {
    let Some(expires_at) = decision.expires_at.as_deref() else {
        return false;
    };
    chrono::DateTime::parse_from_rfc3339(expires_at)
        .map(|expires_at| expires_at.with_timezone(&chrono::Utc) <= chrono::Utc::now())
        .unwrap_or(false)
}

fn default_decision_version() -> u64 {
    1
}

fn default_tool_schema_version() -> String {
    env!("CARGO_PKG_VERSION").to_string()
}

fn default_tool_registry_version() -> String {
    env!("CARGO_PKG_VERSION").to_string()
}

fn default_workflow_schema_version() -> String {
    env!("CARGO_PKG_VERSION").to_string()
}

fn default_max_side_effect_count() -> u8 {
    1
}

fn default_true() -> bool {
    true
}

pub fn current_tool_schema_version() -> String {
    default_tool_schema_version()
}

pub fn current_tool_registry_version() -> String {
    default_tool_registry_version()
}

fn expected_artifacts_from_action(action: &ActionProposal) -> Vec<String> {
    let mut artifacts = Vec::new();
    if matches!(
        action.tool_name.as_str(),
        "write_file" | "append_file" | "read_file" | "delete_file"
    ) {
        if let Some(path) = action
            .parameters
            .get("path")
            .and_then(|value| value.as_str())
        {
            artifacts.push(path.to_string());
        }
    }
    if action.tool_name == "move_file" {
        if let Some(path) = action
            .parameters
            .get("destination")
            .or_else(|| action.parameters.get("target"))
            .and_then(|value| value.as_str())
        {
            artifacts.push(path.to_string());
        }
    }
    if matches!(
        action.tool_name.as_str(),
        "execute_bash" | "execute_shell" | "run_command"
    ) {
        for key in ["expected_output_path", "output_path", "artifact_path"] {
            if let Some(path) = action.parameters.get(key).and_then(|value| value.as_str()) {
                artifacts.push(path.to_string());
            }
        }
    }
    artifacts
}

fn verifier_requirements_from_action(action: &ActionProposal) -> Vec<String> {
    match action.tool_name.as_str() {
        "write_file" | "append_file" => vec!["filesystem_path_exists".to_string()],
        "delete_file" => vec!["filesystem_path_absent".to_string()],
        "move_file" => vec!["destination_path_exists".to_string()],
        "execute_bash" | "execute_shell" | "run_command" => {
            if expected_artifacts_from_action(action).is_empty() {
                vec!["durable_artifact_required".to_string()]
            } else {
                vec!["expected_artifact_exists".to_string()]
            }
        }
        _ => vec!["tool_result_success_and_local_deterministic".to_string()],
    }
}

fn execution_key(decision_id: &str, action_hash: &str) -> String {
    format!("{decision_id}:{action_hash}")
}

fn continuation_key(
    decision_id: &str,
    execution_id: &str,
    checkpoint_id: &str,
    action_hash: &str,
) -> String {
    format!("{decision_id}:{execution_id}:{checkpoint_id}:{action_hash}")
}

fn next_execution_sequence(state: &DecisionStoreState, decision_id: &str) -> u64 {
    state
        .executions
        .values()
        .filter(|record| record.decision_id == decision_id)
        .map(|record| record.sequence)
        .max()
        .unwrap_or(0)
        + 1
}

fn next_continuation_sequence(state: &DecisionStoreState, decision_id: &str) -> u64 {
    state
        .continuations
        .values()
        .filter(|claim| claim.decision_id == decision_id)
        .map(|claim| claim.sequence)
        .max()
        .unwrap_or(0)
        + 1
}

fn continuation_event_type(state: ContinuationState) -> DecisionEventType {
    match state {
        ContinuationState::VerifyingPriorAction => {
            DecisionEventType::ContinuationVerificationStarted
        }
        ContinuationState::VerifiedPriorAction => {
            DecisionEventType::ContinuationVerificationCompleted
        }
        ContinuationState::AdvancingActionState => {
            DecisionEventType::ContinuationActionMarkedComplete
        }
        ContinuationState::ReadyForNextSafeStep => DecisionEventType::ContinuationNextStepReady,
        ContinuationState::ExecutingNextSafeStep => DecisionEventType::ContinuationNextStepStarted,
        ContinuationState::PausedAgain => DecisionEventType::ContinuationPausedAgain,
        ContinuationState::CompletedOneStep => DecisionEventType::ContinuationCompletedOneStep,
        ContinuationState::Failed => DecisionEventType::ContinuationFailed,
        ContinuationState::Cancelled => DecisionEventType::ContinuationCancelled,
        ContinuationState::UnknownAfterCrash => DecisionEventType::ContinuationUnknownAfterCrash,
        ContinuationState::Invalidated | ContinuationState::NotStarted => {
            DecisionEventType::ContinuationInvalidated
        }
    }
}

fn latest_continuation_for_decision(
    state: &DecisionStoreState,
    decision_id: &str,
) -> Option<ContinuationClaim> {
    state
        .continuations
        .values()
        .filter(|claim| claim.decision_id == decision_id)
        .max_by_key(|claim| claim.sequence)
        .cloned()
}

fn latest_verification_for_decision(
    state: &DecisionStoreState,
    decision_id: &str,
) -> Option<PostDecisionVerification> {
    state
        .verifications
        .values()
        .filter(|verification| verification.decision_id == decision_id)
        .max_by(|a, b| a.created_at.cmp(&b.created_at))
        .cloned()
}

pub fn compute_target_hash(target: &TargetBinding) -> TargetHash {
    stable_hash_json(&serde_json::to_value(target).unwrap_or_else(|_| serde_json::json!({})))
}

pub fn compute_action_hash(
    workflow_id: &str,
    attempt_id: &str,
    stage_id: &str,
    tool_name: &str,
    parameters: &serde_json::Value,
    target_hash: &str,
    tool_schema_version: &str,
    tool_registry_version: &str,
) -> ActionHash {
    stable_hash_json(&serde_json::json!({
        "workflow_id": workflow_id,
        "attempt_id": attempt_id,
        "stage_id": stage_id,
        "tool_name": tool_name,
        "parameters": parameters,
        "target_hash": target_hash,
        "tool_schema_version": tool_schema_version,
        "tool_registry_version": tool_registry_version,
    }))
}

fn stable_hash_json(value: &serde_json::Value) -> String {
    let canonical = canonical_json(value);
    blake3::hash(canonical.as_bytes()).to_hex().to_string()
}

fn canonical_json(value: &serde_json::Value) -> String {
    match value {
        serde_json::Value::Null => "null".to_string(),
        serde_json::Value::Bool(v) => v.to_string(),
        serde_json::Value::Number(v) => v.to_string(),
        serde_json::Value::String(v) => serde_json::to_string(v).unwrap_or_default(),
        serde_json::Value::Array(items) => {
            let inner = items
                .iter()
                .map(canonical_json)
                .collect::<Vec<_>>()
                .join(",");
            format!("[{inner}]")
        }
        serde_json::Value::Object(map) => {
            let mut entries = map.iter().collect::<Vec<_>>();
            entries.sort_by(|a, b| a.0.cmp(b.0));
            let inner = entries
                .into_iter()
                .map(|(key, value)| {
                    format!(
                        "{}:{}",
                        serde_json::to_string(key).unwrap_or_default(),
                        canonical_json(value)
                    )
                })
                .collect::<Vec<_>>()
                .join(",");
            format!("{{{inner}}}")
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn aggregator_prefers_higher_authority_for_duplicate_decisions() {
        let low = DecisionCandidate {
            authority: AuthorityLevel::PlannerRecommendation,
            ..DecisionCandidate::target_selection(
                "where?",
                vec!["host".to_string(), "vm".to_string()],
                "execute_bash",
            )
        };
        let high = DecisionCandidate {
            authority: AuthorityLevel::ExecutionAuthority,
            ..DecisionCandidate::target_selection(
                "where?",
                vec!["host".to_string(), "vm".to_string()],
                "execute_bash",
            )
        };

        let out = DecisionAggregator.aggregate(vec![low, high]);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].authority, AuthorityLevel::ExecutionAuthority);
    }

    #[test]
    fn store_persists_and_replays_pending_decision() {
        let path = std::env::temp_dir().join(format!(
            "kria-decision-store-{}.jsonl",
            uuid::Uuid::new_v4()
        ));
        let store = DecisionStore::persistent(&path).expect("store should open");
        let decision = store
            .create_decision(
                "workflow-a",
                Some("stage-1".to_string()),
                DecisionCandidate::target_selection(
                    "Select execution target",
                    vec!["host".to_string(), "vm".to_string()],
                    "execute_bash",
                ),
            )
            .expect("decision should persist");

        let replayed = DecisionStore::persistent(&path).expect("store should replay");
        assert_eq!(replayed.pending_decisions().len(), 1);
        assert_eq!(replayed.pending_decisions()[0].id, decision.id);

        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn store_metrics_and_events_track_resolution() {
        let store = DecisionStore::in_memory();
        let decision = store
            .create_decision(
                "workflow-a",
                Some("stage-1".to_string()),
                DecisionCandidate::target_selection(
                    "Select execution target",
                    vec!["host".to_string(), "vm".to_string()],
                    "execute_bash",
                ),
            )
            .expect("decision should be created");

        store
            .resolve(&decision.id, "host", "test")
            .expect("decision should resolve");

        let metrics = store.metrics();
        assert_eq!(metrics.total_events, 2);
        assert_eq!(metrics.pending_decisions, 0);
        assert_eq!(metrics.resolved_decisions, 1);
        assert_eq!(store.events().len(), 2);
    }

    #[test]
    fn deny_and_cancel_reach_distinct_terminal_statuses() {
        let store = DecisionStore::in_memory();
        let denied = store
            .create_decision(
                "workflow-a",
                Some("stage-1".to_string()),
                DecisionCandidate::approval(
                    "execute_bash",
                    "approval required",
                    RiskLevel::Red,
                    Rollbackability::Unknown,
                    vec!["command:echo safe".to_string()],
                    Some("policy.command_classifier".to_string()),
                ),
            )
            .expect("decision should be created");
        let cancelled = store
            .create_decision(
                "workflow-b",
                Some("stage-1".to_string()),
                DecisionCandidate::approval(
                    "execute_bash",
                    "approval required",
                    RiskLevel::Red,
                    Rollbackability::Unknown,
                    vec!["command:echo safe".to_string()],
                    Some("policy.command_classifier".to_string()),
                ),
            )
            .expect("decision should be created");

        store
            .deny_with_context(
                &denied.id,
                DecisionResolutionContext {
                    expected_version: Some(denied.version),
                    ..DecisionResolutionContext::default()
                },
                "deny",
                "test",
            )
            .expect("decision should deny");
        store
            .cancel_with_context(
                &cancelled.id,
                DecisionResolutionContext {
                    expected_version: Some(cancelled.version),
                    ..DecisionResolutionContext::default()
                },
                "test",
            )
            .expect("decision should cancel");

        assert_eq!(
            store.decision(&denied.id).expect("denied exists").status,
            DecisionStatus::Denied
        );
        assert_eq!(
            store
                .decision(&cancelled.id)
                .expect("cancelled exists")
                .status,
            DecisionStatus::Cancelled
        );
        assert!(matches!(
            store
                .validate_resume_context(&denied.id, DecisionResolutionContext::default(), "test")
                .expect_err("denied decision must not resume"),
            DecisionStoreError::NotResolved { .. }
        ));
    }

    #[test]
    fn action_hash_is_stable_for_json_key_order() {
        let target = TargetBinding::new("host", "local");
        let a = ActionProposal::new(
            "workflow-a",
            "attempt-1",
            "stage-1",
            "execute_bash",
            serde_json::json!({"b": 2, "a": 1}),
            target.clone(),
            Actor::Runtime,
        );
        let b = ActionProposal::new(
            "workflow-a",
            "attempt-1",
            "stage-1",
            "execute_bash",
            serde_json::json!({"a": 1, "b": 2}),
            target,
            Actor::Runtime,
        );

        assert_eq!(a.action_hash, b.action_hash);
        assert_eq!(a.target_hash, b.target_hash);
    }

    #[test]
    fn action_hash_changes_when_parameters_change() {
        let target = TargetBinding::new("host", "local");
        let a = ActionProposal::new(
            "workflow-a",
            "attempt-1",
            "stage-1",
            "execute_bash",
            serde_json::json!({"command": "pwd"}),
            target.clone(),
            Actor::Runtime,
        );
        let b = ActionProposal::new(
            "workflow-a",
            "attempt-1",
            "stage-1",
            "execute_bash",
            serde_json::json!({"command": "rm -rf /tmp/example"}),
            target,
            Actor::Runtime,
        );

        assert_ne!(a.action_hash, b.action_hash);
        assert_eq!(a.target_hash, b.target_hash);
    }

    #[test]
    fn versioned_resolution_rejects_stale_version() {
        let store = DecisionStore::in_memory();
        let action = ActionProposal::new(
            "workflow-a",
            "attempt-1",
            "stage-1",
            "execute_bash",
            serde_json::json!({"command": "pwd"}),
            TargetBinding::new("host", "local"),
            Actor::Runtime,
        );
        let decision = store
            .create_decision_for_action(
                &action,
                DecisionCandidate::target_selection(
                    "Select execution target",
                    vec!["host".to_string(), "vm".to_string()],
                    "execute_bash",
                ),
            )
            .expect("decision should be created");

        let err = store
            .resolve_with_version(&decision.id, decision.version + 1, "host", "test")
            .expect_err("stale version should be rejected");

        assert!(matches!(err, DecisionStoreError::VersionMismatch { .. }));
    }

    #[test]
    fn resolution_rejects_stale_action_hash() {
        let store = DecisionStore::in_memory();
        let action = ActionProposal::new(
            "workflow-a",
            "attempt-1",
            "stage-1",
            "execute_bash",
            serde_json::json!({"command": "pwd"}),
            TargetBinding::new("host", "local"),
            Actor::Runtime,
        );
        let decision = store
            .create_decision_for_action(
                &action,
                DecisionCandidate::target_selection(
                    "Select execution target",
                    vec!["host".to_string(), "vm".to_string()],
                    "execute_bash",
                ),
            )
            .expect("decision should be created");

        let err = store
            .resolve_with_context(
                &decision.id,
                DecisionResolutionContext {
                    expected_version: Some(decision.version),
                    expected_action_hash: Some("stale-action-hash".to_string()),
                    expected_target_hash: Some(decision.target_hash.clone()),
                },
                "host",
                "test",
            )
            .expect_err("stale action hash should be rejected");

        assert!(matches!(err, DecisionStoreError::ActionHashMismatch { .. }));
    }

    #[test]
    fn resolution_invalidates_expired_decision() {
        let store = DecisionStore::in_memory();
        let decision = store
            .create_decision(
                "workflow-a",
                Some("stage-1".to_string()),
                DecisionCandidate::target_selection(
                    "Select execution target",
                    vec!["host".to_string(), "vm".to_string()],
                    "execute_bash",
                ),
            )
            .expect("decision should be created");

        {
            let mut state = store.state.lock().unwrap();
            let decision = state
                .decisions
                .get_mut(&decision.id)
                .expect("decision should exist");
            decision.expires_at = Some(
                (chrono::Utc::now() - chrono::Duration::minutes(1))
                    .to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
            );
        }

        let err = store
            .resolve_with_context(
                &decision.id,
                DecisionResolutionContext {
                    expected_version: Some(decision.version),
                    ..DecisionResolutionContext::default()
                },
                "host",
                "test",
            )
            .expect_err("expired decision should be rejected");

        assert!(matches!(err, DecisionStoreError::DecisionExpired { .. }));
        assert_eq!(
            store
                .decision(&decision.id)
                .expect("decision exists")
                .status,
            DecisionStatus::Invalidated
        );
    }

    #[test]
    fn resume_context_requires_resolved_decision() {
        let store = DecisionStore::in_memory();
        let decision = store
            .create_decision(
                "workflow-a",
                Some("stage-1".to_string()),
                DecisionCandidate::target_selection(
                    "Select execution target",
                    vec!["host".to_string(), "vm".to_string()],
                    "execute_bash",
                ),
            )
            .expect("decision should be created");

        let err = store
            .validate_resume_context(
                &decision.id,
                DecisionResolutionContext {
                    expected_version: Some(decision.version),
                    ..DecisionResolutionContext::default()
                },
                "test",
            )
            .expect_err("pending decision must not resume");

        assert!(matches!(err, DecisionStoreError::NotResolved { .. }));
    }

    #[test]
    fn resume_context_rejects_stale_target_hash() {
        let store = DecisionStore::in_memory();
        let action = ActionProposal::new(
            "workflow-a",
            "attempt-1",
            "stage-1",
            "execute_bash",
            serde_json::json!({"command": "pwd"}),
            TargetBinding::new("host", "local"),
            Actor::Runtime,
        );
        let decision = store
            .create_decision_for_action(
                &action,
                DecisionCandidate::target_selection(
                    "Select execution target",
                    vec!["host".to_string(), "vm".to_string()],
                    "execute_bash",
                ),
            )
            .expect("decision should be created");
        let resolved = store
            .resolve_with_context(
                &decision.id,
                DecisionResolutionContext {
                    expected_version: Some(decision.version),
                    expected_action_hash: Some(decision.action_hash.clone()),
                    expected_target_hash: Some(decision.target_hash.clone()),
                },
                "host",
                "test",
            )
            .expect("resolution should succeed")
            .expect("decision should exist");

        let err = store
            .validate_resume_context(
                &decision.id,
                DecisionResolutionContext {
                    expected_version: Some(resolved.version),
                    expected_action_hash: Some(resolved.action_hash.clone()),
                    expected_target_hash: Some("stale-target-hash".to_string()),
                },
                "test",
            )
            .expect_err("stale target hash should be rejected");

        assert!(matches!(err, DecisionStoreError::TargetHashMismatch { .. }));
    }

    #[test]
    fn resume_context_invalidates_expired_resolved_decision() {
        let store = DecisionStore::in_memory();
        let decision = store
            .create_decision(
                "workflow-a",
                Some("stage-1".to_string()),
                DecisionCandidate::target_selection(
                    "Select execution target",
                    vec!["host".to_string(), "vm".to_string()],
                    "execute_bash",
                ),
            )
            .expect("decision should be created");
        let resolved = store
            .resolve_with_context(
                &decision.id,
                DecisionResolutionContext {
                    expected_version: Some(decision.version),
                    ..DecisionResolutionContext::default()
                },
                "host",
                "test",
            )
            .expect("resolution should succeed")
            .expect("decision should exist");

        {
            let mut state = store.state.lock().unwrap();
            let decision = state
                .decisions
                .get_mut(&decision.id)
                .expect("decision should exist");
            decision.expires_at = Some(
                (chrono::Utc::now() - chrono::Duration::minutes(1))
                    .to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
            );
        }

        let err = store
            .validate_resume_context(
                &decision.id,
                DecisionResolutionContext {
                    expected_version: Some(resolved.version),
                    ..DecisionResolutionContext::default()
                },
                "test",
            )
            .expect_err("expired resolved decision should be rejected");

        assert!(matches!(err, DecisionStoreError::DecisionExpired { .. }));
        assert_eq!(
            store
                .decision(&decision.id)
                .expect("decision exists")
                .status,
            DecisionStatus::Invalidated
        );
    }
}
