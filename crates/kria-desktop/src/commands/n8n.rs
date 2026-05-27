use crate::commands::AppStateCell;
use kria_core::n8n::{N8nCatalog, N8nWorkflowConfig, N8nWorkflowEnvironment, N8nWorkflowStatus};
use tauri::State;

#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportN8nWorkflowRequest {
    pub workflow_id: String,
    #[serde(default = "default_workflow_version")]
    pub workflow_version: String,
    #[serde(default)]
    pub display_name: String,
    pub endpoint_path: String,
    #[serde(default)]
    pub expected_evidence: Vec<String>,
    #[serde(default)]
    pub data_scope: Vec<String>,
    #[serde(default)]
    pub allowed_actions: Vec<String>,
}

fn default_workflow_version() -> String {
    "v1".into()
}

#[tauri::command]
pub async fn get_n8n_status(state: State<'_, AppStateCell>) -> Result<serde_json::Value, String> {
    let app_state = state
        .get()
        .ok_or_else(|| "runtime still initializing".to_string())?;
    let config = app_state.config.read().await;
    let configured = config.n8n.workflows.clone();
    let callback_url = {
        let host = if config.server.host == "0.0.0.0" {
            "127.0.0.1".to_string()
        } else {
            config.server.host.clone()
        };
        format!("http://{host}:{}/api/n8n/callback", config.server.port)
    };
    let enabled = config.n8n.enabled;
    let base_url = config.n8n.base_url.clone();
    drop(config);

    let catalog_workflows = app_state
        .n8n_catalog
        .read()
        .await
        .as_ref()
        .map(|catalog| catalog.workflows())
        .unwrap_or_default();

    Ok(serde_json::json!({
        "enabled": enabled,
        "base_url": base_url,
        "callback_url": callback_url,
        "configured_workflows": configured,
        "catalog_workflows": catalog_workflows,
        "runs": app_state.n8n_state_store.runs(),
        "dead_letters": app_state.n8n_state_store.dead_letters(),
        "governance_log": app_state.n8n_governance_log.read().await.clone(),
        "hitl_responses": app_state.n8n_hitl_responses.read().await.clone(),
        "inbox_path": app_state.n8n_inbox_path,
        "audit_path": app_state.n8n_audit_path,
        "notes": [
            "KRIA owns orchestration authority; n8n callback evidence is not final completion authority.",
            "Imported workflows are saved as draft until explicitly approved in KRIA configuration."
        ],
    }))
}

#[tauri::command]
pub async fn reconcile_n8n_run(
    correlation_id: String,
    state: State<'_, AppStateCell>,
) -> Result<serde_json::Value, String> {
    let app_state = state
        .get()
        .ok_or_else(|| "runtime still initializing".to_string())?;
    let run = app_state
        .n8n_state_store
        .get(correlation_id.trim())
        .ok_or_else(|| format!("no n8n run state for correlation_id '{}'", correlation_id))?;

    let config = app_state.config.read().await;
    if !config.n8n.enabled {
        return Err("n8n integration is disabled".into());
    }
    let base_url = config.n8n.base_url.trim_end_matches('/').to_string();
    let api_key = config.n8n.api_key.clone();
    drop(config);

    if base_url.is_empty() {
        return Err("n8n base_url is empty".into());
    }
    if run.n8n_run_id.trim().is_empty() {
        return Err("n8n_run_id is empty for this run".into());
    }

    let url = format!("{base_url}/api/v1/executions/{}", run.n8n_run_id);
    let mut request = reqwest::Client::new().get(url);
    if !api_key.trim().is_empty() {
        request = request.header("X-N8N-API-KEY", api_key.trim());
    }

    let response = request
        .send()
        .await
        .map_err(|error| format!("failed to reconcile n8n run: {error}"))?;
    let status = response.status();
    let body = response
        .text()
        .await
        .map_err(|error| format!("failed to read n8n reconcile response: {error}"))?;
    if !status.is_success() {
        return Err(format!("n8n reconcile failed with {status}: {body}"));
    }

    let payload = serde_json::from_str::<serde_json::Value>(&body)
        .unwrap_or_else(|_| serde_json::json!({ "raw": body }));
    let decision = kria_core::n8n::evaluate_run(None, &run);
    {
        let mut log = app_state.n8n_governance_log.write().await;
        log.push(decision.clone());
        let overflow = log.len().saturating_sub(100);
        if overflow > 0 {
            log.drain(0..overflow);
        }
    }

    Ok(serde_json::json!({
        "status": "ok",
        "correlation_id": run.correlation_id,
        "n8n_run_id": run.n8n_run_id,
        "governance": decision,
        "n8n_execution": payload,
    }))
}

#[tauri::command]
pub async fn discover_n8n_workflows(
    state: State<'_, AppStateCell>,
) -> Result<serde_json::Value, String> {
    let app_state = state
        .get()
        .ok_or_else(|| "runtime still initializing".to_string())?;
    let config = app_state.config.read().await;
    if !config.n8n.enabled {
        return Err("n8n integration is disabled".into());
    }
    if config.n8n.base_url.trim().is_empty() {
        return Err("n8n base_url is empty".into());
    }

    let url = format!(
        "{}/api/v1/workflows",
        config.n8n.base_url.trim_end_matches('/')
    );
    let api_key = config.n8n.api_key.clone();
    drop(config);

    let mut request = reqwest::Client::new().get(url);
    if !api_key.trim().is_empty() {
        request = request.header("X-N8N-API-KEY", api_key.trim());
    }

    let response = request
        .send()
        .await
        .map_err(|error| format!("failed to discover n8n workflows: {error}"))?;
    let status = response.status();
    let body = response
        .text()
        .await
        .map_err(|error| format!("failed to read n8n discovery response: {error}"))?;
    if !status.is_success() {
        return Err(format!("n8n discovery failed with {status}: {body}"));
    }

    let parsed = serde_json::from_str::<serde_json::Value>(&body)
        .unwrap_or_else(|_| serde_json::json!({ "raw": body }));
    Ok(serde_json::json!({
        "status": "ok",
        "source": "n8n_api",
        "workflows": parsed,
    }))
}

#[tauri::command]
pub async fn import_n8n_workflow(
    request: ImportN8nWorkflowRequest,
    state: State<'_, AppStateCell>,
) -> Result<serde_json::Value, String> {
    let app_state = state
        .get()
        .ok_or_else(|| "runtime still initializing".to_string())?;
    let workflow_id = request.workflow_id.trim();
    let endpoint_path = request.endpoint_path.trim();
    if workflow_id.is_empty() {
        return Err("workflow_id is required".into());
    }
    if endpoint_path.is_empty() {
        return Err("endpoint_path is required".into());
    }

    let workflow = N8nWorkflowConfig {
        workflow_id: workflow_id.into(),
        workflow_version: request.workflow_version.trim().into(),
        display_name: if request.display_name.trim().is_empty() {
            workflow_id.into()
        } else {
            request.display_name.trim().into()
        },
        endpoint_path: endpoint_path.into(),
        status: N8nWorkflowStatus::Draft,
        environment: N8nWorkflowEnvironment::Dev,
        expected_evidence: request.expected_evidence,
        data_scope: request.data_scope,
        allowed_actions: request.allowed_actions,
        ..Default::default()
    };

    let mut config = app_state.config.write().await;
    if config
        .n8n
        .workflows
        .iter()
        .any(|existing| existing.workflow_id == workflow.workflow_id)
    {
        return Err(format!(
            "n8n workflow '{}' already exists in KRIA config",
            workflow.workflow_id
        ));
    }

    config.n8n.workflows.push(workflow.clone());
    config
        .save()
        .map_err(|error| format!("failed to save KRIA config: {error}"))?;

    let rebuilt = if config.n8n.enabled {
        N8nCatalog::new(config.n8n.clone())
            .ok()
            .map(std::sync::Arc::new)
    } else {
        None
    };
    drop(config);
    *app_state.n8n_catalog.write().await = rebuilt;

    Ok(serde_json::json!({
        "status": "imported_as_draft",
        "workflow": workflow,
        "next_step": "Review and approve the workflow in KRIA config before execution.",
    }))
}
