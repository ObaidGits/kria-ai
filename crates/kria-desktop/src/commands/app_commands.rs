use super::*;

#[derive(Clone)]
struct CachedAudioDevices {
    captured_at: std::time::Instant,
    value: serde_json::Value,
}

static AUDIO_DEVICE_CACHE: std::sync::OnceLock<std::sync::Mutex<Option<CachedAudioDevices>>> =
    std::sync::OnceLock::new();

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
) -> Result<serde_json::Value, String> {
    let state = state
        .get()
        .ok_or_else(|| "KRIA is still initializing — please try again in a moment".to_string())?;
    {
        let mut gui_store = state.gui_cognition_hitl_proposals.write().await;
        let now_ms = kria_core::agent::gui_cognition::safety_hitl::now_ms();
        let _ = gui_store.expire_old_proposals(now_ms);
        if gui_store.lookup_by_request_id(&request_id).is_some() {
            let decision = gui_store.record_decision(&request_id, true, now_ms);
            return Ok(serde_json::json!({
                "status": "ok",
                "kind": "gui_cognition_hitl_decision",
                "decision": decision.summary_json(),
            }));
        }
    }
    state
        .hitl
        .respond(&request_id, ApprovalResponse::Approved)
        .await;
    Ok(serde_json::json!({"status": "ok", "kind": "generic_hitl_decision", "decision": "approved"}))
}

#[tauri::command]
pub async fn deny_action(
    request_id: String,
    reason: Option<String>,
    state: State<'_, AppStateCell>,
) -> Result<serde_json::Value, String> {
    let state = state
        .get()
        .ok_or_else(|| "KRIA is still initializing — please try again in a moment".to_string())?;
    {
        let mut gui_store = state.gui_cognition_hitl_proposals.write().await;
        let now_ms = kria_core::agent::gui_cognition::safety_hitl::now_ms();
        let _ = gui_store.expire_old_proposals(now_ms);
        if gui_store.lookup_by_request_id(&request_id).is_some() {
            let mut decision = gui_store.record_decision(&request_id, false, now_ms);
            if let Some(reason) = reason {
                decision.decision_reason =
                    Some(kria_core::agent::gui_cognition::perception::sanitize_gui_text(
                        &reason, 160,
                    ).text);
            }
            return Ok(serde_json::json!({
                "status": "ok",
                "kind": "gui_cognition_hitl_decision",
                "decision": decision.summary_json(),
            }));
        }
    }
    state
        .hitl
        .respond(&request_id, ApprovalResponse::Denied)
        .await;
    Ok(serde_json::json!({"status": "ok", "kind": "generic_hitl_decision", "decision": "denied"}))
}

#[tauri::command]
pub async fn list_interaction_decisions(
    state: State<'_, AppStateCell>,
) -> Result<serde_json::Value, String> {
    let state = state
        .get()
        .ok_or_else(|| "KRIA is still initializing — please try again in a moment".to_string())?;
    state
        .decision_store
        .refresh_from_disk()
        .map_err(|e| e.to_string())?;

    Ok(serde_json::json!({
        "decisions": state.decision_store.all_decisions(),
        "metrics": state.decision_store.metrics(),
    }))
}

#[tauri::command]
pub async fn resolve_interaction_decision(
    decision_id: String,
    option_id: String,
    decision_version: Option<u64>,
    expected_action_hash: Option<String>,
    expected_target_hash: Option<String>,
    state: State<'_, AppStateCell>,
) -> Result<serde_json::Value, String> {
    let state = state
        .get()
        .ok_or_else(|| "KRIA is still initializing — please try again in a moment".to_string())?;
    state
        .decision_store
        .refresh_from_disk()
        .map_err(|e| e.to_string())?;
    let decision = state
        .decision_store
        .decision(&decision_id)
        .ok_or_else(|| format!("Unknown interaction decision: {decision_id}"))?;
    if decision.status != kria_core::agent::collaborative_decision::DecisionStatus::Pending {
        return Err(format!(
            "Decision {decision_id} is not pending; current status is {:?}",
            decision.status
        ));
    }
    if !decision.options.iter().any(|option| option.id == option_id) {
        return Err(format!(
            "Option {option_id} is not valid for decision {decision_id}"
        ));
    }

    let expected_version = decision_version.unwrap_or(decision.version);
    let resolved = state
        .decision_store
        .resolve_with_context(
            &decision_id,
            kria_core::agent::collaborative_decision::DecisionResolutionContext {
                expected_version: Some(expected_version),
                expected_action_hash,
                expected_target_hash,
            },
            &option_id,
            "user_action_center",
        )
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("Unknown interaction decision: {decision_id}"))?;
    let resume = state
        .workflow_continuation
        .resume_workflow(&resolved.workflow_id);

    Ok(serde_json::json!({
        "status": "resolved",
        "decision": resolved,
        "resume": {
            "mode": if resume.success { "resume_prepared" } else { "decision_recorded" },
            "requires_reground": true,
            "success": resume.success,
            "summary": resume.summary,
            "next_action": format!("{:?}", resume.next_action),
            "note": "Workflow continuation must re-ground before executing any side-effecting action."
        }
    }))
}

#[tauri::command]
pub async fn resume_interaction_decision(
    decision_id: String,
    decision_version: Option<u64>,
    expected_action_hash: Option<String>,
    expected_target_hash: Option<String>,
    state: State<'_, AppStateCell>,
) -> Result<serde_json::Value, String> {
    use kria_core::agent::environment_grounder::EnvironmentGrounder;
    use kria_core::agent::execution_gate::{ExecutionGate, ResumeGateOutcome};

    let state = state
        .get()
        .ok_or_else(|| "KRIA is still initializing — please try again in a moment".to_string())?;
    state
        .decision_store
        .refresh_from_disk()
        .map_err(|e| e.to_string())?;

    let validated = state
        .decision_store
        .validate_resume_context(
            &decision_id,
            kria_core::agent::collaborative_decision::DecisionResolutionContext {
                expected_version: decision_version,
                expected_action_hash,
                expected_target_hash,
            },
            "user_action_center_resume",
        )
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("Unknown interaction decision: {decision_id}"))?;

    let grounder = kria_core::agent::environment_grounder::LiveEnvironmentGrounder::new();
    let operational_facts = grounder.ground(&[]).await;
    let resume_gate = ExecutionGate::new(
        std::sync::Arc::clone(&state.policy_engine),
        Arc::clone(&state.decision_store),
    );
    let resume_evaluation = resume_gate.revalidate_resume(&validated, false);
    let invalidation_reason = resume_evaluation.outcome.invalidation_reason();
    let can_execute = resume_evaluation.outcome.can_execute();
    let gate_status = match &resume_evaluation.outcome {
        ResumeGateOutcome::Ready => "ready",
        ResumeGateOutcome::MissingActionProposal => "missing_action_proposal",
        ResumeGateOutcome::StaleActionProposal { .. } => "stale_action_proposal",
        ResumeGateOutcome::Block { .. } => "blocked",
        ResumeGateOutcome::RiskIncreased { .. } => "risk_increased",
        ResumeGateOutcome::RequiresApproval { .. } => "requires_approval",
    };
    let gate_reason = match &resume_evaluation.outcome {
        ResumeGateOutcome::Ready => None,
        ResumeGateOutcome::MissingActionProposal => {
            Some("decision does not contain an immutable action proposal".to_string())
        }
        ResumeGateOutcome::StaleActionProposal { reason } => Some(reason.clone()),
        ResumeGateOutcome::Block { reason } => Some(reason.clone()),
        ResumeGateOutcome::RiskIncreased { reason, .. } => Some(reason.clone()),
        ResumeGateOutcome::RequiresApproval { reason, .. } => Some(reason.clone()),
    };
    let risk_change = match &resume_evaluation.outcome {
        ResumeGateOutcome::RiskIncreased {
            previous, current, ..
        } => Some(serde_json::json!({
            "previous": previous,
            "current": current,
        })),
        _ => None,
    };
    let approval_required = matches!(
        &resume_evaluation.outcome,
        ResumeGateOutcome::RequiresApproval { .. }
    );
    let recomputed_policy = resume_evaluation.policy_decision.as_ref().map(|policy| {
        serde_json::json!({
            "risk_level": policy.risk_level,
            "requires_approval": policy.requires_approval,
            "blocked": policy.blocked,
            "reason": policy.reason.clone(),
            "escalated_from": policy.escalated_from,
        })
    });

    if let Some(reason) = invalidation_reason.as_deref() {
        state
            .decision_store
            .invalidate(&validated.id, reason, "resume_gate")
            .map_err(|e| e.to_string())?;
    }

    let resume = state
        .workflow_continuation
        .resume_workflow(&validated.workflow_id);

    Ok(serde_json::json!({
        "status": if resume.success && can_execute { "resume_ready_after_reground_and_gate" } else { "resume_blocked_after_reground_or_gate" },
        "decision": validated,
        "grounding": {
            "collected": true,
            "facts": operational_facts,
        },
        "execution_gate": {
            "status": gate_status,
            "can_execute": can_execute,
            "execution_started": false,
            "reason": gate_reason,
            "risk_change": risk_change,
            "approval_required": approval_required,
            "policy": recomputed_policy,
            "resource_requirements": resume_evaluation.resource_requirements,
            "action_proposal": resume_evaluation.action_proposal,
        },
        "resume": {
            "mode": if resume.success && can_execute { "resume_ready" } else { "resume_unavailable" },
            "reground_completed": true,
            "requires_reground": false,
            "can_continue": resume.success && can_execute,
            "execution_started": false,
            "summary": resume.summary,
            "next_action": format!("{:?}", resume.next_action),
            "note": "Resume validated current decision context, re-grounded, and recomputed execution risk. Side-effecting workflow execution is intentionally not started by this command."
        }
    }))
}

#[tauri::command]
pub async fn execute_resolved_interaction_decision(
    decision_id: String,
    decision_version: Option<u64>,
    expected_action_hash: Option<String>,
    expected_target_hash: Option<String>,
    state: State<'_, AppStateCell>,
) -> Result<serde_json::Value, String> {
    let state = state
        .get()
        .ok_or_else(|| "KRIA is still initializing — please try again in a moment".to_string())?;
    let session_id = Some(state.current_session_id.read().await.clone());
    let result = state
        .resume_executor
        .execute_resolved_decision(
            kria_core::agent::resume_executor::DecisionExecutionRequest {
                decision_id,
                expected_version: decision_version,
                expected_action_hash,
                expected_target_hash,
                session_id,
                workspace_id: None,
            },
        )
        .await;

    Ok(serde_json::to_value(result).map_err(|e| e.to_string())?)
}

#[tauri::command]
pub async fn cancel_interaction_execution(
    decision_id: String,
    state: State<'_, AppStateCell>,
) -> Result<serde_json::Value, String> {
    let state = state
        .get()
        .ok_or_else(|| "KRIA is still initializing — please try again in a moment".to_string())?;
    let cancelled = state.resume_executor.cancel(&decision_id).await;
    Ok(serde_json::json!({
        "status": if cancelled { "cancel_requested" } else { "not_running" },
        "decision_id": decision_id,
        "cancelled": cancelled,
    }))
}

#[tauri::command]
pub async fn check_continuation_after_decision(
    decision_id: String,
    expected_action_hash: Option<String>,
    expected_target_hash: Option<String>,
    allow_stale_user_intent: Option<bool>,
    state: State<'_, AppStateCell>,
) -> Result<serde_json::Value, String> {
    let state = state
        .get()
        .ok_or_else(|| "KRIA is still initializing — please try again in a moment".to_string())?;
    let session_id = Some(state.current_session_id.read().await.clone());
    let result = state
        .continuation_reentry
        .check_after_decision(
            kria_core::agent::continuation_reentry::ContinuationReentryRequest {
                decision_id,
                expected_action_hash,
                expected_target_hash,
                session_id,
                workspace_id: None,
                allow_stale_user_intent: allow_stale_user_intent.unwrap_or(false),
            },
        )
        .await;
    Ok(serde_json::to_value(result).map_err(|e| e.to_string())?)
}

#[tauri::command]
pub async fn continue_after_decision_execution(
    decision_id: String,
    expected_action_hash: Option<String>,
    expected_target_hash: Option<String>,
    allow_stale_user_intent: Option<bool>,
    state: State<'_, AppStateCell>,
) -> Result<serde_json::Value, String> {
    let state = state
        .get()
        .ok_or_else(|| "KRIA is still initializing — please try again in a moment".to_string())?;
    let session_id = Some(state.current_session_id.read().await.clone());
    let result = state
        .continuation_reentry
        .continue_after_decision(
            kria_core::agent::continuation_reentry::ContinuationReentryRequest {
                decision_id,
                expected_action_hash,
                expected_target_hash,
                session_id,
                workspace_id: None,
                allow_stale_user_intent: allow_stale_user_intent.unwrap_or(false),
            },
        )
        .await;
    Ok(serde_json::to_value(result).map_err(|e| e.to_string())?)
}

#[tauri::command]
pub async fn cancel_continuation(
    decision_id: String,
    state: State<'_, AppStateCell>,
) -> Result<serde_json::Value, String> {
    let state = state
        .get()
        .ok_or_else(|| "KRIA is still initializing — please try again in a moment".to_string())?;
    let cancelled = state.continuation_reentry.cancel(&decision_id).await;
    Ok(serde_json::json!({
        "status": if cancelled { "cancel_requested" } else { "not_running" },
        "decision_id": decision_id,
        "cancelled": cancelled,
    }))
}

#[tauri::command]
pub async fn cancel_interaction_decision(
    decision_id: String,
    state: State<'_, AppStateCell>,
) -> Result<serde_json::Value, String> {
    let state = state
        .get()
        .ok_or_else(|| "KRIA is still initializing — please try again in a moment".to_string())?;
    state
        .decision_store
        .refresh_from_disk()
        .map_err(|e| e.to_string())?;
    let expired = state
        .decision_store
        .expire(&decision_id, "user_action_center")
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("Unknown interaction decision: {decision_id}"))?;

    Ok(serde_json::json!({
        "status": "cancelled",
        "decision": expired,
    }))
}

#[tauri::command]
pub async fn replay_interaction_decisions(
    state: State<'_, AppStateCell>,
) -> Result<serde_json::Value, String> {
    let state = state
        .get()
        .ok_or_else(|| "KRIA is still initializing — please try again in a moment".to_string())?;
    state
        .decision_store
        .refresh_from_disk()
        .map_err(|e| e.to_string())?;

    Ok(serde_json::json!({
        "events": state.decision_store.events(),
        "metrics": state.decision_store.metrics(),
    }))
}

#[tauri::command]
pub async fn get_runtime_diagnostics(
    limit: Option<usize>,
    min_level: Option<String>,
) -> Result<serde_json::Value, String> {
    let limit = limit.unwrap_or(128).clamp(1, 512);
    let min_level = min_level.unwrap_or_else(|| "info".to_string());
    Ok(serde_json::json!({
        "summary": kria_core::infra::diagnostics::diagnostics_summary(),
        "events": kria_core::infra::diagnostics::recent_diagnostics(limit, Some(&min_level)),
    }))
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
            "diagnostics": {
                "summary": kria_core::infra::diagnostics::diagnostics_summary()
            },
            "hardware": {}
        }));
    };
    // Refresh LLM server health on each call
    let mr_status = state.model_router.status().await;
    let mr_healthy = mr_status["active_healthy"]
        .as_bool()
        .or_else(|| mr_status["local_healthy"].as_bool())
        .unwrap_or(false);
    let mr_model = mr_status["active_model"]
        .as_str()
        .or_else(|| mr_status["local_model"].as_str())
        .unwrap_or("unknown");
    let mr_provider = mr_status["active_provider"]
        .as_str()
        .unwrap_or_else(|| mr_status["mode"].as_str().unwrap_or("unknown"));
    if mr_healthy {
        state.health.update(
            "model_router",
            ServiceStatus::Healthy,
            Some(format!("{}: {}", mr_provider, mr_model)),
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

    // Refresh GUI cognition sidecar/substrate readiness so the UI can expose
    // why GUI actions are degraded before a workflow reaches the executor.
    kria_core::agent::gui_services::refresh_gui_service_health(&state.health);
    let gui_readiness = kria_core::agent::gui_production_readiness::assess_gui_production_readiness(
        kria_core::agent::gui_production_readiness::GuiReadinessMode::LiveDesktop,
    );

    let health_snapshot = state.health.snapshot();
    let uptime = state.started_at.elapsed().as_secs();
    let tool_count = state.tool_registry.len();
    let hw = &state.hardware_info;
    let status = health_snapshot.status.clone();
    let event_count = health_snapshot.event_count;
    let services = health_snapshot.services.clone();

    Ok(serde_json::json!({
        "status": status,
        "uptime_secs": uptime,
        "tool_count": tool_count,
        "event_count": event_count,
        "services": services,
        "health_snapshot": health_snapshot,
        "gui_readiness": gui_readiness,
        "diagnostics": {
            "summary": kria_core::infra::diagnostics::diagnostics_summary(),
            "recent": kria_core::infra::diagnostics::recent_diagnostics(25, Some("warn")),
        },
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
    let mut redacted = config.clone();
    redacted.llm.cloud_api_key.clear();
    for provider in &mut redacted.providers.providers {
        provider.endpoint.api_key.clear();
    }
    serde_json::to_value(&redacted).map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn list_audio_devices() -> Result<serde_json::Value, String> {
    let cache = AUDIO_DEVICE_CACHE.get_or_init(|| std::sync::Mutex::new(None));
    if let Ok(guard) = cache.lock() {
        if let Some(cached) = guard.as_ref() {
            if cached.captured_at.elapsed() < std::time::Duration::from_secs(30) {
                return Ok(cached.value.clone());
            }
        }
    }

    let inputs = list_input_devices().unwrap_or_default();
    let outputs = list_output_devices().unwrap_or_default();
    let value = serde_json::json!({
        "inputs": inputs,
        "outputs": outputs,
        "default_input": default_input_device_name(),
        "default_output": default_output_device_name(),
    });

    if let Ok(mut guard) = cache.lock() {
        *guard = Some(CachedAudioDevices {
            captured_at: std::time::Instant::now(),
            value: value.clone(),
        });
    }

    Ok(value)
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
    {
        let current = state.config.read().await;
        // Settings modal saves can carry a stale full-config draft. Provider/model
        // selection is applied through the runtime apply service, so generic saves
        // must not silently roll the live runtime back to an older draft.
        new_config.providers = current.providers.clone();
        new_config.llm.active_model = current.llm.active_model.clone();
        new_config.llm.local_api_url = current.llm.local_api_url.clone();
        new_config.llm.cloud_provider = current.llm.cloud_provider.clone();
        new_config.llm.cloud_api_key = current.llm.cloud_api_key.clone();
        new_config.llm.cloud_model_id = current.llm.cloud_model_id.clone();
        new_config.llm.cloud_endpoint = current.llm.cloud_endpoint.clone();
        new_config.llm.routing_mode = current.llm.routing_mode.clone();
        new_config.llm.models = current.llm.models.clone();
    }
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
    let models = list_resolvable_local_llm_models(&config, &paths);
    Ok(serde_json::to_value(&models).unwrap_or_default())
}

fn list_resolvable_local_llm_models(
    config: &kria_core::config::KriaConfig,
    paths: &kria_core::platform::paths::KriaPaths,
) -> Vec<serde_json::Value> {
    use std::collections::HashSet;
    use std::path::{Path, PathBuf};

    fn add_workspace_model_dirs(start: Option<PathBuf>, dirs: &mut Vec<PathBuf>) {
        let Some(start) = start else { return };
        let mut cursor = Some(start.as_path());
        while let Some(dir) = cursor {
            dirs.push(dir.join("models").join("llm"));
            cursor = dir.parent();
            if cursor.map(|path| path == Path::new("/")).unwrap_or(true) {
                break;
            }
        }
    }

    fn model_dirs(paths: &kria_core::platform::paths::KriaPaths) -> Vec<PathBuf> {
        let mut dirs = vec![paths.llm_models.clone()];
        add_workspace_model_dirs(std::env::current_dir().ok(), &mut dirs);
        add_workspace_model_dirs(
            std::env::current_exe()
                .ok()
                .and_then(|path| path.parent().map(Path::to_path_buf)),
            &mut dirs,
        );

        let mut seen = HashSet::new();
        dirs.into_iter()
            .filter(|dir| seen.insert(dir.clone()))
            .collect()
    }

    fn resolve_in_dirs(file: &str, dirs: &[PathBuf]) -> Option<PathBuf> {
        let direct = PathBuf::from(file);
        if direct.is_absolute() && direct.exists() {
            return Some(direct);
        }

        let candidates = if file.to_ascii_lowercase().ends_with(".gguf") {
            vec![file.to_string()]
        } else {
            vec![file.to_string(), format!("{file}.gguf")]
        };

        for dir in dirs {
            for candidate in &candidates {
                let path = dir.join(candidate);
                if path.exists() {
                    return Some(path);
                }
            }
        }

        None
    }

    fn file_size(path: &Path) -> u64 {
        std::fs::metadata(path)
            .map(|metadata| metadata.len())
            .unwrap_or(0)
    }

    fn is_auxiliary_gguf(path: &Path) -> bool {
        path.file_name()
            .and_then(|name| name.to_str())
            .map(|name| name.to_ascii_lowercase().contains("mmproj"))
            .unwrap_or(false)
    }

    let dirs = model_dirs(paths);
    let mut seen_files = HashSet::<String>::new();
    let mut models = Vec::new();

    for model in &config.llm.models {
        let resolved = resolve_in_dirs(&model.file, &dirs);
        let exists = resolved.is_some();
        let path_string = resolved
            .as_ref()
            .map(|path| path.to_string_lossy().to_string())
            .unwrap_or_else(|| model.file.clone());
        if let Some(path) = resolved.as_ref() {
            seen_files.insert(
                path.canonicalize()
                    .unwrap_or_else(|_| path.clone())
                    .to_string_lossy()
                    .to_string(),
            );
        }

        models.push(serde_json::json!({
            "name": model.name,
            "display_name": model.display_name,
            "file": model.file,
            "path": path_string,
            "size_bytes": resolved.as_ref().map(|path| file_size(path)).unwrap_or(0),
            "configured": true,
            "exists": exists,
            "source": if exists { "configured" } else { "missing_configured_file" },
            "capabilities": model.capabilities,
            "mmproj_file": model.mmproj_file,
        }));
    }

    for dir in dirs {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|ext| ext.to_str()) != Some("gguf") {
                continue;
            }
            if is_auxiliary_gguf(&path) {
                continue;
            }
            let canonical = path
                .canonicalize()
                .unwrap_or_else(|_| path.clone())
                .to_string_lossy()
                .to_string();
            if !seen_files.insert(canonical) {
                continue;
            }

            let file = path
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or_default()
                .to_string();
            let stem = path
                .file_stem()
                .and_then(|name| name.to_str())
                .unwrap_or(&file)
                .to_string();

            models.push(serde_json::json!({
                "name": stem,
                "display_name": stem,
                "file": file,
                "path": path.to_string_lossy().to_string(),
                "size_bytes": file_size(&path),
                "configured": false,
                "exists": true,
                "source": "detected_gguf",
                "capabilities": ["chat"],
                "mmproj_file": null,
            }));
        }
    }

    models.sort_by(|a, b| {
        let a_missing = !a["exists"].as_bool().unwrap_or(false);
        let b_missing = !b["exists"].as_bool().unwrap_or(false);
        let a_configured = a["configured"].as_bool().unwrap_or(false);
        let b_configured = b["configured"].as_bool().unwrap_or(false);
        a_missing
            .cmp(&b_missing)
            .then_with(|| b_configured.cmp(&a_configured))
            .then_with(|| {
                a["display_name"]
                    .as_str()
                    .unwrap_or_default()
                    .to_ascii_lowercase()
                    .cmp(
                        &b["display_name"]
                            .as_str()
                            .unwrap_or_default()
                            .to_ascii_lowercase(),
                    )
            })
    });

    models
}
