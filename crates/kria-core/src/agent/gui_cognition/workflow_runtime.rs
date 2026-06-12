//! Step 10: Multi-Step Workflow Runtime.
//!
//! The workflow runtime executes a typed plan one bound proposal at a time. It
//! does not bypass any Step 1-9 contract: every executable step still goes
//! through target resolution, the safety gate, optional HITL, the deterministic
//! executor, post-action verification, and (when needed) the recovery loop.
//!
//! State is in-memory only. Durable checkpoint/resume is Step 11.

use super::llm_planner::GuiTypedPlanStep;
use super::perception::{sanitize_gui_text, stable_hash};

/// How a typed plan step participates in the workflow.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GuiWorkflowStepKind {
    /// Goes through Step 5-9 (resolve -> safety -> execute -> verify -> recover).
    Executable,
    /// Re-observe only; never calls the executor.
    Observe,
    /// Verified through the verifier/state observation, not fake-completed.
    WaitOrVerify,
    /// Visible-content summary; observation only.
    Summarize,
    /// Pauses the workflow for the user.
    AskClarification,
    /// Pauses the workflow for explicit approval.
    RequireApproval,
}

pub fn workflow_step_kind(step_type: &str) -> GuiWorkflowStepKind {
    match step_type {
        "OpenApp" | "SwitchWindow" | "FocusField" | "TypeText" | "ClickControl" | "PressKey"
        | "BrowserNavigate" | "Scroll" | "Copy" | "Paste" | "Save" | "Download" => {
            GuiWorkflowStepKind::Executable
        }
        "WaitForState" | "VerifyState" => GuiWorkflowStepKind::WaitOrVerify,
        "SummarizeVisibleContent" => GuiWorkflowStepKind::Summarize,
        "AskClarification" => GuiWorkflowStepKind::AskClarification,
        "RequireApproval" => GuiWorkflowStepKind::RequireApproval,
        _ => GuiWorkflowStepKind::Observe,
    }
}

/// Steps whose execution requires resolving a concrete control target.
pub fn workflow_step_requires_target(step_type: &str) -> bool {
    matches!(step_type, "FocusField" | "TypeText" | "ClickControl")
}

/// Executable steps that change GUI state and therefore require a re-observation
/// before the next step resolves its target.
pub fn workflow_step_is_state_changing(step_type: &str) -> bool {
    matches!(
        step_type,
        "OpenApp"
            | "SwitchWindow"
            | "TypeText"
            | "ClickControl"
            | "PressKey"
            | "BrowserNavigate"
            | "Scroll"
            | "Paste"
            | "Save"
            | "Download"
    )
}

fn safe(value: &str) -> String {
    sanitize_gui_text(value, 180).text
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct GuiWorkflowStepState {
    pub step_id: String,
    pub step_index: usize,
    pub step_type: String,
    pub summary: String,
    pub status: String,
    pub target_resolution_id: Option<String>,
    pub proposal_id: Option<String>,
    pub proposal_hash: Option<String>,
    pub hitl_decision_id: Option<String>,
    pub execution_id: Option<String>,
    pub verification_id: Option<String>,
    pub recovery_id: Option<String>,
    pub started_at_ms: i64,
    pub completed_at_ms: i64,
    pub blockers: Vec<String>,
    pub warnings: Vec<String>,
    pub can_continue: bool,
    pub prompt_hash: String,
}

impl GuiWorkflowStepState {
    pub fn pending(step: &GuiTypedPlanStep, index: usize, prompt_hash: &str) -> Self {
        Self {
            step_id: safe(&step.step_id),
            step_index: index,
            step_type: safe(&step.step_type),
            summary: safe(&step.summary),
            status: "pending".into(),
            target_resolution_id: None,
            proposal_id: None,
            proposal_hash: None,
            hitl_decision_id: None,
            execution_id: None,
            verification_id: None,
            recovery_id: None,
            started_at_ms: 0,
            completed_at_ms: 0,
            blockers: Vec::new(),
            warnings: Vec::new(),
            can_continue: false,
            prompt_hash: safe(prompt_hash),
        }
    }

    pub fn summary_json(&self) -> serde_json::Value {
        serde_json::json!({
            "step_id": self.step_id,
            "step_index": self.step_index,
            "step_type": self.step_type,
            "summary": self.summary,
            "status": self.status,
            "target_resolution_id": self.target_resolution_id,
            "proposal_id": self.proposal_id,
            "proposal_hash": self.proposal_hash,
            "hitl_decision_id": self.hitl_decision_id,
            "execution_id": self.execution_id,
            "verification_id": self.verification_id,
            "recovery_id": self.recovery_id,
            "started_at_ms": self.started_at_ms,
            "completed_at_ms": self.completed_at_ms,
            "blockers": self.blockers,
            "warnings": self.warnings,
            "can_continue": self.can_continue,
            "prompt_hash": self.prompt_hash,
        })
    }
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct GuiWorkflowStepReceipt {
    pub receipt_id: String,
    pub workflow_run_id: String,
    pub step_id: String,
    pub step_index: usize,
    pub step_type: String,
    pub status: String,
    pub proposal_id: Option<String>,
    pub action_type: Option<String>,
    pub risk_level: Option<String>,
    pub side_effect_kind: String,
    pub target_hash: Option<String>,
    pub proposal_hash: Option<String>,
    pub execution_id: Option<String>,
    pub verification_id: Option<String>,
    pub verification_status: Option<String>,
    pub recovery_id: Option<String>,
    pub recovery_status: Option<String>,
    pub started_at_ms: i64,
    pub completed_at_ms: i64,
    pub safe_summary: String,
    pub receipt_hash: String,
    pub prompt_hash: String,
}

/// Side-effect classification for the duplicate-risky-action guard (Step 11).
pub fn side_effect_kind_for(action_type: &str, risk_level: &str) -> &'static str {
    let action = action_type.trim().to_ascii_lowercase();
    if action.contains("pay") || action.contains("purchase") {
        return "payment";
    }
    if action.contains("install") || action.contains("system") {
        return "install_system";
    }
    if action.contains("delete") || action.contains("remove") {
        return "destructive";
    }
    if action.contains("send")
        || action.contains("submit")
        || action.contains("git")
        || matches!(risk_level.trim().to_ascii_lowercase().as_str(), "high" | "critical")
    {
        return "external_submit";
    }
    match action.as_str() {
        "openapp" | "switchwindow" | "focusfield" | "typetext" | "clickcontrol" | "presskey"
        | "scroll" | "copy" | "paste" => "local_ui",
        _ => "none",
    }
}

/// Risky side effects that must never be auto-replayed on resume.
pub fn side_effect_is_risky(side_effect_kind: &str) -> bool {
    matches!(
        side_effect_kind,
        "external_submit" | "destructive" | "payment" | "install_system"
    )
}

pub fn compute_receipt_hash(
    workflow_run_id: &str,
    step_id: &str,
    step_index: usize,
    proposal_hash: Option<&str>,
    execution_id: Option<&str>,
    verification_status: Option<&str>,
) -> String {
    stable_hash(&format!(
        "{workflow_run_id}|{step_id}|{step_index}|{}|{}|{}",
        proposal_hash.unwrap_or(""),
        execution_id.unwrap_or(""),
        verification_status.unwrap_or(""),
    ))
}

impl GuiWorkflowStepReceipt {
    pub fn summary_json(&self) -> serde_json::Value {
        serde_json::json!({
            "receipt_id": self.receipt_id,
            "workflow_run_id": self.workflow_run_id,
            "step_id": self.step_id,
            "step_index": self.step_index,
            "step_type": self.step_type,
            "status": self.status,
            "proposal_id": self.proposal_id,
            "action_type": self.action_type,
            "risk_level": self.risk_level,
            "side_effect_kind": self.side_effect_kind,
            "target_hash": self.target_hash,
            "proposal_hash": self.proposal_hash,
            "execution_id": self.execution_id,
            "verification_id": self.verification_id,
            "verification_status": self.verification_status,
            "recovery_id": self.recovery_id,
            "recovery_status": self.recovery_status,
            "started_at_ms": self.started_at_ms,
            "completed_at_ms": self.completed_at_ms,
            "safe_summary": self.safe_summary,
            "receipt_hash": self.receipt_hash,
            "prompt_hash": self.prompt_hash,
        })
    }
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct GuiWorkflowRun {
    pub workflow_run_id: String,
    pub session_id: String,
    pub workflow_id: String,
    pub turn_id: String,
    pub goal_contract_id: String,
    pub plan_id: String,
    pub initial_context_id: String,
    pub current_context_id: String,
    pub status: String,
    pub current_step_index: usize,
    pub step_count: usize,
    pub step_states: Vec<GuiWorkflowStepState>,
    pub completed_step_receipts: Vec<GuiWorkflowStepReceipt>,
    pub blocked_reason: Option<String>,
    pub recovery_summary: Option<String>,
    pub risk_level: String,
    pub requires_user_approval: bool,
    pub execution_mode: String,
    pub prompt_hash: String,
}

impl GuiWorkflowRun {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        session_id: &str,
        workflow_id: &str,
        turn_id: &str,
        goal_contract_id: &str,
        plan_id: &str,
        context_id: &str,
        steps: &[GuiTypedPlanStep],
        risk_level: &str,
        requires_user_approval: bool,
        execution_mode: &str,
        prompt_hash: &str,
    ) -> Self {
        let workflow_run_id = format!(
            "workflow-run-{}",
            stable_hash(&format!("{session_id}|{workflow_id}|{turn_id}|{plan_id}"))
        );
        let step_states = steps
            .iter()
            .enumerate()
            .map(|(index, step)| GuiWorkflowStepState::pending(step, index, prompt_hash))
            .collect::<Vec<_>>();
        Self {
            workflow_run_id,
            session_id: safe(session_id),
            workflow_id: safe(workflow_id),
            turn_id: safe(turn_id),
            goal_contract_id: safe(goal_contract_id),
            plan_id: safe(plan_id),
            initial_context_id: safe(context_id),
            current_context_id: safe(context_id),
            status: "running".into(),
            current_step_index: 0,
            step_count: steps.len(),
            step_states,
            completed_step_receipts: Vec::new(),
            blocked_reason: None,
            recovery_summary: None,
            risk_level: safe(risk_level),
            requires_user_approval,
            execution_mode: safe(execution_mode),
            prompt_hash: safe(prompt_hash),
        }
    }

    pub fn receipt_id(&self, step_index: usize) -> String {
        format!(
            "receipt-{}",
            stable_hash(&format!("{}|{step_index}", self.workflow_run_id))
        )
    }

    pub fn summary_json(&self) -> serde_json::Value {
        serde_json::json!({
            "workflow_run_id": self.workflow_run_id,
            "session_id": self.session_id,
            "workflow_id": self.workflow_id,
            "turn_id": self.turn_id,
            "goal_contract_id": self.goal_contract_id,
            "plan_id": self.plan_id,
            "initial_context_id": self.initial_context_id,
            "current_context_id": self.current_context_id,
            "status": self.status,
            "current_step_index": self.current_step_index,
            "step_count": self.step_count,
            "step_states": self.step_states.iter().map(GuiWorkflowStepState::summary_json).collect::<Vec<_>>(),
            "completed_step_receipts": self.completed_step_receipts.iter().map(GuiWorkflowStepReceipt::summary_json).collect::<Vec<_>>(),
            "blocked_reason": self.blocked_reason,
            "recovery_summary": self.recovery_summary,
            "risk_level": self.risk_level,
            "requires_user_approval": self.requires_user_approval,
            "execution_mode": self.execution_mode,
            "prompt_hash": self.prompt_hash,
        })
    }

    pub fn run_started_event(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "WorkflowRunStarted",
            "workflow_run_id": self.workflow_run_id,
            "plan_id": self.plan_id,
            "goal_contract_id": self.goal_contract_id,
            "step_count": self.step_count,
            "current_step_index": self.current_step_index,
            "risk_level": self.risk_level,
            "requires_user_approval": self.requires_user_approval,
            "execution_mode": self.execution_mode,
            "prompt_hash": self.prompt_hash,
        })
    }

    pub fn run_terminal_event(&self) -> serde_json::Value {
        let event_type = match self.status.as_str() {
            "completed" => "WorkflowRunCompleted",
            "paused" => "WorkflowRunPaused",
            _ => "WorkflowRunBlocked",
        };
        serde_json::json!({
            "type": event_type,
            "workflow_run_id": self.workflow_run_id,
            "plan_id": self.plan_id,
            "goal_contract_id": self.goal_contract_id,
            "status": self.status,
            "current_step_index": self.current_step_index,
            "step_count": self.step_count,
            "completed_step_count": self.completed_step_receipts.len(),
            "blocked_reason": self.blocked_reason,
            "prompt_hash": self.prompt_hash,
        })
    }
}

pub fn step_started_event(run: &GuiWorkflowRun, step: &GuiWorkflowStepState) -> serde_json::Value {
    serde_json::json!({
        "type": "WorkflowStepStarted",
        "workflow_run_id": run.workflow_run_id,
        "plan_id": run.plan_id,
        "goal_contract_id": run.goal_contract_id,
        "step_id": step.step_id,
        "step_index": step.step_index,
        "step_type": step.step_type,
        "status": step.status,
        "current_step_index": run.current_step_index,
        "step_count": run.step_count,
        "prompt_hash": run.prompt_hash,
    })
}

pub fn step_completed_event(
    run: &GuiWorkflowRun,
    step: &GuiWorkflowStepState,
    receipt: &GuiWorkflowStepReceipt,
) -> serde_json::Value {
    serde_json::json!({
        "type": "WorkflowStepCompleted",
        "workflow_run_id": run.workflow_run_id,
        "plan_id": run.plan_id,
        "goal_contract_id": run.goal_contract_id,
        "step_id": step.step_id,
        "step_index": step.step_index,
        "step_type": step.step_type,
        "status": step.status,
        "receipt_id": receipt.receipt_id,
        "current_step_index": run.current_step_index,
        "step_count": run.step_count,
        "warnings": step.warnings,
        "prompt_hash": run.prompt_hash,
    })
}

pub fn step_blocked_event(
    run: &GuiWorkflowRun,
    step: &GuiWorkflowStepState,
) -> serde_json::Value {
    serde_json::json!({
        "type": "WorkflowStepBlocked",
        "workflow_run_id": run.workflow_run_id,
        "plan_id": run.plan_id,
        "goal_contract_id": run.goal_contract_id,
        "step_id": step.step_id,
        "step_index": step.step_index,
        "step_type": step.step_type,
        "status": step.status,
        "current_step_index": run.current_step_index,
        "step_count": run.step_count,
        "blockers": step.blockers,
        "prompt_hash": run.prompt_hash,
    })
}
