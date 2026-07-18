//! Subsystem / OpenClaw / tool / MCP integration surface (design §45–§46, tasks 33/34).
//!
//! * `SkillMemoryView` — the **read-only, namespace-scoped** view given to
//!   OpenClaw skills (L7/N17). Skills can only read their own namespace + the
//!   public `core`; there is **no write method**, so a skill can never persist
//!   durable state directly — outcomes flow to the orchestrator, which submits
//!   `WriteCandidate`s through the Write Policy (design §45.4).
//! * Tool / MCP / CKB helpers — build provenance-tagged `WriteCandidate`s from
//!   tool outcomes and capability observations (design §46.1/§46.4). The single
//!   integration hook is the orchestrator calling these; every tool, MCP tool,
//!   and skill shares this one path.

use std::sync::Arc;

use uuid::Uuid;

use crate::memory::error::MemoryResult;
use crate::memory::retriever::{RetrievalCtx, RetrievalResult, Retriever};
use crate::memory::types::{MemoryType, Scope, Source, WriteCandidate};

/// A read-only, namespace-scoped memory view for an OpenClaw skill (L7/N17).
///
/// Constructed by the orchestrator via [`crate::memory::api::MemorySystem::skill_view`].
/// It exposes only search/recall, filtered to the skill's namespace plus the
/// public `core`. It holds no write capability by construction.
pub struct SkillMemoryView {
    retriever: Arc<Retriever>,
    namespace: String,
}

impl SkillMemoryView {
    pub(crate) fn new(retriever: Arc<Retriever>, skill_id: &str) -> Self {
        Self {
            retriever,
            namespace: format!("openclaw/{skill_id}"),
        }
    }

    /// The skill's own namespace.
    pub fn namespace(&self) -> &str {
        &self.namespace
    }

    /// Search scoped to the skill's namespace + public `core` (L7). Never
    /// returns other skills' or personal-scope memories.
    pub async fn search(&self, query: &str, token_budget: u32) -> MemoryResult<RetrievalResult> {
        let ctx = RetrievalCtx {
            namespaces: vec![self.namespace.clone(), "core".to_string()],
            scopes: vec![Scope::Global],
            include_secret: false,
            token_budget,
        };
        self.retriever.search(query, &ctx).await
    }
}

/// Minimum substantive payload length for a *successful* outcome to be worth
/// persisting (M5). Shorter successes are routine chatter → telemetry only.
const MIN_SALIENT_LEN: usize = 12;

/// Salience gate for tool/MCP/skill outcomes (M5 — memory-volume control).
///
/// Every tool call used to write a durable memory + wake cognition, so routine
/// successes (`"tool X succeeded: ok"`) grew the store without adding value.
/// This keeps the outcomes worth learning from and drops the rest:
/// - **Failures / errors / denials are always salient** (we learn from them).
/// - **Successes are salient only when substantive** — the payload after the
///   `"… succeeded:"` boilerplate is non-trivial and not a generic ack.
///
/// Deterministic + LLM-free (runs on the write hot path).
pub fn outcome_is_salient(content: &str) -> bool {
    let lc = content.to_lowercase();
    // Failure signals — always keep.
    const FAILURE_MARKERS: &[&str] = &[
        "failed",
        "error",
        "denied",
        "rejected",
        "timed out",
        "timeout",
        "panic",
        "exception",
        "unauthorized",
        "forbidden",
    ];
    if FAILURE_MARKERS.iter().any(|m| lc.contains(m)) {
        return true;
    }
    // Judge the payload after the success boilerplate, if present.
    let payload = content
        .split_once("succeeded:")
        .map(|(_, rest)| rest.trim())
        .unwrap_or_else(|| content.trim());
    if payload.len() < MIN_SALIENT_LEN {
        return false;
    }
    const GENERIC: &[&str] = &[
        "ok",
        "done",
        "success",
        "successful",
        "completed",
        "true",
        "false",
        "null",
        "{}",
        "[]",
        "no output",
        "none",
        "n/a",
    ];
    let normalized = payload.to_lowercase();
    let normalized = normalized.trim().trim_end_matches('.').trim();
    !GENERIC.contains(&normalized)
}

/// Build a `WriteCandidate` from a tool/MCP/skill outcome (design §46.1). The
/// orchestrator calls this and submits the result through the Write Policy —
/// tools never write directly (L3).
pub fn tool_outcome_candidate(
    session_id: Uuid,
    source: Source,
    content: impl Into<String>,
) -> WriteCandidate {
    let namespace = match &source {
        Source::OpenClaw(skill) => Some(format!("openclaw/{skill}")),
        Source::Mcp { server, .. } => Some(format!("mcp/{server}")),
        _ => None,
    };
    WriteCandidate {
        source,
        proposed_type: None,
        namespace_hint: namespace,
        ..WriteCandidate::user(session_id, content)
    }
}

/// Build a capability (CKB) memory candidate from a tool/skill outcome
/// (design §46.4). Stored as `memory_type = capability`; the Planner reads these
/// for tool selection, and Memory Worth decays a tool that stops succeeding.
pub fn capability_candidate(
    session_id: Uuid,
    source: Source,
    success: bool,
    detail: impl Into<String>,
) -> WriteCandidate {
    let verb = if success { "succeeded" } else { "failed" };
    let content = format!("capability {} {verb}: {}", source.tag(), detail.into());
    WriteCandidate {
        source,
        proposed_type: Some(MemoryType::Capability),
        namespace_hint: Some("core".to_string()),
        ..WriteCandidate::user(session_id, content)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tool_outcome_carries_provenance_namespace() {
        let s = Uuid::now_v7();
        let c = tool_outcome_candidate(
            s,
            Source::Mcp {
                server: "github".into(),
                tool: "search".into(),
            },
            "42 open issues",
        );
        assert_eq!(c.namespace_hint.as_deref(), Some("mcp/github"));
        assert!(matches!(c.source, Source::Mcp { .. }));

        let skill = tool_outcome_candidate(s, Source::OpenClaw("pdf".into()), "parsed 3 pages");
        assert_eq!(skill.namespace_hint.as_deref(), Some("openclaw/pdf"));
    }

    #[test]
    fn outcome_salience_keeps_failures_and_drops_trivial_successes() {
        // Failures always salient.
        assert!(outcome_is_salient(
            "tool fs_write failed: permission denied"
        ));
        assert!(outcome_is_salient("tool http_get error: 500 internal"));
        assert!(outcome_is_salient("capability tool:x failed: timed out"));
        // Trivial successes dropped.
        assert!(!outcome_is_salient("tool noop succeeded: ok"));
        assert!(!outcome_is_salient("tool noop succeeded: done"));
        assert!(!outcome_is_salient("tool noop succeeded: {}"));
        assert!(!outcome_is_salient("tool noop succeeded: "));
        // Substantive successes kept.
        assert!(outcome_is_salient(
            "tool web_search succeeded: found 3 relevant docs about the axum router"
        ));
        assert!(outcome_is_salient(
            "tool file_read succeeded: /etc/hosts maps localhost to 127.0.0.1"
        ));
    }

    #[test]
    fn capability_candidate_is_typed() {
        let s = Uuid::now_v7();
        let c = capability_candidate(s, Source::Tool("file_ops".into()), true, "read 10 files");
        assert_eq!(c.proposed_type, Some(MemoryType::Capability));
        assert!(c.content.contains("succeeded"));
        assert!(c.content.contains("tool:file_ops"));
    }
}
