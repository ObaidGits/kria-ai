use super::*;

pub(super) fn summarize_colab_dispatch_reason(status_payload: &serde_json::Value) -> String {
    let mut reasons: Vec<String> = status_payload
        .get("warnings")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str())
                .map(|s| s.to_string())
                .collect()
        })
        .unwrap_or_default();

    let missing: Vec<String> = status_payload
        .get("capabilities")
        .and_then(|v| v.get("ready_requirements"))
        .and_then(|v| v.get("missing"))
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str())
                .map(|s| s.to_string())
                .collect()
        })
        .unwrap_or_default();

    if !missing.is_empty() {
        reasons.push(format!("missing capabilities: {}", missing.join(", ")));
    }

    if reasons.is_empty() {
        let runtime_state = status_payload
            .get("runtime_state")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown");
        reasons.push(format!("runtime_state={runtime_state}"));
    }

    reasons.join("; ")
}

pub(super) async fn enforce_colab_dispatch_requirements(
    state: &AppState,
    app: &AppHandle,
) -> Result<(), String> {
    let requested_mode = {
        let config = state.config.read().await;
        config
            .llm
            .routing_mode
            .parse::<RoutingMode>()
            .unwrap_or(RoutingMode::Local)
    };

    state.model_router.set_mode(requested_mode).await;

    if requested_mode != RoutingMode::Colab {
        return Ok(());
    }

    let status_payload = collect_colab_tier_status(state).await;
    let ready_for_cloud_task = status_payload
        .get("ready_for_cloud_task")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);

    if ready_for_cloud_task {
        emit_agent_stage(
            app,
            "colab_dispatch_ready",
            "Colab tier requirements are satisfied",
            Some(serde_json::json!({
                "requested_mode": "colab",
                "effective_mode": "colab",
                "ready_for_cloud_task": true,
            })),
        );
        emit_colab_status_event(app, state).await;
        return Ok(());
    }

    let reason = summarize_colab_dispatch_reason(&status_payload);
    let fallback_to_local = status_payload
        .get("fallback_to_local")
        .and_then(|v| v.as_bool())
        .unwrap_or(true);

    let runtime_state = status_payload
        .get("runtime_state")
        .cloned()
        .unwrap_or(serde_json::Value::Null);
    let capability_requirements = status_payload
        .get("capabilities")
        .and_then(|v| v.get("ready_requirements"))
        .cloned()
        .unwrap_or(serde_json::Value::Null);

    if fallback_to_local {
        state.model_router.set_mode(RoutingMode::Local).await;

        emit_agent_stage(
            app,
            "colab_dispatch_fallback_local",
            "Colab tier requirements were not satisfied; using local fallback",
            Some(serde_json::json!({
                "reason": reason,
                "runtime_state": runtime_state,
                "capability_requirements": capability_requirements,
                "requested_mode": "colab",
                "effective_mode": "local",
                "ready_for_cloud_task": false,
                "fallback_to_local": fallback_to_local,
            })),
        );

        emit_colab_status_event(app, state).await;

        tracing::warn!(
            reason = %reason,
            "colab dispatch requirements not satisfied; using local fallback"
        );
        Ok(())
    } else {
        emit_agent_stage(
            app,
            "colab_dispatch_blocked",
            "Colab tier requirements were not satisfied and fallback is disabled",
            Some(serde_json::json!({
                "reason": reason,
                "runtime_state": runtime_state,
                "capability_requirements": capability_requirements,
                "requested_mode": "colab",
                "effective_mode": "colab",
                "ready_for_cloud_task": false,
                "fallback_to_local": fallback_to_local,
            })),
        );

        emit_colab_status_event(app, state).await;

        Err(format!(
            "Colab tier is not ready for cloud execution and local fallback is disabled: {}",
            reason
        ))
    }
}
