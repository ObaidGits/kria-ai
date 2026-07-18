//! Memory System Tauri façade (memory-upgrade Priority 2).
//!
//! The complete UI-facing surface over the unified [`MemorySystem`]. Every
//! command routes through `state.memory_system` — the single cognitive
//! authority — so the frontend has no bypass path. Reads return plain
//! `serde_json::Value`; writes go through the Write Policy / lifecycle / truth
//! engines. No business logic lives here: these are thin, honest adapters.

use super::*;
use uuid::Uuid;

const INIT_MSG: &str = "KRIA is still initializing — please try again in a moment";

fn parse_uuid(s: &str) -> Result<Uuid, String> {
    Uuid::parse_str(s).map_err(|_| format!("invalid uuid: {s}"))
}

/// Serialize a retrieval hit for the UI (memory row + fused score + strategies).
fn hit_json(h: &kria_core::memory::retriever::RetrievalHit) -> serde_json::Value {
    serde_json::json!({
        "id": h.memory.id.to_string(),
        "content": h.memory.content,
        "memory_type": h.memory.memory_type.as_str(),
        "namespace": h.memory.namespace,
        "confidence": h.memory.confidence,
        "importance": h.memory.importance,
        "decay_score": h.memory.decay_score,
        "access_count": h.memory.access_count,
        "state": format!("{:?}", h.memory.state),
        "created_at": h.memory.created_at.to_rfc3339(),
        "score": h.score,
        "strategies": h.strategies,
    })
}

/// Serialize the retrieval trace (why/how retrieval ran — explainability, L6).
fn trace_json(t: &kria_core::memory::retriever::RetrievalTrace) -> serde_json::Value {
    serde_json::json!({
        "query_class": t.query_class,
        "vector_used": t.vector_used,
        "fts_used": t.fts_used,
        "candidates": t.candidates,
        "returned": t.returned,
    })
}

// ── Core read surface ─────────────────────────────────────────────────────

#[tauri::command]
pub async fn memory_search(
    query: String,
    limit: Option<usize>,
    state: State<'_, AppStateCell>,
) -> Result<serde_json::Value, String> {
    let st = state.get().ok_or_else(|| INIT_MSG.to_string())?;
    let res = st
        .memory_system
        .search(&query, None)
        .await
        .map_err(|e| e.to_string())?;
    let take = limit.unwrap_or(20);
    let hits: Vec<_> = res.hits.iter().take(take).map(hit_json).collect();
    Ok(
        serde_json::json!({ "query": query, "results": hits, "count": hits.len(), "trace": trace_json(&res.trace) }),
    )
}

#[tauri::command]
pub async fn memory_recall(
    query: String,
    limit: Option<usize>,
    state: State<'_, AppStateCell>,
) -> Result<serde_json::Value, String> {
    memory_search(query, limit, state).await
}

#[tauri::command]
pub async fn memory_reason(
    query: String,
    limit: Option<usize>,
    state: State<'_, AppStateCell>,
) -> Result<serde_json::Value, String> {
    let st = state.get().ok_or_else(|| INIT_MSG.to_string())?;
    let ctx = st
        .memory_system
        .reason(&query, None)
        .await
        .map_err(|e| e.to_string())?;
    let take = limit.unwrap_or(20);
    let hits: Vec<_> = ctx.retrieval.hits.iter().take(take).map(hit_json).collect();
    Ok(serde_json::json!({
        "query": query,
        "results": hits,
        "count": hits.len(),
        "trace": trace_json(&ctx.retrieval.trace),
        "reasoning_context": ctx.reasoning,
        "planner_context": ctx.goals,
        "plan_recommendation": ctx.plan,
    }))
}

#[tauri::command]
pub async fn memory_health(state: State<'_, AppStateCell>) -> Result<serde_json::Value, String> {
    let st = state.get().ok_or_else(|| INIT_MSG.to_string())?;
    // Thin adapter over the shared memory-API contract (API-1) — identical shape
    // to the server's `/memory/health`, so the two hosts cannot drift.
    kria_core::memory::contract::health(&st.memory_system)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn memory_metrics(state: State<'_, AppStateCell>) -> Result<serde_json::Value, String> {
    let st = state.get().ok_or_else(|| INIT_MSG.to_string())?;
    let m = st.memory_system.metrics().map_err(|e| e.to_string())?;
    Ok(serde_json::json!({
        "active_memories": m.active_memories,
        "unresolved_gaps": m.unresolved_gaps,
        "goals": {
            "candidate": m.goals.candidate, "active": m.goals.active, "paused": m.goals.paused,
            "completed": m.goals.completed, "failed": m.goals.failed, "abandoned": m.goals.abandoned,
            "total": m.goals.total(), "completion_rate": m.goals.completion_rate(),
        },
        "plans": {
            "distinct_plans": m.plans.distinct_plans,
            "total_executions": m.plans.total_executions,
            "success_rate": m.plans.success_rate(),
        },
        // M5 tool-outcome telemetry (same field names as the server contract).
        "tool_outcomes": {
            "seen": m.tool_outcomes.seen,
            "persisted": m.tool_outcomes.persisted,
            "gated": m.tool_outcomes.gated,
        },
        "summary": m.summary(),
    }))
}

// ── Write surface ─────────────────────────────────────────────────────────

#[tauri::command]
pub async fn memory_remember(
    text: String,
    state: State<'_, AppStateCell>,
) -> Result<serde_json::Value, String> {
    let st = state.get().ok_or_else(|| INIT_MSG.to_string())?;
    // Thin adapter over the shared contract (API-1) — same `{decision}` shape
    // as the server's `/memory/remember`.
    kria_core::memory::contract::remember(&st.memory_system, text).map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn memory_update(
    winner: String,
    loser: String,
    state: State<'_, AppStateCell>,
) -> Result<(), String> {
    let st = state.get().ok_or_else(|| INIT_MSG.to_string())?;
    st.memory_system
        .update(parse_uuid(&winner)?, parse_uuid(&loser)?)
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn memory_verify(
    memory_id: String,
    state: State<'_, AppStateCell>,
) -> Result<bool, String> {
    let st = state.get().ok_or_else(|| INIT_MSG.to_string())?;
    st.memory_system
        .verify(parse_uuid(&memory_id)?)
        .map_err(|e| e.to_string())
}

fn forget_scope(
    kind: &str,
    value: &str,
) -> Result<kria_core::memory::lifecycle::ForgetScope, String> {
    // Shared scope-parsing contract (API-1) — same kinds/semantics as the
    // server's `/memory/forget` + `/memory/delete`.
    kria_core::memory::contract::parse_scope(kind, value).map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn memory_forget(
    kind: String,
    value: String,
    state: State<'_, AppStateCell>,
) -> Result<usize, String> {
    let st = state.get().ok_or_else(|| INIT_MSG.to_string())?;
    let scope = forget_scope(&kind, &value)?;
    st.memory_system.forget(scope).map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn memory_hard_delete(
    kind: String,
    value: String,
    state: State<'_, AppStateCell>,
) -> Result<usize, String> {
    let st = state.get().ok_or_else(|| INIT_MSG.to_string())?;
    let scope = forget_scope(&kind, &value)?;
    st.memory_system
        .hard_delete(scope)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn memory_resolve_entities(
    display_name: String,
    entity_type: String,
    alias: String,
    alias_type: String,
    state: State<'_, AppStateCell>,
) -> Result<serde_json::Value, String> {
    use kria_core::memory::entity_resolution::{AliasType, Resolution};
    let st = state.get().ok_or_else(|| INIT_MSG.to_string())?;
    let at = match alias_type.as_str() {
        "email" => AliasType::Email,
        "handle" => AliasType::Handle,
        "url" => AliasType::Url,
        "repo" => AliasType::Repo,
        _ => AliasType::Name,
    };
    let r = st
        .memory_system
        .resolve_entities(&display_name, &entity_type, &alias, at)
        .map_err(|e| e.to_string())?;
    let json = match r {
        Resolution::Matched(id) => serde_json::json!({ "kind": "matched", "id": id.to_string() }),
        Resolution::Created(id) => serde_json::json!({ "kind": "created", "id": id.to_string() }),
        Resolution::Proposed { existing, created } => serde_json::json!({
            "kind": "proposed", "existing": existing.to_string(), "created": created.to_string(),
        }),
    };
    Ok(json)
}

#[tauri::command]
pub async fn memory_record_feedback(
    target_id: String,
    target_kind: String,
    signal: String,
    detail: Option<String>,
    context: Option<String>,
    state: State<'_, AppStateCell>,
) -> Result<(), String> {
    use kria_core::memory::feedback::FeedbackSignal;
    let st = state.get().ok_or_else(|| INIT_MSG.to_string())?;
    if signal == "correction" {
        let corrected = detail.ok_or_else(|| "correction detail is required".to_string())?;
        return st
            .memory_system
            .correct(parse_uuid(&target_id)?, corrected, context.as_deref())
            .await
            .map_err(|e| e.to_string());
    }

    let d = detail.unwrap_or_default();
    let sig = match signal.as_str() {
        "thumbs_up" => FeedbackSignal::ThumbsUp,
        "thumbs_down" => FeedbackSignal::ThumbsDown,
        "undo" => FeedbackSignal::Undo,
        "cancel" => FeedbackSignal::Cancel,
        "edit" => FeedbackSignal::Edit(d),
        "overwrite" => FeedbackSignal::Overwrite,
        "ignored_suggestion" => FeedbackSignal::IgnoredSuggestion,
        "repeated_task" => FeedbackSignal::RepeatedTask,
        "automation_success" => FeedbackSignal::AutomationSuccess,
        "automation_failure" => FeedbackSignal::AutomationFailure,
        other => return Err(format!("unknown feedback signal: {other}")),
    };
    st.memory_system
        .record_feedback(
            parse_uuid(&target_id)?,
            &target_kind,
            sig,
            context.as_deref(),
        )
        .map_err(|e| e.to_string())?;
    st.memory_system.notify_change(
        "updated",
        serde_json::json!({ "op": "feedback", "target": target_id, "signal": signal }),
    );
    Ok(())
}

#[tauri::command]
pub async fn memory_correct(
    memory_id: String,
    content: String,
    state: State<'_, AppStateCell>,
) -> Result<(), String> {
    let st = state.get().ok_or_else(|| INIT_MSG.to_string())?;
    st.memory_system
        .correct(parse_uuid(&memory_id)?, content, None)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn memory_restore_forgotten(
    memory_id: String,
    state: State<'_, AppStateCell>,
) -> Result<(), String> {
    let st = state.get().ok_or_else(|| INIT_MSG.to_string())?;
    st.memory_system
        .restore_forgotten(parse_uuid(&memory_id)?)
        .map_err(|e| e.to_string())
}

// ── Cognition triggers ────────────────────────────────────────────────────

#[tauri::command]
pub async fn memory_reflect(state: State<'_, AppStateCell>) -> Result<usize, String> {
    let st = state.get().ok_or_else(|| INIT_MSG.to_string())?;
    st.memory_system.reflect().await.map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn memory_consolidate(
    session_id: String,
    state: State<'_, AppStateCell>,
) -> Result<usize, String> {
    let st = state.get().ok_or_else(|| INIT_MSG.to_string())?;
    st.memory_system
        .consolidate(super::history_helpers::memory_session_uuid(&session_id))
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn memory_run_dream(
    max_procedures: Option<usize>,
    state: State<'_, AppStateCell>,
) -> Result<serde_json::Value, String> {
    let st = state.get().ok_or_else(|| INIT_MSG.to_string())?;
    let (procs, goals, worth) = st
        .memory_system
        .run_dream(max_procedures.unwrap_or(5))
        .map_err(|e| e.to_string())?;
    Ok(
        serde_json::json!({ "procedures": procs, "goals_merged": goals, "worth_recalibrated": worth }),
    )
}

#[tauri::command]
pub async fn memory_run_active_learning(
    min_misses: Option<u32>,
    max_new: Option<usize>,
    state: State<'_, AppStateCell>,
) -> Result<usize, String> {
    let st = state.get().ok_or_else(|| INIT_MSG.to_string())?;
    st.memory_system
        .run_active_learning(min_misses.unwrap_or(3), max_new.unwrap_or(5))
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn memory_run_self_improvement(
    max_new: Option<usize>,
    state: State<'_, AppStateCell>,
) -> Result<usize, String> {
    let st = state.get().ok_or_else(|| INIT_MSG.to_string())?;
    st.memory_system
        .run_self_improvement(max_new.unwrap_or(5))
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn memory_run_entity_extraction(
    limit: Option<usize>,
    state: State<'_, AppStateCell>,
) -> Result<serde_json::Value, String> {
    let st = state.get().ok_or_else(|| INIT_MSG.to_string())?;
    let (processed, linked) = st
        .memory_system
        .run_entity_extraction(limit.unwrap_or(100))
        .map_err(|e| e.to_string())?;
    Ok(serde_json::json!({ "processed": processed, "entities_linked": linked }))
}

// ── Library (knowledge documents) ─────────────────────────────────────────

#[tauri::command]
pub async fn memory_library_list(
    state: State<'_, AppStateCell>,
) -> Result<serde_json::Value, String> {
    let st = state.get().ok_or_else(|| INIT_MSG.to_string())?;
    let items = st
        .memory_system
        .library()
        .list_items()
        .map_err(|e| e.to_string())?;
    let docs: Vec<serde_json::Value> = items
        .iter()
        .map(|(item, chunks)| {
            serde_json::json!({
                "doc_id": item.id.to_string(),
                "title": item.title,
                "path": item.path,
                "version": item.version,
                "chunks": chunks,
            })
        })
        .collect();
    Ok(serde_json::json!({ "documents": docs, "count": docs.len() }))
}

#[tauri::command]
pub async fn memory_library_ingest(
    path: String,
    state: State<'_, AppStateCell>,
) -> Result<serde_json::Value, String> {
    let st = state.get().ok_or_else(|| INIT_MSG.to_string())?;
    let ms = st.memory_system.clone();
    // File read + chunking + Write-Policy submits are blocking work; run them on
    // the blocking pool so they never stall the async runtime (H4).
    let (item_id, chunks, indexed, name) = tokio::task::spawn_blocking(move || {
        let file_path = std::path::Path::new(&path);
        if !file_path.exists() {
            return Err(format!("file not found: {path}"));
        }
        let name = file_path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or(&path)
            .to_string();
        let text = std::fs::read_to_string(file_path).map_err(|e| format!("read failed: {e}"))?;
        if text.trim().is_empty() {
            return Err("file is empty".to_string());
        }
        // The ONE ingestion pipeline (M3): record item + chunks in the Library
        // (dedup + versioning) and submit each chunk through the Write Policy.
        let (item_id, chunks, indexed) = ms
            .ingest_document(Some(&name), None, &path, &text)
            .map_err(|e| e.to_string())?;
        ms.notify_change(
            "library",
            serde_json::json!({ "op": "ingest", "doc_id": item_id.to_string(), "chunks": chunks }),
        );
        Ok((item_id, chunks, indexed, name))
    })
    .await
    .map_err(|e| format!("ingest task join error: {e}"))??;
    Ok(
        serde_json::json!({ "doc_id": item_id.to_string(), "name": name, "chunks": chunks, "indexed": indexed }),
    )
}

#[tauri::command]
pub async fn memory_library_delete(
    doc_id: String,
    state: State<'_, AppStateCell>,
) -> Result<usize, String> {
    let st = state.get().ok_or_else(|| INIT_MSG.to_string())?;
    let item_id = parse_uuid(&doc_id)?;
    st.memory_system
        .library()
        .delete_item(item_id)
        .map_err(|e| e.to_string())?;
    let scope =
        kria_core::memory::lifecycle::ForgetScope::SourcePrefix(format!("library:{item_id}"));
    st.memory_system
        .hard_delete(scope)
        .await
        .map_err(|e| e.to_string())
}

// ── Research (timeline + meta) ────────────────────────────────────────────

#[tauri::command]
pub async fn memory_timeline(
    limit: Option<usize>,
    state: State<'_, AppStateCell>,
) -> Result<serde_json::Value, String> {
    let st = state.get().ok_or_else(|| INIT_MSG.to_string())?;
    let entries = st
        .memory_system
        .research()
        .timeline(limit.unwrap_or(200))
        .map_err(|e| e.to_string())?;
    let out: Vec<serde_json::Value> = entries
        .iter()
        .map(|e| {
            serde_json::json!({
                "id": e.id, "content": e.content, "memory_type": e.memory_type,
                "confidence": e.confidence, "created_at": e.created_at,
            })
        })
        .collect();
    Ok(serde_json::json!({ "entries": out, "count": out.len() }))
}

#[tauri::command]
pub async fn memory_meta(state: State<'_, AppStateCell>) -> Result<serde_json::Value, String> {
    let st = state.get().ok_or_else(|| INIT_MSG.to_string())?;
    let m = st
        .memory_system
        .research()
        .meta()
        .map_err(|e| e.to_string())?;
    Ok(serde_json::json!({
        "active": m.active, "archived": m.archived, "superseded": m.superseded,
        "avg_confidence": m.avg_confidence, "avg_worth": m.avg_worth,
    }))
}

// ── Goals ─────────────────────────────────────────────────────────────────

fn goal_json(g: &kria_core::memory::goals::Goal) -> serde_json::Value {
    serde_json::json!({
        "id": g.id.to_string(), "kind": g.kind, "title": g.title,
        "status": g.status.as_str(), "confidence": g.confidence, "priority": g.priority,
        "parent_id": g.parent_id.map(|p| p.to_string()),
        "created_at": g.created_at, "last_progress_at": g.last_progress_at,
    })
}

#[tauri::command]
pub async fn memory_goals_list(
    limit: Option<usize>,
    state: State<'_, AppStateCell>,
) -> Result<serde_json::Value, String> {
    let st = state.get().ok_or_else(|| INIT_MSG.to_string())?;
    let goals = st
        .memory_system
        .goals()
        .active_goals(limit.unwrap_or(100))
        .map_err(|e| e.to_string())?;
    let out: Vec<_> = goals.iter().map(goal_json).collect();
    Ok(serde_json::json!({ "goals": out, "count": out.len() }))
}

#[tauri::command]
pub async fn memory_goal_create(
    title: String,
    state: State<'_, AppStateCell>,
) -> Result<String, String> {
    use kria_core::memory::goals::NewGoal;
    let st = state.get().ok_or_else(|| INIT_MSG.to_string())?;
    let id = st
        .memory_system
        .goals()
        .create(NewGoal::user(title))
        .map_err(|e| e.to_string())?;
    st.memory_system.notify_change(
        "goal",
        serde_json::json!({ "op": "create", "id": id.to_string() }),
    );
    Ok(id.to_string())
}

#[tauri::command]
pub async fn memory_goal_set_status(
    goal_id: String,
    status: String,
    state: State<'_, AppStateCell>,
) -> Result<(), String> {
    use kria_core::memory::goals::GoalStatus;
    let st = state.get().ok_or_else(|| INIT_MSG.to_string())?;
    st.memory_system
        .goals()
        .set_status(parse_uuid(&goal_id)?, GoalStatus::from_str(&status))
        .map_err(|e| e.to_string())?;
    st.memory_system.notify_change(
        "goal",
        serde_json::json!({ "op": "status", "id": goal_id, "status": status }),
    );
    Ok(())
}

// ── Plans ─────────────────────────────────────────────────────────────────

#[tauri::command]
pub async fn memory_plans_analytics(
    state: State<'_, AppStateCell>,
) -> Result<serde_json::Value, String> {
    let st = state.get().ok_or_else(|| INIT_MSG.to_string())?;
    let a = st
        .memory_system
        .plans()
        .analytics()
        .map_err(|e| e.to_string())?;
    Ok(serde_json::json!({
        "distinct_plans": a.distinct_plans, "total_executions": a.total_executions,
        "success_rate": a.success_rate(),
    }))
}

#[tauri::command]
pub async fn memory_plans_for(
    task: String,
    state: State<'_, AppStateCell>,
) -> Result<serde_json::Value, String> {
    let st = state.get().ok_or_else(|| INIT_MSG.to_string())?;
    let plans = st
        .memory_system
        .plans()
        .plans_for(&task)
        .map_err(|e| e.to_string())?;
    let out: Vec<serde_json::Value> = plans
        .iter()
        .map(|p| {
            serde_json::json!({
                "signature": p.signature, "task_label": p.task_label, "steps": p.steps,
                "success": p.success, "failure": p.failure, "samples": p.samples,
                "worth": p.worth(), "trusted": p.is_trusted(),
            })
        })
        .collect();
    Ok(serde_json::json!({ "plans": out, "count": out.len() }))
}

// ── Reasoning ─────────────────────────────────────────────────────────────

#[tauri::command]
pub async fn memory_reasoning_analytics(
    state: State<'_, AppStateCell>,
) -> Result<serde_json::Value, String> {
    let st = state.get().ok_or_else(|| INIT_MSG.to_string())?;
    let a = st
        .memory_system
        .reasoning()
        .analytics()
        .map_err(|e| e.to_string())?;
    Ok(serde_json::json!({
        "chains": a.chains, "hypotheses": a.hypotheses, "counterexamples": a.counterexamples,
        "failed_chains": a.failed_chains, "avg_confidence": a.avg_confidence,
        "hallucination_rate": a.hallucination_rate(),
    }))
}

#[tauri::command]
pub async fn memory_reasoning_history(
    task: String,
    limit: Option<usize>,
    state: State<'_, AppStateCell>,
) -> Result<serde_json::Value, String> {
    let st = state.get().ok_or_else(|| INIT_MSG.to_string())?;
    let traces = st
        .memory_system
        .reasoning()
        .history_for_task(&task, limit.unwrap_or(50))
        .map_err(|e| e.to_string())?;
    let out: Vec<serde_json::Value> = traces
        .iter()
        .map(|t| {
            serde_json::json!({
                "id": t.id.to_string(), "session_id": t.session_id, "task_label": t.task_label,
                "kind": t.kind.as_str(), "content": t.content, "confidence": t.confidence,
                "success": t.success, "created_at": t.created_at,
            })
        })
        .collect();
    Ok(serde_json::json!({ "traces": out, "count": out.len() }))
}

// ── Causal ────────────────────────────────────────────────────────────────

fn causal_links_json(links: &[kria_core::memory::causal::CausalLink]) -> serde_json::Value {
    let out: Vec<serde_json::Value> = links
        .iter()
        .map(|l| {
            serde_json::json!({
                "cause": l.cause, "effect": l.effect, "observations": l.observations,
                "successes": l.successes, "confidence": l.confidence(),
            })
        })
        .collect();
    serde_json::json!({ "links": out, "count": out.len() })
}

#[tauri::command]
pub async fn memory_causal_effects_of(
    cause: String,
    state: State<'_, AppStateCell>,
) -> Result<serde_json::Value, String> {
    let st = state.get().ok_or_else(|| INIT_MSG.to_string())?;
    let links = st
        .memory_system
        .causal()
        .effects_of(&cause)
        .map_err(|e| e.to_string())?;
    Ok(causal_links_json(&links))
}

#[tauri::command]
pub async fn memory_causal_causes_of(
    effect: String,
    state: State<'_, AppStateCell>,
) -> Result<serde_json::Value, String> {
    let st = state.get().ok_or_else(|| INIT_MSG.to_string())?;
    let links = st
        .memory_system
        .causal()
        .causes_of(&effect)
        .map_err(|e| e.to_string())?;
    Ok(causal_links_json(&links))
}

#[tauri::command]
pub async fn memory_causal_chains(
    start: String,
    max_depth: Option<usize>,
    state: State<'_, AppStateCell>,
) -> Result<serde_json::Value, String> {
    let st = state.get().ok_or_else(|| INIT_MSG.to_string())?;
    let chains = st
        .memory_system
        .causal()
        .causal_chains(&start, max_depth.unwrap_or(4))
        .map_err(|e| e.to_string())?;
    let out: Vec<serde_json::Value> = chains
        .iter()
        .map(|c| serde_json::json!({ "path": c.path, "confidence": c.confidence }))
        .collect();
    Ok(serde_json::json!({ "chains": out, "count": out.len() }))
}

// ── Knowledge graph ───────────────────────────────────────────────────────

#[tauri::command]
pub async fn memory_graph_centrality(
    limit: Option<usize>,
    state: State<'_, AppStateCell>,
) -> Result<serde_json::Value, String> {
    let st = state.get().ok_or_else(|| INIT_MSG.to_string())?;
    let hits = st
        .memory_system
        .graph_intelligence()
        .degree_centrality(limit.unwrap_or(50))
        .map_err(|e| e.to_string())?;
    let out: Vec<serde_json::Value> = hits
        .iter()
        .map(|h| {
            serde_json::json!({
                "entity": h.entity.to_string(), "display_name": h.display_name, "degree": h.degree,
            })
        })
        .collect();
    Ok(serde_json::json!({ "nodes": out, "count": out.len() }))
}

#[tauri::command]
pub async fn memory_graph_communities(
    state: State<'_, AppStateCell>,
) -> Result<serde_json::Value, String> {
    let st = state.get().ok_or_else(|| INIT_MSG.to_string())?;
    let comms = st
        .memory_system
        .graph_intelligence()
        .communities()
        .map_err(|e| e.to_string())?;
    let out: Vec<Vec<String>> = comms
        .iter()
        .map(|c| c.iter().map(|id| id.to_string()).collect())
        .collect();
    Ok(serde_json::json!({ "communities": out, "count": out.len() }))
}

#[tauri::command]
pub async fn memory_graph_neighbors(
    entity_id: String,
    hops: Option<u8>,
    state: State<'_, AppStateCell>,
) -> Result<serde_json::Value, String> {
    let st = state.get().ok_or_else(|| INIT_MSG.to_string())?;
    let hits = st
        .memory_system
        .graph_neighbors(parse_uuid(&entity_id)?, hops.unwrap_or(2))
        .map_err(|e| e.to_string())?;
    serde_json::to_value(&hits).map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn memory_graph_relationships(
    entity_id: String,
    state: State<'_, AppStateCell>,
) -> Result<serde_json::Value, String> {
    let st = state.get().ok_or_else(|| INIT_MSG.to_string())?;
    let rels = st
        .memory_system
        .graph_relationships(parse_uuid(&entity_id)?)
        .map_err(|e| e.to_string())?;
    serde_json::to_value(&rels).map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn memory_graph_search(
    query: String,
    state: State<'_, AppStateCell>,
) -> Result<serde_json::Value, String> {
    let st = state.get().ok_or_else(|| INIT_MSG.to_string())?;
    let ents = st
        .memory_system
        .graph_search_entities(&query)
        .map_err(|e| e.to_string())?;
    serde_json::to_value(&ents).map_err(|e| e.to_string())
}

// ── Cold-start consent (onboarding) ───────────────────────────────────────

fn scan_source(s: &str) -> Result<kria_core::memory::cold_start::ScanSource, String> {
    use kria_core::memory::cold_start::ScanSource;
    match s {
        "filesystem" => Ok(ScanSource::Filesystem),
        "git" => Ok(ScanSource::Git),
        "workspace" => Ok(ScanSource::Workspace),
        "shell" => Ok(ScanSource::Shell),
        other => Err(format!("unknown scan source: {other}")),
    }
}

#[tauri::command]
pub async fn memory_cold_start_status(
    state: State<'_, AppStateCell>,
) -> Result<serde_json::Value, String> {
    let st = state.get().ok_or_else(|| INIT_MSG.to_string())?;
    let cs = st.memory_system.cold_start();
    let granted = cs.granted_sources().map_err(|e| e.to_string())?;
    let onboarding = cs.onboarding_complete().map_err(|e| e.to_string())?;
    let granted_str: Vec<&str> = granted.iter().map(|s| s.as_str()).collect();
    Ok(serde_json::json!({ "onboarding_complete": onboarding, "granted": granted_str }))
}

#[tauri::command]
pub async fn memory_cold_start_set(
    source: String,
    granted: bool,
    state: State<'_, AppStateCell>,
) -> Result<(), String> {
    let st = state.get().ok_or_else(|| INIT_MSG.to_string())?;
    let src = scan_source(&source)?;
    let cs = st.memory_system.cold_start();
    if granted {
        cs.grant(src).map_err(|e| e.to_string())
    } else {
        cs.revoke(src).map_err(|e| e.to_string())
    }
}

#[tauri::command]
pub async fn memory_cold_start_preview(
    source: String,
    root: Option<String>,
    limit: Option<usize>,
    state: State<'_, AppStateCell>,
) -> Result<serde_json::Value, String> {
    let st = state.get().ok_or_else(|| INIT_MSG.to_string())?;
    let src = scan_source(&source)?;
    let ms = st.memory_system.clone();
    // Filesystem walk / git subprocess / shell-history read are blocking (H4).
    let cands = tokio::task::spawn_blocking(move || {
        ms.cold_start_preview(src, root.as_deref(), limit.unwrap_or(200))
    })
    .await
    .map_err(|e| format!("cold-start preview task join error: {e}"))?
    .map_err(|e| e.to_string())?;
    let items: Vec<serde_json::Value> = cands
        .iter()
        .map(|c| serde_json::json!({ "source": c.source, "path": c.path, "detail": c.detail }))
        .collect();
    Ok(serde_json::json!({ "candidates": items, "count": items.len() }))
}

#[tauri::command]
pub async fn memory_cold_start_import(
    source: String,
    candidates: Vec<serde_json::Value>,
    state: State<'_, AppStateCell>,
) -> Result<usize, String> {
    let st = state.get().ok_or_else(|| INIT_MSG.to_string())?;
    let src = scan_source(&source)?;
    let cands: Vec<kria_core::memory::cold_start::ScanCandidate> = candidates
        .iter()
        .filter_map(|v| {
            Some(kria_core::memory::cold_start::ScanCandidate {
                source: v.get("source")?.as_str()?.to_string(),
                path: v.get("path")?.as_str()?.to_string(),
                detail: v
                    .get("detail")
                    .and_then(|d| d.as_str())
                    .unwrap_or("")
                    .to_string(),
            })
        })
        .collect();
    let ms = st.memory_system.clone();
    // Register a fresh cancellation token so `memory_cold_start_cancel` can
    // interrupt this import (AUD-03 / L4). Overwrites any prior token; we do NOT
    // clear on completion (that could race-clear a newer import's token). A
    // cancel on an already-finished token is a harmless no-op, and the next
    // import overwrites the slot. Onboarding runs one import at a time.
    let cancel = tokio_util::sync::CancellationToken::new();
    *st.cold_start_cancel
        .lock()
        .unwrap_or_else(|p| p.into_inner()) = Some(cancel.clone());

    // File reads + Write-Policy submits are blocking (H4). Cancellation is
    // cooperative — the import loop checks the token before each candidate.
    tokio::task::spawn_blocking(move || ms.cold_start_import_cancellable(src, &cands, &cancel))
        .await
        .map_err(|e| format!("cold-start import task join error: {e}"))?
        .map_err(|e| e.to_string())
}

/// Cancel an in-flight cold-start import (AUD-03 / L4). Cooperative: the import
/// loop stops before the next candidate; already-imported items are kept
/// (each is committed independently through the Write Policy). No-op if no
/// import is running.
#[tauri::command]
pub async fn memory_cold_start_cancel(state: State<'_, AppStateCell>) -> Result<bool, String> {
    let st = state.get().ok_or_else(|| INIT_MSG.to_string())?;
    let token = st
        .cold_start_cancel
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .clone();
    match token {
        Some(t) => {
            t.cancel();
            Ok(true)
        }
        None => Ok(false),
    }
}

#[tauri::command]
pub async fn memory_cold_start_complete(state: State<'_, AppStateCell>) -> Result<(), String> {
    let st = state.get().ok_or_else(|| INIT_MSG.to_string())?;
    st.memory_system
        .cold_start()
        .complete_onboarding()
        .map_err(|e| e.to_string())
}

// ── Graph analytics + operations (P4) ─────────────────────────────────────

#[tauri::command]
pub async fn memory_graph_predict_links(
    entity_id: String,
    limit: Option<usize>,
    state: State<'_, AppStateCell>,
) -> Result<serde_json::Value, String> {
    let st = state.get().ok_or_else(|| INIT_MSG.to_string())?;
    let preds = st
        .memory_system
        .graph_predict_links(parse_uuid(&entity_id)?, limit.unwrap_or(10))
        .map_err(|e| e.to_string())?;
    let out: Vec<serde_json::Value> = preds
        .iter()
        .map(|p| {
            serde_json::json!({
                "target": p.target.to_string(), "display_name": p.display_name,
                "score": p.score, "shared_neighbors": p.shared_neighbors,
            })
        })
        .collect();
    Ok(serde_json::json!({ "predictions": out, "count": out.len() }))
}

#[tauri::command]
pub async fn memory_graph_create_relationship(
    source_id: String,
    target_id: String,
    rel_type: String,
    strength: Option<f32>,
    state: State<'_, AppStateCell>,
) -> Result<String, String> {
    let st = state.get().ok_or_else(|| INIT_MSG.to_string())?;
    let id = st
        .memory_system
        .create_relationship(
            parse_uuid(&source_id)?,
            parse_uuid(&target_id)?,
            &rel_type,
            strength.unwrap_or(0.7),
        )
        .map_err(|e| e.to_string())?;
    Ok(id.to_string())
}

// ── Explainability + observability (P5) ───────────────────────────────────

#[tauri::command]
pub async fn memory_explain(
    memory_id: String,
    state: State<'_, AppStateCell>,
) -> Result<serde_json::Value, String> {
    let st = state.get().ok_or_else(|| INIT_MSG.to_string())?;
    let ex = st
        .memory_system
        .explain(parse_uuid(&memory_id)?)
        .map_err(|e| e.to_string())?;
    match ex {
        None => Ok(serde_json::Value::Null),
        Some(e) => Ok(serde_json::json!({
            "id": e.id.to_string(),
            "content": e.content,
            "memory_type": e.memory_type,
            "state": e.state,
            "confidence": e.confidence,
            "importance": e.importance,
            "source_event_tag": e.source_event_tag,
            "derived_from": e.derived_from.iter().map(|u| u.to_string()).collect::<Vec<_>>(),
            "contradicts": e.contradicts.iter().map(|u| u.to_string()).collect::<Vec<_>>(),
            "worth_success": e.worth_success,
            "worth_failure": e.worth_failure,
            "worth_samples": e.worth_samples,
            "access_count": e.access_count,
            "staleness_class": e.staleness_class,
            "superseded_by": e.superseded_by.map(|u| u.to_string()),
        })),
    }
}

#[tauri::command]
pub async fn memory_backup(dest: String, state: State<'_, AppStateCell>) -> Result<u64, String> {
    let st = state.get().ok_or_else(|| INIT_MSG.to_string())?;
    let ms = st.memory_system.clone();
    tokio::task::spawn_blocking(move || ms.backup(&dest))
        .await
        .map_err(|e| format!("backup task join error: {e}"))?
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn memory_restore(src: String, state: State<'_, AppStateCell>) -> Result<(), String> {
    let st = state.get().ok_or_else(|| INIT_MSG.to_string())?;
    let ms = st.memory_system.clone();
    tokio::task::spawn_blocking(move || ms.restore(&src))
        .await
        .map_err(|e| format!("restore task join error: {e}"))?
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn memory_health_report(
    state: State<'_, AppStateCell>,
) -> Result<serde_json::Value, String> {
    let st = state.get().ok_or_else(|| INIT_MSG.to_string())?;
    let r = st
        .memory_system
        .memory_health_report()
        .map_err(|e| e.to_string())?;
    Ok(serde_json::json!({
        "total_active": r.total_active,
        "total_archived": r.total_archived,
        "total_superseded": r.total_superseded,
        "total_forgotten": r.total_forgotten,
        "by_type": r.by_type.iter().map(|(k, v)| serde_json::json!({ "label": k, "count": v })).collect::<Vec<_>>(),
        "by_staleness": r.by_staleness.iter().map(|(k, v)| serde_json::json!({ "label": k, "count": v })).collect::<Vec<_>>(),
        "avg_confidence": r.avg_confidence,
        "unresolved_contradictions": r.unresolved_contradictions,
        "knowledge_gaps": r.knowledge_gaps,
        "enrichment_backlog": r.enrichment_backlog,
        "outbox_pending": r.outbox_pending,
    }))
}

#[tauri::command]
pub async fn memory_reasoning_replay(
    session: String,
    state: State<'_, AppStateCell>,
) -> Result<serde_json::Value, String> {
    let st = state.get().ok_or_else(|| INIT_MSG.to_string())?;
    let traces = st
        .memory_system
        .reasoning_replay(&session)
        .map_err(|e| e.to_string())?;
    let out: Vec<serde_json::Value> = traces
        .iter()
        .map(|t| {
            serde_json::json!({
                "id": t.id.to_string(), "session_id": t.session_id, "task_label": t.task_label,
                "kind": t.kind.as_str(), "content": t.content, "confidence": t.confidence,
                "success": t.success, "created_at": t.created_at,
            })
        })
        .collect();
    Ok(serde_json::json!({ "traces": out, "count": out.len() }))
}
