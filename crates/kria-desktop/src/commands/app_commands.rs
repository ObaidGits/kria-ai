use super::*;

#[tauri::command]
pub async fn cancel_request(state: State<'_, AppStateCell>) -> Result<(), String> {
    let state = state
        .get()
        .ok_or_else(|| "KRIA is still initializing — please try again in a moment".to_string())?;
    state.hitl.cancel_all().await;
    Ok(())
}

#[tauri::command]
pub async fn cancel_turn(session_id: String, state: State<'_, AppStateCell>) -> Result<(), String> {
    let state = state
        .get()
        .ok_or_else(|| "KRIA is still initializing — please try again in a moment".to_string())?;
    state.agent_loop.cancel_session(&session_id);
    Ok(())
}

#[tauri::command]
pub async fn cancel_executive_task(
    task_id: String,
    state: State<'_, AppStateCell>,
) -> Result<(), String> {
    let state = state
        .get()
        .ok_or_else(|| "KRIA is still initializing — please try again in a moment".to_string())?;
    let parsed_id = uuid::Uuid::parse_str(&task_id).map_err(|e| format!("Invalid task ID: {e}"))?;

    // Try the executive sender first (when executive.enabled = true)
    {
        let config = state.config.read().await;
        if config.executive.enabled {
            drop(config);
            // The executive sender is stored in the runtime init, not AppState.
            // Fall through to agent_loop cancel as a reliable fallback.
        }
    }

    // Fallback: cancel via the agent loop's turn admission (works for all modes)
    state.agent_loop.cancel_session(&parsed_id.to_string());
    Ok(())
}

/// Submit explicit routing feedback from the UI ("Wrong tool" / "Try differently").
///
/// `outcome_type` must be one of:
/// - `"wrong_tool"` — maps to `RoutingOutcome::Corrected`
/// - `"try_differently"` — maps to `RoutingOutcome::Rephrased`
/// - `"wrong_domain:<DomainName>"` — maps to `RoutingOutcome::Corrected` with a named domain
#[tauri::command]
pub async fn submit_turn_feedback(
    session_id: String,
    user_text: String,
    tool_selected: Option<String>,
    outcome_type: String,
    state: State<'_, AppStateCell>,
) -> Result<serde_json::Value, String> {
    use kria_core::routing::domain::Domain;
    use kria_core::routing::feedback::RoutingOutcome;

    let state = state
        .get()
        .ok_or_else(|| "KRIA is still initializing — please try again in a moment".to_string())?;

    let config = state.config.read().await;
    let learning_rate = config.routing.feedback_learning_rate;
    drop(config);

    // Map outcome_type string to RoutingOutcome
    let outcome = if outcome_type == "wrong_tool" || outcome_type.starts_with("wrong_domain:") {
        let correct_domain = if let Some(d) = outcome_type.strip_prefix("wrong_domain:") {
            match d.to_lowercase().as_str() {
                "systeminfo" | "system" => Domain::SystemInfo,
                "knowledge" => Domain::Knowledge,
                "fileops" | "file" => Domain::FileOps,
                "applifecycle" | "app" => Domain::AppLifecycle,
                "comms" | "communication" => Domain::Comms,
                "workspace" => Domain::Workspace,
                "power" => Domain::Power,
                "vision" => Domain::Vision,
                "packages" => Domain::Packages,
                "developer" | "dev" => Domain::Developer,
                "planner" => Domain::Planner,
                _ => Domain::Conversation,
            }
        } else {
            Domain::Conversation
        };
        RoutingOutcome::Corrected {
            correct_domain,
            correct_tool: None,
        }
    } else {
        // "try_differently" → Rephrased (weak negative signal)
        RoutingOutcome::Rephrased
    };

    // Use the tool name to hint the domain that was originally selected
    let domain = tool_selected
        .as_deref()
        .map(|t| {
            let cat = t.split('_').next().unwrap_or("conversation").to_lowercase();
            kria_core::routing::domain::category_to_domain(&cat)
        })
        .unwrap_or(Domain::Conversation);

    let nudged = state
        .agent_loop
        .submit_routing_feedback(
            &user_text,
            domain,
            outcome,
            tool_selected,
            &session_id,
            learning_rate,
        )
        .await;

    tracing::info!(
        session_id = %session_id,
        outcome_type = %outcome_type,
        nudged,
        "User submitted routing feedback"
    );

    Ok(serde_json::json!({
        "status": "ok",
        "nudged": nudged,
    }))
}

#[tauri::command]
pub async fn approve_action(
    request_id: String,
    state: State<'_, AppStateCell>,
) -> Result<(), String> {
    let state = state
        .get()
        .ok_or_else(|| "KRIA is still initializing — please try again in a moment".to_string())?;
    state
        .hitl
        .respond(&request_id, ApprovalResponse::Approved)
        .await;
    Ok(())
}

#[tauri::command]
pub async fn deny_action(
    request_id: String,
    _reason: Option<String>,
    state: State<'_, AppStateCell>,
) -> Result<(), String> {
    let state = state
        .get()
        .ok_or_else(|| "KRIA is still initializing — please try again in a moment".to_string())?;
    state
        .hitl
        .respond(&request_id, ApprovalResponse::Denied)
        .await;
    Ok(())
}

#[tauri::command]
pub async fn get_health(state: State<'_, AppStateCell>) -> Result<serde_json::Value, String> {
    // If AppState is not yet initialized, return a "starting" payload so the
    // UI can show "Warming up" instead of staying stuck on "Booting".
    let Some(state) = state.get() else {
        return Ok(serde_json::json!({
            "status": "starting",
            "uptime_secs": 0,
            "tool_count": 0,
            "services": [
                {"name": "runtime", "status": "starting", "message": "KRIA is initializing…"}
            ],
            "hardware": {}
        }));
    };
    // Refresh LLM server health on each call
    let mr_status = state.model_router.status().await;
    let mr_healthy = mr_status["local_healthy"].as_bool().unwrap_or(false);
    let mr_model = mr_status["local_model"].as_str().unwrap_or("unknown");
    if mr_healthy {
        state.health.update(
            "model_router",
            ServiceStatus::Healthy,
            Some(format!("model: {}", mr_model)),
        );
    } else {
        state.health.update(
            "model_router",
            ServiceStatus::Degraded,
            Some("LLM server not reachable".into()),
        );
    }

    // Refresh OCR dependency status from sidecar so UI can warn users before first upload.
    {
        let health = state.health.clone();
        let sidecar = state.sidecar.clone();
        tokio::spawn(async move {
            refresh_ocr_dependency_health(&health, &sidecar).await;
        });
    }

    let services = state.health.status_all();
    let all_healthy = state.health.all_healthy();
    let uptime = state.started_at.elapsed().as_secs();
    let tool_count = state.tool_registry.len();
    let hw = &state.hardware_info;

    Ok(serde_json::json!({
        "status": if all_healthy { "healthy" } else { "degraded" },
        "uptime_secs": uptime,
        "tool_count": tool_count,
        "services": services,
        "hardware": {
            "tier": hw.tier.as_str(),
            "cpu_cores": hw.cpu_cores,
            "total_ram_mb": hw.total_ram_mb,
            "vram_mb": hw.vram_mb,
            "gpu_name": hw.gpu_name,
            "os": format!("{:?}", hw.os),
            "hostname": hw.hostname,
        }
    }))
}

#[tauri::command]
pub async fn get_hardware_info(
    state: State<'_, AppStateCell>,
) -> Result<serde_json::Value, String> {
    let state = state
        .get()
        .ok_or_else(|| "KRIA is still initializing — please try again in a moment".to_string())?;
    let hw = &state.hardware_info;
    Ok(serde_json::json!({
        "tier": hw.tier.as_str(),
        "cpu_cores": hw.cpu_cores,
        "total_ram_mb": hw.total_ram_mb,
        "vram_mb": hw.vram_mb,
        "gpu_name": hw.gpu_name,
        "os": format!("{:?}", hw.os),
        "hostname": hw.hostname,
        "package_manager": hw.package_manager.map(|pm| format!("{:?}", pm)),
        "vision_capable": hw.tier.has_vision(),
        "recommended_model": hw.tier.recommended_model(),
        "recommended_stt": hw.tier.stt_model(),
        "context_window": hw.tier.context_window(),
        "gpu_layers": hw.tier.gpu_layers(),
        "threads": hw.tier.thread_count(),
    }))
}

#[tauri::command]
pub async fn get_settings(state: State<'_, AppStateCell>) -> Result<serde_json::Value, String> {
    let state = state
        .get()
        .ok_or_else(|| "KRIA is still initializing — please try again in a moment".to_string())?;
    let config = state.config.read().await;
    serde_json::to_value(&*config).map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn list_audio_devices() -> Result<serde_json::Value, String> {
    let inputs = list_input_devices().unwrap_or_default();
    let outputs = list_output_devices().unwrap_or_default();
    Ok(serde_json::json!({
        "inputs": inputs,
        "outputs": outputs,
        "default_input": default_input_device_name(),
        "default_output": default_output_device_name(),
    }))
}

#[tauri::command]
pub async fn update_settings(
    settings: serde_json::Value,
    state: State<'_, AppStateCell>,
) -> Result<(), String> {
    let state = state
        .get()
        .ok_or_else(|| "KRIA is still initializing — please try again in a moment".to_string())?;
    let mut new_config: KriaConfig = serde_json::from_value(settings).map_err(|e| e.to_string())?;
    sync_telegram_mcp_server_config(&mut new_config);
    sync_google_workspace_server_config(&mut new_config, None);
    apply_google_runtime_env_from_config(&new_config);
    // Persist to disk first
    new_config.save().map_err(|e| e.to_string())?;
    // Then update in-memory config
    let mut config = state.config.write().await;
    *config = new_config;

    drop(config);
    let _ = apply_mcp_runtime_from_config(state).await;

    Ok(())
}

#[tauri::command]
pub async fn list_knowledge_base(
    state: State<'_, AppStateCell>,
) -> Result<serde_json::Value, String> {
    let state = state
        .get()
        .ok_or_else(|| "KRIA is still initializing — please try again in a moment".to_string())?;
    let docs = state
        .memory_store
        .list_documents()
        .map_err(|e| e.to_string())?;
    let items: Vec<serde_json::Value> = docs
        .iter()
        .map(|(id, name, dtype, chunks)| {
            serde_json::json!({
                "doc_id": id,
                "name": name,
                "type": dtype,
                "chunks": chunks,
            })
        })
        .collect();
    Ok(serde_json::json!({ "documents": items, "count": items.len() }))
}

#[tauri::command]
pub async fn get_alerts(state: State<'_, AppStateCell>) -> Result<serde_json::Value, String> {
    let state = state
        .get()
        .ok_or_else(|| "KRIA is still initializing — please try again in a moment".to_string())?;
    let alerts = state.proactive.get_alerts().await;
    let items: Vec<serde_json::Value> = alerts
        .iter()
        .map(|a| {
            serde_json::json!({
                "id": a.id,
                "category": format!("{:?}", a.category).to_lowercase(),
                "title": a.title,
                "message": a.message,
                "suggestion": a.suggestion,
                "timestamp": a.timestamp.to_rfc3339(),
            })
        })
        .collect();
    Ok(serde_json::json!({ "alerts": items, "count": items.len() }))
}

/// Write arbitrary text content to a file chosen by the user via a save dialog.
/// Returns the absolute path of the saved file, or null if cancelled.
#[tauri::command]
pub async fn list_models(state: State<'_, AppStateCell>) -> Result<serde_json::Value, String> {
    let state = state
        .get()
        .ok_or_else(|| "KRIA is still initializing — please try again in a moment".to_string())?;
    let config = state.config.read().await;
    let paths = config.resolve_paths().map_err(|e| e.to_string())?;
    let mgr = kria_core::llm::model_manager::ModelManager::new(paths.models_dir.join("llm"));
    let models = mgr.list_llm_models();
    Ok(serde_json::to_value(&models).unwrap_or_default())
}
