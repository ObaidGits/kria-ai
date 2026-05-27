use super::*;

pub(super) fn memory_turn_write(
    session_id: impl Into<String>,
    user_prompt: impl Into<String>,
    assistant_response: impl Into<String>,
    tool_name: Option<String>,
    tool_result: Option<String>,
    tokens_used: Option<i32>,
) -> MemoryTurnWrite {
    MemoryTurnWrite {
        session_id: session_id.into(),
        user_prompt: user_prompt.into(),
        assistant_response: assistant_response.into(),
        tool_name,
        tool_result,
        tokens_used,
        timestamp: Utc::now(),
        extraction: None,
    }
}

pub(super) fn preference_record(
    key: impl Into<String>,
    value: impl Into<String>,
) -> PreferenceRecord {
    PreferenceRecord {
        key: key.into(),
        value: value.into(),
    }
}

pub(super) fn is_likely_local_llm_transport_error(error: &str) -> bool {
    let lower = error.to_ascii_lowercase();
    let is_gui_workflow_failure = lower.contains("task did not fully complete")
        || lower.contains("gui workflow could not complete")
        || lower.contains("open_application_with_file")
        || lower.contains("launch timeout:")
        || (lower.contains("step ")
            && lower.contains("timed out after")
            && lower.contains("action:"));

    if is_gui_workflow_failure {
        return false;
    }

    let is_llm_scoped_timeout = lower.contains("local llm")
        || lower.contains("llama")
        || lower.contains("/v1/")
        || lower.contains("error sending request");

    lower.contains("local llm transport error")
        || lower.contains("error sending request for url")
        || lower.contains("connection refused")
        || lower.contains("tcp connect")
        || lower.contains("dns error")
        || (lower.contains("timed out") && is_llm_scoped_timeout)
        || lower.contains("connection reset")
        || lower.contains("broken pipe")
}

pub(super) fn is_transient_llm_error_text(text: &str) -> bool {
    let lower = text.to_ascii_lowercase();
    lower.contains("⚠️ llm error")
        || lower.contains("context too large for model")
        || lower.contains("local llm transport error")
        || lower.contains("error sending request for url")
        || lower.contains("local llm unavailable")
        || lower.contains("circuit open")
}

pub(super) fn should_skip_history_turn_for_llm(turn: &ConversationTurn) -> bool {
    turn.role.eq_ignore_ascii_case("assistant") && is_transient_llm_error_text(&turn.content)
}

pub(super) fn truncate_history_content_for_llm(role: &str, content: &str) -> String {
    let max_chars = match role.to_ascii_lowercase().as_str() {
        "assistant" => HISTORY_ITEM_CHAR_CAP,
        "user" => 1_200,
        "tool" => 700,
        "system" => 1_000,
        _ => HISTORY_ITEM_CHAR_CAP,
    };

    let char_count = content.chars().count();
    if char_count <= max_chars {
        return content.to_string();
    }
    if max_chars <= 32 {
        return content.chars().take(max_chars).collect();
    }

    let keep = max_chars.saturating_sub(26);
    let head: String = content.chars().take(keep).collect();
    let omitted = char_count.saturating_sub(keep);
    format!("{head}\n...[truncated {omitted} chars]")
}

pub(super) fn append_recent_turns_for_llm(
    messages: &mut Vec<ChatMessage>,
    recent_turns: &[ConversationTurn],
) {
    let mut inserted_indices: Vec<usize> = Vec::new();

    for turn in recent_turns {
        if should_skip_history_turn_for_llm(turn) {
            continue;
        }
        messages.push(ChatMessage {
            role: turn.role.clone(),
            content: truncate_history_content_for_llm(&turn.role, &turn.content),
            name: turn.tool_name.clone(),
            images: None,
        });
        inserted_indices.push(messages.len() - 1);
    }

    let mut total_history_chars: usize = inserted_indices
        .iter()
        .map(|idx| messages[*idx].content.chars().count())
        .sum();

    while total_history_chars > HISTORY_TOTAL_CHAR_BUDGET && !inserted_indices.is_empty() {
        let remove_idx = inserted_indices.remove(0);
        let removed_chars = messages[remove_idx].content.chars().count();
        messages.remove(remove_idx);
        total_history_chars = total_history_chars.saturating_sub(removed_chars);

        for idx in &mut inserted_indices {
            if *idx > remove_idx {
                *idx -= 1;
            }
        }
    }
}
