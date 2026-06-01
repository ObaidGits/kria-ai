use super::matching::WorkflowConfirmationFlow;
use super::schema::{schema_error_issues, validate_n8n_output_evidence};
use super::types::{N8nRunStatus, N8nWorkflowConfig};
use super::N8nWorkflowRunState;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum N8nVerificationStatus {
    Verified,
    NeedsMoreEvidence,
    Failed,
    HumanReviewRequired,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum N8nContinuationAction {
    AwaitMoreEvents,
    ContinueWorkflow,
    PauseForHitl,
    RecoverWorkflow,
    MarkFailed,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct N8nGovernanceDecision {
    pub correlation_id: String,
    pub workflow_id: String,
    pub workflow_version: String,
    pub run_status: N8nRunStatus,
    pub verification_status: N8nVerificationStatus,
    pub continuation_action: N8nContinuationAction,
    pub missing_evidence: Vec<String>,
    pub explanation: String,
}

pub fn evaluate_run(
    workflow: Option<&N8nWorkflowConfig>,
    run: &N8nWorkflowRunState,
) -> N8nGovernanceDecision {
    if matches!(run.status, N8nRunStatus::WaitingForApproval) {
        return decision(
            run,
            N8nVerificationStatus::HumanReviewRequired,
            N8nContinuationAction::PauseForHitl,
            Vec::new(),
            "n8n workflow requested human approval",
        );
    }

    if !run.terminal {
        return decision(
            run,
            N8nVerificationStatus::NeedsMoreEvidence,
            N8nContinuationAction::AwaitMoreEvents,
            Vec::new(),
            "n8n workflow is still running; KRIA waits for more evidence",
        );
    }

    if matches!(
        run.status,
        N8nRunStatus::Failed
            | N8nRunStatus::Cancelled
            | N8nRunStatus::TimedOut
            | N8nRunStatus::Rejected
    ) {
        return decision(
            run,
            N8nVerificationStatus::Failed,
            N8nContinuationAction::RecoverWorkflow,
            Vec::new(),
            "n8n workflow ended unsuccessfully; KRIA should recover or escalate",
        );
    }

    let expected = workflow
        .map(|workflow| workflow.expected_evidence.as_slice())
        .unwrap_or(&[]);

    if let Some(workflow) = workflow {
        if WorkflowConfirmationFlow::workflow_requires_confirmation(workflow)
            && !run
                .evidence_log
                .iter()
                .any(|evidence| evidence_bool_true(evidence, "confirmed_by_user"))
        {
            return decision(
                run,
                N8nVerificationStatus::HumanReviewRequired,
                N8nContinuationAction::PauseForHitl,
                vec!["confirmed_by_user".into()],
                "n8n workflow requires explicit user review before KRIA can verify completion",
            );
        }

        if !workflow.output_schema_ref.trim().is_empty() {
            if let Err(error) = validate_n8n_output_evidence(workflow, &run.evidence_log) {
                return decision(
                    run,
                    N8nVerificationStatus::NeedsMoreEvidence,
                    N8nContinuationAction::PauseForHitl,
                    schema_error_issues(error),
                    "n8n reported completion but callback evidence does not satisfy the output schema",
                );
            }
        }
    }

    let missing = expected
        .iter()
        .filter(|key| {
            !run.evidence_log
                .iter()
                .any(|evidence| evidence_contains(evidence, key))
        })
        .cloned()
        .collect::<Vec<_>>();

    if !missing.is_empty() {
        return decision(
            run,
            N8nVerificationStatus::NeedsMoreEvidence,
            N8nContinuationAction::PauseForHitl,
            missing,
            "n8n reported completion but expected semantic evidence is missing",
        );
    }

    decision(
        run,
        N8nVerificationStatus::Verified,
        N8nContinuationAction::ContinueWorkflow,
        Vec::new(),
        "n8n evidence satisfies the configured KRIA contract",
    )
}

fn decision(
    run: &N8nWorkflowRunState,
    verification_status: N8nVerificationStatus,
    continuation_action: N8nContinuationAction,
    missing_evidence: Vec<String>,
    explanation: &str,
) -> N8nGovernanceDecision {
    N8nGovernanceDecision {
        correlation_id: run.correlation_id.clone(),
        workflow_id: run.workflow_id.clone(),
        workflow_version: run.workflow_version.clone(),
        run_status: run.status.clone(),
        verification_status,
        continuation_action,
        missing_evidence,
        explanation: explanation.into(),
    }
}

fn evidence_contains(value: &serde_json::Value, expected: &str) -> bool {
    if expected.trim().is_empty() {
        return true;
    }

    if value.pointer(expected).is_some() {
        return true;
    }

    match value {
        serde_json::Value::Object(map) => map
            .iter()
            .any(|(key, value)| key == expected || evidence_contains(value, expected)),
        serde_json::Value::Array(items) => {
            items.iter().any(|item| evidence_contains(item, expected))
        }
        serde_json::Value::String(text) => text.contains(expected),
        _ => false,
    }
}

fn evidence_bool_true(value: &serde_json::Value, expected: &str) -> bool {
    match value {
        serde_json::Value::Object(map) => map.iter().any(|(key, value)| {
            (key == expected && value.as_bool().unwrap_or(false))
                || evidence_bool_true(value, expected)
        }),
        serde_json::Value::Array(items) => {
            items.iter().any(|item| evidence_bool_true(item, expected))
        }
        _ => false,
    }
}
