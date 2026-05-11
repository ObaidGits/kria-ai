use super::*;

// ── Google Workspace Commands ────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub(super) struct GoogleWorkspaceRuntimeSnapshot {
    pub(super) configured_enabled: bool,
    pub(super) mcp_state: String,
    pub(super) mcp_tool_count: usize,
    pub(super) mcp_error: Option<String>,
    pub(super) mcp_running: bool,
    pub(super) gw_client_wired: bool,
}

#[derive(Debug, Clone)]
pub(super) struct GoogleAccountRegistryState {
    pub(super) account_registered: bool,
    pub(super) token_path: PathBuf,
    pub(super) token_present: bool,
}

impl GoogleAccountRegistryState {
    pub(super) fn requires_reauth(&self) -> bool {
        self.account_registered && !self.token_present
    }
}

fn google_accounts_registry_path(config_dir: &Path) -> PathBuf {
    config_dir.join("accounts.json")
}

pub(super) fn inspect_google_account_registry(
    config_dir: &Path,
    account: &str,
) -> GoogleAccountRegistryState {
    let default_token_path = config_dir.join("tokens").join(format!("{}.json", account));
    let mut state = GoogleAccountRegistryState {
        account_registered: false,
        token_path: default_token_path.clone(),
        token_present: default_token_path.exists(),
    };

    let registry_path = google_accounts_registry_path(config_dir);
    let Ok(raw) = std::fs::read_to_string(&registry_path) else {
        return state;
    };

    let Ok(json) = serde_json::from_str::<serde_json::Value>(&raw) else {
        return state;
    };

    let entry = json.get("accounts").and_then(|v| v.get(account)).cloned();
    let Some(entry) = entry else {
        return state;
    };

    state.account_registered = true;
    if let Some(token_path) = entry
        .get("tokenPath")
        .and_then(|v| v.as_str())
        .filter(|v| !v.trim().is_empty())
    {
        state.token_path = PathBuf::from(token_path);
        state.token_present = state.token_path.exists();
    }

    state
}

pub(super) fn remove_google_account_registry_entry(
    config_dir: &Path,
    account: &str,
) -> anyhow::Result<bool> {
    let registry_path = google_accounts_registry_path(config_dir);
    let raw = match std::fs::read_to_string(&registry_path) {
        Ok(raw) => raw,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(err) => return Err(err.into()),
    };

    let mut json: serde_json::Value = serde_json::from_str(&raw)?;
    let Some(accounts) = json.get_mut("accounts").and_then(|v| v.as_object_mut()) else {
        return Ok(false);
    };

    let removed = accounts.remove(account).is_some();
    if removed {
        let serialized = serde_json::to_string_pretty(&json)?;
        std::fs::write(&registry_path, serialized)?;
    }

    Ok(removed)
}

fn oauth_output_text(output: &std::process::Output) -> String {
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if stderr.is_empty() {
        stdout
    } else if stdout.is_empty() {
        stderr
    } else {
        format!("{}\n{}", stderr, stdout)
    }
}

fn oauth_output_detail(output: &std::process::Output) -> String {
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if !stderr.is_empty() {
        format!("; stderr: {}", stderr.chars().take(600).collect::<String>())
    } else if !stdout.is_empty() {
        format!("; output: {}", stdout.chars().take(600).collect::<String>())
    } else {
        String::new()
    }
}

fn should_retry_account_exists(output: &std::process::Output) -> bool {
    let text = oauth_output_text(output).to_ascii_lowercase();
    text.contains("already exists")
        || text.contains("remove first")
        || text.contains("could not load token")
}

pub(super) fn build_google_workspace_status_payload(
    account: &str,
    config_dir: &Path,
    credentials_configured: bool,
    token_present: bool,
    account_registered: bool,
    token_path: &Path,
    runtime: GoogleWorkspaceRuntimeSnapshot,
) -> serde_json::Value {
    let auth_ready = token_present && credentials_configured;
    let runtime_ready = runtime.mcp_running && runtime.gw_client_wired;
    let connected = auth_ready && runtime_ready;
    let credentials_display_path = config_dir.join("credentials.json");
    let requires_reauth = account_registered && !token_present;

    let mut warnings: Vec<String> = Vec::new();
    if !credentials_configured {
        warnings.push(format!(
            "credentials.json missing at {}",
            credentials_display_path.display()
        ));
    }
    if !token_present {
        warnings.push(format!(
            "OAuth token missing for account '{}' (expected at {})",
            account,
            token_path.display()
        ));
    }
    if requires_reauth {
        warnings.push("Account registry exists without a token; re-auth required".into());
    }
    if !runtime.configured_enabled {
        warnings.push("gworkspace MCP server is disabled in config".into());
    }
    if !runtime.mcp_running {
        warnings.push(format!(
            "gworkspace MCP runtime is not running (state={})",
            runtime.mcp_state
        ));
    }
    if runtime.mcp_running && !runtime.gw_client_wired {
        warnings.push("Google tool bridge not yet wired to active MCP client".into());
    }

    serde_json::json!({
        "connected": connected,
        "account": account,
        "credentials_configured": credentials_configured,
        "token_present": token_present,
        "account_registered": account_registered,
        "token_path": token_path.to_string_lossy(),
        "requires_reauth": requires_reauth,
        "auth_ready": auth_ready,
        "runtime_ready": runtime_ready,
        "gw_client_wired": runtime.gw_client_wired,
        "mcp": {
            "configured_enabled": runtime.configured_enabled,
            "state": runtime.mcp_state,
            "tool_count": runtime.mcp_tool_count,
            "error": runtime.mcp_error,
        },
        "capabilities": {
            "gmail": true,
            "drive": true,
            "calendar": true,
            "docs": true,
            "sheets": true,
            "slides": true,
            "forms": true,
            "meet": false,
            "meet_via_calendar": true,
        },
        "config_dir": config_dir.to_string_lossy(),
        "meet_support_mode": "calendar_conference_link",
        "warnings": warnings,
    })
}

/// Return Google Workspace status with separate auth/runtime/capability signals.
///
/// `connected` is true only when OAuth artifacts are present and the
/// gworkspace MCP runtime is currently usable.
#[tauri::command]
pub async fn get_google_workspace_status(
    account: Option<String>,
    state: State<'_, AppStateCell>,
) -> Result<serde_json::Value, String> {
    let state = state
        .get()
        .ok_or_else(|| "KRIA is still initializing — please try again in a moment".to_string())?;
    let config_guard = state.config.read().await;
    let account = account
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| google_account_from_config(&config_guard));
    let config_dir = google_mcp_config_dir_from_config(&config_guard);
    let credentials_path = config_dir.join("credentials.json");

    let account_state = inspect_google_account_registry(&config_dir, &account);
    let token_present = account_state.token_present;
    let credentials_configured = credentials_path.exists();

    let gworkspace_runtime = {
        let manager = state.mcp_manager.lock().await;
        manager
            .status()
            .await
            .into_iter()
            .find(|s| s.name == "gworkspace")
    };

    let configured_enabled = configured_google_workspace_server(&config_guard)
        .map(|s| s.enabled)
        .unwrap_or(false);
    drop(config_guard);

    let (mcp_state, mcp_tool_count, mcp_error, mcp_running) =
        if let Some(status) = gworkspace_runtime {
            (
                mcp_state_name(status.state).to_string(),
                status.tool_count,
                status.error,
                status.state == McpServerState::Running,
            )
        } else {
            ("not_configured".to_string(), 0usize, None, false)
        };

    let gw_client_wired = state.gw_client_ref.read().await.is_some();
    let payload = build_google_workspace_status_payload(
        &account,
        &config_dir,
        credentials_configured,
        token_present,
        account_state.account_registered,
        &account_state.token_path,
        GoogleWorkspaceRuntimeSnapshot {
            configured_enabled,
            mcp_state: mcp_state.clone(),
            mcp_tool_count,
            mcp_error,
            mcp_running,
            gw_client_wired,
        },
    );

    tracing::debug!(
        "[GW] status check: account='{}' connected={} auth_ready={} runtime_ready={} state={}",
        account,
        payload["connected"].as_bool().unwrap_or(false),
        payload["auth_ready"].as_bool().unwrap_or(false),
        payload["runtime_ready"].as_bool().unwrap_or(false),
        mcp_state
    );

    Ok(payload)
}

/// Launch the Google OAuth flow in the system browser.
///
/// Spawns `npx google-workspace-mcp accounts add <account>` which:
/// 1. Starts a local redirect-receiver HTTP server
/// 2. Opens the Google consent page in the default browser
/// 3. Saves the token when the user completes sign-in
///
/// Returns immediately with `status: "pending"`. The frontend should poll
/// `get_google_workspace_status` until `connected` becomes true.
/// Events emitted: `gw:connected` on success, `gw:error` on failure.
#[tauri::command]
pub async fn set_google_workspace_account(
    account: String,
    state: State<'_, AppStateCell>,
) -> Result<serde_json::Value, String> {
    let state = state
        .get()
        .ok_or_else(|| "KRIA is still initializing — please try again in a moment".to_string())?;

    let account = account.trim();
    if account.is_empty() {
        return Err("Google account name cannot be empty".into());
    }

    let mut config = state.config.write().await;
    let updated = sync_google_workspace_server_config(&mut config, Some(account));
    apply_google_runtime_env_from_config(&config);
    if updated {
        config.save().map_err(|e| e.to_string())?;
    }
    drop(config);

    let runtime = apply_mcp_runtime_from_config(state).await;

    Ok(serde_json::json!({
        "account": account,
        "updated": updated,
        "runtime": runtime,
    }))
}

#[tauri::command]
pub async fn connect_google_workspace(
    account: Option<String>,
    state: State<'_, AppStateCell>,
    app_handle: AppHandle,
) -> Result<serde_json::Value, String> {
    let state = state
        .get()
        .ok_or_else(|| "KRIA is still initializing — please try again in a moment".to_string())?;

    if let Some(requested) = account.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
        let mut config = state.config.write().await;
        let changed = sync_google_workspace_server_config(&mut config, Some(requested));
        apply_google_runtime_env_from_config(&config);
        if changed {
            config.save().map_err(|e| e.to_string())?;
        }
    }

    let (account, config_dir) = {
        let config = state.config.read().await;
        let resolved_account = account
            .filter(|s| !s.trim().is_empty())
            .unwrap_or_else(|| google_account_from_config(&config));
        let resolved_dir = google_mcp_config_dir_from_config(&config);
        (resolved_account, resolved_dir)
    };
    let config_dir_display = config_dir.to_string_lossy().to_string();
    let account_state = inspect_google_account_registry(&config_dir, &account);

    // Fail fast if credentials.json is missing
    let creds_path = config_dir.join("credentials.json");
    if !creds_path.exists() {
        return Err(
            format!(
                "credentials.json not found at {}. Please add your Google Cloud OAuth client credentials first.",
                creds_path.display()
            ),
        );
    }

    let mut preflight_notice: Option<String> = None;
    if account_state.requires_reauth() {
        match remove_google_account_registry_entry(&config_dir, &account) {
            Ok(true) => {
                preflight_notice = Some(format!(
                    "Detected stale Google account '{}' without a token. Cleared registry entry before OAuth.",
                    account
                ));
            }
            Ok(false) => {
                preflight_notice = Some(format!(
                    "Detected stale Google account '{}' without a token. OAuth may need a manual cleanup.",
                    account
                ));
            }
            Err(err) => {
                preflight_notice = Some(format!(
                    "Detected stale Google account '{}' but failed to clean registry: {}",
                    account, err
                ));
            }
        }
    }

    let account_clone = account.clone();
    let config_dir_clone = config_dir_display.clone();
    let mcp_manager = state.mcp_manager.clone();
    let tool_registry = state.tool_registry.clone();
    let gw_client_ref = state.gw_client_ref.clone();
    let config_arc = state.config.clone();
    tokio::spawn(async move {
        tracing::info!("[GW] Starting OAuth flow for account '{}'", account_clone);
        let run_oauth = |account: String, config_dir: String| async move {
            tokio::process::Command::new("npx")
                .args([
                    "-y",
                    "google-workspace-mcp",
                    "accounts",
                    "add",
                    account.as_str(),
                ])
                .env(GOOGLE_MCP_CONFIG_DIR_ENV, config_dir)
                .output()
                .await
        };

        let mut result = run_oauth(account_clone.clone(), config_dir_clone.clone()).await;
        if let Ok(output) = result.as_ref() {
            if !output.status.success() && should_retry_account_exists(output) {
                let config_dir = PathBuf::from(&config_dir_clone);
                if let Ok(true) = remove_google_account_registry_entry(&config_dir, &account_clone)
                {
                    let _ = app_handle.emit(
                        "gw:notice",
                        serde_json::json!({
                            "message": "Stale Google account entry removed. Retrying OAuth...",
                        }),
                    );
                    result = run_oauth(account_clone.clone(), config_dir_clone.clone()).await;
                }
            }
        }

        match result {
            Ok(output) if output.status.success() => {
                let runtime_refresh_result = async {
                    let desired = { config_arc.read().await.mcp.servers.clone() };
                    let mut manager = mcp_manager.lock().await;
                    let _ = manager.reconcile(desired, &tool_registry).await;
                    let gw_client = manager.get_client("gworkspace").cloned();
                    drop(manager);

                    if let Some(client) = gw_client {
                        gw::set_client(&gw_client_ref, client).await;
                        Ok::<(), String>(())
                    } else {
                        *gw_client_ref.write().await = None;
                        Err("gworkspace runtime not available after OAuth completion".into())
                    }
                }
                .await;

                tracing::info!("[GW] OAuth completed successfully for '{}'", account_clone);
                let _ = app_handle.emit(
                    "gw:connected",
                    serde_json::json!({
                        "account": account_clone,
                        "runtime_refreshed": runtime_refresh_result.is_ok(),
                    }),
                );

                if let Err(msg) = runtime_refresh_result {
                    let _ = app_handle.emit("gw:error", serde_json::json!({ "message": msg }));
                }
            }
            Ok(output) => {
                let detail = oauth_output_detail(&output);
                let msg = format!("OAuth process exited with: {}{}", output.status, detail);
                tracing::warn!("[GW] {}", msg);
                let _ = app_handle.emit("gw:error", serde_json::json!({ "message": msg }));
            }
            Err(e) => {
                let msg = format!("Failed to spawn OAuth process: {e}");
                tracing::error!("[GW] {}", msg);
                let _ = app_handle.emit("gw:error", serde_json::json!({ "message": msg }));
            }
        }
    });

    Ok(serde_json::json!({
        "status": "pending",
        "account": account,
        "config_dir": config_dir_display,
        "message": preflight_notice
            .map(|notice| format!("{} Browser opened for Google sign-in. Complete authorization and return here.", notice))
            .unwrap_or_else(|| "Browser opened for Google sign-in. Complete authorization and return here.".into()),
        "cleanup_attempted": account_state.requires_reauth(),
    }))
}

/// Remove the OAuth token for a Google Workspace account (sign out).
#[tauri::command]
pub async fn disconnect_google_workspace(
    account: Option<String>,
    state: State<'_, AppStateCell>,
) -> Result<(), String> {
    let state = state
        .get()
        .ok_or_else(|| "KRIA is still initializing — please try again in a moment".to_string())?;

    if let Some(requested) = account.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
        let mut config = state.config.write().await;
        let changed = sync_google_workspace_server_config(&mut config, Some(requested));
        apply_google_runtime_env_from_config(&config);
        if changed {
            config.save().map_err(|e| e.to_string())?;
        }
    }

    let (account, config_dir) = {
        let config = state.config.read().await;
        (
            account
                .filter(|s| !s.trim().is_empty())
                .unwrap_or_else(|| google_account_from_config(&config)),
            google_mcp_config_dir_from_config(&config),
        )
    };

    let account_state = inspect_google_account_registry(&config_dir, &account);
    let token_path = account_state.token_path.clone();

    if token_path.exists() {
        std::fs::remove_file(&token_path).map_err(|e| format!("Failed to remove token: {e}"))?;
        tracing::info!("[GW] Disconnected Google account '{}'", account);
    }

    if account_state.account_registered {
        if let Err(err) = remove_google_account_registry_entry(&config_dir, &account) {
            tracing::warn!("[GW] Failed to remove account registry entry: {}", err);
        }
    }

    let mut manager = state.mcp_manager.lock().await;
    let _ = manager
        .restart_server("gworkspace", &state.tool_registry)
        .await;
    let statuses = manager.status().await;
    let gw_client = manager.get_client("gworkspace").cloned();
    drop(manager);

    sync_google_workspace_client_ref(state, gw_client).await;
    update_mcp_health_status(state, &statuses).await;
    Ok(())
}
