use super::*;

async fn persist_telegram_config(state: &AppState, config: &KriaConfig) -> Result<(), String> {
    let fields = serde_json::to_value(&config.telegram)
        .map_err(|error| format!("failed to serialize Telegram config: {error}"))?
        .as_object()
        .cloned()
        .ok_or_else(|| "Telegram config did not serialize as an object".to_string())?;
    let mut changes: Vec<_> = fields
        .into_iter()
        .map(|(field, value)| kria_core::config::Change::new("telegram", field, value))
        .collect();
    changes.push(kria_core::config::Change::new(
        "mcp",
        "servers",
        serde_json::json!(config.mcp.servers),
    ));
    state
        .config_service
        .patch_batch(changes, kria_core::config::ChangeSource::Ui, None)
        .await
        .map_err(|error| error.to_string())?;
    Ok(())
}

// ── Telegram Integration Commands ───────────────────────────────────

pub(super) async fn reconcile_telegram_feature(
    state: &AppState,
    enabled: bool,
) -> Result<(), String> {
    if !enabled {
        if let Some(bridge) = state.telegram_bridge.write().await.take() {
            bridge.stop();
        }
        let _ = apply_mcp_runtime_from_config(state).await;
        return Ok(());
    }

    let config = state.config.read().await.clone();
    let mcp_configured = config
        .mcp
        .servers
        .iter()
        .any(|server| server.name.eq_ignore_ascii_case("telegram"));
    if mcp_configured {
        let runtime = apply_mcp_runtime_from_config(state).await;
        let running = runtime["servers"]
            .as_array()
            .and_then(|servers| {
                servers.iter().find(|server| {
                    server["name"]
                        .as_str()
                        .map(|name| name.eq_ignore_ascii_case("telegram"))
                        .unwrap_or(false)
                })
            })
            .map(|server| server["state"] == "running")
            .unwrap_or(false);
        return if running {
            Ok(())
        } else {
            Err("Telegram MCP server failed to start".into())
        };
    }

    if config.telegram.bot_token.trim().is_empty() {
        return Err("Telegram bot token is not configured".into());
    }
    let mut bridge = state.telegram_bridge.write().await;
    if bridge.is_none() {
        *bridge = Some(TelegramBridge::spawn(
            config.telegram,
            state.agent_loop.clone(),
            state.memory_store.clone(),
            state.tool_registry.clone(),
            state.embeddings.clone(),
            state.hardware_info.tier.as_str().to_string(),
            state.orchestrator.clone(),
        ));
    }
    Ok(())
}

#[tauri::command]
pub async fn get_telegram_config(
    state: State<'_, AppStateCell>,
) -> Result<serde_json::Value, String> {
    let state = state
        .get()
        .ok_or_else(|| "KRIA is still initializing — please try again in a moment".to_string())?;
    let config = state.config.read().await;
    Ok(serde_json::json!({
        "enabled": config.telegram.enabled,
        "bot_token": config.telegram.bot_token,
        "allowed_chat_ids": config.telegram.allowed_chat_ids,
        "auto_start": config.telegram.auto_start,
    }))
}

#[tauri::command]
pub async fn update_telegram_config(
    enabled: bool,
    bot_token: String,
    allowed_chat_ids: String,
    auto_start: bool,
    state: State<'_, AppStateCell>,
) -> Result<(), String> {
    let state = state
        .get()
        .ok_or_else(|| "KRIA is still initializing — please try again in a moment".to_string())?;
    let mut config = state.config.read().await.clone();
    config.telegram.enabled = enabled;
    config.telegram.bot_token = bot_token;
    config.telegram.allowed_chat_ids = allowed_chat_ids;
    config.telegram.auto_start = auto_start;
    sync_telegram_mcp_server_config(&mut config);
    persist_telegram_config(state, &config).await?;
    reconcile_telegram_feature(state, enabled).await
}

#[tauri::command]
pub async fn start_telegram_mcp(
    state: State<'_, AppStateCell>,
) -> Result<serde_json::Value, String> {
    let state = state
        .get()
        .ok_or_else(|| "KRIA is still initializing — please try again in a moment".to_string())?;
    let mut config = state.config.read().await.clone();
    config.telegram.enabled = true;
    sync_telegram_mcp_server_config(&mut config);
    let tg_config = config.telegram.clone();
    let telegram_mcp_configured = config
        .mcp
        .servers
        .iter()
        .any(|s| s.name.eq_ignore_ascii_case("telegram"));
    persist_telegram_config(state, &config).await?;

    if tg_config.bot_token.is_empty() {
        return Err("Telegram bot token is not configured".into());
    }

    if telegram_mcp_configured {
        let runtime = apply_mcp_runtime_from_config(state).await;
        let telegram_status = runtime["servers"]
            .as_array()
            .and_then(|servers| {
                servers.iter().find(|server| {
                    server["name"]
                        .as_str()
                        .map(|name| name.eq_ignore_ascii_case("telegram"))
                        .unwrap_or(false)
                })
            })
            .cloned()
            .unwrap_or_default();

        if telegram_status["state"] == "running" {
            return Ok(serde_json::json!({
                "status": "running",
                "message": "Telegram MCP server is running and can now forward messages into KRIA.",
                "runtime": runtime,
            }));
        }

        return Err(format!(
            "Telegram MCP server failed to start: {}",
            telegram_status["error"].as_str().unwrap_or("unknown error")
        ));
    }

    // Stop existing bridge if running
    {
        let mut guard = state.telegram_bridge.write().await;
        if let Some(bridge) = guard.take() {
            bridge.stop();
        }
    }

    let hw_tier = state.hardware_info.tier.as_str().to_string();
    let bridge = TelegramBridge::spawn(
        tg_config,
        state.agent_loop.clone(),
        state.memory_store.clone(),
        state.tool_registry.clone(),
        state.embeddings.clone(),
        hw_tier,
        state.orchestrator.clone(),
    );

    *state.telegram_bridge.write().await = Some(bridge);

    Ok(serde_json::json!({
        "status": "running",
        "message": "Telegram bridge started. Bot is now polling for messages.",
    }))
}

#[tauri::command]
pub async fn stop_telegram_mcp(state: State<'_, AppStateCell>) -> Result<(), String> {
    let state = state
        .get()
        .ok_or_else(|| "KRIA is still initializing — please try again in a moment".to_string())?;
    // Stop the bridge
    {
        let mut guard = state.telegram_bridge.write().await;
        if let Some(bridge) = guard.take() {
            bridge.stop();
            tracing::info!("Telegram bridge stopped");
        }
    }

    // Update authoritative config and reconcile MCP/direct bridge state.
    let mut config = state.config.read().await.clone();
    config.telegram.enabled = false;
    sync_telegram_mcp_server_config(&mut config);
    persist_telegram_config(state, &config).await?;
    reconcile_telegram_feature(state, false).await
}

#[tauri::command]
pub async fn test_telegram_connection(bot_token: String) -> Result<serde_json::Value, String> {
    // Test the bot token by calling getMe
    let url = format!("https://api.telegram.org/bot{}/getMe", bot_token);
    let client = reqwest::Client::new();
    let resp: reqwest::Response = client
        .get(&url)
        .timeout(std::time::Duration::from_secs(10))
        .send()
        .await
        .map_err(|e| format!("Connection failed: {e}"))?;

    let body: serde_json::Value = resp
        .json::<serde_json::Value>()
        .await
        .map_err(|e| format!("Invalid response: {e}"))?;

    if body["ok"].as_bool() == Some(true) {
        let result = &body["result"];
        Ok(serde_json::json!({
            "valid": true,
            "bot_name": result["first_name"],
            "bot_username": result["username"],
            "bot_id": result["id"],
        }))
    } else {
        let desc = body
            .get("description")
            .and_then(|d: &serde_json::Value| d.as_str())
            .unwrap_or("unknown error");
        Err(format!("Invalid token: {}", desc))
    }
}
