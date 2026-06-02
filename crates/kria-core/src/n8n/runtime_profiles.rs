use super::input_adaptation::{
    analyze_n8n_input_capability, N8nBinaryInputReport, N8nBranchReport, N8nCodeNodeReport,
    N8nInputCapability, N8nInputParameterCandidate, N8nInputSurfaceType, N8nOutputSelectionReport,
    N8nV5CapabilityStatus,
};
use super::types::N8nWorkflowConfig;
use super::workflow_validation::infer_webhook_endpoint_path;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

pub const N8N_RUNTIME_PROFILE_SCHEMA_VERSION: &str = "kria.n8n.runtime_profiles.v1";

fn is_zero_u64(value: &u64) -> bool {
    *value == 0
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum N8nRuntimeProfileStatus {
    Draft,
    NeedsReview,
    ReadyToTest,
    Unsupported,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum N8nTriggerStrategy {
    Webhook,
    ManualApiExecute,
    SubWorkflowBroker,
    ScheduledMonitor,
    EventMonitor,
    FormSubmit,
    ChatTrigger,
    Unsupported,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum N8nResultMode {
    PollExecution,
    Callback,
    MonitorOnly,
    Unsupported,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum N8nCredentialStatus {
    Present,
    Missing,
    Unknown,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum N8nRuntimeRiskEstimate {
    Green,
    Yellow,
    Red,
    NeedsReview,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum N8nRuntimeHitlStrategy {
    None,
    BeforeRun,
    N8nWaitResume,
    ExternalLink,
    NeedsReview,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum N8nOutputStrategy {
    FinalNonEmptyNode,
    ResponseLikeNode,
    WebhookResponse,
    ExecutionSummaryFallback,
    NeedsReview,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct N8nMetadataSuggestion {
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub category: Option<String>,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub aliases: Vec<String>,
    #[serde(default)]
    pub example_prompts: Vec<String>,
    #[serde(default)]
    pub data_scope: Vec<String>,
    #[serde(default)]
    pub credential_requirements: Vec<String>,
    #[serde(default)]
    pub hitl_policy: Option<String>,
    #[serde(default)]
    pub risk_estimate: Option<N8nRuntimeRiskEstimate>,
    #[serde(default)]
    pub hitl_strategy: Option<N8nRuntimeHitlStrategy>,
    #[serde(default)]
    pub confidence: f32,
    #[serde(default)]
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct N8nMetadataEnrichmentProvenance {
    pub schema_version: String,
    pub source: String,
    pub status: String,
    #[serde(default)]
    pub provider: Option<String>,
    #[serde(default)]
    pub model: Option<String>,
    pub workflow_hash: String,
    pub enriched_at_ms: u64,
    #[serde(default)]
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct N8nRuntimeProfileDraft {
    pub schema_version: String,
    pub profile_id: String,
    pub workflow_id: String,
    pub n8n_workflow_id: String,
    pub display_name: String,
    pub n8n_workflow_name: String,
    pub n8n_workflow_hash: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub n8n_workflow_semantic_hash: String,
    pub n8n_workflow_updated_at: Option<String>,
    pub status: N8nRuntimeProfileStatus,
    pub trigger_strategy: N8nTriggerStrategy,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub webhook_method: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub webhook_path: String,
    pub result_mode: N8nResultMode,
    pub detected_triggers: Vec<String>,
    pub input_candidates: Vec<String>,
    #[serde(default)]
    pub input_capability: N8nInputCapability,
    #[serde(default)]
    pub input_surface_type: N8nInputSurfaceType,
    #[serde(default)]
    pub hardcoded_parameter_candidates: Vec<N8nInputParameterCandidate>,
    #[serde(default)]
    pub code_node_reports: Vec<N8nCodeNodeReport>,
    #[serde(default)]
    pub binary_input_reports: Vec<N8nBinaryInputReport>,
    #[serde(default)]
    pub branch_reports: Vec<N8nBranchReport>,
    #[serde(default)]
    pub output_selection_report: N8nOutputSelectionReport,
    #[serde(default)]
    pub v5_capability_status: N8nV5CapabilityStatus,
    #[serde(default)]
    pub recommended_input_fields: Vec<String>,
    pub output_strategy: N8nOutputStrategy,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub runner_backend: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub runner_target: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub runner_container_name: String,
    pub credential_requirements: Vec<String>,
    pub credential_status: N8nCredentialStatus,
    pub category: String,
    pub risk_estimate: N8nRuntimeRiskEstimate,
    pub irreversibility_estimate: String,
    pub data_scope: Vec<String>,
    pub external_data_transfer: bool,
    pub hitl_detected: bool,
    pub hitl_strategy: N8nRuntimeHitlStrategy,
    pub confidence: f32,
    pub warnings: Vec<String>,
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
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub archived: bool,
    #[serde(default, skip_serializing_if = "is_zero_u64")]
    pub archived_at_ms: u64,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub archived_reason: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub archived_by: String,
    #[serde(default, skip_serializing_if = "is_zero_u64")]
    pub restored_at_ms: u64,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub crud_lifecycle_status: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub crud_lifecycle_warnings: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub enrichment: Option<N8nMetadataEnrichmentProvenance>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub enrichment_suggestion: Option<N8nMetadataSuggestion>,
    pub created_at_ms: u64,
    pub updated_at_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct N8nRuntimeProfileStore {
    pub schema_version: String,
    pub updated_at_ms: u64,
    pub profiles: Vec<N8nRuntimeProfileDraft>,
}

impl Default for N8nRuntimeProfileStore {
    fn default() -> Self {
        Self {
            schema_version: N8N_RUNTIME_PROFILE_SCHEMA_VERSION.into(),
            updated_at_ms: now_ms(),
            profiles: Vec::new(),
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum N8nRuntimeProfileStoreError {
    #[error("failed to read runtime profile store: {0}")]
    Read(#[from] io::Error),
    #[error("failed to parse runtime profile store: {0}")]
    Parse(serde_json::Error),
    #[error("failed to serialize runtime profile store: {0}")]
    Serialize(serde_json::Error),
}

pub fn default_runtime_profile_store_path() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("/tmp"))
        .join(".kria")
        .join("n8n")
        .join("runtime_profiles.json")
}

pub fn load_runtime_profile_store_at(
    path: &Path,
) -> Result<N8nRuntimeProfileStore, N8nRuntimeProfileStoreError> {
    if !path.exists() {
        return Ok(N8nRuntimeProfileStore::default());
    }
    let content = fs::read_to_string(path)?;
    serde_json::from_str(&content).map_err(N8nRuntimeProfileStoreError::Parse)
}

pub fn save_runtime_profile_store_at(
    path: &Path,
    store: &N8nRuntimeProfileStore,
) -> Result<(), N8nRuntimeProfileStoreError> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = fs::set_permissions(parent, fs::Permissions::from_mode(0o700));
        }
    }

    let mut next = store.clone();
    next.schema_version = N8N_RUNTIME_PROFILE_SCHEMA_VERSION.into();
    next.updated_at_ms = now_ms();
    let content =
        serde_json::to_string_pretty(&next).map_err(N8nRuntimeProfileStoreError::Serialize)?;
    fs::write(path, content)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o600))?;
    }
    Ok(())
}

pub fn upsert_runtime_profile(
    store: &mut N8nRuntimeProfileStore,
    mut profile: N8nRuntimeProfileDraft,
) {
    let now = now_ms();
    profile.updated_at_ms = now;
    if profile.created_at_ms == 0 {
        profile.created_at_ms = now;
    }
    if let Some(existing) = store
        .profiles
        .iter_mut()
        .find(|existing| existing.profile_id == profile.profile_id)
    {
        profile.created_at_ms = existing.created_at_ms;
        *existing = profile;
    } else {
        store.profiles.push(profile);
    }
    store.updated_at_ms = now;
}

pub fn delete_runtime_profile(store: &mut N8nRuntimeProfileStore, profile_id: &str) -> bool {
    let before = store.profiles.len();
    store
        .profiles
        .retain(|profile| profile.profile_id != profile_id);
    let removed = store.profiles.len() != before;
    if removed {
        store.updated_at_ms = now_ms();
    }
    removed
}

pub fn analyze_n8n_runtime_profiles(
    workflows: &[Value],
    configured_workflows: &[N8nWorkflowConfig],
) -> Vec<N8nRuntimeProfileDraft> {
    workflows
        .iter()
        .map(|workflow| analyze_n8n_runtime_profile(workflow, configured_workflows))
        .collect()
}

pub fn analyze_n8n_runtime_profile(
    workflow: &Value,
    configured_workflows: &[N8nWorkflowConfig],
) -> N8nRuntimeProfileDraft {
    let now = now_ms();
    let nodes = workflow_nodes(workflow);
    let n8n_workflow_id = string_field(workflow, &["id", "workflow_id", "workflowId"])
        .unwrap_or_else(|| slugify(&n8n_workflow_name(workflow)));
    let n8n_workflow_name = n8n_workflow_name(workflow);
    let workflow_id = configured_workflows
        .iter()
        .find(|configured| {
            configured
                .display_name
                .eq_ignore_ascii_case(&n8n_workflow_name)
        })
        .map(|configured| configured.workflow_id.clone())
        .unwrap_or_else(|| slugify(&n8n_workflow_name));
    let profile_id = slugify(&format!("{n8n_workflow_id}-{workflow_id}"));
    let detected_triggers = detect_triggers(&nodes);
    let trigger_strategy = detect_trigger_strategy(&nodes);
    let webhook_method = detect_webhook_method(&nodes).unwrap_or_default();
    let webhook_path = infer_webhook_endpoint_path(workflow).unwrap_or_default();
    let credential_requirements = detect_credentials(&nodes);
    let credential_status = if credential_requirements.is_empty() {
        N8nCredentialStatus::Unknown
    } else {
        N8nCredentialStatus::Present
    };
    let category = detect_category(&nodes);
    let (risk_estimate, irreversibility_estimate, risk_warnings) = detect_risk(&nodes);
    let hitl_detected = detect_hitl(&nodes);
    let hitl_strategy = if hitl_detected {
        N8nRuntimeHitlStrategy::NeedsReview
    } else {
        N8nRuntimeHitlStrategy::None
    };
    let output_strategy = detect_output_strategy(&nodes, &trigger_strategy);
    let result_mode = detect_result_mode(
        configured_workflows,
        &workflow_id,
        &n8n_workflow_name,
        &nodes,
        &trigger_strategy,
    );
    let input_report = analyze_n8n_input_capability(workflow);
    let mut input_candidates = detect_input_candidates(&nodes, &trigger_strategy);
    input_candidates.extend(input_report.recommended_input_fields.clone());
    input_candidates.sort();
    input_candidates.dedup();
    let data_scope = detect_data_scope(&category, &nodes);
    let external_data_transfer = detect_external_transfer(&nodes);
    let mut warnings = Vec::new();

    if matches!(trigger_strategy, N8nTriggerStrategy::Unsupported) {
        warnings
            .push("No supported trigger was detected; this profile is not runnable yet.".into());
    }
    if matches!(result_mode, N8nResultMode::Unsupported) {
        warnings.push("No safe result mode was detected.".into());
    }
    if matches!(trigger_strategy, N8nTriggerStrategy::Webhook) && webhook_method.trim().is_empty() {
        warnings.push("Webhook HTTP method could not be verified from workflow JSON; choose GET or POST before execution.".into());
    }
    if matches!(
        trigger_strategy,
        N8nTriggerStrategy::FormSubmit | N8nTriggerStrategy::ChatTrigger
    ) && webhook_path.trim().is_empty()
    {
        warnings.push("Form/Chat trigger endpoint could not be verified from workflow JSON; refresh from n8n after saving the trigger URL.".into());
    }
    if nodes.iter().any(|node| {
        lower_node_type(node).contains("chattrigger")
            && !node
                .get("parameters")
                .and_then(|parameters| parameters.get("public"))
                .and_then(Value::as_bool)
                .unwrap_or(false)
    }) {
        warnings.push("Chat Trigger is not public; KRIA can only submit production chat triggers when public access is enabled in n8n.".into());
    }
    if nodes.iter().any(|node| {
        lower_node_type(node).contains("formtrigger")
            && node
                .get("parameters")
                .and_then(|parameters| parameters.get("authentication"))
                .and_then(Value::as_str)
                .map(|value| !value.eq_ignore_ascii_case("none"))
                .unwrap_or(false)
    }) {
        warnings.push("Authenticated Form Trigger detected; KRIA Form Adapter v1 does not submit basic-auth protected forms yet.".into());
    }
    if matches!(trigger_strategy, N8nTriggerStrategy::SubWorkflowBroker) {
        warnings.push("Callable sub-workflow detected. Configure a trusted KRIA broker workflow before execution.".into());
    }
    if matches!(credential_status, N8nCredentialStatus::Unknown) {
        warnings.push("Credential requirements could not be verified from workflow JSON.".into());
    }
    warnings.extend(input_report.warnings.clone());
    if hitl_detected {
        warnings
            .push("Human-in-the-loop or wait behavior detected; review resume strategy.".into());
    }
    if matches!(output_strategy, N8nOutputStrategy::NeedsReview) {
        warnings.push("Output strategy needs review before execution can use this profile.".into());
    }
    warnings.extend(risk_warnings);
    warnings.sort();
    warnings.dedup();

    let status = profile_status(
        &trigger_strategy,
        &result_mode,
        &risk_estimate,
        &credential_status,
        &output_strategy,
        &hitl_strategy,
    );
    let confidence = estimate_confidence(
        &trigger_strategy,
        &result_mode,
        &category,
        &credential_status,
        &output_strategy,
        &warnings,
    );

    N8nRuntimeProfileDraft {
        schema_version: N8N_RUNTIME_PROFILE_SCHEMA_VERSION.into(),
        profile_id,
        workflow_id,
        n8n_workflow_id,
        display_name: title_from_name(&n8n_workflow_name),
        n8n_workflow_name,
        n8n_workflow_hash: workflow_hash(workflow),
        n8n_workflow_semantic_hash: semantic_workflow_hash(workflow),
        n8n_workflow_updated_at: string_field(workflow, &["updatedAt", "updated_at", "modifiedAt"]),
        status,
        trigger_strategy,
        webhook_method,
        webhook_path,
        result_mode,
        detected_triggers,
        input_candidates,
        input_capability: input_report.input_capability,
        input_surface_type: input_report.input_surface_type,
        hardcoded_parameter_candidates: input_report.hardcoded_parameter_candidates,
        code_node_reports: input_report.code_node_reports,
        binary_input_reports: input_report.binary_input_reports,
        branch_reports: input_report.branch_reports,
        output_selection_report: input_report.output_selection_report,
        v5_capability_status: input_report.v5_capability_status,
        recommended_input_fields: input_report.recommended_input_fields,
        output_strategy,
        runner_backend: String::new(),
        runner_target: String::new(),
        runner_container_name: String::new(),
        credential_requirements,
        credential_status,
        category,
        risk_estimate,
        irreversibility_estimate,
        data_scope,
        external_data_transfer,
        hitl_detected,
        hitl_strategy,
        confidence,
        warnings,
        lifecycle_status: String::new(),
        lifecycle_severity: String::new(),
        lifecycle_warnings: Vec::new(),
        last_lifecycle_checked_at_ms: 0,
        last_lifecycle_action: String::new(),
        generated_copy_n8n_verified: false,
        archived: false,
        archived_at_ms: 0,
        archived_reason: String::new(),
        archived_by: String::new(),
        restored_at_ms: 0,
        crud_lifecycle_status: String::new(),
        crud_lifecycle_warnings: Vec::new(),
        enrichment: None,
        enrichment_suggestion: None,
        created_at_ms: now,
        updated_at_ms: now,
    }
}

pub fn mark_profile_drift(
    mut refreshed: N8nRuntimeProfileDraft,
    existing: &N8nRuntimeProfileDraft,
) -> N8nRuntimeProfileDraft {
    refreshed.created_at_ms = existing.created_at_ms;
    refreshed.enrichment = existing.enrichment.clone();
    refreshed.enrichment_suggestion = existing.enrichment_suggestion.clone();
    if refreshed.runner_backend.trim().is_empty() {
        refreshed.runner_backend = existing.runner_backend.clone();
    }
    if refreshed.runner_target.trim().is_empty() {
        refreshed.runner_target = existing.runner_target.clone();
    }
    if refreshed.runner_container_name.trim().is_empty() {
        refreshed.runner_container_name = existing.runner_container_name.clone();
    }
    let refreshed_lifecycle_hash = workflow_lifecycle_hash(&refreshed);
    let existing_lifecycle_hash = workflow_lifecycle_hash(existing);
    if refreshed_lifecycle_hash != existing_lifecycle_hash {
        refreshed.status = N8nRuntimeProfileStatus::NeedsReview;
        refreshed.lifecycle_status = "needs_review".into();
        refreshed.lifecycle_severity = "warning".into();
        refreshed.last_lifecycle_checked_at_ms = now_ms();
        refreshed.last_lifecycle_action = "drift_detected".into();
        refreshed
            .lifecycle_warnings
            .push("n8n workflow changed since this profile was saved.".into());
        refreshed
            .warnings
            .push("n8n workflow changed since this profile was saved; review before use.".into());
        if let Some(enrichment) = refreshed.enrichment.as_mut() {
            enrichment.status = "stale".into();
            enrichment
                .warnings
                .push("Metadata enrichment is stale because the n8n workflow changed.".into());
            enrichment.warnings.sort();
            enrichment.warnings.dedup();
        }
        refreshed.lifecycle_warnings.sort();
        refreshed.lifecycle_warnings.dedup();
        refreshed.warnings.sort();
        refreshed.warnings.dedup();
    } else {
        refreshed.status = existing.status.clone();
        refreshed.lifecycle_status = existing.lifecycle_status.clone();
        refreshed.lifecycle_severity = existing.lifecycle_severity.clone();
        refreshed.lifecycle_warnings = existing.lifecycle_warnings.clone();
        refreshed.last_lifecycle_checked_at_ms = existing.last_lifecycle_checked_at_ms;
        refreshed.last_lifecycle_action = existing.last_lifecycle_action.clone();
        refreshed.generated_copy_n8n_verified = existing.generated_copy_n8n_verified;
    }
    refreshed
}

fn workflow_nodes(workflow: &Value) -> Vec<&Value> {
    workflow
        .get("nodes")
        .and_then(Value::as_array)
        .map(|nodes| nodes.iter().collect())
        .unwrap_or_default()
}

fn n8n_workflow_name(workflow: &Value) -> String {
    string_field(workflow, &["name", "display_name", "workflow_name"])
        .unwrap_or_else(|| string_field(workflow, &["id"]).unwrap_or_else(|| "n8n workflow".into()))
}

fn detect_triggers(nodes: &[&Value]) -> Vec<String> {
    nodes
        .iter()
        .filter(|node| is_trigger_node(node))
        .map(|node| format!("{} ({})", node_name(node), node_type(node)))
        .collect()
}

fn detect_trigger_strategy(nodes: &[&Value]) -> N8nTriggerStrategy {
    if nodes.iter().any(|node| is_webhook_trigger(node)) {
        N8nTriggerStrategy::Webhook
    } else if nodes
        .iter()
        .any(|node| lower_node_type(node).contains("executeworkflowtrigger"))
    {
        N8nTriggerStrategy::SubWorkflowBroker
    } else if nodes
        .iter()
        .any(|node| lower_node_type(node).contains("manualtrigger"))
    {
        N8nTriggerStrategy::ManualApiExecute
    } else if nodes.iter().any(|node| {
        lower_node_type(node).contains("scheduletrigger") || lower_node_type(node).contains("cron")
    }) {
        N8nTriggerStrategy::ScheduledMonitor
    } else if nodes
        .iter()
        .any(|node| lower_node_type(node).contains("formtrigger"))
    {
        N8nTriggerStrategy::FormSubmit
    } else if nodes
        .iter()
        .any(|node| lower_node_type(node).contains("chattrigger"))
    {
        N8nTriggerStrategy::ChatTrigger
    } else if nodes.iter().any(|node| is_trigger_node(node)) {
        N8nTriggerStrategy::EventMonitor
    } else {
        N8nTriggerStrategy::Unsupported
    }
}

fn detect_result_mode(
    configured_workflows: &[N8nWorkflowConfig],
    workflow_id: &str,
    n8n_workflow_name: &str,
    nodes: &[&Value],
    trigger_strategy: &N8nTriggerStrategy,
) -> N8nResultMode {
    let webhook_paths = nodes
        .iter()
        .filter(|node| is_webhook_trigger(node))
        .filter_map(|node| webhook_endpoint_path(node))
        .collect::<Vec<_>>();
    let configured_callback = configured_workflows.iter().any(|workflow| {
        workflow.requires_callback == Some(true)
            && (workflow.workflow_id == workflow_id
                || workflow
                    .display_name
                    .eq_ignore_ascii_case(n8n_workflow_name)
                || webhook_paths.iter().any(|path| {
                    path == &workflow.endpoint_path || path.ends_with(&workflow.endpoint_path)
                }))
    });

    if configured_callback {
        return N8nResultMode::Callback;
    }

    match trigger_strategy {
        N8nTriggerStrategy::Webhook
        | N8nTriggerStrategy::SubWorkflowBroker
        | N8nTriggerStrategy::ManualApiExecute
        | N8nTriggerStrategy::FormSubmit
        | N8nTriggerStrategy::ChatTrigger => N8nResultMode::PollExecution,
        N8nTriggerStrategy::ScheduledMonitor | N8nTriggerStrategy::EventMonitor => {
            N8nResultMode::MonitorOnly
        }
        N8nTriggerStrategy::Unsupported => N8nResultMode::Unsupported,
    }
}

fn detect_credentials(nodes: &[&Value]) -> Vec<String> {
    let mut credentials = BTreeSet::new();
    for node in nodes {
        if let Some(map) = node.get("credentials").and_then(Value::as_object) {
            for (kind, _) in map {
                credentials.insert(kind.to_string());
            }
        }
    }
    credentials.into_iter().collect()
}

fn detect_category(nodes: &[&Value]) -> String {
    let text = node_text(nodes);
    if text.contains("gmail") || text.contains("email") || text.contains("mail") {
        "email".into()
    } else if text.contains("slack") || text.contains("discord") || text.contains("telegram") {
        "messaging".into()
    } else if text.contains("calendar") || text.contains("calendly") {
        "calendar".into()
    } else if text.contains("github") || text.contains("gitlab") || text.contains("jira") {
        "work_tracking".into()
    } else if text.contains("postgres")
        || text.contains("mysql")
        || text.contains("supabase")
        || text.contains("database")
    {
        "database".into()
    } else if text.contains("file") || text.contains("drive") || text.contains("s3") {
        "file".into()
    } else if text.contains("httprequest") || text.contains("http request") {
        "api".into()
    } else {
        "automation".into()
    }
}

fn detect_risk(nodes: &[&Value]) -> (N8nRuntimeRiskEstimate, String, Vec<String>) {
    let mut risk = N8nRuntimeRiskEstimate::Green;
    let mut warnings = Vec::new();
    for node in nodes {
        if is_trigger_node(node) {
            continue;
        }
        let node_type = lower_node_type(node);
        let text = format!(
            "{} {} {}",
            node_type,
            node_name(node).to_ascii_lowercase(),
            node.get("parameters")
                .map(|value| value.to_string().to_ascii_lowercase())
                .unwrap_or_default()
        );

        if has_destructive_intent(&text) {
            risk = N8nRuntimeRiskEstimate::Red;
            warnings.push("Destructive or high-impact node behavior was detected.".into());
            continue;
        }

        if node_type.contains("httprequest") {
            let (http_risk, warning) = detect_http_request_risk(node, &text);
            risk = max_runtime_risk(risk, http_risk);
            if let Some(warning) = warning {
                warnings.push(warning);
            }
            continue;
        }

        if database_node_type(&node_type) {
            let (database_risk, warning) = detect_database_node_risk(node, &text);
            risk = max_runtime_risk(risk, database_risk);
            if let Some(warning) = warning {
                warnings.push(warning);
            }
            continue;
        }

        if node_type.contains("slack")
            && contains_any_risk_term(&text, &["post", "send", "message"])
        {
            risk = max_runtime_risk(risk, N8nRuntimeRiskEstimate::Yellow);
            continue;
        }

        if has_external_write_intent(&text) {
            risk = max_runtime_risk(risk, N8nRuntimeRiskEstimate::Yellow);
        }
    }

    let irreversibility = match risk {
        N8nRuntimeRiskEstimate::Green => "read_only",
        N8nRuntimeRiskEstimate::Yellow => "reversible_external",
        N8nRuntimeRiskEstimate::Red => "destructive_or_irreversible",
        N8nRuntimeRiskEstimate::NeedsReview => "needs_review",
    }
    .to_string();

    (risk, irreversibility, warnings)
}

fn detect_http_request_risk(node: &Value, text: &str) -> (N8nRuntimeRiskEstimate, Option<String>) {
    let method = http_request_method(node);
    if method == "DELETE" || has_destructive_intent(text) {
        return (
            N8nRuntimeRiskEstimate::Red,
            Some("External HTTP request appears destructive or irreversible.".into()),
        );
    }
    if matches!(method.as_str(), "GET" | "HEAD" | "OPTIONS") {
        return (N8nRuntimeRiskEstimate::Green, None);
    }
    if has_http_write_intent(text) {
        return (
            N8nRuntimeRiskEstimate::Yellow,
            Some("External HTTP request may create, send, update, or publish data.".into()),
        );
    }
    if http_request_looks_read_only(text) {
        return (
            N8nRuntimeRiskEstimate::Green,
            Some("External HTTP request appears read-only; review if the endpoint actually has side effects.".into()),
        );
    }
    (
        N8nRuntimeRiskEstimate::NeedsReview,
        Some(
            "External HTTP request method is not clearly read-only; review target and payload."
                .into(),
        ),
    )
}

fn database_node_type(node_type: &str) -> bool {
    [
        "postgres",
        "postgresql",
        "mysql",
        "mariadb",
        "sqlite",
        "mssql",
        "microsoftsql",
        "supabase",
        "database",
    ]
    .iter()
    .any(|term| node_type.contains(term))
}

fn detect_database_node_risk(node: &Value, text: &str) -> (N8nRuntimeRiskEstimate, Option<String>) {
    if has_destructive_intent(text)
        || contains_any_risk_term(
            text,
            &[
                "drop", "truncate", "alter", "grant", "revoke", "delete", "remove",
            ],
        )
    {
        return (
            N8nRuntimeRiskEstimate::Red,
            Some("Database node appears destructive or administrative.".into()),
        );
    }
    if contains_any_risk_term(
        text,
        &[
            "insert", "update", "upsert", "merge", "create", "write", "copy",
        ],
    ) {
        return (
            N8nRuntimeRiskEstimate::Yellow,
            Some("Database node may create or update data.".into()),
        );
    }

    let sql_values = collect_database_sql_values(node);
    if !sql_values.is_empty() {
        for sql in &sql_values {
            match database_sql_read_safety(sql) {
                DatabaseSqlSafety::ReadOnly => {}
                DatabaseSqlSafety::Unsafe(reason) => {
                    return (
                        N8nRuntimeRiskEstimate::Red,
                        Some(format!("Database SQL appears unsafe: {reason}.")),
                    );
                }
                DatabaseSqlSafety::NeedsReview(reason) => {
                    return (
                        N8nRuntimeRiskEstimate::NeedsReview,
                        Some(format!("Database SQL needs review: {reason}.")),
                    );
                }
            }
        }
        return (N8nRuntimeRiskEstimate::Green, None);
    }

    if contains_any_risk_term(
        text,
        &["select", "find", "get", "read", "search", "lookup", "list"],
    ) {
        return (N8nRuntimeRiskEstimate::Green, None);
    }

    (
        N8nRuntimeRiskEstimate::NeedsReview,
        Some("Database node operation is not clearly read-only.".into()),
    )
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum DatabaseSqlSafety {
    ReadOnly,
    NeedsReview(String),
    Unsafe(String),
}

fn collect_database_sql_values(node: &Value) -> Vec<String> {
    let mut values = Vec::new();
    if let Some(parameters) = node.get("parameters") {
        collect_database_sql_values_from_value(parameters, &mut values);
    }
    values
}

fn collect_database_sql_values_from_value(value: &Value, out: &mut Vec<String>) {
    match value {
        Value::Object(map) => {
            for (key, child) in map {
                let normalized = key
                    .chars()
                    .filter(|ch| ch.is_ascii_alphanumeric())
                    .collect::<String>()
                    .to_ascii_lowercase();
                if matches!(
                    normalized.as_str(),
                    "query" | "sql" | "statement" | "rawquery" | "sqlquery"
                ) {
                    if let Some(text) = child.as_str() {
                        let trimmed = text.trim();
                        if !trimmed.is_empty() && !trimmed.contains("={{") {
                            out.push(trimmed.to_string());
                        }
                    }
                }
                collect_database_sql_values_from_value(child, out);
            }
        }
        Value::Array(items) => {
            for item in items {
                collect_database_sql_values_from_value(item, out);
            }
        }
        _ => {}
    }
}

fn database_sql_read_safety(sql: &str) -> DatabaseSqlSafety {
    let trimmed = sql.trim();
    if trimmed.is_empty() {
        return DatabaseSqlSafety::NeedsReview("SQL is empty".into());
    }
    let lower = trimmed.to_ascii_lowercase();
    if lower.contains("--") || lower.contains("/*") || lower.contains("*/") {
        return DatabaseSqlSafety::Unsafe("SQL contains comments".into());
    }
    if lower.contains(';') {
        return DatabaseSqlSafety::Unsafe("SQL contains multiple or terminated statements".into());
    }
    let normalized = lower.split_whitespace().collect::<Vec<_>>().join(" ");
    if contains_any_risk_term(
        &normalized,
        &[
            "insert", "update", "delete", "upsert", "merge", "drop", "truncate", "alter", "create",
            "grant", "revoke", "copy", "call", "execute", "exec", "replace",
        ],
    ) {
        return DatabaseSqlSafety::Unsafe("SQL contains write/admin keywords".into());
    }
    if ["select", "show", "describe", "desc", "explain"]
        .iter()
        .any(|word| normalized == *word || normalized.starts_with(&format!("{word} ")))
    {
        return DatabaseSqlSafety::ReadOnly;
    }
    if normalized.starts_with("with ") && normalized.contains(" select ") {
        return DatabaseSqlSafety::ReadOnly;
    }
    DatabaseSqlSafety::NeedsReview("SQL does not start with a recognized read-only keyword".into())
}

fn max_runtime_risk(
    current: N8nRuntimeRiskEstimate,
    candidate: N8nRuntimeRiskEstimate,
) -> N8nRuntimeRiskEstimate {
    if runtime_risk_rank(&candidate) > runtime_risk_rank(&current) {
        candidate
    } else {
        current
    }
}

fn runtime_risk_rank(risk: &N8nRuntimeRiskEstimate) -> u8 {
    match risk {
        N8nRuntimeRiskEstimate::Green => 0,
        N8nRuntimeRiskEstimate::Yellow => 1,
        N8nRuntimeRiskEstimate::NeedsReview => 2,
        N8nRuntimeRiskEstimate::Red => 3,
    }
}

fn has_destructive_intent(text: &str) -> bool {
    contains_any_risk_term(
        text,
        &[
            "delete",
            "remove",
            "drop",
            "truncate",
            "destroy",
            "wipe",
            "purge",
            "refund",
            "payment",
            "charge",
            "transfer funds",
            "wire transfer",
        ],
    )
}

fn has_external_write_intent(text: &str) -> bool {
    contains_any_risk_term(
        text,
        &[
            "send",
            "create",
            "update",
            "write",
            "publish",
            "upload",
            "insert",
            "append",
            "add row",
            "post message",
            "send message",
            "send email",
            "draft",
            "invite",
            "submit",
        ],
    )
}

fn has_http_write_intent(text: &str) -> bool {
    contains_any_risk_term(
        text,
        &[
            "create",
            "update",
            "write",
            "publish",
            "upload",
            "insert",
            "append",
            "add row",
            "post message",
            "send message",
            "send email",
            "draft",
            "invite",
            "submit",
        ],
    )
}

fn http_request_looks_read_only(text: &str) -> bool {
    contains_any_risk_term(
        text,
        &[
            "get", "get all", "read", "fetch", "search", "query", "lookup", "list", "retrieve",
            "find", "metadata", "summary", "digest", "weather", "movie", "movies", "omdb",
            "tvmaze", "graphql",
        ],
    )
}

fn contains_any_risk_term(text: &str, terms: &[&str]) -> bool {
    terms.iter().any(|term| contains_risk_term(text, term))
}

fn contains_risk_term(text: &str, term: &str) -> bool {
    if term.contains(' ') {
        return text.contains(term);
    }
    text.split(|ch: char| !(ch.is_ascii_alphanumeric() || ch == '_'))
        .any(|token| token == term)
}

fn detect_hitl(nodes: &[&Value]) -> bool {
    nodes.iter().any(|node| {
        let text = format!(
            "{} {}",
            lower_node_type(node),
            node_name(node).to_ascii_lowercase()
        );
        text.contains("wait")
            || text.contains("approval")
            || text.contains("human")
            || text.contains("review")
    })
}

fn detect_output_strategy(
    nodes: &[&Value],
    trigger_strategy: &N8nTriggerStrategy,
) -> N8nOutputStrategy {
    if nodes
        .iter()
        .any(|node| lower_node_type(node).contains("respondtowebhook"))
    {
        N8nOutputStrategy::WebhookResponse
    } else if nodes.iter().any(|node| {
        let name = node_name(node).to_ascii_lowercase();
        name.contains("output") || name.contains("result") || name.contains("response")
    }) {
        N8nOutputStrategy::ResponseLikeNode
    } else if matches!(trigger_strategy, N8nTriggerStrategy::Unsupported) {
        N8nOutputStrategy::NeedsReview
    } else if nodes.is_empty() {
        N8nOutputStrategy::NeedsReview
    } else {
        N8nOutputStrategy::FinalNonEmptyNode
    }
}

fn detect_input_candidates(nodes: &[&Value], trigger_strategy: &N8nTriggerStrategy) -> Vec<String> {
    let mut inputs = BTreeSet::new();
    if !matches!(trigger_strategy, N8nTriggerStrategy::Unsupported) {
        inputs.insert("source_prompt".to_string());
    }
    if matches!(trigger_strategy, N8nTriggerStrategy::ChatTrigger) {
        inputs.insert("chatInput".to_string());
        inputs.insert("sessionId".to_string());
    }
    for node in nodes.iter().filter(|node| is_trigger_node(node)) {
        if let Some(parameters) = node.get("parameters").and_then(Value::as_object) {
            if lower_node_type(node).contains("formtrigger") {
                if let Some(values) = parameters
                    .get("formFields")
                    .and_then(|fields| fields.get("values"))
                    .and_then(Value::as_array)
                {
                    for field in values {
                        if let Some(label) = field
                            .get("fieldLabel")
                            .or_else(|| field.get("label"))
                            .or_else(|| field.get("name"))
                            .and_then(Value::as_str)
                            .map(str::trim)
                            .filter(|label| !label.is_empty())
                        {
                            inputs.insert(label.to_string());
                        }
                    }
                }
            }
            for key in parameters.keys() {
                let key = key.trim();
                if key.contains("field") || key.contains("parameter") || key.contains("path") {
                    inputs.insert(key.to_string());
                }
            }
        }
    }
    inputs.into_iter().collect()
}

fn detect_data_scope(category: &str, nodes: &[&Value]) -> Vec<String> {
    let mut scope = BTreeSet::new();
    match category {
        "email" => {
            scope.insert("email_metadata".to_string());
            if node_text(nodes).contains("body") || node_text(nodes).contains("message") {
                scope.insert("email_body".to_string());
            }
        }
        "calendar" => {
            scope.insert("calendar_events".to_string());
        }
        "messaging" => {
            scope.insert("external_messages".to_string());
        }
        "database" => {
            scope.insert("database_records".to_string());
        }
        "file" => {
            scope.insert("files".to_string());
        }
        _ => {
            scope.insert("user_requested".to_string());
        }
    }
    scope.into_iter().collect()
}

fn detect_external_transfer(nodes: &[&Value]) -> bool {
    nodes.iter().any(|node| {
        let text = lower_node_type(node);
        text.contains("slack")
            || text.contains("gmail")
            || text.contains("calendar")
            || text.contains("httprequest")
            || text.contains("webhook")
            || text.contains("jira")
            || text.contains("github")
    })
}

fn profile_status(
    trigger_strategy: &N8nTriggerStrategy,
    result_mode: &N8nResultMode,
    risk: &N8nRuntimeRiskEstimate,
    credential_status: &N8nCredentialStatus,
    output_strategy: &N8nOutputStrategy,
    hitl_strategy: &N8nRuntimeHitlStrategy,
) -> N8nRuntimeProfileStatus {
    if matches!(trigger_strategy, N8nTriggerStrategy::Unsupported)
        || matches!(result_mode, N8nResultMode::Unsupported)
    {
        return N8nRuntimeProfileStatus::Unsupported;
    }
    if matches!(
        risk,
        N8nRuntimeRiskEstimate::Red | N8nRuntimeRiskEstimate::NeedsReview
    ) || matches!(
        credential_status,
        N8nCredentialStatus::Unknown | N8nCredentialStatus::Missing
    ) || matches!(output_strategy, N8nOutputStrategy::NeedsReview)
        || !matches!(hitl_strategy, N8nRuntimeHitlStrategy::None)
    {
        return N8nRuntimeProfileStatus::NeedsReview;
    }
    N8nRuntimeProfileStatus::ReadyToTest
}

fn estimate_confidence(
    trigger_strategy: &N8nTriggerStrategy,
    result_mode: &N8nResultMode,
    category: &str,
    credential_status: &N8nCredentialStatus,
    output_strategy: &N8nOutputStrategy,
    warnings: &[String],
) -> f32 {
    let mut score: f32 = 0.35;
    if !matches!(trigger_strategy, N8nTriggerStrategy::Unsupported) {
        score += 0.2;
    }
    if !matches!(result_mode, N8nResultMode::Unsupported) {
        score += 0.15;
    }
    if category != "automation" {
        score += 0.1;
    }
    if matches!(credential_status, N8nCredentialStatus::Present) {
        score += 0.1;
    }
    if !matches!(output_strategy, N8nOutputStrategy::NeedsReview) {
        score += 0.1;
    }
    score -= (warnings.len() as f32 * 0.04).min(0.2);
    score.clamp(0.05, 0.98)
}

fn is_trigger_node(node: &Value) -> bool {
    let node_type = lower_node_type(node);
    (node_type.contains("trigger") || is_webhook_trigger(node))
        && !node_type.contains("respondtowebhook")
}

fn is_webhook_trigger(node: &Value) -> bool {
    let node_type = lower_node_type(node);
    node_type.contains("webhook") && !node_type.contains("respondtowebhook")
}

fn webhook_endpoint_path(node: &Value) -> Option<String> {
    let path = node
        .get("parameters")
        .and_then(|parameters| {
            parameters
                .get("path")
                .or_else(|| parameters.get("webhookId"))
        })
        .and_then(Value::as_str)?
        .trim()
        .trim_start_matches('/')
        .to_string();
    if path.is_empty() {
        None
    } else {
        Some(format!("/webhook/{path}"))
    }
}

fn detect_webhook_method(nodes: &[&Value]) -> Option<String> {
    let webhook_method = nodes
        .iter()
        .filter(|node| is_webhook_trigger(node))
        .find_map(|node| {
            node.get("parameters")
                .and_then(|parameters| {
                    parameters
                        .get("httpMethod")
                        .or_else(|| parameters.get("method"))
                })
                .and_then(Value::as_str)
                .map(|value| value.trim().to_ascii_uppercase())
                .filter(|value| matches!(value.as_str(), "GET" | "POST"))
        });
    if webhook_method.is_some() {
        return webhook_method;
    }
    if nodes.iter().any(|node| {
        let node_type = lower_node_type(node);
        node_type.contains("formtrigger") || node_type.contains("chattrigger")
    }) {
        return Some("POST".into());
    }
    None
}

fn http_request_method(node: &Value) -> String {
    node.get("parameters")
        .and_then(|parameters| {
            parameters
                .get("method")
                .or_else(|| parameters.get("httpMethod"))
        })
        .and_then(Value::as_str)
        .unwrap_or("GET")
        .trim()
        .to_ascii_uppercase()
}

fn lower_node_type(node: &Value) -> String {
    node_type(node)
        .to_ascii_lowercase()
        .replace(['-', '_', ' '], "")
}

fn node_type(node: &Value) -> String {
    string_field(node, &["type"]).unwrap_or_default()
}

fn node_name(node: &Value) -> String {
    string_field(node, &["name"]).unwrap_or_else(|| node_type(node))
}

fn node_text(nodes: &[&Value]) -> String {
    nodes
        .iter()
        .map(|node| format!("{} {}", node_type(node), node_name(node)))
        .collect::<Vec<_>>()
        .join(" ")
        .to_ascii_lowercase()
}

fn string_field(value: &Value, keys: &[&str]) -> Option<String> {
    keys.iter()
        .find_map(|key| value.get(*key).and_then(Value::as_str))
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

pub fn raw_workflow_hash(workflow: &Value) -> String {
    let canonical = serde_json::to_vec(workflow).unwrap_or_default();
    let mut hasher = Sha256::new();
    hasher.update(canonical);
    format!("sha256:{}", hex::encode(hasher.finalize()))
}

pub fn semantic_workflow_hash(workflow: &Value) -> String {
    let semantic = semantic_workflow_value(workflow);
    let canonical = serde_json::to_vec(&semantic).unwrap_or_default();
    let mut hasher = Sha256::new();
    hasher.update(canonical);
    format!("sha256:{}", hex::encode(hasher.finalize()))
}

fn workflow_hash(workflow: &Value) -> String {
    raw_workflow_hash(workflow)
}

fn workflow_lifecycle_hash(profile: &N8nRuntimeProfileDraft) -> &str {
    if !profile.n8n_workflow_semantic_hash.trim().is_empty() {
        profile.n8n_workflow_semantic_hash.trim()
    } else {
        profile.n8n_workflow_hash.trim()
    }
}

fn semantic_workflow_value(value: &Value) -> Value {
    match value {
        Value::Object(map) => {
            let mut sorted = BTreeMap::new();
            for (key, child) in map {
                if is_volatile_workflow_key(key) {
                    continue;
                }
                sorted.insert(key.clone(), semantic_workflow_value(child));
            }
            let mut normalized = serde_json::Map::new();
            for (key, child) in sorted {
                normalized.insert(key, child);
            }
            Value::Object(normalized)
        }
        Value::Array(values) => Value::Array(values.iter().map(semantic_workflow_value).collect()),
        _ => value.clone(),
    }
}

fn is_volatile_workflow_key(key: &str) -> bool {
    matches!(
        key,
        "active"
            | "createdAt"
            | "created_at"
            | "updatedAt"
            | "updated_at"
            | "modifiedAt"
            | "modified_at"
            | "versionId"
            | "version_id"
            | "pinData"
            | "staticData"
            | "executionData"
            | "executionCount"
            | "lastExecutionId"
            | "lastRun"
            | "position"
            | "notes"
            | "notesInFlow"
            | "color"
            | "displayOptions"
            | "viewport"
            | "zoom"
    )
}

fn slugify(value: &str) -> String {
    let mut slug = String::new();
    let mut last_was_sep = false;
    for ch in value.trim().chars() {
        if ch.is_ascii_alphanumeric() {
            slug.push(ch.to_ascii_lowercase());
            last_was_sep = false;
        } else if !last_was_sep {
            slug.push('_');
            last_was_sep = true;
        }
    }
    let slug = slug.trim_matches('_').to_string();
    if slug.is_empty() {
        "workflow".into()
    } else {
        slug
    }
}

fn title_from_name(value: &str) -> String {
    value.trim().replace(['_', '-'], " ")
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn workflow(nodes: Vec<Value>) -> Value {
        serde_json::json!({
            "id": "wf_1",
            "name": "Gmail Digest",
            "updatedAt": "2026-05-30T00:00:00.000Z",
            "nodes": nodes,
        })
    }

    fn node(name: &str, kind: &str, parameters: Value) -> Value {
        serde_json::json!({
            "name": name,
            "type": kind,
            "parameters": parameters,
        })
    }

    #[test]
    fn webhook_workflow_generates_webhook_strategy() {
        let draft = analyze_n8n_runtime_profile(
            &workflow(vec![node(
                "Webhook",
                "n8n-nodes-base.webhook",
                serde_json::json!({"path": "movies"}),
            )]),
            &[],
        );
        assert_eq!(draft.trigger_strategy, N8nTriggerStrategy::Webhook);
        assert_eq!(draft.result_mode, N8nResultMode::PollExecution);
    }

    #[test]
    fn manual_and_schedule_triggers_are_classified() {
        let manual = analyze_n8n_runtime_profile(
            &workflow(vec![node(
                "Manual",
                "n8n-nodes-base.manualTrigger",
                serde_json::json!({}),
            )]),
            &[],
        );
        let scheduled = analyze_n8n_runtime_profile(
            &workflow(vec![node(
                "Schedule",
                "n8n-nodes-base.scheduleTrigger",
                serde_json::json!({}),
            )]),
            &[],
        );
        assert_eq!(
            manual.trigger_strategy,
            N8nTriggerStrategy::ManualApiExecute
        );
        assert_eq!(
            scheduled.trigger_strategy,
            N8nTriggerStrategy::ScheduledMonitor
        );
        assert_eq!(scheduled.result_mode, N8nResultMode::MonitorOnly);
    }

    #[test]
    fn execute_workflow_trigger_generates_subworkflow_broker_strategy() {
        let draft = analyze_n8n_runtime_profile(
            &workflow(vec![node(
                "Execute Workflow Trigger",
                "n8n-nodes-base.executeWorkflowTrigger",
                serde_json::json!({}),
            )]),
            &[],
        );

        assert_eq!(
            draft.trigger_strategy,
            N8nTriggerStrategy::SubWorkflowBroker
        );
        assert_eq!(draft.result_mode, N8nResultMode::PollExecution);
        assert!(draft
            .warnings
            .iter()
            .any(|warning| warning.contains("trusted KRIA broker")));
    }

    #[test]
    fn form_and_chat_triggers_are_classified_with_submit_endpoints() {
        let mut form_node = node(
            "Form Trigger",
            "n8n-nodes-base.formTrigger",
            serde_json::json!({
                "formFields": {"values": [{"fieldLabel": "movie"}]},
            }),
        );
        {
            let object = form_node.as_object_mut().expect("node object");
            object.insert("webhookId".into(), serde_json::json!("kria-form-id"));
            object.insert("typeVersion".into(), serde_json::json!(2.5));
        }
        let form = analyze_n8n_runtime_profile(&workflow(vec![form_node]), &[]);
        let mut chat_node = node(
            "Chat Trigger",
            "@n8n/n8n-nodes-langchain.chatTrigger",
            serde_json::json!({"public": true, "mode": "webhook"}),
        );
        chat_node
            .as_object_mut()
            .expect("node object")
            .insert("webhookId".into(), serde_json::json!("kria-chat-id"));
        let chat = analyze_n8n_runtime_profile(&workflow(vec![chat_node]), &[]);

        assert_eq!(form.trigger_strategy, N8nTriggerStrategy::FormSubmit);
        assert_eq!(form.result_mode, N8nResultMode::PollExecution);
        assert_eq!(form.webhook_method, "POST");
        assert_eq!(form.webhook_path, "/form/kria-form-id");
        assert!(form.input_candidates.iter().any(|value| value == "movie"));
        assert_eq!(chat.trigger_strategy, N8nTriggerStrategy::ChatTrigger);
        assert_eq!(chat.webhook_method, "POST");
        assert_eq!(chat.webhook_path, "/webhook/kria-chat-id/chat");
        assert!(chat
            .input_candidates
            .iter()
            .any(|value| value == "chatInput"));
    }

    #[test]
    fn risk_estimates_follow_node_intent() {
        let gmail_read = analyze_n8n_runtime_profile(
            &workflow(vec![node(
                "Get messages",
                "n8n-nodes-base.gmail",
                serde_json::json!({"operation": "getAll"}),
            )]),
            &[],
        );
        let slack_write = analyze_n8n_runtime_profile(
            &workflow(vec![node(
                "Post message",
                "n8n-nodes-base.slack",
                serde_json::json!({"operation": "post"}),
            )]),
            &[],
        );
        let destructive = analyze_n8n_runtime_profile(
            &workflow(vec![node(
                "Delete row",
                "n8n-nodes-base.postgres",
                serde_json::json!({"operation": "delete"}),
            )]),
            &[],
        );
        assert_eq!(gmail_read.risk_estimate, N8nRuntimeRiskEstimate::Green);
        assert_eq!(slack_write.risk_estimate, N8nRuntimeRiskEstimate::Yellow);
        assert_eq!(destructive.risk_estimate, N8nRuntimeRiskEstimate::Red);
    }

    #[test]
    fn read_only_http_and_database_reads_stay_green() {
        let default_http_get = analyze_n8n_runtime_profile(
            &workflow(vec![node(
                "Fetch Movies",
                "n8n-nodes-base.httpRequest",
                serde_json::json!({
                    "url": "https://www.omdbapi.com/",
                    "queryParameters": {
                        "parameters": [
                            {"name": "t", "value": "Inception"}
                        ]
                    }
                }),
            )]),
            &[],
        );
        let read_like_post = analyze_n8n_runtime_profile(
            &workflow(vec![node(
                "Search movies",
                "n8n-nodes-base.httpRequest",
                serde_json::json!({
                    "method": "POST",
                    "url": "https://api.example.test/search",
                }),
            )]),
            &[],
        );
        let postgres_select = analyze_n8n_runtime_profile(
            &workflow(vec![node(
                "Read rows",
                "n8n-nodes-base.postgres",
                serde_json::json!({"operation": "select"}),
            )]),
            &[],
        );

        assert_eq!(
            default_http_get.risk_estimate,
            N8nRuntimeRiskEstimate::Green
        );
        assert_eq!(read_like_post.risk_estimate, N8nRuntimeRiskEstimate::Green);
        assert!(read_like_post
            .warnings
            .iter()
            .any(|warning| warning.contains("appears read-only")));
        assert_eq!(postgres_select.risk_estimate, N8nRuntimeRiskEstimate::Green);
    }

    #[test]
    fn database_sql_risk_scanner_classifies_read_and_destructive_sql() {
        let read_query = analyze_n8n_runtime_profile(
            &workflow(vec![node(
                "Read customers",
                "n8n-nodes-base.postgres",
                serde_json::json!({
                    "operation": "executeQuery",
                    "query": "SELECT id, email FROM customers WHERE email = :email"
                }),
            )]),
            &[],
        );
        let multi_statement = analyze_n8n_runtime_profile(
            &workflow(vec![node(
                "Unsafe query",
                "n8n-nodes-base.postgres",
                serde_json::json!({
                    "operation": "executeQuery",
                    "query": "SELECT * FROM customers; DELETE FROM customers"
                }),
            )]),
            &[],
        );
        let hidden_write = analyze_n8n_runtime_profile(
            &workflow(vec![node(
                "Hidden write",
                "n8n-nodes-base.postgres",
                serde_json::json!({
                    "operation": "executeQuery",
                    "query": "SELECT * FROM customers -- DROP TABLE customers"
                }),
            )]),
            &[],
        );

        assert_eq!(read_query.risk_estimate, N8nRuntimeRiskEstimate::Green);
        assert_eq!(multi_statement.risk_estimate, N8nRuntimeRiskEstimate::Red);
        assert_eq!(hidden_write.risk_estimate, N8nRuntimeRiskEstimate::Red);
    }

    #[test]
    fn uncertain_database_operations_need_review() {
        let draft = analyze_n8n_runtime_profile(
            &workflow(vec![node(
                "Custom database op",
                "n8n-nodes-base.mysql",
                serde_json::json!({"mode": "custom"}),
            )]),
            &[],
        );

        assert_eq!(draft.risk_estimate, N8nRuntimeRiskEstimate::NeedsReview);
        assert!(draft
            .warnings
            .iter()
            .any(|warning| warning.contains("not clearly read-only")));
    }

    #[test]
    fn http_writes_are_yellow_or_review_and_http_delete_is_red() {
        let known_write = analyze_n8n_runtime_profile(
            &workflow(vec![node(
                "Send message",
                "n8n-nodes-base.httpRequest",
                serde_json::json!({
                    "method": "POST",
                    "url": "https://hooks.example.test/slack",
                }),
            )]),
            &[],
        );
        let unknown_post = analyze_n8n_runtime_profile(
            &workflow(vec![node(
                "Call HTTP",
                "n8n-nodes-base.httpRequest",
                serde_json::json!({"method": "POST"}),
            )]),
            &[],
        );
        let delete_call = analyze_n8n_runtime_profile(
            &workflow(vec![node(
                "Delete record",
                "n8n-nodes-base.httpRequest",
                serde_json::json!({"method": "DELETE"}),
            )]),
            &[],
        );

        assert_eq!(known_write.risk_estimate, N8nRuntimeRiskEstimate::Yellow);
        assert_eq!(
            unknown_post.risk_estimate,
            N8nRuntimeRiskEstimate::NeedsReview
        );
        assert_eq!(delete_call.risk_estimate, N8nRuntimeRiskEstimate::Red);
    }

    #[test]
    fn unknown_http_write_needs_review() {
        let draft = analyze_n8n_runtime_profile(
            &workflow(vec![node(
                "Send HTTP",
                "n8n-nodes-base.httpRequest",
                serde_json::json!({"method": "POST"}),
            )]),
            &[],
        );
        assert_eq!(draft.risk_estimate, N8nRuntimeRiskEstimate::NeedsReview);
        assert!(draft
            .warnings
            .iter()
            .any(|warning| warning.contains("not clearly read-only")));
    }

    #[test]
    fn hitl_and_unknown_trigger_are_flagged() {
        let hitl = analyze_n8n_runtime_profile(
            &workflow(vec![
                node(
                    "Manual",
                    "n8n-nodes-base.manualTrigger",
                    serde_json::json!({}),
                ),
                node(
                    "Wait for Approval",
                    "n8n-nodes-base.wait",
                    serde_json::json!({}),
                ),
            ]),
            &[],
        );
        let unsupported = analyze_n8n_runtime_profile(
            &workflow(vec![node(
                "Transform",
                "n8n-nodes-base.set",
                serde_json::json!({}),
            )]),
            &[],
        );
        assert!(hitl.hitl_detected);
        assert_eq!(hitl.hitl_strategy, N8nRuntimeHitlStrategy::NeedsReview);
        assert_eq!(
            unsupported.trigger_strategy,
            N8nTriggerStrategy::Unsupported
        );
        assert_eq!(unsupported.status, N8nRuntimeProfileStatus::Unsupported);
    }

    #[test]
    fn profile_drift_marks_needs_review() {
        let original = analyze_n8n_runtime_profile(
            &workflow(vec![node(
                "Manual",
                "n8n-nodes-base.manualTrigger",
                serde_json::json!({}),
            )]),
            &[],
        );
        let refreshed = analyze_n8n_runtime_profile(
            &workflow(vec![node(
                "Manual changed",
                "n8n-nodes-base.manualTrigger",
                serde_json::json!({}),
            )]),
            &[],
        );
        let marked = mark_profile_drift(refreshed, &original);
        assert_eq!(marked.status, N8nRuntimeProfileStatus::NeedsReview);
        assert!(marked
            .warnings
            .iter()
            .any(|warning| warning.contains("changed")));
    }

    #[test]
    fn semantic_hash_ignores_volatile_n8n_fields() {
        let base = workflow(vec![serde_json::json!({
            "id": "node-1",
            "name": "Webhook",
            "type": "n8n-nodes-base.webhook",
            "position": [10, 20],
            "parameters": {"path": "movies", "httpMethod": "POST"}
        })]);
        let mut volatile = base.clone();
        let object = volatile.as_object_mut().expect("workflow object");
        object.insert(
            "updatedAt".into(),
            serde_json::json!("2026-06-01T10:00:00Z"),
        );
        object.insert("versionId".into(), serde_json::json!("volatile-version"));
        object.insert("active".into(), serde_json::json!(true));
        if let Some(nodes) = object
            .get_mut("nodes")
            .and_then(|value| value.as_array_mut())
        {
            nodes[0]["position"] = serde_json::json!([999, 888]);
            nodes[0]["notes"] = serde_json::json!("canvas note");
        }

        assert_eq!(
            semantic_workflow_hash(&base),
            semantic_workflow_hash(&volatile)
        );
        assert_ne!(raw_workflow_hash(&base), raw_workflow_hash(&volatile));
    }

    #[test]
    fn semantic_hash_changes_on_safety_relevant_parameters() {
        let base = workflow(vec![serde_json::json!({
            "name": "Webhook",
            "type": "n8n-nodes-base.webhook",
            "parameters": {"path": "movies", "httpMethod": "POST"}
        })]);
        let changed = workflow(vec![serde_json::json!({
            "name": "Webhook",
            "type": "n8n-nodes-base.webhook",
            "parameters": {"path": "movies-v2", "httpMethod": "POST"}
        })]);

        assert_ne!(
            semantic_workflow_hash(&base),
            semantic_workflow_hash(&changed)
        );
    }

    #[test]
    fn runtime_profile_store_save_read_delete_roundtrip() {
        let dir = tempdir().expect("tempdir");
        let path = dir.path().join("runtime_profiles.json");
        let mut store = N8nRuntimeProfileStore::default();
        let profile = analyze_n8n_runtime_profile(
            &workflow(vec![node(
                "Manual",
                "n8n-nodes-base.manualTrigger",
                serde_json::json!({}),
            )]),
            &[],
        );
        let profile_id = profile.profile_id.clone();
        upsert_runtime_profile(&mut store, profile);
        save_runtime_profile_store_at(&path, &store).expect("save");
        let mut loaded = load_runtime_profile_store_at(&path).expect("load");
        assert_eq!(loaded.profiles.len(), 1);
        assert!(delete_runtime_profile(&mut loaded, &profile_id));
        assert!(loaded.profiles.is_empty());
    }

    #[test]
    fn profile_store_does_not_serialize_credential_secret_values() {
        let profile = analyze_n8n_runtime_profile(
            &workflow(vec![serde_json::json!({
                "name": "Gmail",
                "type": "n8n-nodes-base.gmail",
                "parameters": {},
                "credentials": {
                    "gmailOAuth2": {
                        "id": "credential-id",
                        "name": "safe account name",
                        "accessToken": "SECRET_TOKEN_VALUE"
                    }
                }
            })]),
            &[],
        );
        let serialized = serde_json::to_string(&profile).expect("serialize");
        assert!(!serialized.contains("SECRET_TOKEN_VALUE"));
        assert!(!serialized.contains("safe account name"));
        assert!(serialized.contains("gmailOAuth2"));
    }
}
