use super::*;

/// Read a chat-management feature flag from the environment.
///
/// Defaults to ON (enabled). Treats the usual falsy spellings as a disable so a
/// behaviour can be rolled back to legacy without a rebuild, matching the
/// `KRIA_GUI_COG_*` runtime-flag convention used elsewhere.
pub(super) fn chat_flag_enabled(var: &str) -> bool {
    match std::env::var(var) {
        Ok(v) => !matches!(
            v.trim().to_ascii_lowercase().as_str(),
            "0" | "false" | "off" | "no" | ""
        ),
        Err(_) => true,
    }
}

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

/// Map a chat session string id to a stable UUID for the cognitive
/// `MemorySystem` (which keys sessions by UUID). Deterministic so the same chat
/// maps to the same memory session across turns and restarts.
pub(super) fn memory_session_uuid(session_id: &str) -> uuid::Uuid {
    // Deterministic 128-bit FNV-1a-style hash over the session string → stable
    // UUID (no `uuid` v5 feature dependency). Same chat → same memory session.
    let mut hi: u64 = 0xcbf2_9ce4_8422_2325;
    let mut lo: u64 = 0x8422_2325_cbf2_9ce4;
    for b in session_id.as_bytes() {
        hi ^= *b as u64;
        hi = hi.wrapping_mul(0x100_0000_01b3);
        lo = lo.rotate_left(7) ^ (*b as u64);
        lo = lo.wrapping_mul(0x100_0000_01b3);
    }
    uuid::Uuid::from_u128(((hi as u128) << 64) | lo as u128)
}

/// Observe a completed user message through the unified `MemorySystem` so it
/// flows through the Write Policy into cognitive memory (event → derived memory
/// → retrieval + background cognition). Best-effort: failures never block chat.
/// Callers must gate on privacy (temporary / long-term-memory-off) first.
pub(super) fn observe_user_message(
    memory_system: &kria_core::memory::api::MemorySystem,
    session_id: &str,
    user_message: &str,
) {
    if user_message.trim().is_empty() {
        return;
    }
    let candidate = kria_core::memory::types::WriteCandidate::user(
        memory_session_uuid(session_id),
        user_message.to_string(),
    );
    if let Err(e) = memory_system.observe(candidate) {
        tracing::debug!(error = %e, "MemorySystem observe(user_message) skipped");
    }
}

/// Record a tool/agent outcome through the unified `MemorySystem` (design §46.1)
/// so procedural/capability knowledge accrues from real executions. Best-effort.
pub(super) fn observe_tool_outcome(
    memory_system: &kria_core::memory::api::MemorySystem,
    session_id: &str,
    tool_name: &str,
    outcome: &str,
) {
    if tool_name.trim().is_empty() || outcome.trim().is_empty() {
        return;
    }
    let source = kria_core::memory::types::Source::Tool(tool_name.to_string());
    if let Err(e) = memory_system.record_tool_outcome(
        memory_session_uuid(session_id),
        source,
        outcome.to_string(),
    ) {
        tracing::debug!(error = %e, tool = tool_name, "MemorySystem record_tool_outcome skipped");
    }
}

/// Record an OpenClaw/capability lifecycle event (acquisition, deletion,
/// enable/disable, generation) as capability memory through the Write Policy
/// (design §46.4). Best-effort; uses a stable `openclaw` pseudo-session.
pub(super) fn observe_capability_lifecycle(
    memory_system: &kria_core::memory::api::MemorySystem,
    event: &str,
    skill_id: &str,
    success: bool,
) {
    let detail = format!(
        "openclaw {event}: skill '{skill_id}' ({})",
        if success { "ok" } else { "failed" }
    );
    // Tagged `Source::OpenClaw` (not `Source::Tool`, task F1.5.4) so this
    // skill-lifecycle event carries OpenClaw's actual trust class and
    // `openclaw/{skill}` namespace (MGR-043 AC1) rather than being collapsed
    // into the generic native-tool source.
    if let Err(e) = memory_system.record_capability(
        memory_session_uuid("openclaw:lifecycle"),
        kria_core::memory::types::Source::OpenClaw(skill_id.to_string()),
        success,
        detail,
    ) {
        tracing::debug!(error = %e, skill = skill_id, "MemorySystem record_capability skipped");
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

/// Auto-extraction of durable user-preference facts from a turn.
///
/// Replaces the removed legacy `FactManager` fast path: when the user message
/// matches the first-person preference classification `kria_core` owns
/// (`governance::is_preference_statement`, task F1.5.2 — this adapter carries
/// no standalone domain-classification decision), the message is persisted as
/// a `user_preference` fact through the unified memory runtime (authority DB +
/// FTS). Full LLM-driven extraction flows through `SemanticMemoryParser` into
/// `MemoryTurnWrite::extraction`. Returns the number of facts stored (0 or 1).
pub(super) fn auto_extract_facts(
    store: &dyn MemoryRuntime,
    user_message: &str,
) -> anyhow::Result<usize> {
    if kria_core::memory::governance::is_preference_statement(user_message) {
        let now = Utc::now();
        store.store_fact(&kria_core::memory::MemoryFact {
            id: None,
            text: user_message.to_string(),
            category: "user_preference".to_string(),
            source: "conversation".to_string(),
            created_at: now,
            last_accessed: now,
            access_count: 0,
            decay_score: 1.0,
        })?;
        Ok(1)
    } else {
        Ok(0)
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

#[cfg(test)]
mod flag_tests {
    use super::chat_flag_enabled;

    #[test]
    fn unset_flag_defaults_on() {
        let var = "KRIA_TEST_FLAG_UNSET_XYZ";
        std::env::remove_var(var);
        assert!(chat_flag_enabled(var));
    }

    #[test]
    fn falsy_values_disable() {
        let var = "KRIA_TEST_FLAG_FALSY_XYZ";
        for v in ["0", "false", "off", "no", "", " OFF "] {
            std::env::set_var(var, v);
            assert!(!chat_flag_enabled(var), "{v:?} should disable");
        }
        std::env::remove_var(var);
    }

    #[test]
    fn truthy_values_enable() {
        let var = "KRIA_TEST_FLAG_TRUTHY_XYZ";
        for v in ["1", "true", "on", "yes", "anything"] {
            std::env::set_var(var, v);
            assert!(chat_flag_enabled(var), "{v:?} should enable");
        }
        std::env::remove_var(var);
    }

    // The desktop session→UUID mapping MUST match the core agent loop's
    // `stable_session_uuid` (identical FNV constants) so the privacy-mode gate
    // set here applies to the writes the core loop performs. This locks the
    // desktop side's determinism; the core side has the mirror test.
    #[test]
    fn memory_session_uuid_is_deterministic_and_distinct() {
        let a1 = super::memory_session_uuid("session-abc");
        let a2 = super::memory_session_uuid("session-abc");
        assert_eq!(a1, a2);
        assert_ne!(a1, super::memory_session_uuid("session-xyz"));
        assert_ne!(a1, uuid::Uuid::nil());
    }
}
