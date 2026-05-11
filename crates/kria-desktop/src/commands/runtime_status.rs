use super::*;

fn classify_ironclad_qos_light(
    high_recovery_wait_p95_ms: u64,
    high_recovery_slo_ms: u64,
    qos_pressure_active: bool,
    target_health_degraded: bool,
    reset_in_flight: bool,
) -> &'static str {
    if reset_in_flight {
        return "yellow";
    }

    if target_health_degraded {
        return "red";
    }

    if qos_pressure_active {
        return "yellow";
    }

    if high_recovery_slo_ms == 0 {
        return "green";
    }

    if high_recovery_wait_p95_ms > high_recovery_slo_ms {
        "yellow"
    } else {
        "green"
    }
}

pub(super) async fn collect_ironclad_status_from_parts(
    orchestrator_cell: &Arc<tokio::sync::RwLock<Option<Arc<Orchestrator>>>>,
    reset_state: &Arc<RwLock<IroncladResetSnapshot>>,
    forensic_log: &Arc<RwLock<Vec<IroncladForensicRecord>>>,
) -> serde_json::Value {
    let reset_snapshot = reset_state.read().await.clone();
    let forensic_snapshot = forensic_log.read().await;
    let forensic_count = forensic_snapshot.len();
    let latest_forensic = forensic_snapshot.last().cloned();
    drop(forensic_snapshot);

    let (enrolled_targets, enrollment_registry_path) = load_enrolled_target_status_snapshots();
    let enrolled_target_count = enrolled_targets.len();

    let orchestrator = orchestrator_cell.read().await.clone();
    if let Some(orchestrator) = orchestrator {
        let snapshot = orchestrator.remote_infra_observability_snapshot();
        let pool_packet = snapshot.latest_pool_packet.clone();
        let qos_packet = snapshot.latest_qos_adaptation.clone();

        let total_targets = pool_packet
            .as_ref()
            .map(|p| p.total_targets)
            .unwrap_or(enrolled_target_count);
        let ready_targets = pool_packet.as_ref().map(|p| p.ready_targets).unwrap_or(0);
        let leased_targets = pool_packet.as_ref().map(|p| p.leased_targets).unwrap_or(0);
        let tainted_targets = pool_packet.as_ref().map(|p| p.tainted_targets).unwrap_or(0);
        let quarantined_targets = pool_packet
            .as_ref()
            .map(|p| p.quarantined_targets)
            .unwrap_or(0);
        let active_leases = pool_packet.as_ref().map(|p| p.active_leases).unwrap_or(0);

        let high_recovery_wait_p95_ms = qos_packet
            .as_ref()
            .map(|p| p.high_recovery_wait_p95_ms)
            .unwrap_or(0);
        let high_recovery_slo_ms = qos_packet
            .as_ref()
            .map(|p| p.high_recovery_slo_ms)
            .unwrap_or(0);

        let qos_traffic_light = classify_ironclad_qos_light(
            high_recovery_wait_p95_ms,
            high_recovery_slo_ms,
            snapshot.qos_pressure_active,
            snapshot.target_health_degraded || tainted_targets > 0 || quarantined_targets > 0,
            reset_snapshot.in_flight,
        );

        serde_json::json!({
            "enabled": true,
            "fleet": {
                "total_targets": total_targets,
                "ready_targets": ready_targets,
                "leased_targets": leased_targets,
                "tainted_targets": tainted_targets,
                "quarantined_targets": quarantined_targets,
                "active_leases": active_leases,
                "health_degraded": snapshot.target_health_degraded || tainted_targets > 0 || quarantined_targets > 0,
                "source_unwired": pool_packet.is_none(),
                "pool_packet": pool_packet,
                "enrolled_target_count": enrolled_target_count,
                "enrolled_targets": enrolled_targets,
                "enrollment_registry_path": enrollment_registry_path.to_string_lossy().to_string(),
            },
            "qos": {
                "traffic_light": qos_traffic_light,
                "pressure_active": snapshot.qos_pressure_active,
                "high_recovery_wait_p95_ms": high_recovery_wait_p95_ms,
                "high_recovery_slo_ms": high_recovery_slo_ms,
                "decision": qos_packet.as_ref().map(|p| format!("{:?}", p.decision)),
                "reason": qos_packet.as_ref().map(|p| p.reason.clone()),
                "adaptation_packet": qos_packet,
            },
            "reset": reset_snapshot,
            "forensics": {
                "count": forensic_count,
                "latest": latest_forensic,
            },
        })
    } else {
        serde_json::json!({
            "enabled": false,
            "fleet": {
                "total_targets": enrolled_target_count,
                "ready_targets": 0,
                "leased_targets": 0,
                "tainted_targets": 0,
                "quarantined_targets": 0,
                "active_leases": 0,
                "health_degraded": false,
                "source_unwired": true,
                "pool_packet": serde_json::Value::Null,
                "enrolled_target_count": enrolled_target_count,
                "enrolled_targets": enrolled_targets,
                "enrollment_registry_path": enrollment_registry_path.to_string_lossy().to_string(),
            },
            "qos": {
                "traffic_light": "gray",
                "pressure_active": false,
                "high_recovery_wait_p95_ms": 0,
                "high_recovery_slo_ms": 0,
                "decision": serde_json::Value::Null,
                "reason": serde_json::Value::Null,
                "adaptation_packet": serde_json::Value::Null,
            },
            "reset": reset_snapshot,
            "forensics": {
                "count": forensic_count,
                "latest": latest_forensic,
            },
        })
    }
}

async fn enqueue_ironclad_reset(
    orchestrator_cell: Arc<tokio::sync::RwLock<Option<Arc<Orchestrator>>>>,
    reset_state: Arc<RwLock<IroncladResetSnapshot>>,
    forensic_log: Arc<RwLock<Vec<IroncladForensicRecord>>>,
    app_handle: AppHandle,
    mode: &'static str,
    reason: String,
    force_shutdown_before_restart: bool,
) -> Result<IroncladResetSnapshot, String> {
    let event_id = uuid::Uuid::new_v4().to_string();
    let queued_snapshot = {
        let mut guard = reset_state.write().await;
        if guard.in_flight {
            return Err("A reset is already in progress; wait for completion first".to_string());
        }

        *guard = IroncladResetSnapshot {
            event_id: event_id.clone(),
            phase: "requested".to_string(),
            reason: reason.clone(),
            detail: format!("{mode} reset queued"),
            started_unix_ms: unix_now_ms(),
            completed_unix_ms: None,
            in_flight: true,
        };

        guard.clone()
    };

    let _ = app_handle.emit("ironclad:reset", serde_json::json!(queued_snapshot.clone()));

    let orchestrator_cell_bg = orchestrator_cell.clone();
    let reset_state_bg = reset_state.clone();
    let forensic_log_bg = forensic_log.clone();
    let app_bg = app_handle.clone();
    let reason_bg = reason.clone();
    let mode_bg = mode.to_string();
    let event_id_bg = event_id.clone();

    tokio::spawn(async move {
        {
            let mut guard = reset_state_bg.write().await;
            guard.phase = "in_progress".to_string();
            guard.detail = format!("{mode_bg} reset in progress");
        }

        if let Ok(in_progress) = serde_json::to_value(reset_state_bg.read().await.clone()) {
            let _ = app_bg.emit("ironclad:reset", in_progress);
        }

        let result = {
            let orchestrator = orchestrator_cell_bg.read().await.clone();
            match orchestrator {
                Some(orch) => {
                    if force_shutdown_before_restart {
                        orch.shutdown().await;
                    }
                    orch.restart(&reason_bg).await.map_err(|e| e.to_string())
                }
                None => Err("Local orchestrator is not available in this runtime".to_string()),
            }
        };

        let (final_phase, final_detail, forensic_severity) = match &result {
            Ok(_) => (
                "healthy".to_string(),
                format!("{mode_bg} reset completed successfully"),
                "info".to_string(),
            ),
            Err(error) => (
                "failed".to_string(),
                format!("{mode_bg} reset failed: {error}"),
                "critical".to_string(),
            ),
        };

        {
            let mut guard = reset_state_bg.write().await;
            guard.phase = final_phase;
            guard.detail = final_detail.clone();
            guard.completed_unix_ms = Some(unix_now_ms());
            guard.in_flight = false;
        }

        let completed_snapshot = reset_state_bg.read().await.clone();
        let _ = app_bg.emit("ironclad:reset", serde_json::json!(completed_snapshot));

        let evidence = match result {
            Ok(_) => format!(
                "event_id={event_id_bg}; mode={mode_bg}; reason={reason_bg}; outcome=ok"
            ),
            Err(error) => format!(
                "event_id={event_id_bg}; mode={mode_bg}; reason={reason_bg}; outcome=error; detail={error}"
            ),
        };

        append_ironclad_forensic_record(
            &forensic_log_bg,
            &app_bg,
            "reset",
            &forensic_severity,
            format!("{mode_bg} reset lifecycle completed"),
            evidence,
            "desktop.reset",
        )
        .await;

        let status_payload = collect_ironclad_status_from_parts(
            &orchestrator_cell_bg,
            &reset_state_bg,
            &forensic_log_bg,
        )
        .await;
        let _ = app_bg.emit("ironclad:status", status_payload);
    });

    Ok(queued_snapshot)
}

/// Return a snapshot of the hardware orchestrator state.
#[tauri::command]
pub async fn get_orchestrator_status(
    state: State<'_, AppStateCell>,
) -> Result<serde_json::Value, String> {
    let state = state
        .get()
        .ok_or_else(|| "KRIA is still initializing — please try again in a moment".to_string())?;
    let orch_guard = state.orchestrator.read().await.clone();
    match orch_guard.as_ref() {
        Some(orch) => {
            let snap = orch.snapshot();
            let process_alive = orch.server_manager.has_live_process().await;
            let state_healthy = snap.server_healthy;
            let active_turns = state
                .orchestrator_active_turns
                .load(std::sync::atomic::Ordering::SeqCst);
            let idle_for_secs = {
                let lock = state.orchestrator_last_activity_at.lock().await;
                lock.elapsed().as_secs()
            };
            Ok(serde_json::json!({
                "enabled": true,
                "backend": format!("{:?}", snap.backend),
                "current_ngl": snap.current_ngl,
                "current_context": snap.current_context,
                "degradation": format!("{:?}", snap.degradation),
                "server_healthy": state_healthy && process_alive,
                "server_healthy_state": state_healthy,
                "process_alive": process_alive,
                "server_state_code": orch.server_manager.state(),
                "server_swapping": orch.server_manager.is_swapping(),
                "idle_release_enabled": orch.config.idle_release_enabled,
                "idle_release_after_secs": orch.config.idle_release_after_secs,
                "idle_release_check_interval_secs": orch.config.idle_release_check_interval_secs,
                "active_turns": active_turns,
                "idle_for_secs": idle_for_secs,
                "api_url": orch.api_url(),
            }))
        }
        None => Ok(serde_json::json!({
            "enabled": false,
        })),
    }
}

#[tauri::command]
pub async fn register_new_target(
    request: NewTargetRequest,
    state: State<'_, AppStateCell>,
    app_handle: AppHandle,
) -> Result<RegisterNewTargetResponse, RegisterNewTargetError> {
    let state = state.get().ok_or_else(|| {
        RegisterNewTargetError::new(
            RegisterNewTargetErrorCode::Unknown,
            "KRIA is still initializing, please try again in a moment",
            None,
        )
    })?;

    let request = normalize_new_target_request(request)?;

    for dependency in ["ssh", "ssh-keyscan", "ssh-keygen"] {
        if which_binary(dependency).is_none() {
            return Err(RegisterNewTargetError::new(
                RegisterNewTargetErrorCode::DependencyMissing,
                format!("Required dependency is missing: {dependency}"),
                Some("Install OpenSSH client tools and retry enrollment".to_string()),
            ));
        }
    }

    let (public_key, public_key_path, created_local_key) =
        ensure_local_ssh_keypair(request.ssh_private_key_path.as_path()).await?;

    let (known_hosts_entries, observed_hostkey_sha256_b64) =
        fetch_ssh_hostkey_fingerprint(&request.host, request.port).await?;

    let registry_path = resolve_target_registry_path(state).await?;
    let mut registry = load_fleet_enrollment_registry(registry_path.as_path())?;

    let existing_index = registry.targets.iter().position(|target| {
        target.host.eq_ignore_ascii_case(&request.host)
            && target.port == request.port
            && target.username.eq_ignore_ascii_case(&request.username)
            && target.mode == "ssh_bootstrap"
    });
    let existing_record = existing_index.map(|idx| registry.targets[idx].clone());

    if let Some(existing) = existing_record.as_ref() {
        if existing.ssh_hostkey_sha256_b64 != observed_hostkey_sha256_b64 {
            return Err(RegisterNewTargetError::new(
                RegisterNewTargetErrorCode::HostKeyChanged,
                "Host key changed for an already enrolled target",
                Some(format!(
                    "existing={} observed={}",
                    existing.ssh_hostkey_sha256_b64, observed_hostkey_sha256_b64
                )),
            ));
        }
    }

    if let Some(expected) = request.expected_hostkey_sha256_b64.as_ref() {
        if expected != &observed_hostkey_sha256_b64 {
            return Err(RegisterNewTargetError::new(
                RegisterNewTargetErrorCode::HostKeyChanged,
                "Observed SSH host key fingerprint does not match expected fingerprint",
                Some(format!(
                    "expected={} observed={}",
                    expected, observed_hostkey_sha256_b64
                )),
            ));
        }
    }

    let known_hosts_temp_path = std::env::temp_dir().join(format!(
        "kria_known_hosts_{}_{}.tmp",
        std::process::id(),
        Uuid::new_v4()
    ));
    std::fs::write(&known_hosts_temp_path, known_hosts_entries.as_bytes()).map_err(|error| {
        RegisterNewTargetError::new(
            RegisterNewTargetErrorCode::PersistenceFailed,
            "Failed to persist temporary known_hosts file",
            Some(error.to_string()),
        )
    })?;
    let known_hosts_guard = TempFileGuard::new(known_hosts_temp_path);

    let mut verify_args = build_ssh_base_args(&request, known_hosts_guard.path());
    verify_args.push(format!("printf '{}'", TARGET_ENROLLMENT_VERIFY_MARKER));
    let verify_output =
        run_external_command("ssh", &verify_args, TARGET_ENROLLMENT_SSH_TIMEOUT_SECS).await?;
    if !verify_output.status.success()
        || !verify_output
            .stdout
            .contains(TARGET_ENROLLMENT_VERIFY_MARKER)
    {
        return Err(classify_ssh_stage_error(
            &verify_output,
            "verify_ssh_access",
        ));
    }

    let quoted_public_key = shell_quote_single(&public_key);
    let bootstrap_command = format!(
        "set -eu; umask 077; mkdir -p \"$HOME/.ssh\"; touch \"$HOME/.ssh/authorized_keys\"; chmod 700 \"$HOME/.ssh\"; chmod 600 \"$HOME/.ssh/authorized_keys\"; if ! grep -qxF {quoted_public_key} \"$HOME/.ssh/authorized_keys\"; then printf '%s\\n' {quoted_public_key} >> \"$HOME/.ssh/authorized_keys\"; fi"
    );
    let mut bootstrap_args = build_ssh_base_args(&request, known_hosts_guard.path());
    bootstrap_args.push(bootstrap_command);
    let bootstrap_output =
        run_external_command("ssh", &bootstrap_args, TARGET_ENROLLMENT_SSH_TIMEOUT_SECS).await?;
    if !bootstrap_output.status.success() {
        return Err(classify_ssh_stage_error(
            &bootstrap_output,
            "bootstrap_authorized_keys",
        ));
    }

    let now_unix_ms = unix_now_ms() as i64;
    let controller_epoch = request
        .controller_epoch
        .or(existing_record
            .as_ref()
            .map(|record| record.controller_epoch))
        .unwrap_or(0);
    let enrolled_at_unix_ms = existing_record
        .as_ref()
        .map(|record| record.enrolled_at_unix_ms)
        .unwrap_or(now_unix_ms);

    let private_key_path = request.ssh_private_key_path.to_string_lossy().to_string();
    let public_key_path_str = public_key_path.to_string_lossy().to_string();

    let created_new_target = existing_index.is_none();
    let target_id = existing_record
        .as_ref()
        .map(|record| record.target_id.clone())
        .unwrap_or_else(|| Uuid::new_v4().to_string());

    let staged_record = EnrolledTargetRecord {
        target_id: target_id.clone(),
        display_name: request.display_name.clone(),
        host: request.host.clone(),
        port: request.port,
        username: request.username.clone(),
        mode: "ssh_bootstrap".to_string(),
        ssh_private_key_path: private_key_path.clone(),
        ssh_public_key_path: public_key_path_str.clone(),
        ssh_hostkey_sha256_b64: observed_hostkey_sha256_b64.clone(),
        controller_epoch,
        enrolled_at_unix_ms,
        last_verified_unix_ms: now_unix_ms,
    };

    let runtime_created =
        admit_enrolled_target_to_fleet_runtime(&state.fleet_runtime, &staged_record)
            .await
            .map_err(|detail| {
                RegisterNewTargetError::new(
                    RegisterNewTargetErrorCode::BootstrapFailed,
                    "Target verified but runtime admission failed",
                    Some(detail),
                )
            })?;

    if let Some(index) = existing_index {
        let record = registry.targets.get_mut(index).ok_or_else(|| {
            RegisterNewTargetError::new(
                RegisterNewTargetErrorCode::PersistenceFailed,
                "Enrollment registry indexing failed",
                None,
            )
        })?;
        *record = staged_record;
    } else {
        registry.targets.push(staged_record);
    }

    save_fleet_enrollment_registry(registry_path.as_path(), &registry)?;

    if let Some(orchestrator) = state.orchestrator.read().await.clone() {
        if let Err(error) = configure_orchestrator_fleet_bridge(&orchestrator, &state.fleet_runtime)
        {
            tracing::warn!(
                error = %error,
                "fleet enrollment: failed to wire orchestrator bridge after runtime admission"
            );
        } else {
            pulse_target_pool_telemetry(&state.fleet_runtime.target_pool).await;
        }
    }

    append_ironclad_forensic_record(
        &state.ironclad_forensic_log,
        &app_handle,
        "fleet_enrollment",
        "info",
        format!(
            "Fleet target enrolled: {}@{}:{}",
            request.username, request.host, request.port
        ),
        format!(
            "target_id={target_id}; display_name={}; fingerprint={}; created_new={created_new_target}; key_created={created_local_key}; runtime_created={runtime_created}",
            request.display_name, observed_hostkey_sha256_b64
        ),
        "desktop.fleet",
    )
    .await;

    let response = RegisterNewTargetResponse {
        target_id: target_id.clone(),
        display_name: request.display_name,
        host: request.host,
        port: request.port,
        username: request.username,
        mode: "ssh_bootstrap".to_string(),
        ssh_hostkey_sha256_b64: observed_hostkey_sha256_b64,
        ssh_private_key_path: private_key_path,
        ssh_public_key_path: public_key_path_str,
        controller_epoch,
        created_new_target,
        created_local_key,
        enrolled_at_unix_ms,
        registry_path: registry_path.to_string_lossy().to_string(),
    };

    let _ = app_handle.emit("fleet:target_enrolled", serde_json::json!(response.clone()));
    let _ = app_handle.emit(
        "ironclad:status",
        collect_ironclad_status_from_parts(
            &state.orchestrator,
            &state.ironclad_reset,
            &state.ironclad_forensic_log,
        )
        .await,
    );

    Ok(response)
}

// ─────────────────────────────────────────────────────────────────────────────
//  DELETE TARGET
// ─────────────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, serde::Serialize)]
pub struct DeleteTargetResponse {
    pub target_id: String,
    pub display_name: String,
    pub removed: bool,
}

#[tauri::command]
pub async fn delete_target(
    target_id: String,
    state: State<'_, AppStateCell>,
    app_handle: AppHandle,
) -> Result<DeleteTargetResponse, String> {
    let state = state
        .get()
        .ok_or_else(|| "KRIA is still initializing — please try again in a moment".to_string())?;

    let registry_path = resolve_target_registry_path(state)
        .await
        .map_err(|e| e.message)?;
    let mut registry =
        load_fleet_enrollment_registry(registry_path.as_path()).map_err(|e| e.message)?;

    let idx = registry
        .targets
        .iter()
        .position(|t| t.target_id == target_id)
        .ok_or_else(|| format!("Target {target_id} not found in registry"))?;

    let removed_record = registry.targets.remove(idx);
    save_fleet_enrollment_registry(registry_path.as_path(), &registry).map_err(|e| e.message)?;

    // Remove from runtime projections and broadcast removal via SSE
    if let Ok(uuid) = uuid::Uuid::parse_str(&target_id) {
        state
            .fleet_control_runtime
            .remove_target_projection(&uuid)
            .await;
        state
            .fleet_control_runtime
            .manager
            .emit_target_removed(uuid);
    }

    append_ironclad_forensic_record(
        &state.ironclad_forensic_log,
        &app_handle,
        "fleet_enrollment",
        "info",
        format!(
            "Fleet target removed: {} ({}@{}:{})",
            removed_record.display_name,
            removed_record.username,
            removed_record.host,
            removed_record.port
        ),
        format!("target_id={target_id}"),
        "desktop.fleet",
    )
    .await;

    let _ = app_handle.emit(
        "fleet:target_deleted",
        serde_json::json!({ "target_id": target_id }),
    );
    let _ = app_handle.emit(
        "ironclad:status",
        collect_ironclad_status_from_parts(
            &state.orchestrator,
            &state.ironclad_reset,
            &state.ironclad_forensic_log,
        )
        .await,
    );

    Ok(DeleteTargetResponse {
        target_id,
        display_name: removed_record.display_name,
        removed: true,
    })
}

// ─────────────────────────────────────────────────────────────────────────────
//  UPDATE TARGET
// ─────────────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateTargetRequest {
    pub target_id: String,
    #[serde(default)]
    pub display_name: Option<String>,
    #[serde(default)]
    pub host: Option<String>,
    #[serde(default)]
    pub port: Option<u16>,
    #[serde(default)]
    pub username: Option<String>,
    #[serde(default)]
    pub ssh_private_key_path: Option<String>,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct UpdateTargetResponse {
    pub target_id: String,
    pub display_name: String,
    pub host: String,
    pub port: u16,
    pub username: String,
    pub updated: bool,
}

#[tauri::command]
pub async fn update_target(
    request: UpdateTargetRequest,
    state: State<'_, AppStateCell>,
    app_handle: AppHandle,
) -> Result<UpdateTargetResponse, String> {
    let state = state
        .get()
        .ok_or_else(|| "KRIA is still initializing — please try again in a moment".to_string())?;

    let registry_path = resolve_target_registry_path(state)
        .await
        .map_err(|e| e.message)?;
    let mut registry =
        load_fleet_enrollment_registry(registry_path.as_path()).map_err(|e| e.message)?;

    let idx = registry
        .targets
        .iter()
        .position(|t| t.target_id == request.target_id)
        .ok_or_else(|| format!("Target {} not found in registry", request.target_id))?;

    let record = &mut registry.targets[idx];
    if let Some(name) = &request.display_name {
        if !name.trim().is_empty() {
            record.display_name = name.trim().to_string();
        }
    }
    if let Some(host) = &request.host {
        if !host.trim().is_empty() {
            record.host = host.trim().to_string();
        }
    }
    if let Some(port) = request.port {
        if port > 0 {
            record.port = port;
        }
    }
    if let Some(username) = &request.username {
        if !username.trim().is_empty() {
            record.username = username.trim().to_string();
        }
    }
    if let Some(key_path) = &request.ssh_private_key_path {
        if !key_path.trim().is_empty() {
            record.ssh_private_key_path = key_path.trim().to_string();
        }
    }

    let updated_record = record.clone();
    save_fleet_enrollment_registry(registry_path.as_path(), &registry).map_err(|e| e.message)?;

    // Update runtime projection display name
    if let Ok(uuid) = uuid::Uuid::parse_str(&request.target_id) {
        state
            .fleet_control_runtime
            .update_target_projection_display_name(&uuid, &updated_record.display_name)
            .await;
    }

    append_ironclad_forensic_record(
        &state.ironclad_forensic_log,
        &app_handle,
        "fleet_enrollment",
        "info",
        format!("Fleet target updated: {}", updated_record.display_name),
        format!("target_id={}", request.target_id),
        "desktop.fleet",
    )
    .await;

    let _ = app_handle.emit(
        "fleet:target_updated",
        serde_json::json!({ "target_id": request.target_id }),
    );
    let _ = app_handle.emit(
        "ironclad:status",
        collect_ironclad_status_from_parts(
            &state.orchestrator,
            &state.ironclad_reset,
            &state.ironclad_forensic_log,
        )
        .await,
    );

    Ok(UpdateTargetResponse {
        target_id: updated_record.target_id,
        display_name: updated_record.display_name,
        host: updated_record.host,
        port: updated_record.port,
        username: updated_record.username,
        updated: true,
    })
}

#[tauri::command]
pub async fn get_ironclad_status(
    state: State<'_, AppStateCell>,
) -> Result<serde_json::Value, String> {
    let state = state
        .get()
        .ok_or_else(|| "KRIA is still initializing — please try again in a moment".to_string())?;

    let mut payload = collect_ironclad_status_from_parts(
        &state.orchestrator,
        &state.ironclad_reset,
        &state.ironclad_forensic_log,
    )
    .await;

    let (commander_host, commander_port) = {
        let cfg = state.config.read().await;
        (cfg.server.host.clone(), cfg.server.port)
    };
    let commander_base_url = local_api_base_url(&commander_host, commander_port);
    let fleet_control_targets = state.fleet_control_runtime.snapshot_targets().await;

    let (config_path, config) = load_ironclad_system_config_with_path();
    let config_summary = serde_json::json!({
        "high_recovery_slo_ms": config.qos.high_recovery_slo_ms,
        "lease_ttl_ms": config.target_pool.lease_ttl_ms,
        "heartbeat_grace_ms": config.target_pool.heartbeat_grace_ms,
        "quarantine_cooldown_ms": config.target_pool.quarantine_cooldown_ms,
        "max_normalized_hash_distance": config.snapshot.max_normalized_hash_distance,
    });

    if let Some(root) = payload.as_object_mut() {
        root.insert(
            "config_path".to_string(),
            serde_json::json!(config_path.to_string_lossy()),
        );
        root.insert("config".to_string(), config_summary);
        root.insert(
            "trust_first".to_string(),
            serde_json::json!({
                "hard_reset_confirmation": IRONCLAD_HARD_RESET_CONFIRMATION,
                "hard_reset_requires_confirmation": true,
            }),
        );

        if let Some(fleet) = root
            .get_mut("fleet")
            .and_then(|value| value.as_object_mut())
        {
            fleet.insert(
                "commander_base_url".to_string(),
                serde_json::json!(commander_base_url),
            );
            fleet.insert(
                "connection_control_wired".to_string(),
                serde_json::json!(true),
            );
            fleet.insert(
                "connection_control_target_count".to_string(),
                serde_json::json!(fleet_control_targets.len()),
            );
            fleet.insert(
                "connection_control_targets".to_string(),
                serde_json::json!(fleet_control_targets),
            );
        }
    }

    Ok(payload)
}

#[tauri::command]
pub async fn get_ironclad_forensics(
    limit: Option<usize>,
    state: State<'_, AppStateCell>,
) -> Result<serde_json::Value, String> {
    let state = state
        .get()
        .ok_or_else(|| "KRIA is still initializing — please try again in a moment".to_string())?;

    let limit = limit
        .unwrap_or(50)
        .min(IRONCLAD_FORENSIC_MAX_ENTRIES)
        .max(1);
    let guard = state.ironclad_forensic_log.read().await;
    let total = guard.len();
    let start = total.saturating_sub(limit);
    let records = guard[start..].to_vec();

    Ok(serde_json::json!({
        "total": total,
        "limit": limit,
        "records": records,
    }))
}

#[tauri::command]
pub async fn request_ironclad_soft_reset(
    reason: Option<String>,
    state: State<'_, AppStateCell>,
    app_handle: AppHandle,
) -> Result<serde_json::Value, String> {
    let state = state
        .get()
        .ok_or_else(|| "KRIA is still initializing — please try again in a moment".to_string())?;

    let reason = reason
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| "manual_soft_reset".to_string());

    let queued = enqueue_ironclad_reset(
        state.orchestrator.clone(),
        state.ironclad_reset.clone(),
        state.ironclad_forensic_log.clone(),
        app_handle,
        "soft",
        reason,
        false,
    )
    .await?;

    Ok(serde_json::json!({
        "accepted": true,
        "event_id": queued.event_id,
        "phase": queued.phase,
        "reason": queued.reason,
        "in_flight": queued.in_flight,
    }))
}

#[tauri::command]
pub async fn request_ironclad_hard_reset(
    reason: Option<String>,
    confirmation_phrase: String,
    state: State<'_, AppStateCell>,
    app_handle: AppHandle,
) -> Result<serde_json::Value, String> {
    let state = state
        .get()
        .ok_or_else(|| "KRIA is still initializing — please try again in a moment".to_string())?;

    if confirmation_phrase.trim() != IRONCLAD_HARD_RESET_CONFIRMATION {
        return Err(format!(
            "Hard reset rejected: confirmation phrase must be '{}'",
            IRONCLAD_HARD_RESET_CONFIRMATION
        ));
    }

    let reason = reason
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| "manual_hard_reset".to_string());

    let queued = enqueue_ironclad_reset(
        state.orchestrator.clone(),
        state.ironclad_reset.clone(),
        state.ironclad_forensic_log.clone(),
        app_handle,
        "hard",
        reason,
        true,
    )
    .await?;

    Ok(serde_json::json!({
        "accepted": true,
        "event_id": queued.event_id,
        "phase": queued.phase,
        "reason": queued.reason,
        "in_flight": queued.in_flight,
        "trust_first": {
            "confirmed": true,
            "phrase": IRONCLAD_HARD_RESET_CONFIRMATION,
        }
    }))
}

#[tauri::command]
pub async fn get_ironclad_config() -> Result<serde_json::Value, String> {
    let (path, config) = load_ironclad_system_config_with_path();
    Ok(serde_json::json!({
        "path": path.to_string_lossy(),
        "exists": path.exists(),
        "config": {
            "high_recovery_slo_ms": config.qos.high_recovery_slo_ms,
            "lease_ttl_ms": config.target_pool.lease_ttl_ms,
            "heartbeat_grace_ms": config.target_pool.heartbeat_grace_ms,
            "quarantine_cooldown_ms": config.target_pool.quarantine_cooldown_ms,
            "max_normalized_hash_distance": config.snapshot.max_normalized_hash_distance,
        }
    }))
}

#[tauri::command]
pub async fn update_ironclad_config(
    payload: IroncladConfigUpdatePayload,
    state: State<'_, AppStateCell>,
    app_handle: AppHandle,
) -> Result<serde_json::Value, String> {
    let state = state
        .get()
        .ok_or_else(|| "KRIA is still initializing — please try again in a moment".to_string())?;

    let (path, mut config) = load_ironclad_system_config_with_path();
    let mut applied: Vec<&'static str> = Vec::new();

    if let Some(value) = payload.high_recovery_slo_ms {
        config.qos.high_recovery_slo_ms = value.clamp(50, 300_000);
        applied.push("high_recovery_slo_ms");
    }
    if let Some(value) = payload.lease_ttl_ms {
        config.target_pool.lease_ttl_ms = value.clamp(500, 3_600_000);
        applied.push("lease_ttl_ms");
    }
    if let Some(value) = payload.heartbeat_grace_ms {
        config.target_pool.heartbeat_grace_ms = value.clamp(100, 120_000);
        applied.push("heartbeat_grace_ms");
    }
    if let Some(value) = payload.quarantine_cooldown_ms {
        config.target_pool.quarantine_cooldown_ms = value.clamp(1_000, 86_400_000);
        applied.push("quarantine_cooldown_ms");
    }
    if let Some(value) = payload.max_normalized_hash_distance {
        config.snapshot.max_normalized_hash_distance = value.clamp(0.0, 1.0);
        applied.push("max_normalized_hash_distance");
    }

    if applied.is_empty() {
        return Ok(serde_json::json!({
            "updated": false,
            "reason": "No fields provided",
        }));
    }

    persist_ironclad_system_config(path.as_path(), &config)?;

    append_ironclad_forensic_record(
        &state.ironclad_forensic_log,
        &app_handle,
        "config",
        "info",
        format!("Ironclad config updated: {}", applied.join(", ")),
        format!(
            "path={}; fields={}",
            path.to_string_lossy(),
            applied.join(",")
        ),
        "desktop.config",
    )
    .await;

    let status_payload = collect_ironclad_status_from_parts(
        &state.orchestrator,
        &state.ironclad_reset,
        &state.ironclad_forensic_log,
    )
    .await;
    let _ = app_handle.emit("ironclad:status", status_payload);

    Ok(serde_json::json!({
        "updated": true,
        "path": path.to_string_lossy(),
        "applied": applied,
        "config": {
            "high_recovery_slo_ms": config.qos.high_recovery_slo_ms,
            "lease_ttl_ms": config.target_pool.lease_ttl_ms,
            "heartbeat_grace_ms": config.target_pool.heartbeat_grace_ms,
            "quarantine_cooldown_ms": config.target_pool.quarantine_cooldown_ms,
            "max_normalized_hash_distance": config.snapshot.max_normalized_hash_distance,
        }
    }))
}
