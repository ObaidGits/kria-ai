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
pub async fn branch_session(
    source_session_id: String,
    through_index: usize,
    title: Option<String>,
    state: State<'_, AppStateCell>,
) -> Result<serde_json::Value, String> {
    let state = state
        .get()
        .ok_or_else(|| "KRIA is still initializing — please try again in a moment".to_string())?;
    if source_session_id.trim().is_empty() {
        return Err("source_session_id is required".to_string());
    }

    let source_turns = state
        .memory_store
        .get_recent_turns(&source_session_id, 1_000)
        .map_err(|e| e.to_string())?;
    let turns_to_copy = source_turns.into_iter().take(through_index + 1);
    let new_id = uuid::Uuid::new_v4().to_string();
    let memory_writer: Arc<dyn MemoryManager> = state.memory_store.clone();
    let mut copied = 0usize;

    for turn in turns_to_copy {
        let write = MemoryTurnWrite {
            session_id: new_id.clone(),
            user_prompt: if turn.role == "user" {
                turn.content.clone()
            } else {
                String::new()
            },
            assistant_response: if turn.role == "user" {
                String::new()
            } else {
                turn.content.clone()
            },
            tool_name: turn.tool_name.clone(),
            tool_result: turn.tool_result.clone(),
            tokens_used: turn.tokens_used.and_then(|value| i32::try_from(value).ok()),
            timestamp: turn.timestamp,
            extraction: None,
        };
        if let Err(error) = memory_writer.store_turn(&write) {
            let _ = memory_writer.delete_session(&new_id);
            return Err(format!("failed to create conversation branch: {error}"));
        }
        copied += 1;
    }

    let source_title = state
        .memory_store
        .get_preference(&format!("session_title:{source_session_id}"))
        .map_err(|e| e.to_string())?
        .unwrap_or_else(|| "Conversation".to_string());
    let branch_title = title
        .as_deref()
        .and_then(normalize_session_title)
        .unwrap_or_else(|| format!("Branch · {source_title}"));
    for preference in [
        preference_record(format!("session_title:{new_id}"), branch_title),
        preference_record(format!("session_title_manual:{new_id}"), "1"),
        preference_record(
            format!("session_created_at:{new_id}"),
            Utc::now().to_rfc3339(),
        ),
    ] {
        if let Err(error) = memory_writer.set_preference(&preference) {
            let _ = memory_writer.delete_session(&new_id);
            let _ = memory_writer.delete_session_preferences(&new_id);
            return Err(format!("failed to create branch metadata: {error}"));
        }
    }

    *state.current_session_id.write().await = new_id.clone();
    Ok(serde_json::json!({
        "session_id": new_id,
        "source_session_id": source_session_id,
        "copied_turns": copied,
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
                .unwrap_or_else(|| {
                    let short: String = id.chars().take(8).collect();
                    format!("Session ({})", short)
                });
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
            let temporary = state
                .memory_store
                .get_preference(&format!("session_temporary:{}", id))
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
                "temporary": temporary,
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
        let pinned = state
            .memory_store
            .get_preference(&format!("session_pinned:{}", current))
            .unwrap_or(None)
            .as_deref()
            == Some("1");
        let archived = state
            .memory_store
            .get_preference(&format!("session_archived:{}", current))
            .unwrap_or(None)
            .as_deref()
            == Some("1");
        let temporary = state
            .memory_store
            .get_preference(&format!("session_temporary:{}", current))
            .unwrap_or(None)
            .as_deref()
            == Some("1");
        result.insert(
            0,
            serde_json::json!({
                "id": current,
                "title": title,
                "turn_count": 0,
                "message_count": 0,
                "last_active": created_at,
                "is_current": true,
                "pinned": pinned,
                "archived": archived,
                "temporary": temporary,
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

    // Session metadata is part of the deleted chat. Always remove it so title,
    // pin/archive/temporary state cannot survive as orphaned preferences.
    if let Err(e) = memory_writer.delete_session_preferences(&session_id) {
        tracing::warn!(session_id = %session_id, error = %e, "failed to clean session preferences on delete");
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

/// Give a conversation a readable name derived from its own first real message.
///
/// The UI calls this after a turn completes; it does not pass a title, because
/// choosing one is domain logic and lives in `kria_core::agent::session_title`.
///
/// Deliberately a no-op in three cases, each reported rather than silently ignored:
/// the user renamed the chat by hand (`session_title_manual` is `1`), the
/// conversation still contains nothing but greetings, or the title already matches.
/// A chat the user named must never be overwritten by a guess.
#[tauri::command]
pub async fn auto_title_session(
    session_id: Option<String>,
    state: State<'_, AppStateCell>,
) -> Result<serde_json::Value, String> {
    let state = state
        .get()
        .ok_or_else(|| "KRIA is still initializing — please try again in a moment".to_string())?;

    let session_id = match session_id {
        Some(id) if !id.trim().is_empty() => id.trim().to_string(),
        _ => state.current_session_id.read().await.clone(),
    };

    // A hand-picked name outranks anything derived. Checked BEFORE reading the
    // conversation so a manual title costs no query.
    let manual_key = format!("session_title_manual:{session_id}");
    if state
        .memory_store
        .get_preference(&manual_key)
        .map_err(|error| error.to_string())?
        .as_deref()
        == Some("1")
    {
        return Ok(serde_json::json!({ "updated": false, "reason": "manual_title" }));
    }

    // Only the opening exchange matters for a title, so this reads a small window
    // rather than the whole conversation.
    let turns = state
        .memory_store
        .get_recent_turns(&session_id, 12)
        .map_err(|error| error.to_string())?;

    let Some(title) = kria_core::agent::session_title::derive_title(&turns) else {
        return Ok(serde_json::json!({ "updated": false, "reason": "no_substantive_message" }));
    };

    let title_key = format!("session_title:{session_id}");
    let existing = state
        .memory_store
        .get_preference(&title_key)
        .map_err(|error| error.to_string())?;
    if existing.as_deref() == Some(title.as_str()) {
        return Ok(serde_json::json!({ "updated": false, "reason": "unchanged", "title": title }));
    }

    let memory_writer: Arc<dyn MemoryManager> = state.memory_store.clone();
    memory_writer
        .set_preference(&preference_record(title_key, title.clone()))
        .map_err(|error| error.to_string())?;
    // Left at "0": this title was derived, so a later derivation may improve it, and
    // a manual rename must still be able to take over.
    memory_writer
        .set_preference(&preference_record(manual_key, "0"))
        .map_err(|error| error.to_string())?;

    Ok(serde_json::json!({ "updated": true, "title": title }))
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
    Ok(state.config.read().await.memory.enabled)
}

#[tauri::command]
pub async fn set_memory_enabled(
    enabled: bool,
    state: State<'_, AppStateCell>,
) -> Result<(), String> {
    let state = state
        .get()
        .ok_or_else(|| "KRIA is still initializing — please try again in a moment".to_string())?;
    state
        .config_service
        .patch(
            "memory",
            "enabled",
            serde_json::json!(enabled),
            kria_core::config::ChangeSource::Ui,
            None,
        )
        .await
        .map_err(|error| error.to_string())?;
    Ok(())
}

/// Render one conversation as a downloadable transcript.
///
/// Returns the rendered content plus a suggested filename and dialog filter, which
/// the caller hands to `save_export_file`. Splitting "build the transcript" from
/// "write it to disk" means the format rules are exercised by unit tests in
/// `kria_core::agent::transcript` rather than only by clicking the button.
///
/// `session_id` defaults to the active session. `format` is one of `text`,
/// `markdown`, or `json`; an unrecognised name is REFUSED rather than defaulted, so
/// a typo cannot produce a plain-text file when the caller asked for JSON.
#[tauri::command]
pub async fn export_session(
    session_id: Option<String>,
    format: String,
    state: State<'_, AppStateCell>,
) -> Result<serde_json::Value, String> {
    use kria_core::agent::transcript::{render, TranscriptFormat};

    let state = state
        .get()
        .ok_or_else(|| "KRIA is still initializing — please try again in a moment".to_string())?;

    let format = TranscriptFormat::parse(&format).ok_or_else(|| {
        format!("Unsupported export format '{format}'. Use text, markdown, or json.")
    })?;

    let session_id = match session_id {
        Some(id) if !id.trim().is_empty() => id,
        _ => state.current_session_id.read().await.clone(),
    };

    // Every turn, not the most recent 100: an export that quietly loses the start of
    // a long conversation is worse than one that refuses.
    let turns = state
        .memory_store
        .get_all_turns(&session_id)
        .map_err(|error| error.to_string())?;

    // Same title the sidebar shows, so the file the user gets is named after the
    // conversation they clicked rather than a raw session id.
    let title = state
        .memory_store
        .get_preference(&format!("session_title:{session_id}"))
        .unwrap_or(None)
        .unwrap_or_else(|| {
            let short: String = session_id.chars().take(8).collect();
            format!("Session ({short})")
        });

    let transcript = render(&title, &turns, format);
    Ok(serde_json::json!({
        "content": transcript.content,
        "suggested_name": transcript.suggested_name,
        "extension": transcript.extension,
        "filter_label": transcript.filter_label,
        "turn_count": transcript.turn_count,
    }))
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
