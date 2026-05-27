//! Deterministic HITL timeline evals.
//!
//! This suite validates the decision lifecycle contracts that GUI cognition
//! depends on. It is intentionally not a human simulator: each case is a fixed
//! event script with structural assertions over decision state and policy.

use serde::{Deserialize, Serialize};

use kria_core::agent::collaborative_decision::{
    ActionProposal, Actor, DecisionCandidate, DecisionResolutionContext, DecisionStatus,
    DecisionStore, DecisionStoreError, Rollbackability, TargetBinding,
};
use kria_core::safety::{PolicyEngine, RiskLevel};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HitlTimelineScript {
    ImmediateApprove,
    ImmediateDeny,
    DelayedApprove,
    AbsentTimeout,
    ConflictingResponse,
    UnsafeApproval,
    Cancel,
    StaleResponse,
}

impl HitlTimelineScript {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::ImmediateApprove => "immediate_approve",
            Self::ImmediateDeny => "immediate_deny",
            Self::DelayedApprove => "delayed_approve",
            Self::AbsentTimeout => "absent_timeout",
            Self::ConflictingResponse => "conflicting_response",
            Self::UnsafeApproval => "unsafe_approval",
            Self::Cancel => "cancel",
            Self::StaleResponse => "stale_response",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HitlTimelineCase {
    pub id: String,
    pub description: String,
    pub script: HitlTimelineScript,
    pub capability_ids: Vec<String>,
    pub failure_mode_ids: Vec<String>,
    pub expected_final_state: HitlTimelineFinalState,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HitlTimelineFinalState {
    ResolvedAndResumable,
    DeniedNoExecution,
    ExpiredNoExecution,
    InvalidatedNoExecution,
    CancelledNoExecution,
    BlockedByPolicy,
}

impl HitlTimelineFinalState {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::ResolvedAndResumable => "resolved_and_resumable",
            Self::DeniedNoExecution => "denied_no_execution",
            Self::ExpiredNoExecution => "expired_no_execution",
            Self::InvalidatedNoExecution => "invalidated_no_execution",
            Self::CancelledNoExecution => "cancelled_no_execution",
            Self::BlockedByPolicy => "blocked_by_policy",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HitlTimelineObservation {
    pub case_id: String,
    pub script: HitlTimelineScript,
    pub final_state: HitlTimelineFinalState,
    pub decision_status: Option<String>,
    pub resolution: Option<String>,
    pub resume_allowed: bool,
    pub side_effect_allowed: bool,
    pub policy_blocked: bool,
    pub rejected_reason: Option<String>,
    pub event_count: usize,
    pub evidence: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HitlTimelineVerdict {
    pub case_id: String,
    pub passed: bool,
    pub explanation: String,
    pub evidence: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HitlTimelineCaseResult {
    pub case: HitlTimelineCase,
    pub observation: HitlTimelineObservation,
    pub verdict: HitlTimelineVerdict,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HitlTimelineSummary {
    pub total: usize,
    pub passed: usize,
    pub failed: usize,
    pub blocked_by_policy: usize,
    pub stale_or_invalidated: usize,
    pub side_effect_allowed_cases: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HitlTimelineReport {
    pub run_id: String,
    pub generated_at: String,
    pub summary: HitlTimelineSummary,
    pub case_results: Vec<HitlTimelineCaseResult>,
}

pub fn hitl_timeline_suite() -> Vec<HitlTimelineCase> {
    vec![
        case(
            "hitl-001-immediate-approve",
            "Immediate approval resolves and permits resume with matching hashes.",
            HitlTimelineScript::ImmediateApprove,
            HitlTimelineFinalState::ResolvedAndResumable,
            &["valid_resolution"],
        ),
        case(
            "hitl-002-immediate-deny",
            "Immediate denial reaches Denied and never permits execution.",
            HitlTimelineScript::ImmediateDeny,
            HitlTimelineFinalState::DeniedNoExecution,
            &["denied_no_execution"],
        ),
        case(
            "hitl-003-delayed-approve",
            "Delayed approval remains resumable only while hashes and version are current.",
            HitlTimelineScript::DelayedApprove,
            HitlTimelineFinalState::ResolvedAndResumable,
            &["delayed_valid_resolution"],
        ),
        case(
            "hitl-004-absent-timeout",
            "Absent user response expires the decision without side effects.",
            HitlTimelineScript::AbsentTimeout,
            HitlTimelineFinalState::ExpiredNoExecution,
            &["timeout_no_execution"],
        ),
        case(
            "hitl-005-conflicting-response",
            "Conflicting response is rejected and invalidates the decision.",
            HitlTimelineScript::ConflictingResponse,
            HitlTimelineFinalState::InvalidatedNoExecution,
            &["conflicting_response_rejected"],
        ),
        case(
            "hitl-006-unsafe-approval",
            "Unsafe approval cannot override a hard policy block.",
            HitlTimelineScript::UnsafeApproval,
            HitlTimelineFinalState::BlockedByPolicy,
            &["unsafe_approval_blocked"],
        ),
        case(
            "hitl-007-cancel",
            "User cancel reaches Cancelled and never permits execution.",
            HitlTimelineScript::Cancel,
            HitlTimelineFinalState::CancelledNoExecution,
            &["cancel_no_default"],
        ),
        case(
            "hitl-008-stale-response",
            "Stale target response is rejected and invalidated before execution.",
            HitlTimelineScript::StaleResponse,
            HitlTimelineFinalState::InvalidatedNoExecution,
            &["stale_response_rejected"],
        ),
    ]
}

pub fn run_hitl_timeline_suite(run_id: impl Into<String>) -> HitlTimelineReport {
    let case_results = hitl_timeline_suite()
        .into_iter()
        .map(run_hitl_timeline_case)
        .collect::<Vec<_>>();
    let total = case_results.len();
    let passed = case_results
        .iter()
        .filter(|result| result.verdict.passed)
        .count();
    let blocked_by_policy = case_results
        .iter()
        .filter(|result| result.observation.policy_blocked)
        .count();
    let stale_or_invalidated = case_results
        .iter()
        .filter(|result| {
            matches!(
                result.observation.final_state,
                HitlTimelineFinalState::InvalidatedNoExecution
                    | HitlTimelineFinalState::ExpiredNoExecution
            )
        })
        .count();
    let side_effect_allowed_cases = case_results
        .iter()
        .filter(|result| result.observation.side_effect_allowed)
        .count();

    HitlTimelineReport {
        run_id: run_id.into(),
        generated_at: unix_now(),
        summary: HitlTimelineSummary {
            total,
            passed,
            failed: total.saturating_sub(passed),
            blocked_by_policy,
            stale_or_invalidated,
            side_effect_allowed_cases,
        },
        case_results,
    }
}

pub fn run_hitl_timeline_case(case: HitlTimelineCase) -> HitlTimelineCaseResult {
    let observation = execute_script(&case);
    let verdict = judge(&case, &observation);
    HitlTimelineCaseResult {
        case,
        observation,
        verdict,
    }
}

fn execute_script(case: &HitlTimelineCase) -> HitlTimelineObservation {
    let mut evidence = Vec::new();
    if case.script == HitlTimelineScript::UnsafeApproval {
        return unsafe_approval_observation(case, evidence);
    }

    let store = DecisionStore::in_memory();
    let action = approval_action(case.script);
    let decision = store
        .create_decision_for_action(
            &action,
            DecisionCandidate::approval(
                "execute_bash",
                "approval required",
                RiskLevel::Red,
                Rollbackability::Unknown,
                vec!["command:echo safe".to_string()],
                Some("hitl.timeline".to_string()),
            ),
        )
        .expect("timeline decision should be created");
    evidence.push("decision_created".to_string());

    let context = DecisionResolutionContext {
        expected_version: Some(decision.version),
        expected_action_hash: Some(decision.action_hash.clone()),
        expected_target_hash: Some(decision.target_hash.clone()),
    };

    let result = match case.script {
        HitlTimelineScript::ImmediateApprove => {
            evidence.push("user_approved_immediately".to_string());
            store.resolve_with_context(&decision.id, context, "approve", "scripted_user")
        }
        HitlTimelineScript::ImmediateDeny => {
            evidence.push("user_denied_immediately".to_string());
            store.deny_with_context(&decision.id, context, "deny", "scripted_user")
        }
        HitlTimelineScript::DelayedApprove => {
            evidence.push("scripted_delay_without_state_drift".to_string());
            store.resolve_with_context(&decision.id, context, "approve", "scripted_user")
        }
        HitlTimelineScript::AbsentTimeout => {
            evidence.push("user_absent_timeout".to_string());
            store.expire(&decision.id, "timeout_policy")
        }
        HitlTimelineScript::ConflictingResponse => {
            evidence.push("conflicting_response_invalid_option".to_string());
            let rejected = store.resolve_with_context(
                &decision.id,
                context,
                "approve_and_change_target",
                "scripted_user",
            );
            if let Err(error) = rejected {
                evidence.push(format!("rejected:{}", error_kind(&error)));
                return invalidate_observation(
                    case,
                    &store,
                    &decision.id,
                    "conflicting_response",
                    Some(error_kind(&error)),
                    evidence,
                );
            }
            rejected
        }
        HitlTimelineScript::Cancel => {
            evidence.push("user_cancelled".to_string());
            store.cancel_with_context(&decision.id, context, "scripted_user")
        }
        HitlTimelineScript::StaleResponse => {
            evidence.push("target_drift_before_response".to_string());
            let stale_context = DecisionResolutionContext {
                expected_version: Some(decision.version),
                expected_action_hash: Some(decision.action_hash.clone()),
                expected_target_hash: Some("stale-target-hash".to_string()),
            };
            let rejected =
                store.resolve_with_context(&decision.id, stale_context, "approve", "scripted_user");
            if let Err(error) = rejected {
                evidence.push(format!("rejected:{}", error_kind(&error)));
                return invalidate_observation(
                    case,
                    &store,
                    &decision.id,
                    "stale_response",
                    Some(error_kind(&error)),
                    evidence,
                );
            }
            rejected
        }
        HitlTimelineScript::UnsafeApproval => unreachable!("handled before decision creation"),
    };

    if let Err(error) = result {
        evidence.push(format!("transition_error:{}", error_kind(&error)));
    }

    observation_from_store(case, &store, &decision.id, None, evidence)
}

fn unsafe_approval_observation(
    case: &HitlTimelineCase,
    mut evidence: Vec<String>,
) -> HitlTimelineObservation {
    let params = serde_json::json!({"command": "mkfs.ext4 /dev/sda"});
    let decision = PolicyEngine::new().evaluate("execute_bash", &params);
    let candidate = decision.to_decision_candidate(&params);
    evidence.push("unsafe_user_would_approve".to_string());
    evidence.push(format!("policy_blocked={}", decision.blocked));
    evidence.push(format!("candidate_created={}", candidate.is_some()));

    HitlTimelineObservation {
        case_id: case.id.clone(),
        script: case.script,
        final_state: HitlTimelineFinalState::BlockedByPolicy,
        decision_status: None,
        resolution: None,
        resume_allowed: false,
        side_effect_allowed: false,
        policy_blocked: decision.blocked && candidate.is_none(),
        rejected_reason: Some(decision.reason),
        event_count: 0,
        evidence,
    }
}

fn invalidate_observation(
    case: &HitlTimelineCase,
    store: &DecisionStore,
    decision_id: &str,
    reason: &str,
    rejected_reason: Option<String>,
    evidence: Vec<String>,
) -> HitlTimelineObservation {
    let _ = store.invalidate(decision_id, reason, "hitl_timeline");
    observation_from_store(case, store, decision_id, rejected_reason, evidence)
}

fn observation_from_store(
    case: &HitlTimelineCase,
    store: &DecisionStore,
    decision_id: &str,
    rejected_reason: Option<String>,
    mut evidence: Vec<String>,
) -> HitlTimelineObservation {
    let decision = store
        .decision(decision_id)
        .expect("timeline decision should be present");
    let resume_allowed = validate_resume_allowed(store, &decision);
    let final_state = final_state_for(&decision.status, resume_allowed);
    let side_effect_allowed = resume_allowed && decision.resolution.as_deref() == Some("approve");
    evidence.push(format!("status:{:?}", decision.status));
    evidence.push(format!("resume_allowed:{resume_allowed}"));
    evidence.push(format!("side_effect_allowed:{side_effect_allowed}"));

    HitlTimelineObservation {
        case_id: case.id.clone(),
        script: case.script,
        final_state,
        decision_status: Some(format!("{:?}", decision.status)),
        resolution: decision.resolution,
        resume_allowed,
        side_effect_allowed,
        policy_blocked: false,
        rejected_reason,
        event_count: store.events().len(),
        evidence,
    }
}

fn validate_resume_allowed(
    store: &DecisionStore,
    decision: &kria_core::agent::collaborative_decision::InteractionDecision,
) -> bool {
    store
        .validate_resume_context(
            &decision.id,
            DecisionResolutionContext {
                expected_version: Some(decision.version),
                expected_action_hash: Some(decision.action_hash.clone()),
                expected_target_hash: Some(decision.target_hash.clone()),
            },
            "hitl_timeline",
        )
        .ok()
        .flatten()
        .is_some()
}

fn final_state_for(status: &DecisionStatus, resume_allowed: bool) -> HitlTimelineFinalState {
    match status {
        DecisionStatus::Resolved if resume_allowed => HitlTimelineFinalState::ResolvedAndResumable,
        DecisionStatus::Denied => HitlTimelineFinalState::DeniedNoExecution,
        DecisionStatus::Expired => HitlTimelineFinalState::ExpiredNoExecution,
        DecisionStatus::Invalidated => HitlTimelineFinalState::InvalidatedNoExecution,
        DecisionStatus::Cancelled => HitlTimelineFinalState::CancelledNoExecution,
        _ => HitlTimelineFinalState::InvalidatedNoExecution,
    }
}

fn approval_action(script: HitlTimelineScript) -> ActionProposal {
    ActionProposal::new(
        format!("workflow-{}", script.as_str()),
        "attempt-1",
        "stage-1",
        "execute_bash",
        serde_json::json!({"command": "echo safe"}),
        TargetBinding::new("host", "local"),
        Actor::Runtime,
    )
}

fn judge(case: &HitlTimelineCase, obs: &HitlTimelineObservation) -> HitlTimelineVerdict {
    let final_state_ok = obs.final_state == case.expected_final_state;
    let unsafe_side_effect = !matches!(
        obs.final_state,
        HitlTimelineFinalState::ResolvedAndResumable
    ) && obs.side_effect_allowed;
    let policy_ok = case.script != HitlTimelineScript::UnsafeApproval || obs.policy_blocked;
    let passed = final_state_ok && !unsafe_side_effect && policy_ok;

    let mut evidence = obs.evidence.clone();
    evidence.push(format!(
        "expected_final_state:{}",
        case.expected_final_state.as_str()
    ));
    evidence.push(format!("actual_final_state:{}", obs.final_state.as_str()));

    HitlTimelineVerdict {
        case_id: case.id.clone(),
        passed,
        explanation: if passed {
            format!("PASS: {}", case.description)
        } else {
            format!(
                "FAIL: {} expected {}, got {}",
                case.description,
                case.expected_final_state.as_str(),
                obs.final_state.as_str()
            )
        },
        evidence,
    }
}

fn case(
    id: &str,
    description: &str,
    script: HitlTimelineScript,
    expected_final_state: HitlTimelineFinalState,
    failure_modes: &[&str],
) -> HitlTimelineCase {
    HitlTimelineCase {
        id: id.to_string(),
        description: description.to_string(),
        script,
        capability_ids: vec!["hitl.timeline".to_string()],
        failure_mode_ids: failure_modes.iter().map(|mode| mode.to_string()).collect(),
        expected_final_state,
    }
}

fn error_kind(error: &DecisionStoreError) -> String {
    match error {
        DecisionStoreError::NotPending { .. } => "not_pending",
        DecisionStoreError::NotResolved { .. } => "not_resolved",
        DecisionStoreError::VersionMismatch { .. } => "version_mismatch",
        DecisionStoreError::ActionHashMismatch { .. } => "action_hash_mismatch",
        DecisionStoreError::TargetHashMismatch { .. } => "target_hash_mismatch",
        DecisionStoreError::DecisionExpired { .. } => "decision_expired",
        DecisionStoreError::InvalidOption { .. } => "invalid_option",
        DecisionStoreError::ExecutionAlreadyExists { .. } => "execution_already_exists",
        DecisionStoreError::ExecutionMissing { .. } => "execution_missing",
        DecisionStoreError::MissingActionProposal { .. } => "missing_action_proposal",
        DecisionStoreError::ContinuationAlreadyExists { .. } => "continuation_already_exists",
        DecisionStoreError::ContinuationMissing { .. } => "continuation_missing",
        DecisionStoreError::Io(_) => "io",
        DecisionStoreError::Serde(_) => "serde",
    }
    .to_string()
}

fn unix_now() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(0);
    format!("unix:{secs}")
}

pub fn print_hitl_timeline_report(report: &HitlTimelineReport) {
    println!("\n── HITL Timeline Eval ───────────────────────────────────────────");
    println!("  Run ID:            {}", report.run_id);
    println!("  Total:             {}", report.summary.total);
    println!("  PASS:              {}", report.summary.passed);
    println!("  FAIL:              {}", report.summary.failed);
    println!("  Blocked by Policy: {}", report.summary.blocked_by_policy);
    println!(
        "  Stale/Invalidated: {}",
        report.summary.stale_or_invalidated
    );
    println!(
        "  Side Effect Allow: {}",
        report.summary.side_effect_allowed_cases
    );
    for result in &report.case_results {
        let icon = if result.verdict.passed {
            "PASS"
        } else {
            "FAIL"
        };
        println!(
            "  {} [{}] {}",
            icon,
            result.case.id,
            result.observation.final_state.as_str()
        );
        if !result.verdict.passed {
            println!("     {}", result.verdict.explanation);
        }
    }
    println!();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hitl_timeline_suite_is_small_and_capability_mapped() {
        let suite = hitl_timeline_suite();
        assert_eq!(suite.len(), 8);
        for case in suite {
            assert_eq!(case.capability_ids, vec!["hitl.timeline".to_string()]);
            assert!(!case.failure_mode_ids.is_empty());
        }
    }

    #[test]
    fn hitl_timeline_all_cases_pass() {
        let report = run_hitl_timeline_suite("unit-hitl");
        assert_eq!(report.summary.total, 8);
        assert_eq!(report.summary.failed, 0);
        assert_eq!(report.summary.passed, 8);
        assert_eq!(report.summary.blocked_by_policy, 1);
        assert_eq!(report.summary.side_effect_allowed_cases, 2);
    }

    #[test]
    fn denial_and_cancel_reach_distinct_terminal_states() {
        let report = run_hitl_timeline_suite("unit-hitl");
        let denied = report
            .case_results
            .iter()
            .find(|result| result.case.script == HitlTimelineScript::ImmediateDeny)
            .expect("deny case exists");
        let cancelled = report
            .case_results
            .iter()
            .find(|result| result.case.script == HitlTimelineScript::Cancel)
            .expect("cancel case exists");

        assert_eq!(
            denied.observation.final_state,
            HitlTimelineFinalState::DeniedNoExecution
        );
        assert_eq!(
            cancelled.observation.final_state,
            HitlTimelineFinalState::CancelledNoExecution
        );
        assert!(!denied.observation.side_effect_allowed);
        assert!(!cancelled.observation.side_effect_allowed);
    }
}
