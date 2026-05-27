//! Deterministic execution gate for tool-bound actions.
//!
//! This is intentionally small. It centralizes the existing readiness,
//! preflight, execution-authority, policy, and durable-decision checks without
//! becoming a scheduler or workflow runtime.

use std::sync::Arc;
use std::time::Duration;

use crate::agent::collaborative_decision::{
    compute_action_hash, compute_target_hash, ActionProposal, Actor, DecisionStore,
    InteractionDecision, TargetBinding,
};
use crate::agent::execution_authority::{self, ValidationResult};
use crate::agent::resource_lease::{AccessMode, ResourceKind, ResourceRequirement};
use crate::agent::turn_memory::ExecutionTarget;
use crate::safety::{PolicyDecision, PolicyEngine, RiskLevel};
use crate::tools::preflight;

#[derive(Debug, Clone)]
pub enum ExecutionGateOutcome {
    Proceed,
    Block {
        reason: String,
    },
    PauseForDecision {
        decision_id: String,
        decision_type: &'static str,
        reason: String,
    },
    RequiresApproval {
        decision: InteractionDecision,
    },
}

#[derive(Debug, Clone)]
pub struct ExecutionGateEvaluation {
    pub action_proposal: Option<ActionProposal>,
    pub authority_result: Option<ValidationResult>,
    pub policy_decision: Option<PolicyDecision>,
    pub resource_requirements: Vec<ResourceRequirement>,
    pub outcome: ExecutionGateOutcome,
}

#[derive(Debug, Clone)]
pub enum ResumeGateOutcome {
    Ready,
    MissingActionProposal,
    StaleActionProposal {
        reason: String,
    },
    Block {
        reason: String,
    },
    RiskIncreased {
        previous: RiskLevel,
        current: RiskLevel,
        reason: String,
    },
    RequiresApproval {
        risk_level: RiskLevel,
        reason: String,
    },
}

impl ResumeGateOutcome {
    pub fn can_execute(&self) -> bool {
        matches!(self, Self::Ready)
    }

    pub fn invalidation_reason(&self) -> Option<String> {
        match self {
            Self::RiskIncreased {
                previous,
                current,
                reason,
            } => Some(format!(
                "risk_increased_before_resume:{previous:?}->{current:?}:{reason}"
            )),
            Self::StaleActionProposal { reason } => {
                Some(format!("stale_action_proposal_before_resume:{reason}"))
            }
            Self::Block { reason } => Some(format!("blocked_before_resume:{reason}")),
            Self::MissingActionProposal => {
                Some("missing_action_proposal_before_resume".to_string())
            }
            Self::RequiresApproval { reason, .. } => {
                Some(format!("approval_required_before_resume:{reason}"))
            }
            Self::Ready => None,
        }
    }
}

#[derive(Debug, Clone)]
pub struct ResumeGateEvaluation {
    pub action_proposal: Option<ActionProposal>,
    pub policy_decision: Option<PolicyDecision>,
    pub resource_requirements: Vec<ResourceRequirement>,
    pub outcome: ResumeGateOutcome,
}

#[derive(Debug, Clone, Copy)]
pub struct ExecutionGateInput<'a> {
    pub session_id: &'a str,
    pub user_text: &'a str,
    pub action: &'a str,
    pub params: &'a serde_json::Value,
    pub destructive_hint: bool,
}

#[derive(Clone)]
pub struct ExecutionGate {
    policy_engine: Arc<PolicyEngine>,
    decision_store: Arc<DecisionStore>,
}

impl ExecutionGate {
    pub fn new(policy_engine: Arc<PolicyEngine>, decision_store: Arc<DecisionStore>) -> Self {
        Self {
            policy_engine,
            decision_store,
        }
    }

    pub fn evaluate(&self, input: ExecutionGateInput<'_>) -> ExecutionGateEvaluation {
        if let Err(reason) = crate::agent::gui_services::check_action_readiness(input.action) {
            return ExecutionGateEvaluation {
                action_proposal: None,
                authority_result: None,
                policy_decision: None,
                resource_requirements: Vec::new(),
                outcome: ExecutionGateOutcome::Block { reason },
            };
        }

        let preflight = preflight::run_preflight(input.action, input.params);
        if !preflight.allowed {
            let reason = preflight
                .blocked_reason
                .unwrap_or_else(|| "preflight validation failed".to_string());
            return ExecutionGateEvaluation {
                action_proposal: None,
                authority_result: None,
                policy_decision: None,
                resource_requirements: Vec::new(),
                outcome: ExecutionGateOutcome::Block {
                    reason: format!("PREFLIGHT_BLOCKED: {reason}"),
                },
            };
        }

        let turn_target = ExecutionTarget::infer(input.user_text, input.action);
        let authority_result = execution_authority::check_execution_authority_with_params(
            input.action,
            input.user_text,
            turn_target,
            Some(input.params),
        );
        let action_proposal = build_action_proposal(
            input.session_id,
            input.action,
            input.params,
            &authority_result,
        );
        let resource_requirements = declare_resource_requirements(input.action, input.params);

        match &authority_result {
            ValidationResult::Blocked { reason, .. } => ExecutionGateEvaluation {
                action_proposal: Some(action_proposal),
                authority_result: Some(authority_result.clone()),
                policy_decision: None,
                resource_requirements,
                outcome: ExecutionGateOutcome::Block {
                    reason: format!("EXECUTION_BLOCKED: {reason}"),
                },
            },
            ValidationResult::NeedsClarification { question, .. } => {
                let outcome = authority_result
                    .to_decision_candidate(input.action)
                    .ok_or_else(|| "authority did not provide a decision candidate".to_string())
                    .and_then(|candidate| {
                        self.decision_store
                            .create_decision_for_action(&action_proposal, candidate)
                            .map_err(|error| error.to_string())
                    })
                    .map(|decision| ExecutionGateOutcome::PauseForDecision {
                        decision_id: decision.id,
                        decision_type: "target_selection",
                        reason: question.clone(),
                    })
                    .unwrap_or_else(|reason| ExecutionGateOutcome::Block {
                        reason: format!("DECISION_STORE_ERROR: {reason}"),
                    });

                ExecutionGateEvaluation {
                    action_proposal: Some(action_proposal),
                    authority_result: Some(authority_result),
                    policy_decision: None,
                    resource_requirements,
                    outcome,
                }
            }
            ValidationResult::Authorized(_) => {
                let policy_decision = self.policy_engine.evaluate_with_modality_hint(
                    input.action,
                    input.params,
                    input.destructive_hint,
                );

                if policy_decision.blocked {
                    return ExecutionGateEvaluation {
                        action_proposal: Some(action_proposal),
                        authority_result: Some(authority_result),
                        policy_decision: Some(policy_decision.clone()),
                        resource_requirements,
                        outcome: ExecutionGateOutcome::Block {
                            reason: format!("POLICY_BLOCKED: {}", policy_decision.reason),
                        },
                    };
                }

                if policy_decision.requires_approval {
                    let outcome = policy_decision
                        .to_decision_candidate(input.params)
                        .ok_or_else(|| "policy did not provide an approval candidate".to_string())
                        .and_then(|candidate| {
                            self.decision_store
                                .create_decision_for_action(&action_proposal, candidate)
                                .map_err(|error| error.to_string())
                        })
                        .map(|decision| ExecutionGateOutcome::RequiresApproval { decision })
                        .unwrap_or_else(|reason| ExecutionGateOutcome::Block {
                            reason: format!("DECISION_STORE_ERROR: {reason}"),
                        });

                    return ExecutionGateEvaluation {
                        action_proposal: Some(action_proposal),
                        authority_result: Some(authority_result),
                        policy_decision: Some(policy_decision),
                        resource_requirements,
                        outcome,
                    };
                }

                ExecutionGateEvaluation {
                    action_proposal: Some(action_proposal),
                    authority_result: Some(authority_result),
                    policy_decision: Some(policy_decision),
                    resource_requirements,
                    outcome: ExecutionGateOutcome::Proceed,
                }
            }
        }
    }

    pub fn revalidate_resume(
        &self,
        decision: &InteractionDecision,
        destructive_hint: bool,
    ) -> ResumeGateEvaluation {
        let Some(action_proposal) = decision.action_proposal.clone() else {
            return ResumeGateEvaluation {
                action_proposal: None,
                policy_decision: None,
                resource_requirements: Vec::new(),
                outcome: ResumeGateOutcome::MissingActionProposal,
            };
        };

        let recomputed_target_hash = compute_target_hash(&action_proposal.target);
        if recomputed_target_hash != decision.target_hash
            || recomputed_target_hash != action_proposal.target_hash
        {
            return ResumeGateEvaluation {
                action_proposal: Some(action_proposal),
                policy_decision: None,
                resource_requirements: Vec::new(),
                outcome: ResumeGateOutcome::StaleActionProposal {
                    reason: "target hash changed since decision was created".to_string(),
                },
            };
        }

        let recomputed_action_hash = compute_action_hash(
            &action_proposal.workflow_id,
            &action_proposal.attempt_id,
            &action_proposal.stage_id,
            &action_proposal.tool_name,
            &action_proposal.parameters,
            &recomputed_target_hash,
            &action_proposal.tool_schema_version,
            &action_proposal.tool_registry_version,
        );
        if recomputed_action_hash != decision.action_hash
            || recomputed_action_hash != action_proposal.action_hash
        {
            return ResumeGateEvaluation {
                action_proposal: Some(action_proposal),
                policy_decision: None,
                resource_requirements: Vec::new(),
                outcome: ResumeGateOutcome::StaleActionProposal {
                    reason: "action hash changed since decision was created".to_string(),
                },
            };
        }

        if let Err(reason) =
            crate::agent::gui_services::check_action_readiness(&action_proposal.tool_name)
        {
            return ResumeGateEvaluation {
                action_proposal: Some(action_proposal),
                policy_decision: None,
                resource_requirements: Vec::new(),
                outcome: ResumeGateOutcome::Block { reason },
            };
        }

        let preflight =
            preflight::run_preflight(&action_proposal.tool_name, &action_proposal.parameters);
        if !preflight.allowed {
            let reason = preflight
                .blocked_reason
                .unwrap_or_else(|| "preflight validation failed".to_string());
            return ResumeGateEvaluation {
                action_proposal: Some(action_proposal),
                policy_decision: None,
                resource_requirements: Vec::new(),
                outcome: ResumeGateOutcome::Block {
                    reason: format!("PREFLIGHT_BLOCKED: {reason}"),
                },
            };
        }

        let policy_decision = self.policy_engine.evaluate_with_modality_hint(
            &action_proposal.tool_name,
            &action_proposal.parameters,
            destructive_hint,
        );
        let resource_requirements =
            declare_resource_requirements(&action_proposal.tool_name, &action_proposal.parameters);

        let outcome = if policy_decision.blocked {
            ResumeGateOutcome::Block {
                reason: format!("POLICY_BLOCKED: {}", policy_decision.reason),
            }
        } else if policy_decision.risk_level > decision.risk_level {
            ResumeGateOutcome::RiskIncreased {
                previous: decision.risk_level,
                current: policy_decision.risk_level,
                reason: policy_decision.reason.clone(),
            }
        } else if policy_decision.requires_approval
            && !(decision.decision_type
                == crate::agent::collaborative_decision::DecisionType::Approval
                && decision.resolution.as_deref() == Some("approve"))
        {
            ResumeGateOutcome::RequiresApproval {
                risk_level: policy_decision.risk_level,
                reason: policy_decision.reason.clone(),
            }
        } else {
            ResumeGateOutcome::Ready
        };

        ResumeGateEvaluation {
            action_proposal: Some(action_proposal),
            policy_decision: Some(policy_decision),
            resource_requirements,
            outcome,
        }
    }
}

pub fn target_binding_from_authority(
    session_id: &str,
    action: &str,
    params: &serde_json::Value,
    authority_result: &ValidationResult,
) -> TargetBinding {
    match authority_result {
        ValidationResult::Authorized(binding) => {
            let mut target = TargetBinding::new("execution_target", binding.target.as_str());
            target.session_id = Some(session_id.to_string());
            target.execution_boundary = Some(binding.target.as_str().to_string());
            target.metadata = serde_json::json!({
                "tool": action,
                "confidence": binding.confidence,
                "source": binding.source.as_str(),
                "is_destructive": binding.is_destructive,
                "is_explicit": binding.is_explicit,
            });
            target
        }
        ValidationResult::NeedsClarification { options, .. } => {
            let mut target = TargetBinding::new("ambiguous_execution_target", action);
            target.session_id = Some(session_id.to_string());
            target.metadata = serde_json::json!({
                "tool": action,
                "options": options,
                "params": params,
            });
            target
        }
        ValidationResult::Blocked { reason, .. } => {
            let mut target = TargetBinding::new("blocked_execution_target", action);
            target.session_id = Some(session_id.to_string());
            target.metadata = serde_json::json!({
                "tool": action,
                "reason": reason,
                "params": params,
            });
            target
        }
    }
}

pub fn build_action_proposal(
    session_id: &str,
    action: &str,
    params: &serde_json::Value,
    authority_result: &ValidationResult,
) -> ActionProposal {
    ActionProposal::new(
        session_id.to_string(),
        "active-attempt".to_string(),
        action.to_string(),
        action.to_string(),
        params.clone(),
        target_binding_from_authority(session_id, action, params, authority_result),
        Actor::Runtime,
    )
}

pub fn declare_resource_requirements(
    action: &str,
    params: &serde_json::Value,
) -> Vec<ResourceRequirement> {
    let mut requirements = Vec::new();
    let short_ttl = Duration::from_secs(30);
    let normal_ttl = Duration::from_secs(120);

    if matches!(
        action,
        "type_text"
            | "click_mouse"
            | "click_element"
            | "press_shortcut"
            | "focus_window"
            | "drag_mouse"
    ) {
        requirements.push(ResourceRequirement::new(
            ResourceKind::GuiForeground,
            "desktop:foreground",
            AccessMode::Exclusive,
            short_ttl,
        ));
        requirements.push(ResourceRequirement::new(
            ResourceKind::KeyboardMouse,
            "desktop:input",
            AccessMode::Exclusive,
            short_ttl,
        ));
    } else if action == "release_all" {
        requirements.push(ResourceRequirement::new(
            ResourceKind::KeyboardMouse,
            "desktop:input",
            AccessMode::Exclusive,
            short_ttl,
        ));
    }

    if matches!(
        action,
        "write_file" | "append_file" | "delete_file" | "move_file"
    ) {
        let scope = params
            .get("path")
            .or_else(|| params.get("target"))
            .or_else(|| params.get("destination"))
            .and_then(|value| value.as_str())
            .unwrap_or("filesystem:unknown");
        requirements.push(ResourceRequirement::new(
            ResourceKind::FilesystemPath,
            scope,
            AccessMode::Write,
            normal_ttl,
        ));
    }

    if matches!(action, "browser_search" | "open_url") {
        requirements.push(ResourceRequirement::new(
            ResourceKind::BrowserProfile,
            "browser:default-profile",
            AccessMode::Write,
            normal_ttl,
        ));
    }

    if matches!(
        action,
        "execute_fleet_command" | "vm_reset" | "vm_snapshot" | "qemu_reset"
    ) {
        let scope = params
            .get("target")
            .or_else(|| params.get("host"))
            .or_else(|| params.get("vm"))
            .and_then(|value| value.as_str())
            .unwrap_or("vm:default");
        requirements.push(ResourceRequirement::new(
            ResourceKind::VmTarget,
            scope,
            AccessMode::Exclusive,
            normal_ttl,
        ));
    }

    requirements
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::execution_authority::{BindingSource, ExecutionBinding};
    use crate::safety::RiskLevel;

    #[test]
    fn action_proposal_binds_session_params_and_authority_target() {
        let authority = ValidationResult::Authorized(ExecutionBinding {
            target: ExecutionTarget::Host,
            confidence: 0.9,
            source: BindingSource::ExplicitUser,
            is_destructive: false,
            is_explicit: true,
        });

        let first = build_action_proposal(
            "session-1",
            "execute_bash",
            &serde_json::json!({ "command": "echo one" }),
            &authority,
        );
        let second = build_action_proposal(
            "session-1",
            "execute_bash",
            &serde_json::json!({ "command": "echo two" }),
            &authority,
        );

        assert_eq!(first.workflow_id, "session-1");
        assert_eq!(first.target.session_id.as_deref(), Some("session-1"));
        assert_eq!(first.target.id, "host");
        assert_eq!(first.target.execution_boundary.as_deref(), Some("host"));
        assert_ne!(first.action_hash, second.action_hash);
        assert_eq!(first.target_hash, second.target_hash);
    }

    #[test]
    fn declares_gui_input_and_filesystem_requirements() {
        let gui = declare_resource_requirements("type_text", &serde_json::json!({}));
        assert!(gui.iter().any(|requirement| {
            requirement.kind == ResourceKind::GuiForeground
                && requirement.access_mode == AccessMode::Exclusive
        }));
        assert!(gui.iter().any(|requirement| {
            requirement.kind == ResourceKind::KeyboardMouse
                && requirement.access_mode == AccessMode::Exclusive
        }));

        let file = declare_resource_requirements(
            "write_file",
            &serde_json::json!({ "path": "/tmp/kria-resource-test.txt" }),
        );
        assert_eq!(file.len(), 1);
        assert_eq!(file[0].kind, ResourceKind::FilesystemPath);
        assert_eq!(file[0].scope, "/tmp/kria-resource-test.txt");
        assert_eq!(file[0].access_mode, AccessMode::Write);
    }

    #[test]
    fn gate_blocks_policy_black_before_execution() {
        let gate = ExecutionGate::new(
            Arc::new(PolicyEngine::new()),
            Arc::new(DecisionStore::in_memory()),
        );
        let params = serde_json::json!({ "command": "rm -rf /" });

        let evaluated = gate.evaluate(ExecutionGateInput {
            session_id: "session-1",
            user_text: "run locally",
            action: "execute_bash",
            params: &params,
            destructive_hint: true,
        });

        match evaluated.outcome {
            ExecutionGateOutcome::Block { reason } => {
                assert!(reason.contains("POLICY_BLOCKED") || reason.contains("PREFLIGHT_BLOCKED"));
            }
            other => panic!("expected block, got {other:?}"),
        }
    }

    #[test]
    fn gate_creates_durable_decision_for_red_policy_approval() {
        let store = Arc::new(DecisionStore::in_memory());
        let gate = ExecutionGate::new(Arc::new(PolicyEngine::new()), Arc::clone(&store));
        let params = serde_json::json!({ "command": "sudo apt install cowsay" });

        let evaluated = gate.evaluate(ExecutionGateInput {
            session_id: "session-1",
            user_text: "run locally",
            action: "execute_bash",
            params: &params,
            destructive_hint: true,
        });

        match evaluated.outcome {
            ExecutionGateOutcome::RequiresApproval { decision } => {
                assert_eq!(decision.workflow_id, "session-1");
                assert_eq!(decision.risk_level, RiskLevel::Red);
                assert!(!decision.action_hash.is_empty());
                assert!(store.decision(&decision.id).is_some());
            }
            other => panic!("expected approval decision, got {other:?}"),
        }
    }

    #[test]
    fn resume_gate_revalidates_resolved_action_without_executing() {
        let store = Arc::new(DecisionStore::in_memory());
        let gate = ExecutionGate::new(Arc::new(PolicyEngine::new()), Arc::clone(&store));
        let authority = ValidationResult::Authorized(ExecutionBinding {
            target: ExecutionTarget::Host,
            confidence: 0.9,
            source: BindingSource::ExplicitUser,
            is_destructive: false,
            is_explicit: true,
        });
        let action = build_action_proposal(
            "session-1",
            "write_file",
            &serde_json::json!({
                "path": "/tmp/kria-resume-gate.txt",
                "content": "ok"
            }),
            &authority,
        );
        let decision = store
            .create_decision_for_action(
                &action,
                crate::agent::collaborative_decision::DecisionCandidate::target_selection(
                    "Select execution target",
                    vec!["host".to_string()],
                    "write_file",
                ),
            )
            .expect("decision should be created");
        let resolved = store
            .resolve_with_version(&decision.id, decision.version, "host", "test")
            .expect("resolution should succeed")
            .expect("decision should exist");

        let evaluated = gate.revalidate_resume(&resolved, false);

        assert!(matches!(evaluated.outcome, ResumeGateOutcome::Ready));
        assert!(!evaluated.resource_requirements.is_empty());
    }

    #[test]
    fn resume_gate_blocks_when_risk_increases_before_resume() {
        let store = Arc::new(DecisionStore::in_memory());
        let gate = ExecutionGate::new(Arc::new(PolicyEngine::new()), Arc::clone(&store));
        let authority = ValidationResult::Authorized(ExecutionBinding {
            target: ExecutionTarget::Host,
            confidence: 0.9,
            source: BindingSource::ExplicitUser,
            is_destructive: true,
            is_explicit: true,
        });
        let action = build_action_proposal(
            "session-1",
            "execute_bash",
            &serde_json::json!({ "command": "sudo apt install cowsay" }),
            &authority,
        );
        let decision = store
            .create_decision_for_action(
                &action,
                crate::agent::collaborative_decision::DecisionCandidate::target_selection(
                    "Select execution target",
                    vec!["host".to_string()],
                    "execute_bash",
                ),
            )
            .expect("decision should be created");
        let resolved = store
            .resolve_with_version(&decision.id, decision.version, "host", "test")
            .expect("resolution should succeed")
            .expect("decision should exist");

        let evaluated = gate.revalidate_resume(&resolved, false);

        assert!(matches!(
            evaluated.outcome,
            ResumeGateOutcome::RiskIncreased {
                previous: RiskLevel::Yellow,
                current: RiskLevel::Red,
                ..
            }
        ));
    }
}
