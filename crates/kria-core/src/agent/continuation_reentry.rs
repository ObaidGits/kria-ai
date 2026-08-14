//! Phase 6 bounded continuation re-entry.
//!
//! This service does not resume whole workflows. It verifies one previously
//! executed decision-bound action, records exact action-level progress, and
//! stops. Any future side-effect must still pass through the normal execution
//! gate path.

use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use tokio::sync::Mutex;
use tokio_util::sync::CancellationToken;

use crate::agent::collaborative_decision::{
    current_tool_registry_version, current_tool_schema_version, ActionProposal, CheckpointSummary,
    ConfidenceBand, ContinuationClaim, ContinuationState, ContinuationUpdate,
    DecisionExecutionRecord, DecisionExecutionState, DecisionResolutionContext,
    DecisionStageBinding, DecisionStatus, DecisionStore, DecisionStoreError, EvidenceSummary,
    InteractionDecision, PostDecisionVerification,
};
use crate::agent::workflow_continuation::WorkflowContinuationRuntime;

const STALE_CONTINUATION_HOURS: i64 = 24;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum ContinuationReentryStatus {
    VerificationReady,
    VerifiedNoFurtherSafeStep,
    ReadyForNextSafeStep,
    VerificationFailed,
    Duplicate,
    Invalidated,
    UnknownDecision,
    NotExecuted,
    UnsupportedCheckpoint,
    Cancelled,
    UnknownAfterCrash,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContinuationReentryRequest {
    pub decision_id: String,
    pub expected_action_hash: Option<String>,
    pub expected_target_hash: Option<String>,
    pub session_id: Option<String>,
    pub workspace_id: Option<String>,
    #[serde(default)]
    pub allow_stale_user_intent: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContinuationReentryResult {
    pub status: ContinuationReentryStatus,
    pub decision: Option<InteractionDecision>,
    pub execution: Option<DecisionExecutionRecord>,
    pub continuation: Option<ContinuationClaim>,
    pub verification: Option<PostDecisionVerification>,
    pub checkpoint_summary: Option<CheckpointSummary>,
    pub can_run_next_safe_step: bool,
    pub next_action_preview: Option<serde_json::Value>,
    pub error: Option<String>,
}

#[derive(Clone)]
struct ReadyContinuation {
    decision: InteractionDecision,
    action: ActionProposal,
    execution: DecisionExecutionRecord,
    binding: DecisionStageBinding,
    checkpoint_summary: Option<CheckpointSummary>,
}

pub struct ContinuationReentryService {
    decision_store: Arc<DecisionStore>,
    workflow_continuation: Arc<WorkflowContinuationRuntime>,
    active_tokens: Arc<Mutex<HashMap<String, CancellationToken>>>,
}

impl ContinuationReentryService {
    pub fn new(
        decision_store: Arc<DecisionStore>,
        workflow_continuation: Arc<WorkflowContinuationRuntime>,
    ) -> Self {
        Self {
            decision_store,
            workflow_continuation,
            active_tokens: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    pub async fn cancel(&self, decision_id: &str) -> bool {
        let token = self.active_tokens.lock().await.get(decision_id).cloned();
        if let Some(token) = token {
            token.cancel();
            true
        } else {
            false
        }
    }

    pub async fn check_after_decision(
        &self,
        request: ContinuationReentryRequest,
    ) -> ContinuationReentryResult {
        let ready = match self.load_ready(&request) {
            Ok(ready) => ready,
            Err(result) => return result,
        };

        let token = CancellationToken::new();
        let verification = verify_prior_action(&ready, &token);
        let status = if verification.passed {
            ContinuationReentryStatus::VerificationReady
        } else {
            ContinuationReentryStatus::VerificationFailed
        };
        let error = verification.failure_reason.clone();
        result(
            status,
            Some(ready.decision),
            Some(ready.execution),
            None,
            Some(verification),
            ready.checkpoint_summary,
            false,
            None,
            error,
        )
    }

    pub async fn continue_after_decision(
        &self,
        request: ContinuationReentryRequest,
    ) -> ContinuationReentryResult {
        let ready = match self.load_ready(&request) {
            Ok(ready) => ready,
            Err(result) => return result,
        };

        let token = CancellationToken::new();
        self.active_tokens
            .lock()
            .await
            .insert(ready.decision.id.clone(), token.clone());

        let mut claim = match self.decision_store.claim_continuation(
            &ready.decision,
            &ready.execution,
            &ready.binding,
            "continuation_reentry",
        ) {
            Ok(claim) => claim,
            Err(DecisionStoreError::ContinuationAlreadyExists { .. }) => {
                self.active_tokens.lock().await.remove(&ready.decision.id);
                let existing = self.decision_store.continuation_record(
                    &ready.decision.id,
                    &ready.execution.execution_id,
                    &ready.binding.checkpoint_id,
                    &ready.binding.action_hash,
                );
                return result(
                    ContinuationReentryStatus::Duplicate,
                    Some(ready.decision),
                    Some(ready.execution),
                    existing,
                    None,
                    ready.checkpoint_summary,
                    false,
                    None,
                    Some("continuation already claimed for this execution".to_string()),
                );
            }
            Err(error) => {
                self.active_tokens.lock().await.remove(&ready.decision.id);
                return result(
                    ContinuationReentryStatus::Invalidated,
                    Some(ready.decision),
                    Some(ready.execution),
                    None,
                    None,
                    ready.checkpoint_summary,
                    false,
                    None,
                    Some(error.to_string()),
                );
            }
        };

        if token.is_cancelled() {
            claim = self
                .mark_continuation(
                    &claim,
                    ContinuationState::Cancelled,
                    ContinuationUpdate {
                        completed: true,
                        error_class: Some("Cancelled".to_string()),
                        error_message: Some(
                            "continuation cancelled before verification".to_string(),
                        ),
                        ..ContinuationUpdate::default()
                    },
                )
                .unwrap_or(claim);
            self.active_tokens.lock().await.remove(&ready.decision.id);
            return result(
                ContinuationReentryStatus::Cancelled,
                Some(ready.decision),
                Some(ready.execution),
                Some(claim),
                None,
                ready.checkpoint_summary,
                false,
                None,
                Some("continuation cancelled".to_string()),
            );
        }

        let verification = verify_prior_action(&ready, &token);
        let verification_id = verification.verification_id.clone();
        let verification = match self
            .decision_store
            .record_post_decision_verification(verification, "continuation_reentry")
        {
            Ok(verification) => verification,
            Err(error) => {
                claim = self
                    .mark_continuation(
                        &claim,
                        ContinuationState::Failed,
                        ContinuationUpdate {
                            completed: true,
                            error_class: Some("VerificationPersistenceFailed".to_string()),
                            error_message: Some(error.to_string()),
                            ..ContinuationUpdate::default()
                        },
                    )
                    .unwrap_or(claim);
                self.active_tokens.lock().await.remove(&ready.decision.id);
                return result(
                    ContinuationReentryStatus::VerificationFailed,
                    Some(ready.decision),
                    Some(ready.execution),
                    Some(claim),
                    None,
                    ready.checkpoint_summary,
                    false,
                    None,
                    Some("failed to persist verification evidence".to_string()),
                );
            }
        };

        if !verification.passed {
            claim = self
                .mark_continuation(
                    &claim,
                    ContinuationState::Failed,
                    ContinuationUpdate {
                        completed: true,
                        verification_id: Some(verification_id),
                        error_class: Some("PostDecisionVerificationFailed".to_string()),
                        error_message: verification.failure_reason.clone(),
                        ..ContinuationUpdate::default()
                    },
                )
                .unwrap_or(claim);
            self.active_tokens.lock().await.remove(&ready.decision.id);
            return result(
                ContinuationReentryStatus::VerificationFailed,
                Some(ready.decision),
                Some(ready.execution),
                Some(claim),
                Some(verification.clone()),
                ready.checkpoint_summary,
                false,
                None,
                verification.failure_reason,
            );
        }

        claim = self
            .mark_continuation(
                &claim,
                ContinuationState::VerifiedPriorAction,
                ContinuationUpdate {
                    verification_id: Some(verification_id.clone()),
                    ..ContinuationUpdate::default()
                },
            )
            .unwrap_or(claim);

        claim = self
            .mark_continuation(
                &claim,
                ContinuationState::AdvancingActionState,
                ContinuationUpdate::default(),
            )
            .unwrap_or(claim);

        let session_result = self.workflow_continuation.record_decision_action_completed(
            &ready.decision.workflow_id,
            &ready.binding.action_id,
            serde_json::json!({
                "decision_id": ready.decision.id,
                "execution_id": ready.execution.execution_id,
                "action_hash": ready.action.action_hash,
                "target_hash": ready.action.target_hash,
            }),
            verification
                .evidence
                .first()
                .map(|evidence| evidence.summary.clone())
                .unwrap_or_else(|| "post-decision verification passed".to_string()),
        );

        if let Err(error) = session_result {
            claim = self
                .mark_continuation(
                    &claim,
                    ContinuationState::Failed,
                    ContinuationUpdate {
                        completed: true,
                        verification_id: Some(verification_id),
                        error_class: Some("ActionProgressPersistenceFailed".to_string()),
                        error_message: Some(error.clone()),
                        ..ContinuationUpdate::default()
                    },
                )
                .unwrap_or(claim);
            self.active_tokens.lock().await.remove(&ready.decision.id);
            return result(
                ContinuationReentryStatus::VerificationFailed,
                Some(ready.decision),
                Some(ready.execution),
                Some(claim),
                Some(verification),
                ready.checkpoint_summary,
                false,
                None,
                Some(error),
            );
        }

        let next_preview = ready
            .checkpoint_summary
            .as_ref()
            .and_then(|summary| summary.next_safe_action_preview.clone());
        let has_next_preview = next_preview.is_some();
        let terminal_state = if has_next_preview {
            ContinuationState::ReadyForNextSafeStep
        } else {
            ContinuationState::CompletedOneStep
        };
        claim = self
            .mark_continuation(
                &claim,
                terminal_state,
                ContinuationUpdate {
                    completed: !has_next_preview,
                    verification_id: Some(verification_id),
                    ..ContinuationUpdate::default()
                },
            )
            .unwrap_or(claim);

        self.active_tokens.lock().await.remove(&ready.decision.id);
        result(
            if has_next_preview {
                ContinuationReentryStatus::ReadyForNextSafeStep
            } else {
                ContinuationReentryStatus::VerifiedNoFurtherSafeStep
            },
            Some(ready.decision),
            Some(ready.execution),
            Some(claim),
            Some(verification),
            ready.checkpoint_summary,
            false,
            next_preview,
            None,
        )
    }

    fn load_ready(
        &self,
        request: &ContinuationReentryRequest,
    ) -> Result<ReadyContinuation, ContinuationReentryResult> {
        if let Err(error) = self.decision_store.refresh_from_disk() {
            return Err(error_result(
                ContinuationReentryStatus::UnknownDecision,
                None,
                None,
                format!("failed to refresh decision store: {error}"),
            ));
        }

        let decision = match self.decision_store.validate_resume_context(
            &request.decision_id,
            DecisionResolutionContext {
                expected_version: None,
                expected_action_hash: request.expected_action_hash.clone(),
                expected_target_hash: request.expected_target_hash.clone(),
            },
            "continuation_reentry",
        ) {
            Ok(Some(decision)) => decision,
            Ok(None) => {
                return Err(error_result(
                    ContinuationReentryStatus::UnknownDecision,
                    None,
                    None,
                    format!("unknown interaction decision: {}", request.decision_id),
                ))
            }
            Err(error) => {
                return Err(error_result(
                    ContinuationReentryStatus::Invalidated,
                    None,
                    None,
                    error.to_string(),
                ))
            }
        };

        if decision.status != DecisionStatus::Resolved {
            return Err(error_result(
                ContinuationReentryStatus::Invalidated,
                Some(decision),
                None,
                "decision is not resolved".to_string(),
            ));
        }

        let Some(action) = decision.action_proposal.clone() else {
            return Err(error_result(
                ContinuationReentryStatus::UnsupportedCheckpoint,
                Some(decision),
                None,
                "decision is missing immutable action proposal".to_string(),
            ));
        };
        let Some(binding) = decision.stage_binding.clone() else {
            return Err(error_result(
                ContinuationReentryStatus::UnsupportedCheckpoint,
                Some(decision),
                None,
                "decision has no DecisionStageBinding; restart or replan required".to_string(),
            ));
        };

        if let Err(reason) = validate_binding(&decision, &action, &binding, request) {
            return Err(error_result(
                ContinuationReentryStatus::Invalidated,
                Some(decision),
                None,
                reason,
            ));
        }

        let Some(execution) = self
            .decision_store
            .execution_record(&decision.id, &decision.action_hash)
        else {
            return Err(error_result(
                ContinuationReentryStatus::NotExecuted,
                Some(decision),
                None,
                "decision execution record missing".to_string(),
            ));
        };

        match execution.state {
            DecisionExecutionState::Executed => {}
            DecisionExecutionState::UnknownAfterCrash => {
                return Err(error_result(
                    ContinuationReentryStatus::UnknownAfterCrash,
                    Some(decision),
                    Some(execution),
                    "cannot continue after crash without manual verification".to_string(),
                ))
            }
            other => {
                return Err(error_result(
                    ContinuationReentryStatus::NotExecuted,
                    Some(decision),
                    Some(execution),
                    format!("decision execution is not completed: {other:?}"),
                ))
            }
        }

        if let Err(reason) =
            validate_execution_freshness(&execution, request.allow_stale_user_intent)
        {
            return Err(error_result(
                ContinuationReentryStatus::Invalidated,
                Some(decision),
                Some(execution),
                reason,
            ));
        }

        Ok(ReadyContinuation {
            decision: decision.clone(),
            action,
            execution,
            binding,
            checkpoint_summary: decision.checkpoint_summary.clone(),
        })
    }

    fn mark_continuation(
        &self,
        claim: &ContinuationClaim,
        state: ContinuationState,
        update: ContinuationUpdate,
    ) -> Result<ContinuationClaim, DecisionStoreError> {
        self.decision_store
            .update_continuation_state(claim, state, update, "continuation_reentry")
    }
}

fn validate_binding(
    decision: &InteractionDecision,
    action: &ActionProposal,
    binding: &DecisionStageBinding,
    request: &ContinuationReentryRequest,
) -> Result<(), String> {
    if binding.decision_id != decision.id {
        return Err("decision_stage_binding_id_mismatch".to_string());
    }
    if binding.workflow_id != action.workflow_id || binding.workflow_id != decision.workflow_id {
        return Err("decision_stage_binding_workflow_mismatch".to_string());
    }
    if binding.attempt_id != action.attempt_id {
        return Err("decision_stage_binding_attempt_mismatch".to_string());
    }
    if binding.stage_id != action.stage_id {
        return Err("decision_stage_binding_stage_mismatch".to_string());
    }
    if binding.action_hash != action.action_hash || binding.action_hash != decision.action_hash {
        return Err("decision_stage_binding_action_hash_mismatch".to_string());
    }
    if binding.target_hash != action.target_hash || binding.target_hash != decision.target_hash {
        return Err("decision_stage_binding_target_hash_mismatch".to_string());
    }
    if binding.workflow_schema_version != env!("CARGO_PKG_VERSION") {
        return Err(format!(
            "workflow_schema_version_changed:{}!={}",
            binding.workflow_schema_version,
            env!("CARGO_PKG_VERSION")
        ));
    }
    if action.tool_schema_version != current_tool_schema_version() {
        return Err(format!(
            "tool_schema_version_changed:{}!={}",
            action.tool_schema_version,
            current_tool_schema_version()
        ));
    }
    if action.tool_registry_version != current_tool_registry_version() {
        return Err(format!(
            "tool_registry_version_changed:{}!={}",
            action.tool_registry_version,
            current_tool_registry_version()
        ));
    }
    if binding.max_side_effect_count > 1 {
        return Err("continuation_side_effect_count_exceeds_phase6_limit".to_string());
    }
    if !binding.local_deterministic_only || is_external_or_delegated(action) {
        return Err("delegated_or_external_continuation_not_supported_in_phase6".to_string());
    }
    if let (Some(expected), Some(actual)) = (
        action.target.session_id.as_deref(),
        request.session_id.as_deref(),
    ) {
        if expected != actual {
            return Err(format!(
                "session_changed_before_continuation:{expected}!={actual}"
            ));
        }
    }
    if let (Some(expected), Some(actual)) = (
        action.target.workspace_id.as_deref(),
        request.workspace_id.as_deref(),
    ) {
        if expected != actual {
            return Err(format!(
                "workspace_changed_before_continuation:{expected}!={actual}"
            ));
        }
    }
    Ok(())
}

fn validate_execution_freshness(
    execution: &DecisionExecutionRecord,
    allow_stale_user_intent: bool,
) -> Result<(), String> {
    if allow_stale_user_intent {
        return Ok(());
    }
    let timestamp = execution
        .completed_at
        .as_deref()
        .unwrap_or(execution.started_at.as_str());
    let Ok(parsed) = chrono::DateTime::parse_from_rfc3339(timestamp) else {
        return Err("execution_timestamp_unparseable_requires_replan".to_string());
    };
    let age = chrono::Utc::now() - parsed.with_timezone(&chrono::Utc);
    if age > chrono::Duration::hours(STALE_CONTINUATION_HOURS) {
        return Err("stale_execution_requires_user_confirmation".to_string());
    }
    Ok(())
}

fn verify_prior_action(
    ready: &ReadyContinuation,
    token: &CancellationToken,
) -> PostDecisionVerification {
    if token.is_cancelled() {
        return verification(
            ready,
            false,
            ConfidenceBand::Unknown,
            "cancelled",
            vec![],
            vec![],
            Some("verification cancelled".to_string()),
        );
    }

    if !redacted_tool_success(&ready.execution) {
        return verification(
            ready,
            false,
            ConfidenceBand::Low,
            "tool_result",
            vec![evidence(
                "tool_result",
                ConfidenceBand::Low,
                "stored execution result was not successful",
            )],
            vec!["tool_result".to_string()],
            Some("stored execution result is not successful".to_string()),
        );
    }

    let tool = ready.action.tool_name.as_str();
    match tool {
        "write_file" | "append_file" => verify_path_exists(ready, "path", "filesystem_path_exists"),
        "read_file" => verify_path_exists(ready, "path", "filesystem_read_target_exists"),
        "delete_file" => verify_path_absent(ready, "path", "filesystem_path_absent"),
        "move_file" => verify_path_exists(ready, "destination", "destination_path_exists"),
        "execute_bash" | "execute_shell" | "run_command" => verify_command_artifact(ready),
        _ if is_read_only_tool(tool) => verification(
            ready,
            true,
            ConfidenceBand::Medium,
            "read_only_tool_result",
            vec![evidence(
                "tool_result",
                ConfidenceBand::Medium,
                "read-only action returned a successful tool result",
            )],
            vec!["tool_result_summary".to_string()],
            None,
        ),
        _ => verification(
            ready,
            false,
            ConfidenceBand::Unknown,
            "unsupported_verifier",
            vec![],
            vec![],
            Some(format!(
                "no deterministic post-decision verifier for tool '{}'",
                ready.action.tool_name
            )),
        ),
    }
}

fn verify_path_exists(
    ready: &ReadyContinuation,
    param_key: &str,
    verifier_kind: &str,
) -> PostDecisionVerification {
    let Some(path) = ready
        .action
        .parameters
        .get(param_key)
        .and_then(|v| v.as_str())
    else {
        return verification(
            ready,
            false,
            ConfidenceBand::Unknown,
            verifier_kind,
            vec![],
            vec![],
            Some(format!("missing path parameter '{param_key}'")),
        );
    };
    let exists = Path::new(path).exists();
    verification(
        ready,
        exists,
        if exists {
            ConfidenceBand::High
        } else {
            ConfidenceBand::Low
        },
        verifier_kind,
        vec![evidence(
            "filesystem",
            if exists {
                ConfidenceBand::High
            } else {
                ConfidenceBand::Low
            },
            if exists {
                format!("verified path exists: {path}")
            } else {
                format!("expected path missing: {path}")
            },
        )],
        vec!["filesystem_metadata".to_string()],
        (!exists).then(|| format!("expected path missing: {path}")),
    )
}

fn verify_path_absent(
    ready: &ReadyContinuation,
    param_key: &str,
    verifier_kind: &str,
) -> PostDecisionVerification {
    let Some(path) = ready
        .action
        .parameters
        .get(param_key)
        .and_then(|v| v.as_str())
    else {
        return verification(
            ready,
            false,
            ConfidenceBand::Unknown,
            verifier_kind,
            vec![],
            vec![],
            Some(format!("missing path parameter '{param_key}'")),
        );
    };
    let absent = !Path::new(path).exists();
    verification(
        ready,
        absent,
        if absent {
            ConfidenceBand::High
        } else {
            ConfidenceBand::Conflicted
        },
        verifier_kind,
        vec![evidence(
            "filesystem",
            if absent {
                ConfidenceBand::High
            } else {
                ConfidenceBand::Conflicted
            },
            if absent {
                format!("verified path absent: {path}")
            } else {
                format!("path still exists after delete action: {path}")
            },
        )],
        vec!["filesystem_metadata".to_string()],
        (!absent).then(|| format!("path still exists after delete action: {path}")),
    )
}

fn verify_command_artifact(ready: &ReadyContinuation) -> PostDecisionVerification {
    for key in ["expected_output_path", "output_path", "artifact_path"] {
        if let Some(path) = ready.action.parameters.get(key).and_then(|v| v.as_str()) {
            let exists = Path::new(path).exists();
            return verification(
                ready,
                exists,
                if exists {
                    ConfidenceBand::High
                } else {
                    ConfidenceBand::Low
                },
                "command_expected_artifact",
                vec![evidence(
                    "filesystem",
                    if exists {
                        ConfidenceBand::High
                    } else {
                        ConfidenceBand::Low
                    },
                    if exists {
                        format!("verified command artifact exists: {path}")
                    } else {
                        format!("expected command artifact missing: {path}")
                    },
                )],
                vec!["filesystem_metadata".to_string()],
                (!exists).then(|| format!("expected command artifact missing: {path}")),
            );
        }
    }
    verification(
        ready,
        false,
        ConfidenceBand::Unknown,
        "command_expected_artifact",
        vec![],
        vec![],
        Some("shell command has no durable expected artifact for Phase 6 continuation".to_string()),
    )
}

fn verification(
    ready: &ReadyContinuation,
    passed: bool,
    confidence: ConfidenceBand,
    verifier_kind: impl Into<String>,
    evidence: Vec<EvidenceSummary>,
    sensitivity_tags: Vec<String>,
    failure_reason: Option<String>,
) -> PostDecisionVerification {
    PostDecisionVerification {
        verification_id: uuid::Uuid::new_v4().to_string(),
        decision_id: ready.decision.id.clone(),
        execution_id: ready.execution.execution_id.clone(),
        workflow_id: ready.decision.workflow_id.clone(),
        action_hash: ready.action.action_hash.clone(),
        target_hash: ready.action.target_hash.clone(),
        verifier_kind: verifier_kind.into(),
        evidence,
        confidence,
        deterministic: true,
        passed,
        failure_reason,
        sensitivity_tags,
        created_at: now_rfc3339(),
        expires_at: Some(
            (chrono::Utc::now() + chrono::Duration::hours(STALE_CONTINUATION_HOURS))
                .to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
        ),
    }
}

fn evidence(
    source: impl Into<String>,
    confidence: ConfidenceBand,
    summary: impl Into<String>,
) -> EvidenceSummary {
    EvidenceSummary {
        source: source.into(),
        confidence,
        freshness: "fresh".to_string(),
        reliability: "structural".to_string(),
        summary: summary.into(),
    }
}

fn redacted_tool_success(execution: &DecisionExecutionRecord) -> bool {
    execution
        .redacted_tool_result
        .as_ref()
        .and_then(|value| value.get("success"))
        .and_then(|value| value.as_bool())
        .unwrap_or(false)
}

fn is_read_only_tool(tool: &str) -> bool {
    matches!(
        tool,
        "read_file"
            | "list_files"
            | "get_system_info"
            | "get_hardware_info"
            | "get_health"
            | "search_knowledge"
    )
}

fn is_external_or_delegated(action: &ActionProposal) -> bool {
    let mut text = format!(
        "{} {} {}",
        action.target.kind,
        action.target.id,
        action.target.execution_boundary.clone().unwrap_or_default()
    )
    .to_lowercase();
    if let Some(metadata) = action.target.metadata.as_object() {
        for value in metadata.values() {
            text.push(' ');
            text.push_str(&value.to_string().to_lowercase());
        }
    }
    ["n8n", "mcp", "openclaw", "external", "delegated"]
        .iter()
        .any(|needle| text.contains(needle))
}

fn now_rfc3339() -> String {
    chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
}

fn error_result(
    status: ContinuationReentryStatus,
    decision: Option<InteractionDecision>,
    execution: Option<DecisionExecutionRecord>,
    error: String,
) -> ContinuationReentryResult {
    result(
        status,
        decision,
        execution,
        None,
        None,
        None,
        false,
        None,
        Some(error),
    )
}

fn result(
    status: ContinuationReentryStatus,
    decision: Option<InteractionDecision>,
    execution: Option<DecisionExecutionRecord>,
    continuation: Option<ContinuationClaim>,
    verification: Option<PostDecisionVerification>,
    checkpoint_summary: Option<CheckpointSummary>,
    can_run_next_safe_step: bool,
    next_action_preview: Option<serde_json::Value>,
    error: Option<String>,
) -> ContinuationReentryResult {
    ContinuationReentryResult {
        status,
        decision,
        execution,
        continuation,
        verification,
        checkpoint_summary,
        can_run_next_safe_step,
        next_action_preview,
        error,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::collaborative_decision::{
        Actor, DecisionCandidate, DecisionOption, DecisionType, Rollbackability, TargetBinding,
    };
    use crate::safety::RiskLevel;

    fn test_decision_store() -> Arc<DecisionStore> {
        Arc::new(DecisionStore::in_memory())
    }

    fn candidate() -> DecisionCandidate {
        DecisionCandidate {
            decision_type: DecisionType::Approval,
            authority: crate::agent::collaborative_decision::AuthorityLevel::PolicyRisk,
            risk_level: RiskLevel::Yellow,
            reason: "approve write".to_string(),
            options: vec![DecisionOption {
                id: "approve".to_string(),
                label: "Approve".to_string(),
                impact: "writes file".to_string(),
                risk: RiskLevel::Yellow,
            }],
            recommended_option: Some("approve".to_string()),
            evidence: vec![],
            affected_resources: vec![],
            rule_id: None,
            invalidation_rules: vec![],
            rollbackability: Rollbackability::Reversible,
            confidence: ConfidenceBand::High,
        }
    }

    fn action(path: &str) -> ActionProposal {
        ActionProposal::new(
            "workflow-a",
            "attempt-a",
            "stage-a",
            "write_file",
            serde_json::json!({ "path": path, "content": "ok" }),
            TargetBinding::new("execution_target", "host"),
            Actor::Runtime,
        )
    }

    fn executed_record(
        store: &DecisionStore,
        decision: &InteractionDecision,
    ) -> DecisionExecutionRecord {
        store
            .claim_execution(
                decision,
                crate::agent::collaborative_decision::DecisionExecutionContext {
                    execution_actor: "test".to_string(),
                    source_command: "test".to_string(),
                    session_id: None,
                    workspace_id: None,
                },
            )
            .unwrap();
        store
            .update_execution_state(
                &decision.id,
                &decision.action_hash,
                DecisionExecutionState::Executed,
                crate::agent::collaborative_decision::DecisionExecutionUpdate {
                    redacted_tool_result: Some(serde_json::json!({ "success": true })),
                    completed: true,
                    ..Default::default()
                },
                "test",
            )
            .unwrap()
    }

    #[serial_test::serial]
    #[tokio::test]
    async fn rejects_legacy_decision_without_binding() {
        let store = test_decision_store();
        let decision = store
            .create_decision("workflow-a", Some("stage-a".to_string()), candidate())
            .unwrap();
        store.resolve(&decision.id, "approve", "test").unwrap();
        let decision = store.decision(&decision.id).unwrap();
        let service = ContinuationReentryService::new(
            store,
            Arc::new(WorkflowContinuationRuntime::new(None)),
        );
        let result = service
            .check_after_decision(ContinuationReentryRequest {
                decision_id: decision.id,
                expected_action_hash: None,
                expected_target_hash: None,
                session_id: None,
                workspace_id: None,
                allow_stale_user_intent: false,
            })
            .await;
        assert_eq!(
            result.status,
            ContinuationReentryStatus::UnsupportedCheckpoint
        );
    }

    #[serial_test::serial]
    #[tokio::test]
    async fn verifies_and_records_one_action() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("out.txt");
        std::fs::write(&path, "ok").unwrap();

        let store = test_decision_store();
        let decision = store
            .create_decision_for_action(&action(path.to_str().unwrap()), candidate())
            .unwrap();
        store.resolve(&decision.id, "approve", "test").unwrap();
        let decision = store.decision(&decision.id).unwrap();
        let _record = executed_record(&store, &decision);

        let service = ContinuationReentryService::new(
            store,
            Arc::new(WorkflowContinuationRuntime::new(None)),
        );
        let result = service
            .continue_after_decision(ContinuationReentryRequest {
                decision_id: decision.id,
                expected_action_hash: Some(decision.action_hash),
                expected_target_hash: Some(decision.target_hash),
                session_id: None,
                workspace_id: None,
                allow_stale_user_intent: false,
            })
            .await;
        assert_eq!(
            result.status,
            ContinuationReentryStatus::VerifiedNoFurtherSafeStep
        );
        assert!(result.verification.unwrap().passed);
        assert_eq!(
            result.continuation.unwrap().state,
            ContinuationState::CompletedOneStep
        );
    }

    #[serial_test::serial]
    #[tokio::test]
    // Serialized: this test's verification step re-reads a temp file and was
    // observed returning `VerificationFailed` under heavy parallel load (~2 of 4
    // full-suite runs) while passing 3/3 in isolation. `#[serial]` must precede
    // `#[tokio::test]` so it wraps the generated runtime body. Serializing makes
    // the gate deterministic; it does NOT prove the verifier is load-insensitive,
    // which is worth investigating separately.
    async fn duplicate_continuation_is_rejected() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("out.txt");
        std::fs::write(&path, "ok").unwrap();

        let store = test_decision_store();
        let decision = store
            .create_decision_for_action(&action(path.to_str().unwrap()), candidate())
            .unwrap();
        store.resolve(&decision.id, "approve", "test").unwrap();
        let decision = store.decision(&decision.id).unwrap();
        let _record = executed_record(&store, &decision);

        let service = ContinuationReentryService::new(
            store,
            Arc::new(WorkflowContinuationRuntime::new(None)),
        );
        let request = ContinuationReentryRequest {
            decision_id: decision.id,
            expected_action_hash: Some(decision.action_hash),
            expected_target_hash: Some(decision.target_hash),
            session_id: None,
            workspace_id: None,
            allow_stale_user_intent: false,
        };
        let first = service.continue_after_decision(request.clone()).await;
        assert_eq!(
            first.status,
            ContinuationReentryStatus::VerifiedNoFurtherSafeStep
        );
        let second = service.continue_after_decision(request).await;
        assert_eq!(second.status, ContinuationReentryStatus::Duplicate);
    }
}
