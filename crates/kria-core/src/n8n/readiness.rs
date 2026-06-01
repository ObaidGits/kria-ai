use super::config::N8nConfig;
use super::types::{N8nWorkflowConfig, N8nWorkflowStatus};
use serde::{Deserialize, Serialize};

pub const N8N_STAGE3_REQUIRED_WORKFLOW_COUNT: usize = 3;

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct N8nReadinessGateEvidence {
    pub phase0_complete: bool,
    pub phase1_complete: bool,
    pub phase1_5_complete: bool,
    pub phase2_complete: bool,
    pub phase3_complete: bool,
    pub phase4_complete: bool,
    pub phase4_5_complete: bool,
    pub phase5_complete: bool,
    pub reliability_17_of_17_passed: bool,
    pub workflow_cards_history_stable: bool,
    pub terminal_callback_verified_with_real_n8n: bool,
    pub unknown_workflow_user_visible_tested: bool,
    pub disabled_workflow_user_visible_tested: bool,
    pub bad_signature_user_visible_tested: bool,
    pub timeout_user_visible_tested: bool,
    pub workflow_selection_eval_set_exists: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct N8nReadinessGateCheck {
    pub key: String,
    pub label: String,
    pub passed: bool,
    pub detail: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct N8nStage3ReadinessReport {
    pub status: String,
    pub ready: bool,
    pub required_workflow_count: usize,
    pub workflow_metadata_count: usize,
    pub checked_at_ms: u64,
    pub checks: Vec<N8nReadinessGateCheck>,
    pub missing_gates: Vec<String>,
    pub first_slice: Vec<String>,
}

fn has_meaningful_list(values: &[String]) -> bool {
    values.iter().any(|value| !value.trim().is_empty())
}

pub fn workflow_has_stage3_ready_metadata(workflow: &N8nWorkflowConfig) -> bool {
    matches!(&workflow.status, N8nWorkflowStatus::Approved)
        && workflow.is_ready_for_approval()
        && !workflow.display_name.trim().is_empty()
        && !workflow.category.trim().is_empty()
        && !workflow.description.trim().is_empty()
        && has_meaningful_list(&workflow.example_prompts)
        && (has_meaningful_list(&workflow.tags) || has_meaningful_list(&workflow.aliases))
}

fn readiness_check(
    key: &str,
    label: &str,
    passed: bool,
    detail: impl Into<String>,
) -> N8nReadinessGateCheck {
    N8nReadinessGateCheck {
        key: key.into(),
        label: label.into(),
        passed,
        detail: detail.into(),
    }
}

pub fn evaluate_stage3_readiness(
    config: &N8nConfig,
    evidence: N8nReadinessGateEvidence,
    checked_at_ms: u64,
) -> N8nStage3ReadinessReport {
    let workflow_metadata_count = config
        .workflows
        .iter()
        .filter(|workflow| workflow_has_stage3_ready_metadata(workflow))
        .count();

    let previous_phases_complete = evidence.phase0_complete
        && evidence.phase1_complete
        && evidence.phase1_5_complete
        && evidence.phase2_complete
        && evidence.phase3_complete
        && evidence.phase4_complete
        && evidence.phase4_5_complete
        && evidence.phase5_complete;

    let workflow_count_ready = workflow_metadata_count >= N8N_STAGE3_REQUIRED_WORKFLOW_COUNT;

    let checks = vec![
        readiness_check(
            "phase_0_to_5_complete",
            "Phase 0 through Phase 5 evidence is complete, including Phase 4.5 authoring safety",
            previous_phases_complete,
            format!(
                "phase0={}, phase1={}, phase1_5={}, phase2={}, phase3={}, phase4={}, phase4_5={}, phase5={}",
                evidence.phase0_complete,
                evidence.phase1_complete,
                evidence.phase1_5_complete,
                evidence.phase2_complete,
                evidence.phase3_complete,
                evidence.phase4_complete,
                evidence.phase4_5_complete,
                evidence.phase5_complete
            ),
        ),
        readiness_check(
            "reliability_17_of_17",
            "Reliability suite passed 17/17 on a running app",
            evidence.reliability_17_of_17_passed,
            "latest reliability report must show 17 passed / 0 failed / 17 total",
        ),
        readiness_check(
            "three_real_workflows_with_metadata",
            "At least three approved workflows have routing-quality metadata",
            workflow_count_ready,
            format!(
                "{workflow_metadata_count}/{N8N_STAGE3_REQUIRED_WORKFLOW_COUNT} workflows are approved and have display name, approval metadata, category, description, example prompts, and tags or aliases"
            ),
        ),
        readiness_check(
            "workflow_cards_history_stable",
            "Workflow cards and history are stable",
            evidence.workflow_cards_history_stable,
            "Phase 2/3/4 UI reports must pass",
        ),
        readiness_check(
            "terminal_callback_real_n8n",
            "Terminal callback path verified with real n8n",
            evidence.terminal_callback_verified_with_real_n8n,
            "live E2E report must include signed terminal callback acceptance",
        ),
        readiness_check(
            "unknown_workflow_user_visible",
            "Unknown workflow is user-visible and tested",
            evidence.unknown_workflow_user_visible_tested,
            "eval or reliability report must cover unknown workflow behavior",
        ),
        readiness_check(
            "disabled_workflow_user_visible",
            "Disabled workflow is user-visible and tested",
            evidence.disabled_workflow_user_visible_tested,
            "Phase 4 test coverage must prove disabled workflows cannot execute",
        ),
        readiness_check(
            "bad_signature_user_visible",
            "Bad callback signature is user-visible and tested",
            evidence.bad_signature_user_visible_tested,
            "reliability report must cover invalid HMAC signature behavior",
        ),
        readiness_check(
            "timeout_user_visible",
            "Timeout behavior is user-visible and tested",
            evidence.timeout_user_visible_tested,
            "Phase 3 progress and reliability evidence must cover timeout/recovery behavior",
        ),
        readiness_check(
            "workflow_selection_eval_set",
            "Workflow selection prompt eval set exists",
            evidence.workflow_selection_eval_set_exists,
            "deterministic invocation evals must cover exact id, display name, alias, ambiguity, and no-match prompts",
        ),
    ];

    let missing_gates = checks
        .iter()
        .filter(|check| !check.passed)
        .map(|check| check.label.clone())
        .collect::<Vec<_>>();
    let ready = missing_gates.is_empty();

    N8nStage3ReadinessReport {
        status: if ready { "ready" } else { "blocked" }.into(),
        ready,
        required_workflow_count: N8N_STAGE3_REQUIRED_WORKFLOW_COUNT,
        workflow_metadata_count,
        checked_at_ms,
        checks,
        missing_gates,
        first_slice: vec![
            "Rank workflows using existing metadata only".into(),
            "Return top 3 suggestions".into(),
            "Ask user to confirm".into(),
            "Do not auto-run".into(),
        ],
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::safety::RiskLevel;

    fn workflow(id: &str, status: N8nWorkflowStatus) -> N8nWorkflowConfig {
        N8nWorkflowConfig {
            workflow_id: id.into(),
            workflow_version: "v1".into(),
            display_name: format!("Workflow {id}"),
            endpoint_path: format!("/webhook/{id}"),
            status,
            risk_tier: RiskLevel::Green,
            owner: "kria-test".into(),
            requires_callback: Some(true),
            input_schema_ref: format!("schemas/n8n/{id}.input.json"),
            output_schema_ref: format!("schemas/n8n/{id}.output.json"),
            credential_requirements: vec!["none".into()],
            hitl_policy: "none".into(),
            category: "diagnostic".into(),
            description: format!("Safe test workflow {id}"),
            example_prompts: vec![format!("Run {id}")],
            tags: vec!["diagnostic".into()],
            aliases: vec![format!("{id} alias")],
            data_scope: vec!["diagnostic".into()],
            expected_evidence: vec!["result".into()],
            allowed_actions: vec!["diagnostic.echo".into()],
            ..Default::default()
        }
    }

    fn complete_evidence() -> N8nReadinessGateEvidence {
        N8nReadinessGateEvidence {
            phase0_complete: true,
            phase1_complete: true,
            phase1_5_complete: true,
            phase2_complete: true,
            phase3_complete: true,
            phase4_complete: true,
            phase4_5_complete: true,
            phase5_complete: true,
            reliability_17_of_17_passed: true,
            workflow_cards_history_stable: true,
            terminal_callback_verified_with_real_n8n: true,
            unknown_workflow_user_visible_tested: true,
            disabled_workflow_user_visible_tested: true,
            bad_signature_user_visible_tested: true,
            timeout_user_visible_tested: true,
            workflow_selection_eval_set_exists: true,
        }
    }

    #[test]
    fn stage3_readiness_blocks_when_less_than_three_workflows_are_registered() {
        let config = N8nConfig {
            workflows: vec![workflow("one", N8nWorkflowStatus::Approved)],
            ..Default::default()
        };

        let report = evaluate_stage3_readiness(&config, complete_evidence(), 10);

        assert!(!report.ready);
        assert_eq!(report.status, "blocked");
        assert_eq!(report.workflow_metadata_count, 1);
        assert!(report
            .missing_gates
            .iter()
            .any(|gate| gate.contains("At least three approved workflows")));
    }

    #[test]
    fn stage3_readiness_blocks_incomplete_phase_evidence() {
        let config = N8nConfig {
            workflows: vec![
                workflow("one", N8nWorkflowStatus::Approved),
                workflow("two", N8nWorkflowStatus::Approved),
                workflow("three", N8nWorkflowStatus::Approved),
            ],
            ..Default::default()
        };
        let evidence = N8nReadinessGateEvidence {
            phase5_complete: false,
            ..complete_evidence()
        };

        let report = evaluate_stage3_readiness(&config, evidence, 10);

        assert!(!report.ready);
        assert!(report
            .checks
            .iter()
            .any(|check| check.key == "phase_0_to_5_complete" && !check.passed));
    }

    #[test]
    fn stage3_readiness_passes_only_with_all_gates() {
        let config = N8nConfig {
            workflows: vec![
                workflow("one", N8nWorkflowStatus::Approved),
                workflow("two", N8nWorkflowStatus::Approved),
                workflow("three", N8nWorkflowStatus::Approved),
            ],
            ..Default::default()
        };

        let report = evaluate_stage3_readiness(&config, complete_evidence(), 10);

        assert!(report.ready);
        assert_eq!(report.status, "ready");
        assert!(report
            .first_slice
            .iter()
            .any(|step| step == "Do not auto-run"));
    }
}
