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

fn require_memory_enabled(state: &AppState) -> Result<(), String> {
    if state.memory_system.is_enabled() {
        Ok(())
    } else {
        Err("memory feature is disabled".to_string())
    }
}

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
    // No preview token for desktop UI fast-forget (None = no stale-guard).
    st.memory_system
        .forget(scope, None)
        .map_err(|e| e.to_string())
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
    // Alias-taxonomy parsing is a domain decision owned by `kria_core`
    // (`AliasType::from_str`, task F1.5.2) — this adapter only converts the
    // caller-supplied wire tag through it.
    let at = AliasType::from_str(&alias_type);
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

    // Feedback-taxonomy parsing is a domain decision owned by `kria_core`
    // (`FeedbackSignal::from_str`, task F1.5.2) — this adapter only converts
    // the caller-supplied wire tag through it.
    let d = detail.unwrap_or_default();
    let sig = FeedbackSignal::from_str(&signal, d)
        .ok_or_else(|| format!("unknown feedback signal: {signal}"))?;
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
    require_memory_enabled(st)?;
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
    require_memory_enabled(st)?;
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
    require_memory_enabled(st)?;
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
    require_memory_enabled(st)?;
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
    require_memory_enabled(st)?;
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
    require_memory_enabled(st)?;
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
    require_memory_enabled(st)?;
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
    require_memory_enabled(st)?;
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
    require_memory_enabled(st)?;
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
    require_memory_enabled(st)?;
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
    require_memory_enabled(st)?;
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
    require_memory_enabled(st)?;
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
    require_memory_enabled(st)?;
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
    require_memory_enabled(st)?;
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
    require_memory_enabled(st)?;
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
    require_memory_enabled(st)?;
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

// Scan-source-taxonomy parsing is a domain decision owned by `kria_core`
// (`ScanSource::from_str`, task F1.5.2) — this adapter only converts the
// caller-supplied wire tag through it.
fn scan_source(s: &str) -> Result<kria_core::memory::cold_start::ScanSource, String> {
    kria_core::memory::cold_start::ScanSource::from_str(s)
        .ok_or_else(|| format!("unknown scan source: {s}"))
}

#[tauri::command]
pub async fn memory_cold_start_status(
    state: State<'_, AppStateCell>,
) -> Result<serde_json::Value, String> {
    let st = state.get().ok_or_else(|| INIT_MSG.to_string())?;
    let (onboarding, granted) = st
        .memory_system
        .cold_start_status()
        .map_err(|e| e.to_string())?;
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
    st.memory_system
        .set_cold_start_consent(src, granted)
        .map_err(|e| e.to_string())
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
    require_memory_enabled(st)?;
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
        // MGR-041: honest crypto capability — never falsely claims erasure.
        "crypto_shred_capability": r.crypto_shred_capability,
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

// ── v2 dispatch ───────────────────────────────────────────────────────────────

/// Tauri adapter for the Memory API v2 unified dispatch endpoint (task 3.9.6 /
/// 3.9.8, design §8).
///
/// Accepts the flat envelope that [`buildEnvelope`] in the UI client produces:
///
/// ```json
/// {
///   "operation":      "search",
///   "params":         { … },
///   "correlation_id": "uuid",
///   "deadline_ms":    5000,
///   "revision_base":  42,     // optional
///   "cursor":         "…",    // optional
///   "schema_version": "2.0"   // optional
/// }
/// ```
///
/// Builds a `CallerContext` (Tauri loopback — LocalDesktop, no auth required)
/// and delegates to the `UnifiedRouter` stub which covers both query and command
/// operations. Returns a serialized `GraphResponseV2` on success or an error
/// string that the UI client maps to `UnsupportedCapabilityError` when the word
/// "Unsupported" appears in the message.
///
/// **This is the v2 authority path. Do not delete it.**
#[tauri::command]
pub async fn memory_v2_dispatch(
    operation: String,
    params: Option<serde_json::Value>,
    correlation_id: Option<String>,
    deadline_ms: Option<u64>,
    revision_base: Option<i64>,
    cursor: Option<String>,
    schema_version: Option<String>,
    _state: State<'_, AppStateCell>,
) -> Result<serde_json::Value, String> {
    use kria_core::memory::api::v2::{
        validate_adapter_request, AdapterContext, AdapterKind, CallerContext, GraphRequestV2,
        UnifiedRouter,
    };

    // The desktop is always a local Tauri loopback caller — no authentication
    // token required (AdapterLimits::requires_auth(Tauri) == false).
    let caller: CallerContext = AdapterContext::build_caller_context(
        AdapterKind::Tauri,
        "local-desktop",
        "personal", // default namespace for desktop
        "",         // no scope restriction
        0,          // max sensitivity ceiling (owner's own data)
        "v2",
    );

    // Build the v2 request envelope.
    let request = GraphRequestV2 {
        operation: operation.clone(),
        params_json: params.unwrap_or(serde_json::Value::Object(Default::default())),
        revision: revision_base,
        schema_version: schema_version.unwrap_or_else(|| "2.0".to_string()),
        policy_hash: None,
        cursor,
        deadline_ms,
    };

    // Validate adapter-level capability constraints (e.g. local-only ops are
    // allowed for Tauri, rejected for Axum).
    validate_adapter_request(AdapterKind::Tauri, &request)
        .map_err(|e| format!("Unsupported: {:?}", e))?;

    // correlation_id is carried for trace / future use.
    let _ = correlation_id;

    // Delegate to the UnifiedRouter which tries OperationRouter first (query
    // operations: search/neighborhood/path/…) then CommandRouter (command
    // operations: command.preview/commit/undo/lifecycle/…).
    let response = UnifiedRouter::dispatch(&caller, &request)
        .map_err(|e| serde_json::to_string(&e).unwrap_or_else(|_| format!("{:?}", e)))?;

    serde_json::to_value(&response).map_err(|e| format!("serialization error: {e}"))
}

// ── Dev / demo knowledge seeder ─────────────────────────────────────────────

/// Seed a curated set of realistic demo memories and entities into the memory
/// system so the Knowledge Graph UI has items to show on a fresh install.
///
/// Idempotent: calling it multiple times produces no duplicates because the
/// Write Policy deduplicates by content hash. Safe to call from the UI "Seed
/// demo data" button or on first launch.
///
/// The items mimic real conversational memories — what a user would have after
/// a week of using KRIA. They cover a variety of namespaces, types, and
/// relationship styles so the graph has interesting topology to display.
#[tauri::command]
pub async fn memory_seed_demo_knowledge(
    state: State<'_, AppStateCell>,
) -> Result<serde_json::Value, String> {
    use kria_core::memory::types::WriteCandidate;

    let st = state.get().ok_or_else(|| INIT_MSG.to_string())?;
    require_memory_enabled(st)?;

    let ms = &st.memory_system;

    // Demo memories — varied topics, realistic content, different sources.
    let demo_items: &[(&str, &str)] = &[
        // Personal preferences & context
        ("core", "The user prefers dark mode in all applications and tools."),
        ("core", "User's primary programming languages are Rust and TypeScript."),
        ("core", "User works on KRIA — a local-first AI desktop assistant built with Tauri and SolidJS."),
        ("core", "The user uses an Ubuntu Linux laptop with 16 GB RAM and an i7 processor."),
        // Project knowledge
        ("core", "KRIA uses SQLite as its sole transactional authority — no external database."),
        ("core", "Memory architecture follows a production-grade design: authority store, write policy engine, and rebuild-safe derived indexes."),
        ("core", "The memory graph uses frontier-level batch BFS to avoid N+1 query patterns during traversal."),
        ("core", "KRIA's voice pipeline uses Whisper STT and Piper TTS for hands-free interaction."),
        // Learning & research
        ("core", "Rust's ownership model prevents data races at compile time — zero-cost abstraction over memory safety."),
        ("core", "SolidJS uses fine-grained reactivity with signals — DOM updates are surgical, no virtual DOM diffing."),
        ("core", "RRF (Reciprocal Rank Fusion) combines vector, FTS5, and graph retrieval scores without score normalization."),
        ("core", "WAL mode in SQLite enables concurrent reads during writes, critical for KRIA's memory architecture."),
        // Tool observations
        ("core", "The memory_search Tauri command returns ranked results with provenance trace metadata."),
        ("core", "cargo fmt enforces Rust code style; the KRIA validation hook runs it on every agent stop."),
        ("core", "PropTest property-based testing found that BFS with cycles terminates correctly with visited-set guards."),
        // Goals & plans
        ("core", "Goal: Complete the Memory Graph Production Redesign spec — F0 through F5 gates."),
        ("core", "Goal: Add GPU-accelerated local image generation via ComfyUI integration."),
        ("core", "Goal: Implement wake-word detection for hands-free KRIA activation."),
        // Factual knowledge
        ("core", "The authorize_read gate enforces A5: policy precedes planning, counts, ranking, serialization, and caching."),
        ("core", "Cryptographic shredding requires payload encryption and external key destruction — currently unavailable; relying on OS disk encryption."),
    ];

    let session = kria_core::memory::ids::new_id();
    let mut stored = 0usize;
    let mut skipped = 0usize;

    for (_ns, content) in demo_items {
        let candidate = WriteCandidate {
            namespace_hint: Some("core".to_string()),
            ..WriteCandidate::user(session, *content)
        };
        match ms.remember(candidate) {
            Ok(_) => stored += 1,
            Err(_) => skipped += 1,
        }
    }

    // Flush enrichment so items are immediately searchable and indexed.
    let _ = ms.flush().await;

    // ── Seed graph entities + relationships so the graph has real topology ──
    // Without edges the 3D/2D graph shows only disconnected nodes. These
    // entities and `related_to` edges give the renderer actual structure.
    let edges_created = seed_demo_graph(ms).unwrap_or(0);

    // Return a summary so the UI can show feedback.
    Ok(serde_json::json!({
        "stored": stored,
        "skipped": skipped,
        "total": demo_items.len(),
        "edges": edges_created,
        "message": format!(
            "Seeded {stored} demo memories and {edges_created} graph relationships \
             ({skipped} skipped as duplicates). Knowledge Graph is ready."
        )
    }))
}

/// Seed a small connected entity graph so the Knowledge Graph renderer has
/// visible topology (nodes + edges), not just isolated points.
///
/// Idempotent: uses `INSERT OR IGNORE` keyed on a deterministic identity hash,
/// so repeated calls do not duplicate entities or edges.
fn seed_demo_graph(
    ms: &std::sync::Arc<kria_core::memory::api::MemorySystem>,
) -> Result<usize, String> {
    use kria_core::memory::stores::ports::GraphStore;
    use kria_core::memory::stores::SqliteGraphStore;
    use kria_core::memory::types::Entity;

    let db = ms.database();
    let graph = SqliteGraphStore::new(db.clone());

    // A small realistic knowledge graph: KRIA's own architecture.
    // Deterministic UUIDv5-style ids so re-seeding is idempotent.
    let names: &[&str] = &[
        "KRIA",
        "Memory System",
        "SQLite Authority",
        "Retrieval Engine",
        "Voice Pipeline",
        "Rust",
        "SolidJS",
        "Tauri",
        "Whisper STT",
        "Piper TTS",
        "FTS5 Index",
        "Vector Index",
    ];

    // Stable ids derived from a deterministic hash of the name so seeding twice
    // reuses the same rows (the `uuid` v5 feature is not enabled in this crate).
    fn stable_id(seed: u128, key: &str) -> Uuid {
        // FNV-1a 64-bit over the key, mixed into a fixed namespace constant.
        let mut h: u64 = 0xcbf2_9ce4_8422_2325;
        for b in key.as_bytes() {
            h ^= *b as u64;
            h = h.wrapping_mul(0x0000_0100_0000_01b3);
        }
        Uuid::from_u128(seed ^ ((h as u128) << 32) ^ (h as u128))
    }

    let ns_seed: u128 = 0x4D47_5220_0000_0000_0000_0000_0000_0001;
    let ids: Vec<Uuid> = names.iter().map(|n| stable_id(ns_seed, n)).collect();

    // Insert entities.
    {
        let mut tx = db.begin().map_err(|e| e.to_string())?;
        for (i, name) in names.iter().enumerate() {
            let entity = Entity {
                id: ids[i],
                canonical_id: ids[i],
                entity_type: "concept".to_string(),
                display_name: (*name).to_string(),
                created_at: chrono::Utc::now(),
            };
            graph
                .add_entity(&mut tx, &entity)
                .map_err(|e| e.to_string())?;
        }
        tx.commit().map_err(|e| e.to_string())?;
    }

    // Edges by index pair — a connected architecture graph.
    let edge_pairs: &[(usize, usize)] = &[
        (0, 1),  // KRIA → Memory System
        (0, 3),  // KRIA → Retrieval Engine
        (0, 4),  // KRIA → Voice Pipeline
        (0, 5),  // KRIA → Rust
        (0, 6),  // KRIA → SolidJS
        (0, 7),  // KRIA → Tauri
        (1, 2),  // Memory System → SQLite Authority
        (1, 3),  // Memory System → Retrieval Engine
        (2, 10), // SQLite Authority → FTS5 Index
        (2, 11), // SQLite Authority → Vector Index
        (3, 10), // Retrieval Engine → FTS5 Index
        (3, 11), // Retrieval Engine → Vector Index
        (4, 8),  // Voice Pipeline → Whisper STT
        (4, 9),  // Voice Pipeline → Piper TTS
        (7, 5),  // Tauri → Rust
        (7, 6),  // Tauri → SolidJS
    ];

    let mut created = 0usize;
    let now = chrono::Utc::now().to_rfc3339();
    for (a, b) in edge_pairs {
        let src = ids[*a];
        let tgt = ids[*b];
        // Deterministic identity so re-seeding is a no-op.
        let identity = format!("{src}-{tgt}-related_to");
        let rel_id = stable_id(ns_seed ^ 0x0EDE_0000, &identity);
        let tx = db.begin().map_err(|e| e.to_string())?;
        let n = tx
            .conn()
            .execute(
                "INSERT OR IGNORE INTO relationships_v2(
                     id, source_kind, source_id, target_kind, target_id,
                     relation_name, relation_version, direction_class,
                     valid_from, valid_until, truth_state,
                     namespace, owner_id, scope, sensitivity,
                     policy_source_id, policy_version, identity_hash)
                 VALUES (?1,'entity',?2,'entity',?3,'related_to',1,'directed',?4,NULL,NULL,
                         'core','','global',0,'core','demo-seed',?5)",
                rusqlite::params![
                    rel_id.to_string(),
                    src.to_string(),
                    tgt.to_string(),
                    now,
                    identity,
                ],
            )
            .map_err(|e| e.to_string())?;
        created += n;
        tx.commit().map_err(|e| e.to_string())?;
    }

    Ok(created)
}

/// Return all active memories for Knowledge Graph display.
/// Searches with an empty query to get recent/relevant memories, then
/// returns them shaped as KnowledgeItem DTOs for the UI.
#[tauri::command]
pub async fn memory_knowledge_items(
    limit: Option<usize>,
    state: State<'_, AppStateCell>,
) -> Result<serde_json::Value, String> {
    let st = state.get().ok_or_else(|| INIT_MSG.to_string())?;
    require_memory_enabled(st)?;
    let projection = st
        .memory_system
        .knowledge_projection(limit.unwrap_or(30))
        .await
        .map_err(|error| error.to_string())?;
    serde_json::to_value(projection).map_err(|error| error.to_string())
}
