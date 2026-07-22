//! The single memory-API contract (memory-upgrade API-1).
//!
//! One canonical set of operations over `&MemorySystem` that produce the
//! authoritative JSON response shapes. Both hosts — the desktop Tauri commands
//! and the standalone server's Axum routes — are **thin adapters** over these
//! functions, so the two surfaces can never drift (identical field names,
//! identical structure). Transport concerns (HTTP status codes, Tauri
//! `Result<_, String>`) stay in the adapters; the shaping lives here.
//!
//! Every function delegates to the [`MemorySystem`] façade — there is no memory
//! logic here and no parallel path.

use serde_json::{json, Value};
use uuid::Uuid;

use crate::memory::api::MemorySystem;
use crate::memory::error::{MemoryResult, StorageError};
use crate::memory::lifecycle::ForgetScope;
use crate::memory::retriever::RetrievalHit;
use crate::memory::types::WriteCandidate;

/// Canonical JSON for a single retrieval hit (shared by search + reason).
pub fn hit_json(h: &RetrievalHit) -> Value {
    json!({
        "id": h.memory.id.to_string(),
        "content": h.memory.content,
        "memory_type": h.memory.memory_type.as_str(),
        "confidence": h.memory.confidence,
        "importance": h.memory.importance,
        "score": h.score,
        "strategies": h.strategies,
    })
}

/// Parse a `(kind, value)` pair into a [`ForgetScope`] (shared scope contract).
pub fn parse_scope(kind: &str, value: &str) -> MemoryResult<ForgetScope> {
    let bad_uuid = |v: &str| -> crate::memory::error::MemoryError {
        StorageError::Serde(format!("invalid uuid: {v}")).into()
    };
    match kind {
        "memory" => Ok(ForgetScope::Memory(
            Uuid::parse_str(value).map_err(|_| bad_uuid(value))?,
        )),
        "source" => Ok(ForgetScope::SourcePrefix(value.to_string())),
        "session" => Ok(ForgetScope::Session(
            Uuid::parse_str(value).map_err(|_| bad_uuid(value))?,
        )),
        other => Err(StorageError::Serde(format!("unknown scope kind: {other}")).into()),
    }
}

fn parse_id(s: &str) -> MemoryResult<Uuid> {
    Uuid::parse_str(s).map_err(|_| StorageError::Serde(format!("invalid uuid: {s}")).into())
}

// ── Read surface ──

pub async fn search(ms: &MemorySystem, q: &str, limit: usize) -> MemoryResult<Value> {
    let res = ms.search(q, None).await?;
    let hits: Vec<Value> = res.hits.iter().take(limit).map(hit_json).collect();
    Ok(json!({
        "query": q,
        "results": hits,
        "count": hits.len(),
        "trace": {
            "query_class": res.trace.query_class,
            "vector_used": res.trace.vector_used,
            "fts_used": res.trace.fts_used,
            "candidates": res.trace.candidates,
            "returned": res.trace.returned,
        }
    }))
}

pub async fn reason(ms: &MemorySystem, q: &str, limit: usize) -> MemoryResult<Value> {
    let ctx = ms.reason(q, None).await?;
    let hits: Vec<Value> = ctx
        .retrieval
        .hits
        .iter()
        .take(limit)
        .map(hit_json)
        .collect();
    Ok(json!({
        "query": q,
        "results": hits,
        "count": hits.len(),
        "reasoning_context": ctx.reasoning,
        "planner_context": ctx.goals,
        "plan_recommendation": ctx.plan,
    }))
}

pub async fn health(ms: &MemorySystem) -> MemoryResult<Value> {
    let h = ms.health().await?;
    Ok(json!({
        "api_version": h.api_version,
        "schema_version": h.schema_version,
        "embedder": format!("{:?}", h.embedder),
        "event_count": h.event_count,
        "memory_count": h.memory_count,
        "pending_enrichment": h.pending_enrichment,
    }))
}

pub fn metrics(ms: &MemorySystem) -> MemoryResult<Value> {
    let m = ms.metrics()?;
    Ok(json!({
        "active_memories": m.active_memories,
        "unresolved_gaps": m.unresolved_gaps,
        "tool_outcomes": {
            "seen": m.tool_outcomes.seen,
            "persisted": m.tool_outcomes.persisted,
            "gated": m.tool_outcomes.gated,
        },
        "summary": m.summary(),
    }))
}

pub fn timeline(ms: &MemorySystem, limit: usize) -> MemoryResult<Value> {
    ms.ensure_enabled()?;
    let entries = ms.research().timeline(limit)?;
    let out: Vec<Value> = entries
        .iter()
        .map(|e| {
            json!({ "id": e.id, "content": e.content, "memory_type": e.memory_type,
                    "confidence": e.confidence, "created_at": e.created_at })
        })
        .collect();
    Ok(json!({ "entries": out, "count": out.len() }))
}

pub fn goals(ms: &MemorySystem, limit: usize) -> MemoryResult<Value> {
    ms.ensure_enabled()?;
    let gs = ms.goals().active_goals(limit)?;
    let out: Vec<Value> = gs
        .iter()
        .map(|g| {
            json!({ "id": g.id.to_string(), "title": g.title, "status": g.status.as_str(),
                    "priority": g.priority, "confidence": g.confidence })
        })
        .collect();
    Ok(json!({ "goals": out, "count": out.len() }))
}

pub fn plans(ms: &MemorySystem) -> MemoryResult<Value> {
    ms.ensure_enabled()?;
    let a = ms.plans().analytics()?;
    Ok(json!({
        "distinct_plans": a.distinct_plans,
        "total_executions": a.total_executions,
        "success_rate": a.success_rate(),
    }))
}

pub fn reasoning(ms: &MemorySystem) -> MemoryResult<Value> {
    ms.ensure_enabled()?;
    let a = ms.reasoning().analytics()?;
    Ok(json!({
        "chains": a.chains,
        "hypotheses": a.hypotheses,
        "counterexamples": a.counterexamples,
        "failed_chains": a.failed_chains,
        "avg_confidence": a.avg_confidence,
        "hallucination_rate": a.hallucination_rate(),
    }))
}

pub fn research(ms: &MemorySystem) -> MemoryResult<Value> {
    ms.ensure_enabled()?;
    let m = ms.research().meta()?;
    Ok(json!({
        "active": m.active,
        "archived": m.archived,
        "superseded": m.superseded,
        "avg_confidence": m.avg_confidence,
        "avg_worth": m.avg_worth,
    }))
}

pub fn graph(ms: &MemorySystem, limit: usize) -> MemoryResult<Value> {
    ms.ensure_enabled()?;
    let hits = ms.graph_intelligence().degree_centrality(limit)?;
    let out: Vec<Value> = hits
        .iter()
        .map(|h| {
            json!({ "entity": h.entity.to_string(), "display_name": h.display_name, "degree": h.degree })
        })
        .collect();
    Ok(json!({ "nodes": out, "count": out.len() }))
}

pub fn library(ms: &MemorySystem) -> MemoryResult<Value> {
    ms.ensure_enabled()?;
    let items = ms.library().list_items()?;
    let out: Vec<Value> = items
        .iter()
        .map(|(item, chunks)| {
            json!({ "doc_id": item.id.to_string(), "title": item.title, "path": item.path,
                    "version": item.version, "chunks": chunks })
        })
        .collect();
    Ok(json!({ "documents": out, "count": out.len() }))
}

/// Returns `Ok(None)` when the memory id is not found (adapters map to 404).
pub fn explain(ms: &MemorySystem, id: &str) -> MemoryResult<Option<Value>> {
    let id = parse_id(id)?;
    Ok(ms.explain(id)?.map(|e| {
        json!({
            "id": e.id.to_string(), "content": e.content, "state": e.state,
            "confidence": e.confidence, "source_event_tag": e.source_event_tag,
            "worth_success": e.worth_success, "worth_failure": e.worth_failure,
            "contradicts": e.contradicts.len(), "staleness_class": e.staleness_class,
        })
    }))
}

pub fn report(ms: &MemorySystem) -> MemoryResult<Value> {
    let r = ms.memory_health_report()?;
    Ok(json!({
        "total_active": r.total_active,
        "total_archived": r.total_archived,
        "total_superseded": r.total_superseded,
        "total_forgotten": r.total_forgotten,
        "avg_confidence": r.avg_confidence,
        "unresolved_contradictions": r.unresolved_contradictions,
        "knowledge_gaps": r.knowledge_gaps,
        "enrichment_backlog": r.enrichment_backlog,
        "by_type": r.by_type.iter().map(|(k, v)| json!({ "label": k, "count": v })).collect::<Vec<_>>(),
    }))
}

// ── Write / mutation surface ──

pub fn remember(ms: &MemorySystem, text: impl Into<String>) -> MemoryResult<Value> {
    let d = ms.remember(WriteCandidate::global(text.into()))?;
    Ok(json!({ "decision": format!("{d:?}") }))
}

pub fn forget(ms: &MemorySystem, kind: &str, value: &str) -> MemoryResult<Value> {
    let n = ms.forget(parse_scope(kind, value)?)?;
    Ok(json!({ "forgotten": n }))
}

pub async fn hard_delete(ms: &MemorySystem, kind: &str, value: &str) -> MemoryResult<Value> {
    let n = ms.hard_delete(parse_scope(kind, value)?).await?;
    Ok(json!({ "deleted": n }))
}

pub fn verify(ms: &MemorySystem, id: &str) -> MemoryResult<Value> {
    let ok = ms.verify(parse_id(id)?)?;
    Ok(json!({ "verified": ok }))
}

pub async fn reflect(ms: &MemorySystem) -> MemoryResult<Value> {
    let n = ms.reflect().await?;
    Ok(json!({ "accepted": n }))
}

pub async fn consolidate(ms: &MemorySystem, id: &str) -> MemoryResult<Value> {
    let n = ms.consolidate(parse_id(id)?).await?;
    Ok(json!({ "accepted": n }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::memory::api::MemoryConfig;
    use crate::memory::types::{Availability, ModelVersion};
    use async_trait::async_trait;
    use std::sync::Arc;

    struct FakeEmbedder;
    #[async_trait]
    impl crate::memory::stores::ports::Embedder for FakeEmbedder {
        fn model_version(&self) -> ModelVersion {
            ModelVersion("fake_v1".into())
        }
        fn dim(&self) -> usize {
            8
        }
        async fn embed(&self, texts: &[String]) -> MemoryResult<Vec<Vec<f32>>> {
            Ok(texts.iter().map(|_| vec![0.1f32; 8]).collect())
        }
        async fn health(&self) -> Availability {
            Availability::Up
        }
    }

    #[tokio::test]
    async fn contract_shapes_are_stable() {
        let ms =
            MemorySystem::open_for_test(MemoryConfig::default(), Arc::new(FakeEmbedder)).unwrap();
        // Write + read through the shared contract.
        let d = remember(&ms, "the contract layer is the single source of truth").unwrap();
        assert!(d.get("decision").is_some());
        ms.flush().await.unwrap();

        let s = search(&ms, "contract", 10).await.unwrap();
        assert!(s.get("results").unwrap().is_array());
        assert!(s.get("trace").unwrap().get("query_class").is_some());

        let h = health(&ms).await.unwrap();
        assert!(h.get("memory_count").is_some());
        assert!(h.get("api_version").is_some());
        // AUD-01: pending-enrichment gauge is part of the health contract.
        assert!(h.get("pending_enrichment").is_some());

        // AUD-02: metrics contract carries tool-outcome telemetry.
        let mx = metrics(&ms).unwrap();
        let to = mx.get("tool_outcomes").expect("tool_outcomes present");
        assert!(to.get("seen").is_some());
        assert!(to.get("persisted").is_some());
        assert!(to.get("gated").is_some());

        // Scope parsing contract.
        assert!(parse_scope("source", "tool:x").is_ok());
        assert!(parse_scope("bogus", "x").is_err());
        assert!(explain(&ms, "not-a-uuid").is_err());
    }
}
