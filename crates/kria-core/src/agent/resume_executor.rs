//! Deterministic one-step execution for resolved collaborative decisions.
//!
//! This is not workflow replay. It executes exactly one persisted
//! `ActionProposal` after the resume gate, grounding, policy, and leases all
//! validate against current state.

use std::collections::HashMap;
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use tokio::sync::Mutex;
use tokio_util::sync::CancellationToken;

use crate::agent::collaborative_decision::{
    current_tool_registry_version, current_tool_schema_version, DecisionExecutionContext,
    DecisionExecutionRecord, DecisionExecutionState, DecisionExecutionUpdate,
    DecisionResolutionContext, DecisionStatus, DecisionStore, InteractionDecision,
};
use crate::agent::environment_grounder::{
    EnvironmentGrounder, LiveEnvironmentGrounder, OperationalFacts,
};
use crate::agent::execution_gate::{ExecutionGate, ResumeGateOutcome};
use crate::agent::resource_lease::{ResourceLeaseGuard, ResourceLeaseManager};
use crate::infra::ToolResult;
use crate::safety::audit::{DecidedBy, Decision};
use crate::safety::{AuditLogger, PolicyEngine};
use crate::tools::registry::{ToolRegistry, ToolResumeCapability};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum DecisionExecutionStatus {
    Executed,
    ToolFailed,
    Cancelled,
    Duplicate,
    Invalidated,
    BlockedByGate,
    BlockedByLease,
    UnsupportedTool,
    UnknownDecision,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DecisionExecutionRequest {
    pub decision_id: String,
    pub expected_version: Option<u64>,
    pub expected_action_hash: Option<String>,
    pub expected_target_hash: Option<String>,
    pub session_id: Option<String>,
    pub workspace_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DecisionExecutionResult {
    pub status: DecisionExecutionStatus,
    pub execution_started: bool,
    pub decision: Option<InteractionDecision>,
    pub execution: Option<DecisionExecutionRecord>,
    pub gate: serde_json::Value,
    pub grounding: serde_json::Value,
    pub lease_conflict: Option<serde_json::Value>,
    pub tool_result: Option<serde_json::Value>,
    pub error: Option<String>,
}

pub struct ResumeExecutor {
    decision_store: Arc<DecisionStore>,
    execution_gate: ExecutionGate,
    resource_lease: ResourceLeaseManager,
    tool_registry: Arc<ToolRegistry>,
    audit_logger: Arc<AuditLogger>,
    grounder: Arc<dyn EnvironmentGrounder>,
    active_tokens: Arc<Mutex<HashMap<String, CancellationToken>>>,
}

impl ResumeExecutor {
    pub fn new(
        tool_registry: Arc<ToolRegistry>,
        policy_engine: Arc<PolicyEngine>,
        decision_store: Arc<DecisionStore>,
        audit_logger: Arc<AuditLogger>,
    ) -> Self {
        Self {
            execution_gate: ExecutionGate::new(
                Arc::clone(&policy_engine),
                Arc::clone(&decision_store),
            ),
            decision_store,
            resource_lease: ResourceLeaseManager::global(),
            tool_registry,
            audit_logger,
            grounder: Arc::new(LiveEnvironmentGrounder::new()),
            active_tokens: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    pub fn with_grounder(mut self, grounder: Arc<dyn EnvironmentGrounder>) -> Self {
        self.grounder = grounder;
        self
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

    pub async fn execute_resolved_decision(
        &self,
        request: DecisionExecutionRequest,
    ) -> DecisionExecutionResult {
        if let Err(error) = self.decision_store.refresh_from_disk() {
            return result_error(
                DecisionExecutionStatus::UnknownDecision,
                None,
                None,
                format!("failed to refresh decision store: {error}"),
            );
        }

        let decision = match self.decision_store.validate_resume_context(
            &request.decision_id,
            DecisionResolutionContext {
                expected_version: request.expected_version,
                expected_action_hash: request.expected_action_hash.clone(),
                expected_target_hash: request.expected_target_hash.clone(),
            },
            "resume_executor",
        ) {
            Ok(Some(decision)) => decision,
            Ok(None) => {
                return result_error(
                    DecisionExecutionStatus::UnknownDecision,
                    None,
                    None,
                    format!("Unknown interaction decision: {}", request.decision_id),
                )
            }
            Err(error) => {
                return result_error(
                    DecisionExecutionStatus::Invalidated,
                    None,
                    None,
                    error.to_string(),
                )
            }
        };

        if decision.status != DecisionStatus::Resolved {
            return result_error(
                DecisionExecutionStatus::Invalidated,
                Some(decision),
                None,
                "decision is not resolved".to_string(),
            );
        }

        let Some(action) = decision.action_proposal.clone() else {
            let _ = self.decision_store.invalidate(
                &decision.id,
                "missing_action_proposal_before_execution",
                "resume_executor",
            );
            return result_error(
                DecisionExecutionStatus::Invalidated,
                Some(decision),
                None,
                "decision is missing a persisted action proposal".to_string(),
            );
        };

        if let Err(reason) = validate_action_context(&decision, &request) {
            let _ = self
                .decision_store
                .invalidate(&decision.id, reason.clone(), "resume_executor");
            return result_error(
                DecisionExecutionStatus::Invalidated,
                Some(decision),
                None,
                reason,
            );
        }

        let mut execution = match self.decision_store.claim_execution(
            &decision,
            DecisionExecutionContext::user_action_center(
                request.session_id.clone(),
                request.workspace_id.clone(),
            ),
        ) {
            Ok(record) => record,
            Err(
                crate::agent::collaborative_decision::DecisionStoreError::ExecutionAlreadyExists {
                    ..
                },
            ) => {
                let existing = self
                    .decision_store
                    .execution_record(&decision.id, &decision.action_hash);
                return DecisionExecutionResult {
                    status: DecisionExecutionStatus::Duplicate,
                    execution_started: false,
                    decision: Some(decision),
                    execution: existing,
                    gate: serde_json::json!({ "status": "duplicate" }),
                    grounding: serde_json::json!({ "collected": false }),
                    lease_conflict: None,
                    tool_result: None,
                    error: Some("decision already has an execution record".to_string()),
                };
            }
            Err(error) => {
                return result_error(
                    DecisionExecutionStatus::Invalidated,
                    Some(decision),
                    None,
                    error.to_string(),
                )
            }
        };

        if let Err(reason) = validate_tool_versions(&action) {
            execution = self
                .mark_execution(
                    &execution,
                    DecisionExecutionState::Invalidated,
                    DecisionExecutionUpdate {
                        error_class: Some("ToolVersionMismatch".to_string()),
                        error_message: Some(reason.clone()),
                        completed: true,
                        ..DecisionExecutionUpdate::default()
                    },
                )
                .unwrap_or(execution);
            let _ = self
                .decision_store
                .invalidate(&decision.id, reason.clone(), "resume_executor");
            return result_error(
                DecisionExecutionStatus::Invalidated,
                Some(decision),
                Some(execution),
                reason,
            );
        }

        match self.tool_registry.resume_capability(&action.tool_name) {
            ToolResumeCapability::DeterministicLocal => {}
            other => {
                let reason = format!(
                    "tool '{}' is not supported for Phase 5 resume execution: {other:?}",
                    action.tool_name
                );
                execution = self
                    .mark_execution(
                        &execution,
                        DecisionExecutionState::Invalidated,
                        DecisionExecutionUpdate {
                            error_class: Some("UnsupportedTool".to_string()),
                            error_message: Some(reason.clone()),
                            completed: true,
                            ..DecisionExecutionUpdate::default()
                        },
                    )
                    .unwrap_or(execution);
                let _ =
                    self.decision_store
                        .invalidate(&decision.id, reason.clone(), "resume_executor");
                return result_error(
                    DecisionExecutionStatus::UnsupportedTool,
                    Some(decision),
                    Some(execution),
                    reason,
                );
            }
        }

        if self.tool_registry.get_handler(&action.tool_name).is_none() {
            let reason = format!("Tool '{}' not found in registry", action.tool_name);
            execution = self
                .mark_execution(
                    &execution,
                    DecisionExecutionState::Invalidated,
                    DecisionExecutionUpdate {
                        error_class: Some("ToolMissing".to_string()),
                        error_message: Some(reason.clone()),
                        completed: true,
                        ..DecisionExecutionUpdate::default()
                    },
                )
                .unwrap_or(execution);
            return result_error(
                DecisionExecutionStatus::UnsupportedTool,
                Some(decision),
                Some(execution),
                reason,
            );
        }

        let facts = self.grounder.ground(&[]).await;
        let grounding_summary = grounding_summary(&facts);

        let gate = self.execution_gate.revalidate_resume(&decision, false);
        if !gate.outcome.can_execute() {
            let gate_summary = gate_summary(&gate.outcome);
            let reason = gate
                .outcome
                .invalidation_reason()
                .unwrap_or_else(|| "resume gate blocked execution".to_string());
            execution = self
                .mark_execution(
                    &execution,
                    DecisionExecutionState::Invalidated,
                    DecisionExecutionUpdate {
                        gate_summary: Some(gate_summary.clone()),
                        grounding_summary: Some(grounding_summary.clone()),
                        error_class: Some("BlockedByGate".to_string()),
                        error_message: Some(reason.clone()),
                        completed: true,
                        ..DecisionExecutionUpdate::default()
                    },
                )
                .unwrap_or(execution);
            let _ = self
                .decision_store
                .invalidate(&decision.id, reason.clone(), "resume_executor");
            return DecisionExecutionResult {
                status: DecisionExecutionStatus::BlockedByGate,
                execution_started: false,
                decision: Some(decision),
                execution: Some(execution),
                gate: gate_summary,
                grounding: grounding_summary,
                lease_conflict: None,
                tool_result: None,
                error: Some(reason),
            };
        }

        let lease_guards = match self
            .resource_lease
            .acquire_requirements(&action.tool_name, &action, &gate.resource_requirements)
            .await
        {
            Ok(guards) => guards,
            Err(error) => {
                let conflict = serde_json::json!({
                    "error": error.to_string(),
                });
                execution = self
                    .mark_execution(
                        &execution,
                        DecisionExecutionState::BlockedByLease,
                        DecisionExecutionUpdate {
                            gate_summary: Some(gate_summary(&gate.outcome)),
                            grounding_summary: Some(grounding_summary),
                            error_class: Some("BlockedByLease".to_string()),
                            error_message: Some(error.to_string()),
                            completed: true,
                            ..DecisionExecutionUpdate::default()
                        },
                    )
                    .unwrap_or(execution);
                return DecisionExecutionResult {
                    status: DecisionExecutionStatus::BlockedByLease,
                    execution_started: false,
                    decision: Some(decision),
                    execution: Some(execution),
                    gate: gate_summary(&gate.outcome),
                    grounding: serde_json::json!({ "collected": true }),
                    lease_conflict: Some(conflict),
                    tool_result: None,
                    error: Some(error.to_string()),
                };
            }
        };

        let final_gate = self.execution_gate.revalidate_resume(&decision, false);
        if !final_gate.outcome.can_execute() {
            release_guards(lease_guards).await;
            let gate_summary = gate_summary(&final_gate.outcome);
            let reason = final_gate
                .outcome
                .invalidation_reason()
                .unwrap_or_else(|| "resume gate blocked execution after leases".to_string());
            execution = self
                .mark_execution(
                    &execution,
                    DecisionExecutionState::Invalidated,
                    DecisionExecutionUpdate {
                        gate_summary: Some(gate_summary.clone()),
                        grounding_summary: Some(grounding_summary.clone()),
                        error_class: Some("BlockedByFinalGate".to_string()),
                        error_message: Some(reason.clone()),
                        completed: true,
                        ..DecisionExecutionUpdate::default()
                    },
                )
                .unwrap_or(execution);
            let _ = self
                .decision_store
                .invalidate(&decision.id, reason.clone(), "resume_executor");
            return DecisionExecutionResult {
                status: DecisionExecutionStatus::BlockedByGate,
                execution_started: false,
                decision: Some(decision),
                execution: Some(execution),
                gate: gate_summary,
                grounding: grounding_summary,
                lease_conflict: None,
                tool_result: None,
                error: Some(reason),
            };
        }

        let lease_refs = lease_guards
            .iter()
            .map(|guard| {
                serde_json::json!({
                    "lease_id": guard.lease.lease_id,
                    "kind": guard.lease.kind,
                    "scope": guard.lease.scope,
                    "access_mode": guard.lease.access_mode,
                    "owner": guard.lease.owner,
                })
            })
            .collect::<Vec<_>>();
        execution = self
            .mark_execution(
                &execution,
                DecisionExecutionState::Executing,
                DecisionExecutionUpdate {
                    gate_summary: Some(gate_summary(&final_gate.outcome)),
                    grounding_summary: Some(grounding_summary.clone()),
                    lease_refs: Some(lease_refs),
                    side_effect_started: true,
                    ..DecisionExecutionUpdate::default()
                },
            )
            .unwrap_or(execution);

        self.audit_logger.log(
            &decision.workflow_id,
            &action.tool_name,
            &action.parameters,
            decision.risk_level,
            Decision::Approved,
            DecidedBy::UserGui,
        );

        let token = CancellationToken::new();
        self.active_tokens
            .lock()
            .await
            .insert(decision.id.clone(), token.clone());
        let handler = self.tool_registry.get_handler(&action.tool_name);
        let tool_result = if let Some(handler) = handler {
            let ctx = self.tool_registry.make_tool_context(token.clone());
            handler
                .execute_with_context(action.parameters.clone(), ctx)
                .await
        } else {
            ToolResult::err(format!("Tool '{}' not found in registry", action.tool_name))
        };
        self.active_tokens.lock().await.remove(&decision.id);
        release_guards(lease_guards).await;

        let redacted = redact_tool_result(&tool_result);
        let (state, status, error_class) = if token.is_cancelled() {
            (
                DecisionExecutionState::Cancelled,
                DecisionExecutionStatus::Cancelled,
                "Cancelled",
            )
        } else if tool_result.success {
            (
                DecisionExecutionState::Executed,
                DecisionExecutionStatus::Executed,
                "",
            )
        } else {
            (
                DecisionExecutionState::Failed,
                DecisionExecutionStatus::ToolFailed,
                "ToolFailed",
            )
        };
        execution = self
            .mark_execution(
                &execution,
                state,
                DecisionExecutionUpdate {
                    redacted_tool_result: Some(redacted.clone()),
                    error_class: if error_class.is_empty() {
                        None
                    } else {
                        Some(error_class.to_string())
                    },
                    error_message: tool_result.error.clone(),
                    completed: true,
                    ..DecisionExecutionUpdate::default()
                },
            )
            .unwrap_or(execution);

        DecisionExecutionResult {
            status,
            execution_started: true,
            decision: Some(decision),
            execution: Some(execution),
            gate: gate_summary(&final_gate.outcome),
            grounding: grounding_summary,
            lease_conflict: None,
            tool_result: Some(redacted),
            error: tool_result.error,
        }
    }

    fn mark_execution(
        &self,
        execution: &DecisionExecutionRecord,
        state: DecisionExecutionState,
        update: DecisionExecutionUpdate,
    ) -> Result<DecisionExecutionRecord, crate::agent::collaborative_decision::DecisionStoreError>
    {
        self.decision_store.update_execution_state(
            &execution.decision_id,
            &execution.action_hash,
            state,
            update,
            "resume_executor",
        )
    }
}

fn validate_action_context(
    decision: &InteractionDecision,
    request: &DecisionExecutionRequest,
) -> Result<(), String> {
    let Some(action) = decision.action_proposal.as_ref() else {
        return Err("missing_action_proposal_before_execution".to_string());
    };
    if action.action_hash != decision.action_hash || action.target_hash != decision.target_hash {
        return Err("stored_action_proposal_hash_mismatch".to_string());
    }
    if let (Some(expected_session), Some(actual_session)) = (
        action.target.session_id.as_deref(),
        request.session_id.as_deref(),
    ) {
        if expected_session != actual_session {
            return Err(format!(
                "session_changed_before_execution:{expected_session}!={actual_session}"
            ));
        }
    }
    if let (Some(expected_workspace), Some(actual_workspace)) = (
        action.target.workspace_id.as_deref(),
        request.workspace_id.as_deref(),
    ) {
        if expected_workspace != actual_workspace {
            return Err(format!(
                "workspace_changed_before_execution:{expected_workspace}!={actual_workspace}"
            ));
        }
    }
    Ok(())
}

fn validate_tool_versions(
    action: &crate::agent::collaborative_decision::ActionProposal,
) -> Result<(), String> {
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
    Ok(())
}

fn gate_summary(outcome: &ResumeGateOutcome) -> serde_json::Value {
    match outcome {
        ResumeGateOutcome::Ready => serde_json::json!({ "status": "ready" }),
        ResumeGateOutcome::MissingActionProposal => {
            serde_json::json!({ "status": "missing_action_proposal" })
        }
        ResumeGateOutcome::StaleActionProposal { reason } => {
            serde_json::json!({ "status": "stale_action_proposal", "reason": reason })
        }
        ResumeGateOutcome::Block { reason } => {
            serde_json::json!({ "status": "blocked", "reason": reason })
        }
        ResumeGateOutcome::RiskIncreased {
            previous,
            current,
            reason,
        } => serde_json::json!({
            "status": "risk_increased",
            "previous": previous,
            "current": current,
            "reason": reason
        }),
        ResumeGateOutcome::RequiresApproval { risk_level, reason } => serde_json::json!({
            "status": "requires_approval",
            "risk_level": risk_level,
            "reason": reason
        }),
    }
}

fn grounding_summary(facts: &OperationalFacts) -> serde_json::Value {
    serde_json::json!({
        "collected": true,
        "focused_app": facts.focused_app,
        "terminal_cwd": facts.terminal_cwd,
        "open_project_path": facts.open_project_path,
        "visible_window_count": facts.visible_windows.len(),
        "monitor_count": facts.monitors.len(),
        "process_subset_count": facts.running_process_subset.len(),
    })
}

fn redact_tool_result(result: &ToolResult) -> serde_json::Value {
    let data_kind = match &result.data {
        serde_json::Value::Null => "null",
        serde_json::Value::Bool(_) => "bool",
        serde_json::Value::Number(_) => "number",
        serde_json::Value::String(_) => "string",
        serde_json::Value::Array(_) => "array",
        serde_json::Value::Object(_) => "object",
    };
    serde_json::json!({
        "success": result.success,
        "error": result.error,
        "data_kind": data_kind,
        "data_summary": summarize_json(&result.data),
    })
}

fn summarize_json(value: &serde_json::Value) -> serde_json::Value {
    match value {
        serde_json::Value::String(text) => serde_json::json!({
            "chars": text.chars().count(),
            "preview": text.chars().take(256).collect::<String>(),
            "truncated": text.chars().count() > 256,
        }),
        serde_json::Value::Array(items) => serde_json::json!({ "items": items.len() }),
        serde_json::Value::Object(map) => {
            let mut keys = map.keys().take(16).cloned().collect::<Vec<_>>();
            keys.sort();
            serde_json::json!({ "keys": keys, "key_count": map.len() })
        }
        other => other.clone(),
    }
}

async fn release_guards(guards: Vec<ResourceLeaseGuard>) {
    for guard in guards {
        guard.release().await;
    }
}

fn result_error(
    status: DecisionExecutionStatus,
    decision: Option<InteractionDecision>,
    execution: Option<DecisionExecutionRecord>,
    error: String,
) -> DecisionExecutionResult {
    DecisionExecutionResult {
        status,
        execution_started: false,
        decision,
        execution,
        gate: serde_json::json!({ "status": "not_evaluated" }),
        grounding: serde_json::json!({ "collected": false }),
        lease_conflict: None,
        tool_result: None,
        error: Some(error),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use rusqlite::Connection;

    use crate::agent::collaborative_decision::{
        Actor, DecisionCandidate, Rollbackability, TargetBinding,
    };
    use crate::agent::execution_authority::{BindingSource, ExecutionBinding, ValidationResult};
    use crate::agent::execution_gate::build_action_proposal;
    use crate::agent::turn_memory::ExecutionTarget;
    use crate::safety::RiskLevel;
    use crate::tools::registry::{ParamDef, ToolDef, ToolHandler};

    struct OkTool;

    #[async_trait]
    impl ToolHandler for OkTool {
        async fn execute_with_context(
            &self,
            _params: serde_json::Value,
            _ctx: crate::tools::ToolContext,
        ) -> ToolResult {
            ToolResult::ok(serde_json::json!({ "secret": "redact-me", "ok": true }))
        }
    }

    fn registry_with_tool(name: &str, category: &str) -> Arc<ToolRegistry> {
        let registry = Arc::new(ToolRegistry::new());
        registry.register(
            ToolDef {
                name: name.to_string(),
                description: "test tool".to_string(),
                category: category.to_string(),
                default_tier: RiskLevel::Yellow,
                min_tier: "lite",
                parameters: vec![ParamDef {
                    name: "path".to_string(),
                    param_type: "string".to_string(),
                    description: "path".to_string(),
                    required: false,
                    default: None,
                }],
            },
            Arc::new(OkTool),
        );
        registry
    }

    fn executor(registry: Arc<ToolRegistry>, store: Arc<DecisionStore>) -> ResumeExecutor {
        ResumeExecutor::new(
            registry,
            Arc::new(PolicyEngine::new()),
            store,
            Arc::new(AuditLogger::new(Connection::open_in_memory().unwrap())),
        )
        .with_grounder(Arc::new(
            crate::agent::environment_grounder::NoopEnvironmentGrounder,
        ))
    }

    fn host_authority() -> ValidationResult {
        ValidationResult::Authorized(ExecutionBinding {
            target: ExecutionTarget::Host,
            confidence: 0.95,
            source: BindingSource::ExplicitUser,
            is_destructive: false,
            is_explicit: true,
        })
    }

    async fn resolved_approval_decision(
        store: &DecisionStore,
        tool_name: &str,
        params: serde_json::Value,
    ) -> InteractionDecision {
        let action = build_action_proposal("session-1", tool_name, &params, &host_authority());
        let decision = store
            .create_decision_for_action(
                &action,
                DecisionCandidate::approval(
                    tool_name,
                    "approval required",
                    RiskLevel::Yellow,
                    Rollbackability::Compensatable,
                    vec![tool_name.to_string()],
                    Some("test.policy".to_string()),
                ),
            )
            .unwrap();
        store
            .resolve_with_version(&decision.id, decision.version, "approve", "test")
            .unwrap()
            .unwrap()
    }

    #[tokio::test]
    async fn executes_resolved_proposal_once() {
        let store = Arc::new(DecisionStore::in_memory());
        let registry = registry_with_tool("write_file", "file_ops");
        let executor = executor(registry, Arc::clone(&store));
        let decision = resolved_approval_decision(
            &store,
            "write_file",
            serde_json::json!({ "path": "/tmp/kria-phase5.txt", "content": "ok" }),
        )
        .await;

        let result = executor
            .execute_resolved_decision(DecisionExecutionRequest {
                decision_id: decision.id.clone(),
                expected_version: Some(decision.version),
                expected_action_hash: Some(decision.action_hash.clone()),
                expected_target_hash: Some(decision.target_hash.clone()),
                session_id: Some("session-1".to_string()),
                workspace_id: None,
            })
            .await;

        assert!(matches!(result.status, DecisionExecutionStatus::Executed));
        assert!(result.execution_started);
        assert_eq!(
            result.execution.as_ref().unwrap().state,
            DecisionExecutionState::Executed
        );

        let duplicate = executor
            .execute_resolved_decision(DecisionExecutionRequest {
                decision_id: decision.id.clone(),
                expected_version: Some(decision.version),
                expected_action_hash: Some(decision.action_hash.clone()),
                expected_target_hash: Some(decision.target_hash.clone()),
                session_id: Some("session-1".to_string()),
                workspace_id: None,
            })
            .await;

        assert!(matches!(
            duplicate.status,
            DecisionExecutionStatus::Duplicate
        ));
        assert!(!duplicate.execution_started);
    }

    #[tokio::test]
    async fn risk_increase_invalidates_before_execution() {
        let store = Arc::new(DecisionStore::in_memory());
        let registry = registry_with_tool("execute_bash", "shell");
        let executor = executor(registry, Arc::clone(&store));
        let action = build_action_proposal(
            "session-1",
            "execute_bash",
            &serde_json::json!({ "command": "sudo apt install cowsay" }),
            &host_authority(),
        );
        let decision = store
            .create_decision_for_action(
                &action,
                DecisionCandidate::target_selection(
                    "Select target",
                    vec!["host".to_string()],
                    "execute_bash",
                ),
            )
            .unwrap();
        let resolved = store
            .resolve_with_version(&decision.id, decision.version, "host", "test")
            .unwrap()
            .unwrap();

        let result = executor
            .execute_resolved_decision(DecisionExecutionRequest {
                decision_id: resolved.id.clone(),
                expected_version: Some(resolved.version),
                expected_action_hash: Some(resolved.action_hash.clone()),
                expected_target_hash: Some(resolved.target_hash.clone()),
                session_id: Some("session-1".to_string()),
                workspace_id: None,
            })
            .await;

        assert!(matches!(
            result.status,
            DecisionExecutionStatus::BlockedByGate
        ));
        assert!(!result.execution_started);
        assert_eq!(
            store.decision(&resolved.id).unwrap().status,
            DecisionStatus::Invalidated
        );
    }

    #[test]
    fn action_proposal_versions_participate_in_hash() {
        let target = TargetBinding::new("host", "local");
        let action = crate::agent::collaborative_decision::ActionProposal::new(
            "workflow-a",
            "attempt-1",
            "stage-1",
            "write_file",
            serde_json::json!({ "path": "/tmp/a", "content": "ok" }),
            target,
            Actor::Runtime,
        );

        assert_eq!(
            action.tool_schema_version,
            crate::agent::collaborative_decision::current_tool_schema_version()
        );
        assert_eq!(
            action.tool_registry_version,
            crate::agent::collaborative_decision::current_tool_registry_version()
        );
        assert!(!action.action_hash.is_empty());
    }
}
