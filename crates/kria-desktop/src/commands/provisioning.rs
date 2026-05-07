use tauri::{AppHandle, Emitter};

// ────────────────────────────────────────────────────────────────────────────────
// Provisioning commands — first-boot setup wizard
// ────────────────────────────────────────────────────────────────────────────────

#[tauri::command]
pub async fn get_provisioning_state() -> Result<serde_json::Value, String> {
    let state = kria_core::infra::provisioning::ProvisioningState::load();
    serde_json::to_value(&state).map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn start_provisioning(handle: AppHandle) -> Result<serde_json::Value, String> {
    let cancel = tokio_util::sync::CancellationToken::new();
    let handle_clone = handle.clone();

    let mut engine = kria_core::infra::provisioning::ProvisioningEngine::new(cancel);

    // Run hardware detection synchronously (fast)
    engine.run_hardware_detection().map_err(|e| e.to_string())?;

    let profile = engine
        .state
        .hardware_profile
        .as_ref()
        .ok_or("hardware detection failed")?;

    let event_payload = serde_json::json!({
        "step": "hardware_detection",
        "status": "done",
        "profile": profile,
    });

    // Emit event to frontend
    let _ = handle_clone.emit("provisioning:state_changed", &event_payload);

    serde_json::to_value(&engine.state).map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn complete_provisioning() -> Result<serde_json::Value, String> {
    let mut state = kria_core::infra::provisioning::ProvisioningState::load();
    state.current_step = kria_core::infra::provisioning::ProvisioningStep::Complete;
    state.complete_step(kria_core::infra::provisioning::ProvisioningStep::Complete);
    state.save().map_err(|e| e.to_string())?;
    serde_json::to_value(&state).map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn set_provisioning_backend(
    choice_type: String,
    url: Option<String>,
    api_key: Option<String>,
    model_name: Option<String>,
) -> Result<serde_json::Value, String> {
    let cancel = tokio_util::sync::CancellationToken::new();
    let mut engine = kria_core::infra::provisioning::ProvisioningEngine::new(cancel);

    let choice = match choice_type.as_str() {
        "external" => {
            let url = url.ok_or("url is required for external backend")?;
            kria_core::infra::provisioning::BackendChoice::External {
                url,
                api_key,
                model_name,
            }
        }
        _ => kria_core::infra::provisioning::BackendChoice::Local,
    };

    engine.set_backend_choice(choice);
    serde_json::to_value(&engine.state).map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn run_provisioning_step(
    handle: AppHandle,
    step: String,
) -> Result<serde_json::Value, String> {
    let cancel = tokio_util::sync::CancellationToken::new();
    let mut engine = kria_core::infra::provisioning::ProvisioningEngine::new(cancel);
    let handle_clone = handle.clone();

    let progress_callback = move |progress: kria_core::infra::download::DownloadProgress| {
        let _ = handle_clone.emit("provisioning:progress", &progress);
    };

    match step.as_str() {
        "model_download" => engine
            .run_model_download(progress_callback)
            .await
            .map_err(|e| e.to_string())?,
        "sidecar_setup" => engine
            .run_sidecar_setup()
            .await
            .map_err(|e| e.to_string())?,
        "server_verification" => engine
            .run_server_verification(progress_callback)
            .await
            .map_err(|e| e.to_string())?,
        _ => return Err(format!("unknown provisioning step: {step}")),
    };

    let _ = handle.emit(
        "provisioning:state_changed",
        serde_json::json!({ "step": step, "status": "done" }),
    );

    serde_json::to_value(&engine.state).map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn get_provisioning_diagnostics() -> Result<String, String> {
    let cancel = tokio_util::sync::CancellationToken::new();
    let engine = kria_core::infra::provisioning::ProvisioningEngine::new(cancel);
    Ok(engine.diagnostic_info())
}

#[tauri::command]
pub async fn get_hardware_profile() -> Result<serde_json::Value, String> {
    // Try loading saved profile first
    if let Some(profile) = kria_core::infra::hardware_profiler::load_profile() {
        return serde_json::to_value(&profile).map_err(|e| e.to_string());
    }
    // Otherwise, run detection
    let profile = kria_core::infra::hardware_profiler::profile_hardware();
    serde_json::to_value(&profile).map_err(|e| e.to_string())
}
