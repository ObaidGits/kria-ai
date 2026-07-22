use super::*;

// ── Colab Cloud Tier Commands ───────────────────────────────────────────────

pub(super) fn migrate_legacy_colab_server_command(
    server: &mut kria_core::config::McpServerConfig,
) -> bool {
    if server.command == COLAB_LEGACY_NPX_COMMAND {
        if !server
            .args
            .iter()
            .any(|arg| arg == COLAB_LEGACY_NPX_PACKAGE)
        {
            return false;
        }

        server.command = COLAB_OFFICIAL_COMMAND.to_string();
        server.args = vec![
            "--from".to_string(),
            COLAB_OFFICIAL_SOURCE.to_string(),
            COLAB_OFFICIAL_ENTRYPOINT.to_string(),
        ];
        return true;
    }

    if server.command == COLAB_OFFICIAL_COMMAND
        && server.args.len() == 1
        && server.args[0] == COLAB_OFFICIAL_SOURCE
    {
        server.args = vec![
            "--from".to_string(),
            COLAB_OFFICIAL_SOURCE.to_string(),
            COLAB_OFFICIAL_ENTRYPOINT.to_string(),
        ];
        return true;
    }

    if server.command == COLAB_OFFICIAL_COMMAND
        && server.args.len() >= 3
        && server.args[0] == "--from"
        && server.args[1] == COLAB_OFFICIAL_SOURCE
        && server
            .args
            .iter()
            .any(|arg| arg == COLAB_OFFICIAL_ENTRYPOINT)
    {
        return false;
    }

    false
}

fn default_colab_server_config() -> kria_core::config::McpServerConfig {
    kria_core::config::McpServerConfig {
        name: COLAB_DEFAULT_SERVER_NAME.to_string(),
        command: COLAB_OFFICIAL_COMMAND.to_string(),
        args: vec![
            "--from".to_string(),
            COLAB_OFFICIAL_SOURCE.to_string(),
            COLAB_OFFICIAL_ENTRYPOINT.to_string(),
        ],
        env: std::collections::HashMap::new(),
        enabled: true,
        trust_level: "YELLOW".into(),
        tool_overrides: std::collections::HashMap::new(),
    }
}

pub(super) fn build_colab_tier_status_payload(
    config: &ColabConfig,
    runtime: &ColabRuntimeSnapshot,
    mcp_runtime: Option<&McpServerStatus>,
    capability_summary: &serde_json::Value,
    additional_warnings: &[String],
) -> serde_json::Value {
    let (mcp_state, mcp_tool_count, mcp_error, mcp_running) = match mcp_runtime {
        Some(status) => (
            mcp_state_name(status.state).to_string(),
            status.tool_count,
            status.error.clone(),
            status.state == McpServerState::Running,
        ),
        None => ("not_configured".to_string(), 0usize, None, false),
    };

    let browser_connected = matches!(
        runtime.state,
        ColabRuntimeState::NotebookSelectionRequired | ColabRuntimeState::Ready
    );
    let connected = config.enabled && mcp_running && browser_connected;

    let selected_notebook = runtime
        .selected_notebook
        .as_ref()
        .map(|value| !value.trim().is_empty())
        .unwrap_or(false);

    let mut capability_missing: Vec<String> = capability_summary
        .get("ready_requirements")
        .and_then(|v| v.get("missing"))
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str())
                .map(|s| s.to_string())
                .collect()
        })
        .unwrap_or_default();

    if selected_notebook {
        capability_missing.retain(|item| item != "notebook_selection_or_discovery");
    }

    let capability_ready = capability_missing.is_empty();
    let ready_for_cloud_task =
        connected && runtime.state == ColabRuntimeState::Ready && capability_ready;
    let notebook_selection_required =
        connected && runtime.state == ColabRuntimeState::NotebookSelectionRequired;

    let discovered_tool_count = capability_summary
        .get("tool_count")
        .and_then(|v| v.as_u64())
        .unwrap_or(0) as usize;

    let bootstrap_only = capability_summary
        .get("discovered_tools")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.len() == 1
                && arr.iter().any(|entry| {
                    entry
                        .get("operation")
                        .and_then(|v| v.as_str())
                        .map(is_colab_bootstrap_tool_name)
                        .unwrap_or(false)
                        || entry
                            .get("name")
                            .and_then(|v| v.as_str())
                            .map(is_colab_bootstrap_tool_name)
                            .unwrap_or(false)
                })
        })
        .unwrap_or(false);

    let mut warnings: Vec<String> = Vec::new();
    if !config.enabled {
        warnings.push("Colab tier is disabled in config".into());
    }
    if !mcp_running {
        warnings.push(format!(
            "Colab MCP runtime is not running (state={})",
            mcp_state
        ));
    }
    if runtime.state == ColabRuntimeState::AwaitingBrowserConnection {
        warnings.push("Awaiting browser connection to Colab session".into());
    }
    if mcp_running && bootstrap_only {
        warnings.push(
            "Colab MCP is exposing only bootstrap tooling. Use Connect Colab to open browser session and unlock notebook tools".into(),
        );
    }
    if mcp_running && discovered_tool_count == 0 {
        warnings.push(format!(
            "Colab MCP server '{}' is running but no tools were discovered",
            runtime.sidecar_server_name
        ));
    }
    if connected && !capability_ready && !capability_missing.is_empty() {
        warnings.push(format!(
            "Colab capability requirements are not satisfied: {}",
            capability_missing.join(", ")
        ));
    }
    if notebook_selection_required {
        warnings.push("Notebook must be selected before executing cloud tasks".into());
    }
    if let Some(err) = runtime.last_error.as_ref() {
        warnings.push(format!("Last runtime error: {err}"));
    }
    warnings.extend(additional_warnings.iter().cloned());

    serde_json::json!({
        "enabled": config.enabled,
        "connected": connected,
        "ready_for_cloud_task": ready_for_cloud_task,
        "notebook_selection_required": notebook_selection_required,
        "runtime_state": runtime.state.as_str(),
        "selected_notebook": runtime.selected_notebook,
        "mcp_server_name": runtime.sidecar_server_name,
        "auto_escalate": config.auto_escalate,
        "fallback_to_local": config.fallback_to_local,
        "connect_timeout_secs": config.connect_timeout_secs,
        "keepalive_interval_secs": config.keepalive_interval_secs,
        "checkpoint_interval_secs": config.checkpoint_interval_secs,
        "mcp": {
            "state": mcp_state,
            "tool_count": mcp_tool_count,
            "error": mcp_error,
        },
        "capabilities": capability_summary.clone(),
        "warnings": warnings,
    })
}

pub(super) async fn maybe_bootstrap_colab_browser_connection(state: &AppState, server_name: &str) {
    let client = {
        let manager = state.mcp_manager.lock().await;
        manager.get_client(server_name).cloned()
    };

    let Some(client) = client else {
        return;
    };

    let tools = client.tools().await;
    let has_bootstrap_tool = tools
        .iter()
        .any(|tool| is_colab_bootstrap_tool_name(&tool.name));

    if !has_bootstrap_tool {
        return;
    }

    match client.call_tool(COLAB_BROWSER_BOOTSTRAP_TOOL, None).await {
        Ok(result) => {
            let connected = result.content.iter().any(|content| {
                content
                    .text
                    .as_ref()
                    .map(|text| {
                        let normalized = text.trim().to_ascii_lowercase();
                        normalized == "true"
                            || normalized.contains("\"result\": true")
                            || normalized.contains("connected")
                    })
                    .unwrap_or(false)
            });

            tracing::info!(
                server = %server_name,
                connected,
                "invoked Colab browser bootstrap MCP tool"
            );

            if connected {
                let mut runtime = state.colab_runtime.write().await;
                runtime.last_error = None;
            }

            let mut manager = state.mcp_manager.lock().await;
            if let Err(err) = manager
                .refresh_server_tools(server_name, &state.tool_registry)
                .await
            {
                tracing::warn!(
                    server = %server_name,
                    error = %err,
                    "colab MCP tool refresh after bootstrap failed"
                );
                let mut runtime = state.colab_runtime.write().await;
                runtime.last_error = Some(format!(
                    "Colab MCP tool refresh after bootstrap failed: {err}"
                ));
            }
        }
        Err(err) => {
            tracing::warn!(
                server = %server_name,
                error = %err,
                "colab browser bootstrap tool invocation failed"
            );
            let mut runtime = state.colab_runtime.write().await;
            runtime.last_error = Some(format!("Colab browser bootstrap failed: {err}"));
        }
    }
}

pub(super) async fn collect_colab_tier_status(state: &AppState) -> serde_json::Value {
    let colab_config = {
        let config = state.config.read().await;
        config.colab.clone()
    };

    let colab_server_name = {
        let runtime = state.colab_runtime.read().await;
        runtime.sidecar_server_name.clone()
    };

    let mut transient_warnings: Vec<String> = Vec::new();

    let statuses = {
        let mut manager = state.mcp_manager.lock().await;
        let mut statuses = manager.status().await;

        if colab_config.enabled {
            match statuses.iter().find(|s| s.name == colab_server_name) {
                Some(status) if status.state == McpServerState::Running => {
                    if let Err(err) = manager
                        .refresh_server_tools(&colab_server_name, &state.tool_registry)
                        .await
                    {
                        tracing::warn!(
                            server = %colab_server_name,
                            error = %err,
                            "colab MCP tool refresh failed"
                        );
                        transient_warnings.push(format!("Colab MCP tool refresh failed: {err}"));
                    }
                    statuses = manager.status().await;
                }
                Some(status) => {
                    transient_warnings.push(format!(
                        "Colab MCP runtime is not running (state={})",
                        mcp_state_name(status.state)
                    ));
                }
                None => {
                    transient_warnings.push(format!(
                        "Colab MCP runtime '{}' not found",
                        colab_server_name
                    ));
                }
            }
        }

        statuses
    };

    sync_colab_runtime_snapshot(state, &statuses).await;

    let runtime = state.colab_runtime.read().await.clone();
    let mcp_runtime = statuses
        .iter()
        .find(|s| s.name == runtime.sidecar_server_name);

    let capability_summary =
        build_colab_capability_summary(&state.tool_registry, &runtime.sidecar_server_name);

    build_colab_tier_status_payload(
        &colab_config,
        &runtime,
        mcp_runtime,
        &capability_summary,
        &transient_warnings,
    )
}

#[tauri::command]
pub async fn get_colab_tier_status(
    state: State<'_, AppStateCell>,
) -> Result<serde_json::Value, String> {
    let state = state
        .get()
        .ok_or_else(|| "KRIA is still initializing — please try again in a moment".to_string())?;

    Ok(collect_colab_tier_status(state).await)
}

#[tauri::command]
pub async fn connect_colab_tier(
    server_name: Option<String>,
    app: AppHandle,
    state: State<'_, AppStateCell>,
) -> Result<serde_json::Value, String> {
    let state = state
        .get()
        .ok_or_else(|| "KRIA is still initializing — please try again in a moment".to_string())?;

    let mut changed = false;
    let mut server_found = false;
    let resolved_server_name = {
        let mut config = state.config.read().await.clone();

        if let Some(name) = server_name
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            let next = name.to_string();
            if config.colab.mcp_server_name != next {
                config.colab.mcp_server_name = next;
                changed = true;
            }
        }

        if !config.colab.enabled {
            config.colab.enabled = true;
            changed = true;
        }

        let server_name = config.colab.mcp_server_name.clone();
        if let Some(server) = config
            .mcp
            .servers
            .iter_mut()
            .find(|s| s.name == server_name)
        {
            server_found = true;
            if migrate_legacy_colab_server_command(server) {
                changed = true;
            }
            if !server.enabled {
                server.enabled = true;
                changed = true;
            }
        } else if server_name == COLAB_DEFAULT_SERVER_NAME {
            config.mcp.servers.push(default_colab_server_config());
            server_found = true;
            changed = true;
        }

        if changed {
            state
                .config_service
                .patch_batch(
                    vec![
                        kria_core::config::Change::new(
                            "colab",
                            "enabled",
                            serde_json::json!(config.colab.enabled),
                        ),
                        kria_core::config::Change::new(
                            "colab",
                            "mcp_server_name",
                            serde_json::json!(config.colab.mcp_server_name),
                        ),
                        kria_core::config::Change::new(
                            "mcp",
                            "servers",
                            serde_json::json!(config.mcp.servers),
                        ),
                    ],
                    kria_core::config::ChangeSource::Ui,
                    None,
                )
                .await
                .map_err(|error| error.to_string())?;
        }

        server_name
    };

    {
        let mut runtime = state.colab_runtime.write().await;
        runtime.sidecar_server_name = resolved_server_name.clone();
        runtime.selected_notebook = None;
        runtime.state = ColabRuntimeState::SidecarStarting;
        runtime.last_error = if server_found {
            None
        } else {
            Some(format!(
                "Configured MCP server '{}' is missing from mcp.servers",
                resolved_server_name
            ))
        };
    }

    let runtime_report = apply_mcp_runtime_from_config(state).await;

    if server_found {
        maybe_bootstrap_colab_browser_connection(state, &resolved_server_name).await;
    }

    let colab_status = collect_colab_tier_status(state).await;
    let _ = app.emit("colab:status", colab_status.clone());

    Ok(serde_json::json!({
        "status": "connecting",
        "server_name": resolved_server_name,
        "server_found": server_found,
        "runtime": runtime_report,
        "colab": colab_status,
    }))
}

#[tauri::command]
pub async fn disconnect_colab_tier(
    app: AppHandle,
    state: State<'_, AppStateCell>,
) -> Result<serde_json::Value, String> {
    let state = state
        .get()
        .ok_or_else(|| "KRIA is still initializing — please try again in a moment".to_string())?;

    let mut changed = false;
    {
        let mut config = state.config.read().await.clone();
        if config.colab.enabled {
            config.colab.enabled = false;
            changed = true;
        }

        let target_server = config.colab.mcp_server_name.clone();
        if let Some(server) = config
            .mcp
            .servers
            .iter_mut()
            .find(|s| s.name == target_server)
        {
            if server.enabled {
                server.enabled = false;
                changed = true;
            }
        }

        if changed {
            state
                .config_service
                .patch_batch(
                    vec![
                        kria_core::config::Change::new(
                            "colab",
                            "enabled",
                            serde_json::json!(false),
                        ),
                        kria_core::config::Change::new(
                            "mcp",
                            "servers",
                            serde_json::json!(config.mcp.servers),
                        ),
                    ],
                    kria_core::config::ChangeSource::Ui,
                    None,
                )
                .await
                .map_err(|error| error.to_string())?;
        }
    }

    {
        let mut runtime = state.colab_runtime.write().await;
        runtime.state = ColabRuntimeState::Disconnected;
        runtime.selected_notebook = None;
        runtime.last_error = None;
    }

    let runtime_report = apply_mcp_runtime_from_config(state).await;

    let colab_status = collect_colab_tier_status(state).await;
    let _ = app.emit("colab:status", colab_status.clone());

    Ok(serde_json::json!({
        "status": "disconnected",
        "runtime": runtime_report,
        "colab": colab_status,
    }))
}

#[tauri::command]
pub async fn set_colab_selected_notebook(
    notebook_id: String,
    app: AppHandle,
    state: State<'_, AppStateCell>,
) -> Result<serde_json::Value, String> {
    let state = state
        .get()
        .ok_or_else(|| "KRIA is still initializing — please try again in a moment".to_string())?;

    let notebook_id = notebook_id.trim();
    if notebook_id.is_empty() {
        return Err("Notebook identifier cannot be empty".into());
    }

    {
        let mut runtime = state.colab_runtime.write().await;
        if runtime.state == ColabRuntimeState::Disconnected {
            return Err("Colab tier is disconnected. Connect it first.".into());
        }
        runtime.selected_notebook = Some(notebook_id.to_string());
        runtime.state = ColabRuntimeState::Ready;
        runtime.last_error = None;
    }

    let colab_status = collect_colab_tier_status(state).await;
    let _ = app.emit("colab:status", colab_status.clone());
    Ok(colab_status)
}
