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
use crate::agent::os_action_authority::is_native_os_action;
use crate::agent::resource_lease::{AccessMode, ResourceKind, ResourceRequirement};
use crate::agent::turn_memory::ExecutionTarget;
use crate::os_control::contract::{
    DecisionId, GrantDecision, GrantId, GrantNonce, SnapshotRevision,
};
use crate::safety::{PolicyDecision, PolicyEngine, RiskLevel};
use crate::tools::preflight;

use sha2::{Digest, Sha256};
use std::time::SystemTime;

/// Non-forgeable authority proof that `ExecutionGate` — the **only** native-OS
/// admission authority (design §2.1, OSC-001) — admitted one typed native-OS
/// action.
///
/// # Authority reconciliation (Task 0.3)
///
/// A native-OS tool handler (wired in later tasks) will require a borrowed
/// `OsActionGrant` before it can dispatch a host effect. The grant can be minted
/// **only** inside this module ([`OsActionGrant::mint`] is private to
/// `execution_gate`), so neither the extension capability plane
/// (`CapabilityPlatform`, `DefaultPermissionEngine`, `GrantStore`), a GUI
/// approval override, nor the resume path can fabricate one — they have no way to
/// obtain the authority a native-OS mutation requires.
///
/// The grant is bound to the session, action, exact parameters (argv), host
/// target, canonical resource set, and risk that were admitted. [`Self::matches`]
/// recomputes that binding from live values, so a changed argv / action / target
/// / resource invalidates the authority (OSC-001 acceptance criteria 3–5).
///
/// # Deferred to Tasks 1.1 / 1.6 / 1.7
///
/// This is the 0.3 authority seam. Task 1.1 enriches it with the capability
/// revision, expiry, and durable decision linkage; Task 1.7 seals it together
/// with held resource leases and a committed audit-admission token into the
/// non-cloneable `MutationPermit` that providers actually consume. Nothing here
/// grants lease/audit authority on its own.
/// Default lifetime of a freshly minted grant. A grant is only valid for the
/// short mutation window between admission/resume and runtime sealing.
const GRANT_TTL: Duration = Duration::from_secs(120);

#[derive(Debug, Clone)]
pub struct OsActionGrant {
    grant_id: GrantId,
    session_id: String,
    action: String,
    params_digest: String,
    target: ExecutionTarget,
    resource_set_digest: String,
    risk_level: RiskLevel,
    authority_digest: String,
    // ── Task 1.1 enrichment (design §4 `ExecutionGrant`) ────────────────────
    decision: GrantDecision,
    decision_id: Option<DecisionId>,
    capability_snapshot_revision: SnapshotRevision,
    issued_at: SystemTime,
    expires_at: SystemTime,
    nonce: GrantNonce,
}

impl OsActionGrant {
    /// Mint an authority proof. **Private to `execution_gate`** so only the gate
    /// (this module) can issue native-OS authority.
    fn mint(
        session_id: &str,
        action: &str,
        params: &serde_json::Value,
        target: ExecutionTarget,
        resource_requirements: &[ResourceRequirement],
        risk_level: RiskLevel,
        decision: GrantDecision,
        decision_id: Option<DecisionId>,
        capability_snapshot_revision: SnapshotRevision,
    ) -> Self {
        let params_digest = digest_params(params);
        let resource_set_digest = digest_resource_set(resource_requirements);
        let authority_digest = compute_authority_digest(
            session_id,
            action,
            &params_digest,
            target,
            &resource_set_digest,
            risk_level,
        );
        let issued_at = SystemTime::now();
        Self {
            grant_id: GrantId::new(uuid::Uuid::new_v4().to_string()),
            session_id: session_id.to_string(),
            action: action.to_string(),
            params_digest,
            target,
            resource_set_digest,
            risk_level,
            authority_digest,
            decision,
            decision_id,
            capability_snapshot_revision,
            issued_at,
            expires_at: issued_at + GRANT_TTL,
            nonce: GrantNonce::new(uuid::Uuid::new_v4().to_string()),
        }
    }

    /// The opaque grant identity.
    pub fn grant_id(&self) -> &GrantId {
        &self.grant_id
    }

    /// How this grant's admission was decided.
    pub fn decision(&self) -> GrantDecision {
        self.decision
    }

    /// The durable decision linkage, when the grant resumed an approval.
    pub fn decision_id(&self) -> Option<&DecisionId> {
        self.decision_id.as_ref()
    }

    /// The capability snapshot revision the grant was issued under (OSC-001.5).
    pub fn capability_snapshot_revision(&self) -> SnapshotRevision {
        self.capability_snapshot_revision
    }

    /// The single-use grant nonce (replay defence for runtime sealing).
    pub fn nonce(&self) -> &GrantNonce {
        &self.nonce
    }

    /// When the grant was issued.
    pub fn issued_at(&self) -> SystemTime {
        self.issued_at
    }

    /// When the grant expires.
    pub fn expires_at(&self) -> SystemTime {
        self.expires_at
    }

    /// Whether the grant has expired relative to `now`.
    pub fn is_expired(&self, now: SystemTime) -> bool {
        now >= self.expires_at
    }

    pub fn session_id(&self) -> &str {
        &self.session_id
    }

    pub fn action(&self) -> &str {
        &self.action
    }

    pub fn target(&self) -> ExecutionTarget {
        self.target
    }

    pub fn risk_level(&self) -> RiskLevel {
        self.risk_level
    }

    pub fn params_digest(&self) -> &str {
        &self.params_digest
    }

    pub fn resource_set_digest(&self) -> &str {
        &self.resource_set_digest
    }

    /// The opaque authority digest binding all admitted facts together.
    pub fn authority_digest(&self) -> &str {
        &self.authority_digest
    }

    /// Recompute the authority binding from live values and compare. Any change
    /// to argv (`params`), `action`, host `target`, or the canonical resource set
    /// yields a different digest, so a stale/forged grant is rejected before a
    /// provider is ever reached (OSC-001).
    pub fn matches(
        &self,
        session_id: &str,
        action: &str,
        params: &serde_json::Value,
        target: ExecutionTarget,
        resource_requirements: &[ResourceRequirement],
    ) -> bool {
        let params_digest = digest_params(params);
        let resource_set_digest = digest_resource_set(resource_requirements);
        let recomputed = compute_authority_digest(
            session_id,
            action,
            &params_digest,
            target,
            &resource_set_digest,
            self.risk_level,
        );
        recomputed == self.authority_digest
    }
}

#[cfg(feature = "os-control-test")]
impl OsActionGrant {
    /// Mint a grant directly for deny-live OS-control tests. Gated to
    /// `os-control-test`; production grants are minted only by the gate's
    /// admission/resume paths in this module.
    #[must_use]
    pub fn for_test(
        session_id: &str,
        action: &str,
        params: &serde_json::Value,
        target: ExecutionTarget,
        resource_requirements: &[ResourceRequirement],
        risk_level: RiskLevel,
    ) -> Self {
        Self::mint(
            session_id,
            action,
            params,
            target,
            resource_requirements,
            risk_level,
            GrantDecision::Approved,
            None,
            SnapshotRevision(1),
        )
    }

    /// Like [`Self::for_test`] but already expired, for exercising the
    /// expired-grant rejection path. Gated to `os-control-test`.
    #[must_use]
    pub fn for_test_expired(
        session_id: &str,
        action: &str,
        params: &serde_json::Value,
        target: ExecutionTarget,
        resource_requirements: &[ResourceRequirement],
        risk_level: RiskLevel,
    ) -> Self {
        let mut grant = Self::for_test(
            session_id,
            action,
            params,
            target,
            resource_requirements,
            risk_level,
        );
        grant.expires_at = SystemTime::now() - Duration::from_secs(1);
        grant
    }
}

fn digest_params(params: &serde_json::Value) -> String {
    // serde_json's default `Map` is a sorted `BTreeMap`, so serialization is
    // deterministic for equal logical values.
    let mut hasher = Sha256::new();
    hasher.update(params.to_string().as_bytes());
    hex::encode(hasher.finalize())
}

/// Delegate to the single canonical resource-set digest (Task 1.6): the exact
/// same derivation the OS resource coordinator recomputes when acquiring leases,
/// so there is no divergent second computation of a grant's resource binding.
fn digest_resource_set(requirements: &[ResourceRequirement]) -> String {
    crate::agent::resource_lease::canonical_resource_set_digest(requirements)
}

fn compute_authority_digest(
    session_id: &str,
    action: &str,
    params_digest: &str,
    target: ExecutionTarget,
    resource_set_digest: &str,
    risk_level: RiskLevel,
) -> String {
    let mut hasher = Sha256::new();
    for field in [
        session_id,
        action,
        params_digest,
        target.as_str(),
        resource_set_digest,
    ] {
        hasher.update(field.as_bytes());
        hasher.update(b"\x1f");
    }
    hasher.update(format!("{risk_level:?}").as_bytes());
    hex::encode(hasher.finalize())
}

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
    /// Present only when the gate admitted a *typed native-OS action* to
    /// [`ExecutionGateOutcome::Proceed`]. This is the sole authority proof a
    /// native-OS handler may consume; a bare `Proceed` is insufficient for a host
    /// mutation. `None` for every non-OS action and for any non-`Proceed`
    /// outcome (design §2.1, OSC-001).
    pub os_action_grant: Option<OsActionGrant>,
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
    /// Present only when a *typed native-OS action* revalidated to
    /// [`ResumeGateOutcome::Ready`] after durable approval. Same authority rules
    /// as [`ExecutionGateEvaluation::os_action_grant`].
    pub os_action_grant: Option<OsActionGrant>,
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
                os_action_grant: None,
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
                os_action_grant: None,
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
                os_action_grant: None,
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
                    os_action_grant: None,
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
                        os_action_grant: None,
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
                        os_action_grant: None,
                    };
                }

                // ExecutionGate is the ONE native-OS admission authority: only a
                // typed native-OS action that reaches `Proceed` (Host-bound,
                // unblocked, no pending approval) is issued an `OsActionGrant`.
                let os_action_grant = mint_os_action_grant(
                    input.session_id,
                    input.action,
                    input.params,
                    &authority_result,
                    &resource_requirements,
                    policy_decision.risk_level,
                );

                ExecutionGateEvaluation {
                    action_proposal: Some(action_proposal),
                    authority_result: Some(authority_result),
                    policy_decision: Some(policy_decision),
                    resource_requirements,
                    outcome: ExecutionGateOutcome::Proceed,
                    os_action_grant,
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
                os_action_grant: None,
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
                os_action_grant: None,
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
                os_action_grant: None,
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
                os_action_grant: None,
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
                os_action_grant: None,
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

        // Issue the native-OS authority proof only when a typed native-OS action
        // revalidated cleanly to `Ready`. Host-bound by invariant (OSC-002).
        let os_action_grant = if matches!(outcome, ResumeGateOutcome::Ready)
            && is_native_os_action(&action_proposal.tool_name)
        {
            Some(OsActionGrant::mint(
                &action_proposal.workflow_id,
                &action_proposal.tool_name,
                &action_proposal.parameters,
                ExecutionTarget::Host,
                &resource_requirements,
                policy_decision.risk_level,
                // A resume grant is backed by a durable approved decision.
                GrantDecision::Approved,
                Some(DecisionId::new(decision.id.clone())),
                SnapshotRevision::UNPROBED,
            ))
        } else {
            None
        };

        ResumeGateEvaluation {
            action_proposal: Some(action_proposal),
            policy_decision: Some(policy_decision),
            resource_requirements,
            outcome,
            os_action_grant,
        }
    }
}

/// Mint an [`OsActionGrant`] for a native-OS action that has been admitted to
/// `Proceed`. Returns `None` for non-OS actions and refuses to bind to any target
/// other than the local host (OSC-002 host-only invariant).
fn mint_os_action_grant(
    session_id: &str,
    action: &str,
    params: &serde_json::Value,
    authority_result: &ValidationResult,
    resource_requirements: &[ResourceRequirement],
    risk_level: RiskLevel,
) -> Option<OsActionGrant> {
    if !is_native_os_action(action) {
        return None;
    }
    let target = match authority_result {
        ValidationResult::Authorized(binding) => binding.target,
        _ => return None,
    };
    // Native OS effects are statically host-bound; never issue authority for a
    // VM / container / remote / cloud target.
    if target != ExecutionTarget::Host {
        return None;
    }
    Some(OsActionGrant::mint(
        session_id,
        action,
        params,
        target,
        resource_requirements,
        risk_level,
        // A `Proceed` grant is a no-confirmation admission (GREEN read /
        // idempotent); it carries no durable decision linkage. Capability
        // probing (Task 1.3) will thread the live snapshot revision here.
        GrantDecision::NoConfirmationRequired,
        None,
        SnapshotRevision::UNPROBED,
    ))
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
    // A canonical native-OS action (a frozen §10 tool) derives its exclusive
    // write-resource set from its manifest `canonical_resource_derivation`
    // (Task 1.6, OSC-008/OSC-009). This is the single authoritative mapping for
    // every OS tool; the legacy per-name file branch below is superseded because
    // the file tools are themselves native-OS actions.
    if is_native_os_action(action) {
        return crate::os_control::resource::os_write_requirements(action, params);
    }

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

        // `write_file` is a canonical native-OS action, so its resources now come
        // from the manifest-driven Task 1.6 derivation: one exclusive OS-control
        // path resource whose scope is the canonical `path/<value>` key.
        let file = declare_resource_requirements(
            "write_file",
            &serde_json::json!({ "path": "/tmp/kria-resource-test.txt" }),
        );
        assert_eq!(file.len(), 1);
        assert_eq!(file[0].kind, ResourceKind::OsControl);
        assert_eq!(file[0].scope, "path//tmp/kria-resource-test.txt");
        assert_eq!(file[0].access_mode, AccessMode::Exclusive);
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
    fn native_os_proceed_issues_os_action_grant_bound_to_action() {
        // A typed native-OS read (`get_audio_state`) is GREEN per the frozen
        // contract (`get_wifi_networks` is RED there — visible SSIDs reveal location) → Proceed, and the
        // gate — the sole native-OS authority — issues a bound `OsActionGrant`.
        let gate = ExecutionGate::new(
            Arc::new(PolicyEngine::new()),
            Arc::new(DecisionStore::in_memory()),
        );
        let params = serde_json::json!({});
        let evaluated = gate.evaluate(ExecutionGateInput {
            session_id: "session-os",
            user_text: "what wifi networks are nearby",
            action: "get_audio_state",
            params: &params,
            destructive_hint: false,
        });

        assert!(matches!(evaluated.outcome, ExecutionGateOutcome::Proceed));
        let grant = evaluated
            .os_action_grant
            .expect("typed native-OS Proceed must carry an OsActionGrant");
        assert_eq!(grant.action(), "get_audio_state");
        assert_eq!(grant.session_id(), "session-os");
        assert_eq!(grant.target(), ExecutionTarget::Host);
        assert!(grant.matches(
            "session-os",
            "get_audio_state",
            &params,
            ExecutionTarget::Host,
            &evaluated.resource_requirements,
        ));
    }

    #[test]
    fn generic_action_proceed_issues_no_os_action_grant() {
        // A non-OS GREEN action reaches Proceed but is NOT a native-OS action, so
        // no authority is minted — nothing to consume for a host mutation.
        let gate = ExecutionGate::new(
            Arc::new(PolicyEngine::new()),
            Arc::new(DecisionStore::in_memory()),
        );
        let params = serde_json::json!({});
        let evaluated = gate.evaluate(ExecutionGateInput {
            session_id: "session-generic",
            user_text: "what is two plus two",
            action: "calculate",
            params: &params,
            destructive_hint: false,
        });

        assert!(matches!(evaluated.outcome, ExecutionGateOutcome::Proceed));
        assert!(
            evaluated.os_action_grant.is_none(),
            "a non-OS action must never carry an OsActionGrant"
        );
    }

    #[test]
    fn os_action_grant_is_invalidated_by_changed_argv_action_target_or_resource() {
        let gate = ExecutionGate::new(
            Arc::new(PolicyEngine::new()),
            Arc::new(DecisionStore::in_memory()),
        );
        let params = serde_json::json!({ "domain": "connectivity" });
        let evaluated = gate.evaluate(ExecutionGateInput {
            session_id: "session-os",
            user_text: "list nearby wifi",
            action: "get_audio_state",
            params: &params,
            destructive_hint: false,
        });
        let grant = evaluated
            .os_action_grant
            .expect("native-OS Proceed must carry a grant");
        let reqs = &evaluated.resource_requirements;

        // Exact rebind still matches.
        assert!(grant.matches(
            "session-os",
            "get_audio_state",
            &params,
            ExecutionTarget::Host,
            reqs
        ));

        // Changed argv (params) invalidates authority.
        let changed_params = serde_json::json!({ "domain": "power" });
        assert!(!grant.matches(
            "session-os",
            "get_audio_state",
            &changed_params,
            ExecutionTarget::Host,
            reqs
        ));

        // Changed action invalidates authority.
        assert!(!grant.matches(
            "session-os",
            "toggle_wifi",
            &params,
            ExecutionTarget::Host,
            reqs
        ));

        // Changed target invalidates authority (host-only binding).
        assert!(!grant.matches(
            "session-os",
            "get_audio_state",
            &params,
            ExecutionTarget::Vm,
            reqs
        ));

        // Changed resource set invalidates authority.
        let extra_resource = vec![ResourceRequirement::new(
            ResourceKind::FilesystemPath,
            "connectivity:radio",
            AccessMode::Exclusive,
            Duration::from_secs(30),
        )];
        assert!(!grant.matches(
            "session-os",
            "get_audio_state",
            &params,
            ExecutionTarget::Host,
            &extra_resource
        ));
    }

    #[test]
    fn resume_ready_for_native_os_action_issues_grant_only_via_the_gate() {
        // `write_file` is a canonical native-OS action; a clean revalidation to
        // Ready is the ONLY way the resume path yields native-OS authority.
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
            "session-os",
            "write_file",
            &serde_json::json!({
                "path": "/tmp/kria-os-resume.txt",
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
        let grant = evaluated
            .os_action_grant
            .expect("native-OS resume Ready must carry an OsActionGrant");
        assert_eq!(grant.action(), "write_file");
        assert_eq!(grant.target(), ExecutionTarget::Host);
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

    // ── Task 1.1 (OSC-001.9): committed resolution gates the OS grant ───────

    fn host_authority() -> ValidationResult {
        ValidationResult::Authorized(ExecutionBinding {
            target: ExecutionTarget::Host,
            confidence: 0.9,
            source: BindingSource::ExplicitUser,
            is_destructive: true,
            is_explicit: true,
        })
    }

    fn os_approval_candidate() -> crate::agent::collaborative_decision::DecisionCandidate {
        crate::agent::collaborative_decision::DecisionCandidate::approval(
            "reboot_system",
            "approval required",
            RiskLevel::Red,
            crate::agent::collaborative_decision::Rollbackability::Irreversible,
            vec!["power:session".to_string()],
            Some("policy.os".to_string()),
        )
    }

    #[test]
    fn raw_os_approval_without_committed_resolution_mints_no_grant() {
        // A RED native-OS action requires approval. Until the durable resolution
        // commits (status Resolved + resolution "approve"), the resume gate never
        // reaches Ready and mints NO OsActionGrant — a raw UI approval is inert.
        let store = Arc::new(DecisionStore::in_memory());
        let gate = ExecutionGate::new(Arc::new(PolicyEngine::new()), Arc::clone(&store));
        let action = build_action_proposal(
            "session-os",
            "reboot_system",
            &serde_json::json!({}),
            &host_authority(),
        );
        let decision = store
            .create_decision_for_action(&action, os_approval_candidate())
            .expect("OS decision creation commits to SQLite");

        // Pending (unresolved) → no grant.
        let pending_eval = gate.revalidate_resume(&decision, false);
        assert!(matches!(
            pending_eval.outcome,
            ResumeGateOutcome::RequiresApproval { .. }
        ));
        assert!(
            pending_eval.os_action_grant.is_none(),
            "an uncommitted (pending) OS approval must mint no grant"
        );

        // Committed approval → Ready → grant.
        let resolved = store
            .resolve_with_version(&decision.id, decision.version, "approve", "user_gui")
            .expect("resolution commits")
            .expect("decision exists");
        let ready_eval = gate.revalidate_resume(&resolved, false);
        assert!(matches!(ready_eval.outcome, ResumeGateOutcome::Ready));
        let grant = ready_eval
            .os_action_grant
            .expect("a committed OS approval mints exactly one grant");
        assert_eq!(grant.action(), "reboot_system");
        assert_eq!(grant.target(), ExecutionTarget::Host);
    }

    #[test]
    fn os_resolution_that_fails_to_commit_mints_no_grant() {
        // Create through a healthy authority, then force the resolution commit to
        // fail. The projection stays Pending, so the resume gate mints no grant —
        // proving the grant is gated on the SQLite commit, not the raw UI click.
        let store = Arc::new(DecisionStore::in_memory());
        let gate = ExecutionGate::new(Arc::new(PolicyEngine::new()), Arc::clone(&store));
        let action = build_action_proposal(
            "session-os",
            "reboot_system",
            &serde_json::json!({}),
            &host_authority(),
        );
        let decision = store
            .create_decision_for_action(&action, os_approval_candidate())
            .expect("create commits");

        store.force_os_persistence_failure();
        let err = store
            .resolve_with_version(&decision.id, decision.version, "approve", "user_gui")
            .expect_err("resolution must fail closed on persistence failure");
        assert!(matches!(
            err,
            crate::agent::collaborative_decision::DecisionStoreError::OsPersistence { .. }
        ));

        let uncommitted = store.decision(&decision.id).expect("decision present");
        let eval = gate.revalidate_resume(&uncommitted, false);
        assert!(matches!(
            eval.outcome,
            ResumeGateOutcome::RequiresApproval { .. }
        ));
        assert!(
            eval.os_action_grant.is_none(),
            "an OS approval whose resolution failed to commit must mint no grant"
        );
    }
}
