use super::*;

// ── Telegram Integration Commands ───────────────────────────────────

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
    let mut config = state.config.write().await;
    config.telegram.enabled = enabled;
    config.telegram.bot_token = bot_token;
    config.telegram.allowed_chat_ids = allowed_chat_ids;
    config.telegram.auto_start = auto_start;
    sync_telegram_mcp_server_config(&mut config);
    config.save().map_err(|e| e.to_string())?;
    drop(config);

    let _ = apply_mcp_runtime_from_config(state).await;
    Ok(())
}

#[tauri::command]
pub async fn start_telegram_mcp(
    state: State<'_, AppStateCell>,
) -> Result<serde_json::Value, String> {
    let state = state
        .get()
        .ok_or_else(|| "KRIA is still initializing — please try again in a moment".to_string())?;
    let mut config = state.config.write().await;
    config.telegram.enabled = true;
    sync_telegram_mcp_server_config(&mut config);
    let tg_config = config.telegram.clone();
    let telegram_mcp_configured = config
        .mcp
        .servers
        .iter()
        .any(|s| s.name.eq_ignore_ascii_case("telegram"));
    config.save().map_err(|e| e.to_string())?;
    drop(config);

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
        state.vectors.clone(),
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

    // Update config
    let mut config = state.config.write().await;
    config.telegram.enabled = false;
    sync_telegram_mcp_server_config(&mut config);
    config.save().map_err(|e| e.to_string())?;
    drop(config);

    let _ = apply_mcp_runtime_from_config(state).await;
    Ok(())
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
