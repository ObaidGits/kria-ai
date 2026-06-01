use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::collections::{HashMap, HashSet};

const REQUIRED_CALLBACK_FIELDS: &[&str] = &[
    "correlation_id",
    "event_id",
    "sequence_number",
    "workflow_id",
    "workflow_version",
    "n8n_run_id",
    "status",
    "occurred_at_ms",
];

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum N8nWorkflowValidationCheckStatus {
    Passed,
    Failed,
    Warning,
    Skipped,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum N8nWorkflowValidationReportStatus {
    Passed,
    Failed,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct N8nWorkflowValidationCheck {
    pub id: String,
    pub status: N8nWorkflowValidationCheckStatus,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct N8nWorkflowValidationOptions {
    pub workflow_id: String,
    pub requires_callback: bool,
    pub expected_n8n_major_version: Option<u64>,
    pub installed_n8n_version: Option<String>,
    pub allow_version_mismatch: bool,
}

impl Default for N8nWorkflowValidationOptions {
    fn default() -> Self {
        Self {
            workflow_id: String::new(),
            requires_callback: true,
            expected_n8n_major_version: Some(2),
            installed_n8n_version: None,
            allow_version_mismatch: false,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct N8nWorkflowValidationReport {
    pub status: N8nWorkflowValidationReportStatus,
    pub workflow_id: String,
    pub checks: Vec<N8nWorkflowValidationCheck>,
    pub safe_to_import: bool,
    pub safe_to_activate: bool,
    pub backup_required_before_update: bool,
    pub normalized_hash: String,
}

impl N8nWorkflowValidationReport {
    pub fn failed_checks(&self) -> Vec<&N8nWorkflowValidationCheck> {
        self.checks
            .iter()
            .filter(|check| check.status == N8nWorkflowValidationCheckStatus::Failed)
            .collect()
    }
}

fn check(
    id: &str,
    status: N8nWorkflowValidationCheckStatus,
    message: impl Into<String>,
) -> N8nWorkflowValidationCheck {
    N8nWorkflowValidationCheck {
        id: id.into(),
        status,
        message: message.into(),
    }
}

fn push_pass(checks: &mut Vec<N8nWorkflowValidationCheck>, id: &str, message: impl Into<String>) {
    checks.push(check(id, N8nWorkflowValidationCheckStatus::Passed, message));
}

fn push_fail(checks: &mut Vec<N8nWorkflowValidationCheck>, id: &str, message: impl Into<String>) {
    checks.push(check(id, N8nWorkflowValidationCheckStatus::Failed, message));
}

fn push_warning(
    checks: &mut Vec<N8nWorkflowValidationCheck>,
    id: &str,
    message: impl Into<String>,
) {
    checks.push(check(
        id,
        N8nWorkflowValidationCheckStatus::Warning,
        message,
    ));
}

fn push_skipped(
    checks: &mut Vec<N8nWorkflowValidationCheck>,
    id: &str,
    message: impl Into<String>,
) {
    checks.push(check(
        id,
        N8nWorkflowValidationCheckStatus::Skipped,
        message,
    ));
}

fn normalize_json_hash(value: &Value) -> String {
    let bytes = serde_json::to_vec(value).unwrap_or_default();
    let digest = Sha256::digest(&bytes);
    hex::encode(digest)
}

fn node_name(node: &Value) -> Option<&str> {
    node.get("name").and_then(Value::as_str).map(str::trim)
}

fn node_id(node: &Value) -> Option<&str> {
    node.get("id").and_then(Value::as_str).map(str::trim)
}

fn node_type(node: &Value) -> &str {
    node.get("type")
        .and_then(Value::as_str)
        .map(str::trim)
        .unwrap_or_default()
}

fn lower_json(value: &Value) -> String {
    serde_json::to_string(value)
        .unwrap_or_default()
        .to_ascii_lowercase()
}

fn installed_major(version: &str) -> Option<u64> {
    version
        .trim()
        .split('.')
        .next()
        .and_then(|part| part.parse::<u64>().ok())
}

fn collect_connection_targets(value: &Value, targets: &mut Vec<String>) {
    match value {
        Value::Object(map) => {
            if let Some(node) = map.get("node").and_then(Value::as_str) {
                targets.push(node.trim().to_string());
            }
            for nested in map.values() {
                collect_connection_targets(nested, targets);
            }
        }
        Value::Array(values) => {
            for nested in values {
                collect_connection_targets(nested, targets);
            }
        }
        _ => {}
    }
}

fn is_safe_secret_reference(value: &str) -> bool {
    let trimmed = value.trim();
    trimmed.is_empty()
        || trimmed.starts_with("={{")
        || trimmed.contains("$env")
        || trimmed.contains("process.env")
        || trimmed.contains("KRIA_N8N_")
        || trimmed.contains("credentials")
        || trimmed.eq_ignore_ascii_case("none")
}

fn looks_like_secret_key(key: &str) -> bool {
    let lower = key.to_ascii_lowercase();
    lower.contains("secret")
        || lower.contains("password")
        || lower.contains("token")
        || lower.contains("api_key")
        || lower.contains("apikey")
        || lower.contains("authorization")
}

fn looks_like_secret_literal(value: &str) -> bool {
    let trimmed = value.trim();
    if is_safe_secret_reference(trimmed) {
        return false;
    }
    if trimmed.len() < 16 {
        return false;
    }
    let alpha_num = trimmed
        .chars()
        .filter(|ch| ch.is_ascii_alphanumeric())
        .count();
    alpha_num >= 16 && alpha_num * 2 >= trimmed.len()
}

fn collect_secret_leaks(value: &Value, path: &str, leaks: &mut Vec<String>) {
    match value {
        Value::Object(map) => {
            for (key, nested) in map {
                let nested_path = if path.is_empty() {
                    key.to_string()
                } else {
                    format!("{path}.{key}")
                };
                if looks_like_secret_key(key) {
                    if let Some(text) = nested.as_str() {
                        if looks_like_secret_literal(text) {
                            leaks.push(nested_path.clone());
                        }
                    }
                }
                collect_secret_leaks(nested, &nested_path, leaks);
            }
        }
        Value::Array(values) => {
            for (index, nested) in values.iter().enumerate() {
                collect_secret_leaks(nested, &format!("{path}[{index}]"), leaks);
            }
        }
        _ => {}
    }
}

pub fn validate_n8n_workflow_json_str(
    raw_json: &str,
    options: N8nWorkflowValidationOptions,
) -> N8nWorkflowValidationReport {
    match serde_json::from_str::<Value>(raw_json) {
        Ok(value) => validate_n8n_workflow_json(&value, options),
        Err(error) => {
            let workflow_id = options.workflow_id;
            N8nWorkflowValidationReport {
                status: N8nWorkflowValidationReportStatus::Failed,
                workflow_id,
                checks: vec![check(
                    "json_parse",
                    N8nWorkflowValidationCheckStatus::Failed,
                    format!("Workflow JSON did not parse: {error}"),
                )],
                safe_to_import: false,
                safe_to_activate: false,
                backup_required_before_update: true,
                normalized_hash: String::new(),
            }
        }
    }
}

pub fn validate_n8n_workflow_json(
    workflow_json: &Value,
    options: N8nWorkflowValidationOptions,
) -> N8nWorkflowValidationReport {
    let mut checks = Vec::new();
    push_pass(&mut checks, "json_parse", "Workflow JSON parsed");

    let workflow_id = if options.workflow_id.trim().is_empty() {
        workflow_json
            .get("id")
            .or_else(|| workflow_json.get("name"))
            .and_then(Value::as_str)
            .unwrap_or("draft_workflow")
            .trim()
            .to_string()
    } else {
        options.workflow_id.trim().to_string()
    };

    let empty_nodes = Vec::new();
    let nodes = workflow_json.get("nodes").and_then(Value::as_array);
    let connections = workflow_json.get("connections").and_then(Value::as_object);

    let nodes = match nodes {
        Some(nodes) if !nodes.is_empty() => {
            push_pass(
                &mut checks,
                "top_level_fields",
                "Workflow contains non-empty nodes array",
            );
            nodes
        }
        Some(_) => {
            push_fail(
                &mut checks,
                "top_level_fields",
                "Workflow nodes array is empty",
            );
            &empty_nodes
        }
        None => {
            push_fail(
                &mut checks,
                "top_level_fields",
                "Workflow is missing required top-level nodes array",
            );
            &empty_nodes
        }
    };

    if connections.is_some() {
        push_pass(
            &mut checks,
            "connections_present",
            "Workflow contains connections object",
        );
    } else {
        push_fail(
            &mut checks,
            "connections_present",
            "Workflow is missing required top-level connections object",
        );
    }

    let mut seen_ids = HashSet::new();
    let mut duplicate_ids = Vec::new();
    let mut seen_names = HashSet::new();
    let mut duplicate_names = Vec::new();
    let mut names = HashSet::new();

    for node in nodes {
        if let Some(id) = node_id(node).filter(|id| !id.is_empty()) {
            if !seen_ids.insert(id.to_string()) {
                duplicate_ids.push(id.to_string());
            }
        }
        if let Some(name) = node_name(node).filter(|name| !name.is_empty()) {
            names.insert(name.to_string());
            if !seen_names.insert(name.to_string()) {
                duplicate_names.push(name.to_string());
            }
        }
    }

    if duplicate_ids.is_empty() && duplicate_names.is_empty() {
        push_pass(&mut checks, "unique_nodes", "Node IDs and names are unique");
    } else {
        let mut parts = Vec::new();
        if !duplicate_ids.is_empty() {
            parts.push(format!("duplicate ids: {}", duplicate_ids.join(", ")));
        }
        if !duplicate_names.is_empty() {
            parts.push(format!("duplicate names: {}", duplicate_names.join(", ")));
        }
        push_fail(&mut checks, "unique_nodes", parts.join("; "));
    }

    if let Some(connections) = connections {
        let mut missing_sources = Vec::new();
        let mut missing_targets = Vec::new();
        for (source, value) in connections {
            if !names.contains(source) {
                missing_sources.push(source.clone());
            }
            let mut targets = Vec::new();
            collect_connection_targets(value, &mut targets);
            for target in targets {
                if !names.contains(&target) {
                    missing_targets.push(target);
                }
            }
        }

        if missing_sources.is_empty() && missing_targets.is_empty() {
            push_pass(
                &mut checks,
                "graph_integrity",
                "Connections reference existing nodes",
            );
        } else {
            let mut parts = Vec::new();
            if !missing_sources.is_empty() {
                parts.push(format!(
                    "missing source nodes: {}",
                    missing_sources.join(", ")
                ));
            }
            if !missing_targets.is_empty() {
                parts.push(format!(
                    "missing target nodes: {}",
                    missing_targets.join(", ")
                ));
            }
            push_fail(&mut checks, "graph_integrity", parts.join("; "));
        }
    }

    let webhook_nodes = nodes
        .iter()
        .filter(|node| node_type(node).to_ascii_lowercase().contains("webhook"))
        .collect::<Vec<_>>();
    if webhook_nodes.is_empty() {
        push_fail(
            &mut checks,
            "webhook_node",
            "KRIA-invoked workflow is missing a webhook node",
        );
    } else {
        push_pass(
            &mut checks,
            "webhook_node",
            format!("Found {} webhook node(s)", webhook_nodes.len()),
        );
    }

    if options.requires_callback {
        let callback_nodes = nodes
            .iter()
            .filter(|node| {
                node_type(node).to_ascii_lowercase().contains("httprequest")
                    && lower_json(node).contains("/api/n8n/callback")
            })
            .collect::<Vec<_>>();

        if callback_nodes.is_empty() {
            push_fail(
                &mut checks,
                "callback_node",
                "Async KRIA workflow is missing an HTTP Request callback node",
            );
        } else {
            push_pass(
                &mut checks,
                "callback_node",
                "Found KRIA callback HTTP Request node",
            );

            let callback_transport = callback_nodes
                .iter()
                .map(|node| lower_json(node))
                .collect::<Vec<_>>()
                .join("\n");
            let callback_body = lower_json(workflow_json);
            let missing_fields = REQUIRED_CALLBACK_FIELDS
                .iter()
                .copied()
                .filter(|field| !callback_body.contains(field))
                .collect::<Vec<_>>();
            if missing_fields.is_empty() {
                push_pass(
                    &mut checks,
                    "callback_contract",
                    "Callback body includes the required KRIA envelope fields",
                );
            } else {
                push_fail(
                    &mut checks,
                    "callback_contract",
                    format!("Callback body is missing: {}", missing_fields.join(", ")),
                );
            }

            if callback_transport.contains("callback_body")
                && callback_transport.contains("callback_signature")
                && callback_transport.contains("x-kria-signature")
            {
                push_pass(
                    &mut checks,
                    "callback_signature_body_match",
                    "Callback node sends the same named callback body that is signed",
                );
            } else {
                push_fail(
                    &mut checks,
                    "callback_signature_body_match",
                    "Callback node must sign and send the same JSON body",
                );
            }
        }
    } else {
        push_skipped(
            &mut checks,
            "callback_contract",
            "Workflow metadata does not require callbacks",
        );
    }

    let mut leaks = Vec::new();
    collect_secret_leaks(workflow_json, "", &mut leaks);
    if leaks.is_empty() {
        push_pass(
            &mut checks,
            "secret_leak",
            "No hardcoded secret-like literals detected",
        );
    } else {
        leaks.sort();
        leaks.dedup();
        push_fail(
            &mut checks,
            "secret_leak",
            format!(
                "Hardcoded secret-like values found at: {}",
                leaks.join(", ")
            ),
        );
    }

    if let (Some(expected), Some(installed)) = (
        options.expected_n8n_major_version,
        options.installed_n8n_version.as_deref(),
    ) {
        match installed_major(installed) {
            Some(actual) if actual == expected || options.allow_version_mismatch => push_pass(
                &mut checks,
                "n8n_version_compatibility",
                format!("Installed n8n major version {actual} is allowed"),
            ),
            Some(actual) => push_fail(
                &mut checks,
                "n8n_version_compatibility",
                format!("Workflow expects n8n major {expected}, installed major is {actual}"),
            ),
            None => push_warning(
                &mut checks,
                "n8n_version_compatibility",
                format!("Could not parse installed n8n version '{installed}'"),
            ),
        }
    } else {
        push_skipped(
            &mut checks,
            "n8n_version_compatibility",
            "Installed n8n version was not supplied for static validation",
        );
    }

    push_warning(
        &mut checks,
        "activation_gate",
        "Validation permits draft import only; activation still requires backup, safe test execution, and approval",
    );

    let failed = checks
        .iter()
        .any(|check| check.status == N8nWorkflowValidationCheckStatus::Failed);
    N8nWorkflowValidationReport {
        status: if failed {
            N8nWorkflowValidationReportStatus::Failed
        } else {
            N8nWorkflowValidationReportStatus::Passed
        },
        workflow_id,
        checks,
        safe_to_import: !failed,
        safe_to_activate: false,
        backup_required_before_update: true,
        normalized_hash: normalize_json_hash(workflow_json),
    }
}

pub fn infer_webhook_endpoint_path(workflow_json: &Value) -> Option<String> {
    workflow_json
        .get("nodes")
        .and_then(Value::as_array)?
        .iter()
        .find_map(infer_trigger_endpoint_path)
}

fn infer_trigger_endpoint_path(node: &Value) -> Option<String> {
    let node_type = node_type(node).to_ascii_lowercase();
    let parameters = node.get("parameters");
    if node_type.contains("formtrigger") {
        let segment = node
            .get("webhookId")
            .and_then(Value::as_str)
            .or_else(|| {
                parameters
                    .and_then(|parameters| parameters.get("path"))
                    .and_then(Value::as_str)
            })
            .or_else(|| {
                parameters
                    .and_then(|parameters| parameters.get("options"))
                    .and_then(|options| options.get("path"))
                    .and_then(Value::as_str)
            })?
            .trim()
            .trim_start_matches('/');
        if segment.is_empty() {
            return None;
        }
        let version = node
            .get("typeVersion")
            .and_then(Value::as_f64)
            .unwrap_or(2.5);
        if version < 2.0 {
            return Some(format!("/form/{segment}/form"));
        }
        return Some(format!("/form/{segment}"));
    }
    if node_type.contains("chattrigger") {
        let webhook_id = node
            .get("webhookId")
            .and_then(Value::as_str)?
            .trim()
            .trim_start_matches('/');
        if webhook_id.is_empty() {
            return None;
        }
        return Some(format!("/webhook/{webhook_id}/chat"));
    }
    if node_type.contains("webhook") && !node_type.contains("respondtowebhook") {
        let path = parameters
            .and_then(|parameters| {
                parameters
                    .get("path")
                    .or_else(|| parameters.get("webhookId"))
            })
            .and_then(Value::as_str)?
            .trim()
            .trim_start_matches('/');
        if path.is_empty() {
            None
        } else {
            Some(format!("/webhook/{path}"))
        }
    } else {
        None
    }
}

pub fn workflow_validation_summary(
    report: &N8nWorkflowValidationReport,
) -> HashMap<String, N8nWorkflowValidationCheckStatus> {
    report
        .checks
        .iter()
        .map(|check| (check.id.clone(), check.status.clone()))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn valid_workflow() -> Value {
        serde_json::json!({
            "name": "Valid KRIA Workflow",
            "nodes": [
                {
                    "id": "webhook-node",
                    "name": "Webhook",
                    "type": "n8n-nodes-base.webhook",
                    "parameters": { "path": "kria-valid" }
                },
                {
                    "id": "code-node",
                    "name": "Build Callback",
                    "type": "n8n-nodes-base.code",
                    "parameters": {
                        "jsCode": "const callback_body = { correlation_id, event_id, sequence_number, workflow_id, workflow_version, n8n_run_id, status, occurred_at_ms }; const callback_signature = sign(callback_body);"
                    }
                },
                {
                    "id": "callback-node",
                    "name": "Send Callback to KRIA",
                    "type": "n8n-nodes-base.httpRequest",
                    "parameters": {
                        "url": "http://host.docker.internal:3001/api/n8n/callback",
                        "jsonBody": "={{ $json.callback_body }}",
                        "headerParameters": {
                            "parameters": [
                                { "name": "x-kria-signature", "value": "={{ $json.callback_signature }}" }
                            ]
                        }
                    }
                }
            ],
            "connections": {
                "Webhook": {
                    "main": [[{ "node": "Build Callback", "type": "main", "index": 0 }]]
                },
                "Build Callback": {
                    "main": [[{ "node": "Send Callback to KRIA", "type": "main", "index": 0 }]]
                }
            }
        })
    }

    #[test]
    fn n8n_workflow_validation_accepts_valid_callback_workflow() {
        let report = validate_n8n_workflow_json(&valid_workflow(), Default::default());
        let summary = workflow_validation_summary(&report);

        assert_eq!(report.status, N8nWorkflowValidationReportStatus::Passed);
        assert!(report.safe_to_import);
        assert!(!report.safe_to_activate);
        assert_eq!(
            summary.get("callback_contract"),
            Some(&N8nWorkflowValidationCheckStatus::Passed)
        );
    }

    #[test]
    fn n8n_workflow_validation_rejects_invalid_json() {
        let report = validate_n8n_workflow_json_str("{bad", Default::default());
        assert_eq!(report.status, N8nWorkflowValidationReportStatus::Failed);
        assert!(!report.safe_to_import);
        assert!(report.checks.iter().any(|check| check.id == "json_parse"));
    }

    #[test]
    fn n8n_workflow_validation_rejects_duplicate_nodes() {
        let mut workflow = valid_workflow();
        workflow["nodes"][1]["id"] = Value::String("webhook-node".into());

        let report = validate_n8n_workflow_json(&workflow, Default::default());

        assert!(report
            .failed_checks()
            .iter()
            .any(|check| check.id == "unique_nodes"));
    }

    #[test]
    fn n8n_workflow_validation_rejects_broken_connection_target() {
        let mut workflow = valid_workflow();
        workflow["connections"]["Webhook"]["main"][0][0]["node"] = Value::String("Missing".into());

        let report = validate_n8n_workflow_json(&workflow, Default::default());

        assert!(report
            .failed_checks()
            .iter()
            .any(|check| check.id == "graph_integrity"));
    }

    #[test]
    fn n8n_workflow_validation_rejects_missing_callback_contract_field() {
        let mut workflow = valid_workflow();
        workflow["nodes"][1]["parameters"]["jsCode"] = Value::String(
            "const callback_body = { event_id, sequence_number, workflow_id, workflow_version, n8n_run_id, status, occurred_at_ms }; const callback_signature = sign(callback_body);".into(),
        );

        let report = validate_n8n_workflow_json(&workflow, Default::default());

        assert!(report
            .failed_checks()
            .iter()
            .any(|check| check.id == "callback_contract"));
    }

    #[test]
    fn n8n_workflow_validation_rejects_hardcoded_secret_literals() {
        let mut workflow = valid_workflow();
        workflow["nodes"][1]["parameters"]["apiKey"] =
            Value::String("abcdefghijklmnopqrstuvwxyz123456".into());

        let report = validate_n8n_workflow_json(&workflow, Default::default());

        assert!(report
            .failed_checks()
            .iter()
            .any(|check| check.id == "secret_leak"));
    }

    #[test]
    fn n8n_workflow_validation_infers_webhook_endpoint() {
        assert_eq!(
            infer_webhook_endpoint_path(&valid_workflow()).as_deref(),
            Some("/webhook/kria-valid")
        );
    }

    #[test]
    fn n8n_workflow_validation_infers_form_and_chat_endpoints() {
        let form = serde_json::json!({
            "nodes": [{
                "name": "Form Trigger",
                "type": "n8n-nodes-base.formTrigger",
                "typeVersion": 2.5,
                "webhookId": "kria-form-id",
                "parameters": {}
            }]
        });
        let chat = serde_json::json!({
            "nodes": [{
                "name": "Chat Trigger",
                "type": "@n8n/n8n-nodes-langchain.chatTrigger",
                "typeVersion": 1.4,
                "webhookId": "kria-chat-id",
                "parameters": {"public": true, "mode": "webhook"}
            }]
        });

        assert_eq!(
            infer_webhook_endpoint_path(&form).as_deref(),
            Some("/form/kria-form-id")
        );
        assert_eq!(
            infer_webhook_endpoint_path(&chat).as_deref(),
            Some("/webhook/kria-chat-id/chat")
        );
    }
}
