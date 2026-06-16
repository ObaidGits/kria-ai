use super::*;

#[tauri::command]
pub async fn get_session_history(
    session_id: Option<String>,
    state: State<'_, AppStateCell>,
) -> Result<Vec<serde_json::Value>, String> {
    let state = state
        .get()
        .ok_or_else(|| "KRIA is still initializing — please try again in a moment".to_string())?;
    let session_id = match session_id {
        Some(id) if !id.trim().is_empty() => id,
        _ => state.current_session_id.read().await.clone(),
    };
    let turns = state
        .memory_store
        .get_recent_turns(&session_id, 100)
        .map_err(|e| e.to_string())?;
    let messages: Vec<serde_json::Value> = turns
        .iter()
        .map(|t| {
            serde_json::json!({
                "role": t.role,
                "content": t.content,
                "tool_name": t.tool_name,
                "tool_result": t.tool_result,
                "timestamp": t.timestamp.to_rfc3339(),
            })
        })
        .collect();
    Ok(messages)
}

/// Remove rows that repeat a session id, keeping the first occurrence.
///
/// Acts as a backend safety net so the session list never carries a duplicate
/// id regardless of how the synthetic "current" row is injected. Rows without a
/// usable id are passed through unchanged.
fn dedupe_session_rows(rows: Vec<serde_json::Value>) -> Vec<serde_json::Value> {
    let mut seen = std::collections::HashSet::new();
    let mut out = Vec::with_capacity(rows.len());
    for row in rows {
        let id = row
            .get("id")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        if id.is_empty() || seen.insert(id) {
            out.push(row);
        }
    }
    out
}

fn normalize_session_title(raw: &str) -> Option<String> {
    const SESSION_TITLE_MAX_CHARS: usize = 72;

    let collapsed = raw.split_whitespace().collect::<Vec<_>>().join(" ");
    let trimmed = collapsed.trim();
    if trimmed.is_empty() {
        return None;
    }

    let mut title: String = trimmed.chars().take(SESSION_TITLE_MAX_CHARS).collect();
    if trimmed.chars().count() > SESSION_TITLE_MAX_CHARS {
        title.push('…');
    }
    Some(title)
}

#[tauri::command]
pub async fn create_session(
    title: Option<String>,
    state: State<'_, AppStateCell>,
) -> Result<serde_json::Value, String> {
    let state = state
        .get()
        .ok_or_else(|| "KRIA is still initializing — please try again in a moment".to_string())?;
    let new_id = uuid::Uuid::new_v4().to_string();
    *state.current_session_id.write().await = new_id.clone();

    // Store metadata preferences so empty sessions are still visible in the UI.
    let provided_title = title.as_deref().and_then(normalize_session_title);
    let resolved_title = provided_title
        .clone()
        .unwrap_or_else(|| "New chat".to_string());
    let memory_writer: Arc<dyn MemoryManager> = state.memory_store.clone();
    let _ = memory_writer.set_preference(&preference_record(
        format!("session_title:{}", new_id),
        resolved_title,
    ));
    let _ = memory_writer.set_preference(&preference_record(
        format!("session_title_manual:{}", new_id),
        if provided_title.is_some() {
            "1".to_string()
        } else {
            "0".to_string()
        },
    ));
    let _ = memory_writer.set_preference(&preference_record(
        format!("session_created_at:{}", new_id),
        Utc::now().to_rfc3339(),
    ));

    tracing::info!(session_id = %new_id, "new session created");
    Ok(serde_json::json!({
        "session_id": new_id,
    }))
}

#[tauri::command]
pub async fn list_sessions(
    state: State<'_, AppStateCell>,
) -> Result<Vec<serde_json::Value>, String> {
    let state = state
        .get()
        .ok_or_else(|| "KRIA is still initializing — please try again in a moment".to_string())?;
    let sessions = state
        .memory_store
        .list_sessions()
        .map_err(|e| e.to_string())?;
    let current = state.current_session_id.read().await.clone();
    let mut result: Vec<serde_json::Value> = sessions
        .into_iter()
        .map(|(id, count, last_active)| {
            let title = state
                .memory_store
                .get_preference(&format!("session_title:{}", id))
                .unwrap_or(None)
                .unwrap_or_else(|| format!("Session ({})", &id[..8]));
            let pinned = state
                .memory_store
                .get_preference(&format!("session_pinned:{}", id))
                .unwrap_or(None)
                .as_deref()
                == Some("1");
            let archived = state
                .memory_store
                .get_preference(&format!("session_archived:{}", id))
                .unwrap_or(None)
                .as_deref()
                == Some("1");
            serde_json::json!({
                "id": id,
                "title": title,
                "turn_count": count,
                "message_count": count,
                "last_active": last_active,
                "is_current": id == current,
                "pinned": pinned,
                "archived": archived,
            })
        })
        .collect();

    // Include the current session even when it has no turns yet.
    if !current.trim().is_empty()
        && !result
            .iter()
            .any(|row| row.get("id").and_then(|v| v.as_str()) == Some(current.as_str()))
    {
        let title = state
            .memory_store
            .get_preference(&format!("session_title:{}", current))
            .unwrap_or(None)
            .unwrap_or_else(|| "New chat".to_string());
        let created_at = state
            .memory_store
            .get_preference(&format!("session_created_at:{}", current))
            .unwrap_or(None)
            .unwrap_or_else(|| Utc::now().to_rfc3339());
        result.insert(
            0,
            serde_json::json!({
                "id": current,
                "title": title,
                "turn_count": 0,
                "message_count": 0,
                "last_active": created_at,
                "is_current": true,
                "pinned": false,
                "archived": false,
            }),
        );
    }

    if chat_flag_enabled("KRIA_CHAT_COHERENT_SESSIONS") {
        result = dedupe_session_rows(result);
    }

    Ok(result)
}

#[tauri::command]
pub async fn switch_session(
    session_id: String,
    state: State<'_, AppStateCell>,
) -> Result<serde_json::Value, String> {
    let state = state
        .get()
        .ok_or_else(|| "KRIA is still initializing — please try again in a moment".to_string())?;
    *state.current_session_id.write().await = session_id.clone();
    // Load history for the new session
    let turns = state
        .memory_store
        .get_recent_turns(&session_id, 100)
        .map_err(|e| e.to_string())?;
    let messages: Vec<serde_json::Value> = turns
        .iter()
        .map(|t| {
            serde_json::json!({
                "role": t.role,
                "content": t.content,
                "tool_name": t.tool_name,
                "tool_result": t.tool_result,
                "timestamp": t.timestamp.to_rfc3339(),
            })
        })
        .collect();
    Ok(serde_json::json!({
        "session_id": session_id,
        "messages": messages,
    }))
}

#[tauri::command]
pub async fn delete_session(
    session_id: String,
    state: State<'_, AppStateCell>,
) -> Result<serde_json::Value, String> {
    let state = state
        .get()
        .ok_or_else(|| "KRIA is still initializing — please try again in a moment".to_string())?;
    let session_id = session_id.trim().to_string();
    if session_id.is_empty() {
        return Err("Session id cannot be empty".into());
    }

    let current = state.current_session_id.read().await.clone();
    state
        .memory_store
        .delete_session(&session_id)
        .map_err(|e| e.to_string())?;
    let memory_writer: Arc<dyn MemoryManager> = state.memory_store.clone();

    // Best-effort cleanup of session-scoped preferences so deleting a chat
    // leaves no orphaned title/flag/pin/archive metadata behind. Never block the
    // delete on a preference failure.
    if chat_flag_enabled("KRIA_CHAT_PREF_CLEANUP") {
        if let Err(e) = memory_writer.delete_session_preferences(&session_id) {
            tracing::warn!(session_id = %session_id, error = %e, "failed to clean session preferences on delete");
        }
    }

    let mut replacement_session_id: Option<String> = None;

    // If we deleted the current session, create a new one
    if session_id == current {
        let new_id = uuid::Uuid::new_v4().to_string();
        *state.current_session_id.write().await = new_id.clone();

        let _ = memory_writer.set_preference(&preference_record(
            format!("session_title:{}", new_id),
            "New chat",
        ));
        let _ = memory_writer.set_preference(&preference_record(
            format!("session_title_manual:{}", new_id),
            "0",
        ));
        let _ = memory_writer.set_preference(&preference_record(
            format!("session_created_at:{}", new_id),
            Utc::now().to_rfc3339(),
        ));

        replacement_session_id = Some(new_id);
    }

    Ok(serde_json::json!({
        "deleted_session_id": session_id,
        "replacement_session_id": replacement_session_id,
    }))
}

#[tauri::command]
pub async fn clear_all_chat_sessions(
    state: State<'_, AppStateCell>,
) -> Result<serde_json::Value, String> {
    let state = state
        .get()
        .ok_or_else(|| "KRIA is still initializing — please try again in a moment".to_string())?;
    let sessions = state
        .memory_store
        .list_sessions()
        .map_err(|e| e.to_string())?;
    let deleted_session_count = sessions.len();
    let mut deleted_turn_count = 0_usize;

    let cleanup_prefs = chat_flag_enabled("KRIA_CHAT_PREF_CLEANUP");
    let memory_writer: Arc<dyn MemoryManager> = state.memory_store.clone();

    for (session_id, _, _) in sessions {
        deleted_turn_count += state
            .memory_store
            .delete_session(&session_id)
            .map_err(|e| e.to_string())?;
        if cleanup_prefs {
            if let Err(e) = memory_writer.delete_session_preferences(&session_id) {
                tracing::warn!(session_id = %session_id, error = %e, "failed to clean session preferences on clear-all");
            }
        }
    }

    let new_id = uuid::Uuid::new_v4().to_string();
    *state.current_session_id.write().await = new_id.clone();

    let _ = memory_writer.set_preference(&preference_record(
        format!("session_title:{}", new_id),
        "New chat",
    ));
    let _ = memory_writer.set_preference(&preference_record(
        format!("session_title_manual:{}", new_id),
        "0",
    ));
    let _ = memory_writer.set_preference(&preference_record(
        format!("session_created_at:{}", new_id),
        Utc::now().to_rfc3339(),
    ));

    tracing::info!(
        deleted_session_count,
        deleted_turn_count,
        replacement_session_id = %new_id,
        "all chat sessions cleared"
    );

    Ok(serde_json::json!({
        "deleted_session_count": deleted_session_count,
        "deleted_turn_count": deleted_turn_count,
        "replacement_session_id": new_id,
    }))
}

#[tauri::command]
pub async fn rename_session(
    session_id: String,
    title: String,
    state: State<'_, AppStateCell>,
) -> Result<(), String> {
    let state = state
        .get()
        .ok_or_else(|| "KRIA is still initializing — please try again in a moment".to_string())?;
    let session_id = session_id.trim().to_string();
    if session_id.is_empty() {
        return Err("Session id cannot be empty".into());
    }

    let resolved_title = normalize_session_title(&title)
        .ok_or_else(|| "Session title cannot be empty".to_string())?;

    let key = format!("session_title:{}", session_id);
    let memory_writer: Arc<dyn MemoryManager> = state.memory_store.clone();
    memory_writer
        .set_preference(&preference_record(key, resolved_title))
        .map_err(|e| e.to_string())?;
    memory_writer
        .set_preference(&preference_record(
            format!("session_title_manual:{}", session_id),
            "1",
        ))
        .map_err(|e| e.to_string())?;
    Ok(())
}

#[tauri::command]
pub async fn auto_rename_session(
    session_id: String,
    title: String,
    state: State<'_, AppStateCell>,
) -> Result<serde_json::Value, String> {
    let state = state
        .get()
        .ok_or_else(|| "KRIA is still initializing — please try again in a moment".to_string())?;
    let session_id = session_id.trim().to_string();
    if session_id.is_empty() {
        return Err("Session id cannot be empty".into());
    }

    let resolved_title = match normalize_session_title(&title) {
        Some(t) => t,
        None => {
            return Ok(serde_json::json!({
                "updated": false,
                "reason": "empty_title",
            }))
        }
    };

    let manual_key = format!("session_title_manual:{}", session_id);
    let manual_flag = state
        .memory_store
        .get_preference(&manual_key)
        .map_err(|e| e.to_string())?
        .unwrap_or_else(|| "0".to_string());

    if manual_flag == "1" {
        let existing_title = state
            .memory_store
            .get_preference(&format!("session_title:{}", session_id))
            .map_err(|e| e.to_string())?
            .unwrap_or_else(|| "New chat".to_string());

        return Ok(serde_json::json!({
            "updated": false,
            "reason": "manual_title",
            "title": existing_title,
        }));
    }

    let memory_writer: Arc<dyn MemoryManager> = state.memory_store.clone();
    memory_writer
        .set_preference(&preference_record(
            format!("session_title:{}", session_id),
            resolved_title.clone(),
        ))
        .map_err(|e| e.to_string())?;
    memory_writer
        .set_preference(&preference_record(manual_key, "0"))
        .map_err(|e| e.to_string())?;

    Ok(serde_json::json!({
        "updated": true,
        "title": resolved_title,
    }))
}

#[tauri::command]
pub async fn search_sessions(
    query: String,
    state: State<'_, AppStateCell>,
) -> Result<Vec<serde_json::Value>, String> {
    let state = state
        .get()
        .ok_or_else(|| "KRIA is still initializing — please try again in a moment".to_string())?;
    let results = state
        .memory_store
        .search_conversations(&query, 20)
        .map_err(|e| e.to_string())?;
    let items: Vec<serde_json::Value> = results
        .into_iter()
        .map(|t| {
            serde_json::json!({
                "session_id": t.session_id,
                "role": t.role,
                "content": t.content,
                "timestamp": t.timestamp.to_rfc3339(),
            })
        })
        .collect();
    Ok(items)
}

#[tauri::command]
pub async fn set_session_pinned(
    session_id: String,
    pinned: bool,
    state: State<'_, AppStateCell>,
) -> Result<(), String> {
    let state = state
        .get()
        .ok_or_else(|| "KRIA is still initializing — please try again in a moment".to_string())?;
    let session_id = session_id.trim().to_string();
    if session_id.is_empty() {
        return Err("Session id cannot be empty".into());
    }
    let memory_writer: Arc<dyn MemoryManager> = state.memory_store.clone();
    memory_writer
        .set_preference(&preference_record(
            format!("session_pinned:{}", session_id),
            if pinned { "1" } else { "0" },
        ))
        .map_err(|e| e.to_string())?;
    Ok(())
}

#[tauri::command]
pub async fn set_session_archived(
    session_id: String,
    archived: bool,
    state: State<'_, AppStateCell>,
) -> Result<(), String> {
    let state = state
        .get()
        .ok_or_else(|| "KRIA is still initializing — please try again in a moment".to_string())?;
    let session_id = session_id.trim().to_string();
    if session_id.is_empty() {
        return Err("Session id cannot be empty".into());
    }
    let memory_writer: Arc<dyn MemoryManager> = state.memory_store.clone();
    memory_writer
        .set_preference(&preference_record(
            format!("session_archived:{}", session_id),
            if archived { "1" } else { "0" },
        ))
        .map_err(|e| e.to_string())?;
    Ok(())
}

#[tauri::command]
pub async fn set_session_temporary(
    session_id: String,
    temporary: bool,
    state: State<'_, AppStateCell>,
) -> Result<(), String> {
    let state = state
        .get()
        .ok_or_else(|| "KRIA is still initializing — please try again in a moment".to_string())?;
    let session_id = session_id.trim().to_string();
    if session_id.is_empty() {
        return Err("Session id cannot be empty".into());
    }
    let memory_writer: Arc<dyn MemoryManager> = state.memory_store.clone();
    memory_writer
        .set_preference(&preference_record(
            format!("session_temporary:{}", session_id),
            if temporary { "1" } else { "0" },
        ))
        .map_err(|e| e.to_string())?;
    Ok(())
}

#[tauri::command]
pub async fn get_memory_enabled(state: State<'_, AppStateCell>) -> Result<bool, String> {
    let state = state
        .get()
        .ok_or_else(|| "KRIA is still initializing — please try again in a moment".to_string())?;
    // Default ON: only an explicit "0" disables long-term memory writes.
    let enabled = state
        .memory_store
        .get_preference("memory_enabled")
        .map_err(|e| e.to_string())?
        .as_deref()
        != Some("0");
    Ok(enabled)
}

#[tauri::command]
pub async fn set_memory_enabled(
    enabled: bool,
    state: State<'_, AppStateCell>,
) -> Result<(), String> {
    let state = state
        .get()
        .ok_or_else(|| "KRIA is still initializing — please try again in a moment".to_string())?;
    let memory_writer: Arc<dyn MemoryManager> = state.memory_store.clone();
    memory_writer
        .set_preference(&preference_record(
            "memory_enabled",
            if enabled { "1" } else { "0" },
        ))
        .map_err(|e| e.to_string())?;
    Ok(())
}

#[cfg(test)]
mod dedupe_tests {
    use super::dedupe_session_rows;

    fn row(id: &str) -> serde_json::Value {
        serde_json::json!({ "id": id, "title": id })
    }

    #[test]
    fn dedupe_keeps_first_occurrence_only() {
        let rows = vec![row("a"), row("b"), row("a"), row("c"), row("b")];
        let out = dedupe_session_rows(rows);
        let ids: Vec<&str> = out
            .iter()
            .map(|r| r.get("id").and_then(|v| v.as_str()).unwrap())
            .collect();
        assert_eq!(ids, vec!["a", "b", "c"]);
    }

    #[test]
    fn dedupe_passes_through_unique_rows() {
        let rows = vec![row("x"), row("y")];
        let out = dedupe_session_rows(rows);
        assert_eq!(out.len(), 2);
    }

    #[test]
    fn dedupe_keeps_rows_without_id() {
        let rows = vec![serde_json::json!({ "title": "no-id" }), row("a"), row("a")];
        let out = dedupe_session_rows(rows);
        // Two kept: the id-less row and the first "a".
        assert_eq!(out.len(), 2);
    }
}
