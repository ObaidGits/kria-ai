use super::*;

// ── Automation Commands ─────────────────────────────────────────────

#[tauri::command]
pub async fn list_scheduled_tasks(
    state: State<'_, AppStateCell>,
) -> Result<serde_json::Value, String> {
    let state = state
        .get()
        .ok_or_else(|| "KRIA is still initializing — please try again in a moment".to_string())?;
    let scheduler = state.scheduler.read().await;
    let tasks: Vec<serde_json::Value> = scheduler
        .list_tasks()
        .iter()
        .map(|t| {
            serde_json::json!({
                "id": t.id,
                "name": t.name,
                "interval_secs": t.interval_secs,
                "prompt": t.prompt,
                "enabled": t.enabled,
            })
        })
        .collect();
    Ok(serde_json::json!(tasks))
}

#[tauri::command]
pub async fn add_scheduled_task(
    name: String,
    interval_secs: u64,
    prompt: String,
    state: State<'_, AppStateCell>,
) -> Result<serde_json::Value, String> {
    let state = state
        .get()
        .ok_or_else(|| "KRIA is still initializing — please try again in a moment".to_string())?;
    use kria_core::automation::scheduler::ScheduledTask;

    let id = uuid::Uuid::new_v4().to_string();
    let task = ScheduledTask {
        id: id.clone(),
        name,
        interval_secs,
        prompt,
        enabled: true,
    };

    let mut scheduler = state.scheduler.write().await;
    scheduler.add_task(task);
    Ok(serde_json::json!({"id": id}))
}

#[tauri::command]
pub async fn remove_scheduled_task(
    task_id: String,
    state: State<'_, AppStateCell>,
) -> Result<(), String> {
    let state = state
        .get()
        .ok_or_else(|| "KRIA is still initializing — please try again in a moment".to_string())?;
    let mut scheduler = state.scheduler.write().await;
    scheduler.remove_task(&task_id);
    Ok(())
}

#[tauri::command]
pub async fn list_macros(state: State<'_, AppStateCell>) -> Result<serde_json::Value, String> {
    let state = state
        .get()
        .ok_or_else(|| "KRIA is still initializing — please try again in a moment".to_string())?;
    let recorder = state.macro_recorder.read().await;
    let macros: Vec<serde_json::Value> = recorder
        .list()
        .iter()
        .map(|m| {
            serde_json::json!({
                "name": m.name,
                "description": m.description,
                "step_count": m.steps.len(),
                "created_at": m.created_at,
            })
        })
        .collect();
    Ok(serde_json::json!(macros))
}

#[tauri::command]
pub async fn start_macro_recording(
    name: String,
    state: State<'_, AppStateCell>,
) -> Result<(), String> {
    let state = state
        .get()
        .ok_or_else(|| "KRIA is still initializing — please try again in a moment".to_string())?;
    let mut recorder = state.macro_recorder.write().await;
    recorder.start_recording(&name);
    Ok(())
}

#[tauri::command]
pub async fn stop_macro_recording(
    description: String,
    state: State<'_, AppStateCell>,
) -> Result<serde_json::Value, String> {
    let state = state
        .get()
        .ok_or_else(|| "KRIA is still initializing — please try again in a moment".to_string())?;
    let mut recorder = state.macro_recorder.write().await;
    match recorder.stop_recording(&description) {
        Some(m) => Ok(serde_json::json!({
            "name": m.name,
            "steps": m.steps.len(),
        })),
        None => Err("No recording in progress".into()),
    }
}

#[tauri::command]
pub async fn delete_macro(name: String, state: State<'_, AppStateCell>) -> Result<(), String> {
    let state = state
        .get()
        .ok_or_else(|| "KRIA is still initializing — please try again in a moment".to_string())?;
    let mut recorder = state.macro_recorder.write().await;
    if recorder.delete(&name) {
        Ok(())
    } else {
        Err(format!("Macro '{}' not found", name))
    }
}

#[tauri::command]
pub async fn list_workflows(state: State<'_, AppStateCell>) -> Result<serde_json::Value, String> {
    let state = state
        .get()
        .ok_or_else(|| "KRIA is still initializing — please try again in a moment".to_string())?;
    let engine = state.workflow_engine.read().await;
    let workflows: Vec<serde_json::Value> = engine
        .list()
        .iter()
        .map(|w| {
            serde_json::json!({
                "id": w.id,
                "name": w.name,
                "description": w.description,
                "step_count": w.steps.len(),
                "created_at": w.created_at,
            })
        })
        .collect();
    Ok(serde_json::json!(workflows))
}

#[tauri::command]
pub async fn delete_workflow(
    workflow_id: String,
    state: State<'_, AppStateCell>,
) -> Result<(), String> {
    let state = state
        .get()
        .ok_or_else(|| "KRIA is still initializing — please try again in a moment".to_string())?;
    let mut engine = state.workflow_engine.write().await;
    if engine.delete(&workflow_id) {
        Ok(())
    } else {
        Err(format!("Workflow '{}' not found", workflow_id))
    }
}
