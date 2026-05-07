use super::*;

// ── MCP Runtime Helpers ──────────────────────────────────────────────

pub(super) fn mcp_state_name(state: McpServerState) -> &'static str {
    match state {
        McpServerState::Stopped => "stopped",
        McpServerState::Starting => "starting",
        McpServerState::Running => "running",
        McpServerState::Error => "error",
    }
}

pub(super) fn mcp_status_to_json(status: &McpServerStatus) -> serde_json::Value {
    serde_json::json!({
        "name": status.name.clone(),
        "command": status.command.clone(),
        "enabled": status.enabled,
        "state": mcp_state_name(status.state),
        "tool_count": status.tool_count,
        "error": status.error.clone(),
    })
}

pub(super) async fn sync_google_workspace_client_ref(
    state: &AppState,
    gw_client: Option<Arc<kria_core::mcp::McpClient>>,
) {
    if let Some(client) = gw_client {
        gw::set_client(&state.gw_client_ref, client).await;
    } else {
        *state.gw_client_ref.write().await = None;
    }
}

pub(super) async fn sync_colab_runtime_snapshot(state: &AppState, statuses: &[McpServerStatus]) {
    let colab_cfg = { state.config.read().await.colab.clone() };
    let mut runtime = state.colab_runtime.write().await;
    runtime.sidecar_server_name = colab_cfg.mcp_server_name.clone();

    if !colab_cfg.enabled {
        runtime.state = ColabRuntimeState::Disconnected;
        runtime.selected_notebook = None;
        runtime.last_error = None;
        return;
    }

    match statuses
        .iter()
        .find(|s| s.name == runtime.sidecar_server_name)
    {
        Some(status) if status.state == McpServerState::Running => {
            let category = format!("mcp_{}", runtime.sidecar_server_name);
            let category_tools = state.tool_registry.list_by_category(&category);
            let bootstrap_only = status.tool_count == 1
                && category_tools.len() == 1
                && category_tools
                    .first()
                    .map(|tool| is_colab_bootstrap_tool_name(&tool.name))
                    .unwrap_or(false);

            let has_notebook = runtime
                .selected_notebook
                .as_ref()
                .map(|value| !value.trim().is_empty())
                .unwrap_or(false);

            let next_state = if status.tool_count == 0 {
                runtime.selected_notebook = None;
                ColabRuntimeState::AwaitingBrowserConnection
            } else if bootstrap_only {
                ColabRuntimeState::AwaitingBrowserConnection
            } else if has_notebook {
                ColabRuntimeState::Ready
            } else {
                ColabRuntimeState::NotebookSelectionRequired
            };

            runtime.state = next_state;
            if matches!(
                next_state,
                ColabRuntimeState::Ready | ColabRuntimeState::NotebookSelectionRequired
            ) {
                runtime.last_error = None;
            }
        }
        Some(status) => {
            runtime.state = ColabRuntimeState::Degraded;
            runtime.last_error = status.error.clone().or_else(|| {
                Some(format!(
                    "MCP server '{}' is {}",
                    runtime.sidecar_server_name,
                    mcp_state_name(status.state)
                ))
            });
        }
        None => {
            runtime.state = ColabRuntimeState::Degraded;
            runtime.last_error = Some(format!(
                "MCP server '{}' not found in runtime status",
                runtime.sidecar_server_name
            ));
        }
    }
}

pub(super) async fn update_mcp_health_status(state: &AppState, statuses: &[McpServerStatus]) {
    let total = statuses.len();
    let running = statuses
        .iter()
        .filter(|s| s.state == McpServerState::Running)
        .count();
    let total_tools: usize = statuses.iter().map(|s| s.tool_count).sum();

    let unhealthy_enabled: Vec<&str> = statuses
        .iter()
        .filter(|s| s.enabled && s.state != McpServerState::Running)
        .map(|s| s.name.as_str())
        .collect();

    let (service, detail) = if total == 0 {
        (
            ServiceStatus::Healthy,
            "no MCP servers configured".to_string(),
        )
    } else if unhealthy_enabled.is_empty() {
        (
            ServiceStatus::Healthy,
            format!("{running}/{total} servers running, {total_tools} tools"),
        )
    } else {
        (
            ServiceStatus::Degraded,
            format!(
                "{running}/{total} servers running, {total_tools} tools; degraded: {}",
                unhealthy_enabled.join(", ")
            ),
        )
    };

    state.health.update("mcp_servers", service, Some(detail));
}

pub(super) async fn apply_mcp_runtime_from_config(state: &AppState) -> serde_json::Value {
    let desired = { state.config.read().await.mcp.servers.clone() };

    let mut manager = state.mcp_manager.lock().await;
    let report = manager.reconcile(desired, &state.tool_registry).await;
    let statuses = manager.status().await;
    let gw_client = manager.get_client("gworkspace").cloned();
    drop(manager);

    sync_google_workspace_client_ref(state, gw_client).await;
    sync_colab_runtime_snapshot(state, &statuses).await;
    update_mcp_health_status(state, &statuses).await;

    let status_json: Vec<serde_json::Value> = statuses.iter().map(mcp_status_to_json).collect();
    serde_json::json!({
        "report": report,
        "servers": status_json,
    })
}

// ── MCP Server Management Commands ──────────────────────────────────

#[tauri::command]
pub async fn list_mcp_servers(state: State<'_, AppStateCell>) -> Result<serde_json::Value, String> {
    let state = state
        .get()
        .ok_or_else(|| "KRIA is still initializing — please try again in a moment".to_string())?;
    let configured_servers = { state.config.read().await.mcp.servers.clone() };
    let runtime_statuses = {
        let manager = state.mcp_manager.lock().await;
        manager.status().await
    };

    let runtime_by_name: std::collections::HashMap<String, McpServerStatus> = runtime_statuses
        .into_iter()
        .map(|s| (s.name.clone(), s))
        .collect();

    let servers: Vec<serde_json::Value> = configured_servers
        .iter()
        .map(|s| {
            let runtime = runtime_by_name.get(&s.name);
            serde_json::json!({
                "name": s.name.clone(),
                "command": s.command.clone(),
                "args": s.args.clone(),
                "enabled": s.enabled,
                "trust_level": s.trust_level.clone(),
                "runtime_state": runtime.map(|r| mcp_state_name(r.state)).unwrap_or("stopped"),
                "runtime_tool_count": runtime.map(|r| r.tool_count).unwrap_or(0),
                "runtime_error": runtime.and_then(|r| r.error.clone()),
            })
        })
        .collect();
    Ok(serde_json::json!(servers))
}

#[tauri::command]
pub async fn reconcile_mcp_runtime(
    app: AppHandle,
    state: State<'_, AppStateCell>,
) -> Result<serde_json::Value, String> {
    let state = state
        .get()
        .ok_or_else(|| "KRIA is still initializing — please try again in a moment".to_string())?;
    let report = apply_mcp_runtime_from_config(state).await;
    emit_colab_status_event(&app, state).await;
    Ok(report)
}

#[tauri::command]
pub async fn restart_mcp_server_runtime(
    name: String,
    app: AppHandle,
    state: State<'_, AppStateCell>,
) -> Result<serde_json::Value, String> {
    let state = state
        .get()
        .ok_or_else(|| "KRIA is still initializing — please try again in a moment".to_string())?;

    let mut manager = state.mcp_manager.lock().await;
    manager
        .restart_server(&name, &state.tool_registry)
        .await
        .map_err(|e| e.to_string())?;
    let statuses = manager.status().await;
    let gw_client = manager.get_client("gworkspace").cloned();
    drop(manager);

    sync_google_workspace_client_ref(state, gw_client).await;
    sync_colab_runtime_snapshot(state, &statuses).await;
    update_mcp_health_status(state, &statuses).await;
    emit_colab_status_event(&app, state).await;

    let servers: Vec<serde_json::Value> = statuses.iter().map(mcp_status_to_json).collect();
    Ok(serde_json::json!({
        "status": "restarted",
        "name": name,
        "servers": servers,
    }))
}

#[tauri::command]
pub async fn add_mcp_server(
    name: String,
    command: String,
    args: Vec<String>,
    trust_level: Option<String>,
    state: State<'_, AppStateCell>,
) -> Result<(), String> {
    let state = state
        .get()
        .ok_or_else(|| "KRIA is still initializing — please try again in a moment".to_string())?;
    use kria_core::config::McpServerConfig;

    let server = McpServerConfig {
        name: name.clone(),
        command,
        args,
        env: std::collections::HashMap::new(),
        enabled: true,
        trust_level: trust_level.unwrap_or_else(|| "YELLOW".into()),
        tool_overrides: std::collections::HashMap::new(),
    };

    let mut config = state.config.write().await;
    // Prevent duplicate names
    if config.mcp.servers.iter().any(|s| s.name == name) {
        return Err(format!("MCP server '{}' already configured", name));
    }
    config.mcp.servers.push(server);
    config.save().map_err(|e| e.to_string())?;

    drop(config);
    let _ = apply_mcp_runtime_from_config(state).await;

    Ok(())
}

#[tauri::command]
pub async fn remove_mcp_server(name: String, state: State<'_, AppStateCell>) -> Result<(), String> {
    let state = state
        .get()
        .ok_or_else(|| "KRIA is still initializing — please try again in a moment".to_string())?;
    let mut config = state.config.write().await;
    let before = config.mcp.servers.len();
    config.mcp.servers.retain(|s| s.name != name);
    if config.mcp.servers.len() == before {
        return Err(format!("MCP server '{}' not found", name));
    }
    config.save().map_err(|e| e.to_string())?;

    drop(config);
    let _ = apply_mcp_runtime_from_config(state).await;

    Ok(())
}

#[tauri::command]
pub async fn toggle_mcp_server(
    name: String,
    enabled: bool,
    state: State<'_, AppStateCell>,
) -> Result<(), String> {
    let state = state
        .get()
        .ok_or_else(|| "KRIA is still initializing — please try again in a moment".to_string())?;
    let mut config = state.config.write().await;
    if let Some(server) = config.mcp.servers.iter_mut().find(|s| s.name == name) {
        server.enabled = enabled;
        if name.eq_ignore_ascii_case("telegram") {
            config.telegram.enabled = enabled;
            sync_telegram_mcp_server_config(&mut config);
        }
        config.save().map_err(|e| e.to_string())?;

        drop(config);
        let _ = apply_mcp_runtime_from_config(state).await;

        Ok(())
    } else {
        Err(format!("MCP server '{}' not found", name))
    }
}
