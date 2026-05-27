use crate::safety::RiskLevel;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::time::{SystemTime, UNIX_EPOCH};

pub const N8N_COMMAND_SCHEMA_VERSION: &str = "kria.n8n.command.v1";
pub const N8N_CALLBACK_SCHEMA_VERSION: &str = "kria.n8n.callback.v1";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum N8nWorkflowStatus {
    Draft,
    Test,
    Approved,
    Deprecated,
    Disabled,
}

impl Default for N8nWorkflowStatus {
    fn default() -> Self {
        Self::Draft
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum N8nIrreversibilityClass {
    ReadOnly,
    ReversibleLocal,
    ReversibleExternal,
    DestructiveRecoverable,
    DestructiveHardToRecover,
    IrreversibleHighImpact,
}

impl Default for N8nIrreversibilityClass {
    fn default() -> Self {
        Self::ReadOnly
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum N8nTimeoutClass {
    Interactive,
    Background,
    LongRunning,
}

impl Default for N8nTimeoutClass {
    fn default() -> Self {
        Self::Background
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum N8nWorkflowEnvironment {
    Dev,
    Staging,
    Production,
    DestructiveEval,
}

impl Default for N8nWorkflowEnvironment {
    fn default() -> Self {
        Self::Dev
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum N8nRunStatus {
    Accepted,
    Running,
    WaitingForApproval,
    Completed,
    Partial,
    Failed,
    Cancelled,
    TimedOut,
    Rejected,
}

impl N8nRunStatus {
    pub fn is_terminal(&self) -> bool {
        matches!(
            self,
            Self::Completed
                | Self::Partial
                | Self::Failed
                | Self::Cancelled
                | Self::TimedOut
                | Self::Rejected
        )
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum N8nCallbackErrorClass {
    ExternalAuthRequired,
    ExternalRateLimited,
    ContractViolation,
    PartialExternalMutation,
    ExternalTimeout,
    CapabilityUnavailable,
    ExternalUnknown,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct N8nWorkflowConfig {
    pub workflow_id: String,
    pub workflow_version: String,
    pub display_name: String,
    pub endpoint_path: String,
    pub status: N8nWorkflowStatus,
    pub environment: N8nWorkflowEnvironment,
    pub risk_tier: RiskLevel,
    pub irreversibility_class: N8nIrreversibilityClass,
    pub timeout_class: N8nTimeoutClass,
    pub allowed_actions: Vec<String>,
    pub data_scope: Vec<String>,
    pub expected_evidence: Vec<String>,
}

impl Default for N8nWorkflowConfig {
    fn default() -> Self {
        Self {
            workflow_id: String::new(),
            workflow_version: "v1".into(),
            display_name: String::new(),
            endpoint_path: String::new(),
            status: N8nWorkflowStatus::Draft,
            environment: N8nWorkflowEnvironment::Dev,
            risk_tier: RiskLevel::Yellow,
            irreversibility_class: N8nIrreversibilityClass::ReadOnly,
            timeout_class: N8nTimeoutClass::Background,
            allowed_actions: Vec::new(),
            data_scope: Vec::new(),
            expected_evidence: Vec::new(),
        }
    }
}

impl N8nWorkflowConfig {
    pub fn is_approved_for_execution(&self) -> bool {
        matches!(self.status, N8nWorkflowStatus::Approved)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct N8nCommandEnvelope {
    pub schema_version: String,
    pub correlation_id: String,
    pub causation_id: String,
    pub idempotency_key: String,
    pub workflow_id: String,
    pub workflow_version: String,
    pub risk_tier: RiskLevel,
    pub irreversibility_class: N8nIrreversibilityClass,
    pub requested_by: String,
    pub deadline_ms: u64,
    pub allowed_actions: Vec<String>,
    pub input_payload: serde_json::Value,
    pub expected_evidence: Vec<String>,
    pub data_scope: Vec<String>,
    pub issued_at_ms: u128,
}

impl N8nCommandEnvelope {
    pub fn new(
        workflow: &N8nWorkflowConfig,
        input_payload: serde_json::Value,
        correlation_id: String,
        causation_id: String,
        idempotency_key: String,
        requested_by: String,
        deadline_ms: u64,
    ) -> Self {
        Self {
            schema_version: N8N_COMMAND_SCHEMA_VERSION.into(),
            correlation_id,
            causation_id,
            idempotency_key,
            workflow_id: workflow.workflow_id.clone(),
            workflow_version: workflow.workflow_version.clone(),
            risk_tier: workflow.risk_tier,
            irreversibility_class: workflow.irreversibility_class.clone(),
            requested_by,
            deadline_ms,
            allowed_actions: workflow.allowed_actions.clone(),
            input_payload,
            expected_evidence: workflow.expected_evidence.clone(),
            data_scope: workflow.data_scope.clone(),
            issued_at_ms: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|d| d.as_millis())
                .unwrap_or_default(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct N8nInvocationResult {
    pub workflow_id: String,
    pub workflow_version: String,
    pub correlation_id: String,
    pub idempotency_key: String,
    pub status_code: u16,
    pub accepted: bool,
    pub response: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct N8nCallbackEnvelope {
    pub schema_version: String,
    pub correlation_id: String,
    pub causation_id: String,
    pub event_id: String,
    pub sequence_number: u64,
    pub workflow_id: String,
    pub workflow_version: String,
    pub n8n_run_id: String,
    pub status: N8nRunStatus,
    #[serde(default)]
    pub evidence: serde_json::Value,
    #[serde(default)]
    pub side_effects: Vec<String>,
    #[serde(default)]
    pub error_class: Option<N8nCallbackErrorClass>,
    pub occurred_at_ms: u128,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct N8nToolRequest {
    pub workflow_id: String,
    #[serde(default)]
    pub workflow_version: Option<String>,
    #[serde(default)]
    pub input_payload: serde_json::Value,
    #[serde(default)]
    pub correlation_id: Option<String>,
    #[serde(default)]
    pub causation_id: Option<String>,
    #[serde(default)]
    pub idempotency_key: Option<String>,
    #[serde(default)]
    pub requested_by: Option<String>,
    #[serde(default)]
    pub metadata: HashMap<String, serde_json::Value>,
}

impl Default for N8nToolRequest {
    fn default() -> Self {
        Self {
            workflow_id: String::new(),
            workflow_version: None,
            input_payload: serde_json::json!({}),
            correlation_id: None,
            causation_id: None,
            idempotency_key: None,
            requested_by: None,
            metadata: HashMap::new(),
        }
    }
}
