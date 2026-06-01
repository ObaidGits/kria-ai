use crate::safety::RiskLevel;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::time::{SystemTime, UNIX_EPOCH};

pub const N8N_COMMAND_SCHEMA_VERSION: &str = "kria.n8n.command.v1";
pub const N8N_CALLBACK_SCHEMA_VERSION: &str = "kria.n8n.callback.v1";

fn is_zero_u64(value: &u64) -> bool {
    *value == 0
}

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

impl N8nTimeoutClass {
    /// Get the KRIA-side deadline in milliseconds for this timeout class.
    /// If no callback arrives within this window, the run is marked TimedOut.
    pub fn deadline_ms(&self) -> u64 {
        match self {
            Self::Interactive => 60_000,    // 1 minute
            Self::Background => 300_000,    // 5 minutes
            Self::LongRunning => 3_600_000, // 1 hour
        }
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

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct N8nWorkflowConfig {
    pub workflow_id: String,
    pub workflow_version: String,
    pub display_name: String,
    pub endpoint_path: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub n8n_workflow_id: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub trigger_strategy: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub result_mode: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub webhook_method: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub webhook_path: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub preferred_output_node: Option<String>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub output_strategy: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub n8n_workflow_hash: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub n8n_workflow_semantic_hash: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub runner_backend: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub runner_target: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub runner_container_name: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub broker_workflow_id: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub broker_webhook_method: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub broker_webhook_path: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub execution_timeout_secs: Option<u64>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub adapted_from_workflow_id: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub adapted_from_n8n_workflow_id: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub adaptation_strategy: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub adaptation_status: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub source_workflow_hash: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub copy_workflow_hash: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub source_workflow_semantic_hash: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub copy_workflow_semantic_hash: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub lifecycle_status: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub lifecycle_severity: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub lifecycle_warnings: Vec<String>,
    #[serde(default, skip_serializing_if = "is_zero_u64")]
    pub last_lifecycle_checked_at_ms: u64,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub last_lifecycle_action: String,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub generated_copy_n8n_verified: bool,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub test_execution_id: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub test_result_preview: String,
    pub status: N8nWorkflowStatus,
    pub environment: N8nWorkflowEnvironment,
    pub risk_tier: RiskLevel,
    pub irreversibility_class: N8nIrreversibilityClass,
    pub timeout_class: N8nTimeoutClass,
    pub owner: String,
    pub requires_callback: Option<bool>,
    pub input_schema_ref: String,
    pub output_schema_ref: String,
    pub credential_requirements: Vec<String>,
    pub hitl_policy: String,
    pub category: String,
    pub description: String,
    pub example_prompts: Vec<String>,
    pub tags: Vec<String>,
    pub aliases: Vec<String>,
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
            n8n_workflow_id: String::new(),
            trigger_strategy: String::new(),
            result_mode: String::new(),
            webhook_method: String::new(),
            webhook_path: String::new(),
            preferred_output_node: None,
            output_strategy: String::new(),
            n8n_workflow_hash: String::new(),
            n8n_workflow_semantic_hash: String::new(),
            runner_backend: String::new(),
            runner_target: String::new(),
            runner_container_name: String::new(),
            broker_workflow_id: String::new(),
            broker_webhook_method: String::new(),
            broker_webhook_path: String::new(),
            execution_timeout_secs: None,
            adapted_from_workflow_id: String::new(),
            adapted_from_n8n_workflow_id: String::new(),
            adaptation_strategy: String::new(),
            adaptation_status: String::new(),
            source_workflow_hash: String::new(),
            copy_workflow_hash: String::new(),
            source_workflow_semantic_hash: String::new(),
            copy_workflow_semantic_hash: String::new(),
            lifecycle_status: String::new(),
            lifecycle_severity: String::new(),
            lifecycle_warnings: Vec::new(),
            last_lifecycle_checked_at_ms: 0,
            last_lifecycle_action: String::new(),
            generated_copy_n8n_verified: false,
            test_execution_id: String::new(),
            test_result_preview: String::new(),
            status: N8nWorkflowStatus::Draft,
            environment: N8nWorkflowEnvironment::Dev,
            risk_tier: RiskLevel::Yellow,
            irreversibility_class: N8nIrreversibilityClass::ReadOnly,
            timeout_class: N8nTimeoutClass::Background,
            owner: String::new(),
            requires_callback: None,
            input_schema_ref: String::new(),
            output_schema_ref: String::new(),
            credential_requirements: Vec::new(),
            hitl_policy: String::new(),
            category: String::new(),
            description: String::new(),
            example_prompts: Vec::new(),
            tags: Vec::new(),
            aliases: Vec::new(),
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

    pub fn requires_direct_endpoint_path(&self) -> bool {
        if self.requires_callback == Some(true) {
            return true;
        }

        let trigger = self.trigger_strategy.trim();
        let result_mode = self.result_mode.trim();
        if result_mode == "monitor_only" {
            return false;
        }

        !matches!(
            trigger,
            "manual_api_execute" | "scheduled_monitor" | "event_monitor" | "sub_workflow_broker"
        )
    }

    pub fn missing_approval_metadata(&self) -> Vec<&'static str> {
        let mut missing = Vec::new();
        if self.workflow_id.trim().is_empty() {
            missing.push("workflow_id");
        }
        if self.workflow_version.trim().is_empty() {
            missing.push("workflow_version");
        }
        if self.display_name.trim().is_empty() {
            missing.push("display_name");
        }
        if self.requires_direct_endpoint_path() && self.endpoint_path.trim().is_empty() {
            missing.push("endpoint_path");
        }
        if self.owner.trim().is_empty() {
            missing.push("owner");
        }
        if self.requires_callback.is_none() {
            missing.push("requires_callback");
        }
        if self.input_schema_ref.trim().is_empty() {
            missing.push("input_schema_ref");
        }
        if self.output_schema_ref.trim().is_empty() {
            missing.push("output_schema_ref");
        }
        if self
            .expected_evidence
            .iter()
            .all(|value| value.trim().is_empty())
        {
            missing.push("expected_evidence");
        }
        if self
            .credential_requirements
            .iter()
            .all(|value| value.trim().is_empty())
        {
            missing.push("credential_requirements");
        }
        if self.data_scope.iter().all(|value| value.trim().is_empty()) {
            missing.push("data_scope");
        }
        if self.hitl_policy.trim().is_empty() {
            missing.push("hitl_policy");
        }
        if self.category.trim().is_empty() {
            missing.push("category");
        }
        if self.description.trim().is_empty() {
            missing.push("description");
        }
        if self
            .example_prompts
            .iter()
            .all(|value| value.trim().is_empty())
        {
            missing.push("example_prompts");
        }
        if self.tags.iter().all(|value| value.trim().is_empty())
            && self.aliases.iter().all(|value| value.trim().is_empty())
        {
            missing.push("tags_or_aliases");
        }
        missing
    }

    pub fn is_ready_for_approval(&self) -> bool {
        self.missing_approval_metadata().is_empty()
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
    #[serde(alias = "workflow_name")]
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
