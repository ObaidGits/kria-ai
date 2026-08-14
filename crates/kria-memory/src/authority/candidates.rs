//! Typed command candidates — the per-writer-category constructors that turn a
//! writer's intent into a governed [`CommandEnvelope`] (task **F1.5.1**, design
//! §19.1 "adapters/writers construct caller/command only").
//!
//! A [`CommandCandidate`] is the *semantic* half of a durable write that a
//! writer knows about: the governed [`CommandKind`], the provenance
//! ([`SourceKind`] + [`SourceTrust`]), and the canonical payload. It deliberately
//! carries **no** caller identity, idempotency key, base revision, mode, or
//! deadline — those are the "who / when / under what authority" context an
//! adapter/composition boundary supplies via [`WriteContext`] when it finalizes
//! the candidate into a validated [`CommandEnvelope`] with
//! [`CommandCandidate::into_envelope`].
//!
//! This split is exactly the F1.5 invariant "adapters construct caller/command
//! only; one command bus": each writer category has a typed constructor here (so
//! payload shape is centralized and never a raw ad-hoc JSON blob at a call
//! site), and the envelope is submitted through the single
//! [`AuthorityCommandBus`](super::bus::AuthorityCommandBus).
//!
//! ## Kind coverage (F1.5.1 scaffolding)
//!
//! The constructors here cover the **observation** (append-a-new-claim) writes
//! of the core/native/conversation/library/feedback/goal/cognition writers —
//! i.e. [`CommandKind::Observe`], the only non-previewed kind. Corrective /
//! lifecycle writes (goal *status* transitions, Memory-Worth updates, decay,
//! forget/restore/hard-delete) map to [`CommandKind::Correct`]/`Forget`/… which
//! are **preview-gated** and belong to the lifecycle preview/confirm flow
//! (F1.7); the concrete cognitive-record persistence for every kind is **F2**.
//! Each payload carries a stable `"candidate"` discriminator so the F2 per-kind
//! semantic builder can dispatch on the writer category without re-parsing free
//! text.

use serde_json::{json, Value};

use crate::error::MemoryResult;
use crate::model::{
    CallerContext, GraphRevision, IdempotencyKey, InvocationId, UtcTimestamp,
};
use crate::types::MemoryMode;

use super::command::{
    CommandEnvelope, Deadline, PreviewToken, SourceContext, SourceKind, SourceTrust,
};
use super::CommandKind;

/// The caller / execution context an adapter supplies to finalize a
/// [`CommandCandidate`] into a governed [`CommandEnvelope`]. Every field is a
/// validated value object, so the finalized envelope can never carry a raw
/// unchecked identifier.
#[derive(Debug, Clone)]
pub struct WriteContext {
    /// The authenticated caller (identity + policy partition).
    pub caller: CallerContext,
    /// The caller-chosen idempotency token (paired with the caller partition).
    pub idempotency_key: IdempotencyKey,
    /// The revision the caller issues against (optimistic concurrency).
    pub base_revision: GraphRevision,
    /// The invocation this write belongs to (start/completion correlation).
    pub invocation_id: InvocationId,
    /// The source's stable identity (`events_v2.source_id`).
    pub source_id: String,
    /// The admission memory mode.
    pub mode: MemoryMode,
    /// The bounded execution deadline.
    pub deadline: Deadline,
}

impl WriteContext {
    /// Build the [`WriteContext`] for a **core-internal** writer (task F1.5.1
    /// cutover of `goals`/`feedback`/`library`/`conversation`/cognition
    /// engines) that has no per-request adapter-authenticated caller of its own
    /// today.
    ///
    /// Every existing caller of these engines (desktop Tauri commands,
    /// dreaming/active-learning/self-improvement background cognition) runs
    /// in-process on the single local device — there is no remote/authenticated
    /// caller for this path yet (single-user pre-production laptop) — so this
    /// asserts [`CallerContext::local_desktop`] on `"kria-core"` under a fixed
    /// internal policy partition. This mirrors the existing precedent of
    /// core-internal provenance tags (`Source::SelfReflection`/`Tool`) that
    /// already write without adapter-level per-request identity threading.
    ///
    /// Real per-request caller-context threading from the Tauri/Axum adapter
    /// through to these engines is **F1.5.2/F1.5.3 scope**, not this
    /// constructor — it exists so the F1.5.1 writer cutover has one correct,
    /// documented seam today rather than fabricating an ad hoc caller at each
    /// call site.
    ///
    /// A fresh [`InvocationId`] and [`IdempotencyKey`] (UUID v7) are minted per
    /// call so unrelated writes never collide on the idempotency key (each call
    /// is a distinct intent); `base_revision` is [`GraphRevision::base`] since
    /// an [`Observe`](CommandKind::Observe) command never checks preview
    /// freshness; `mode` is [`MemoryMode::Permanent`], matching every
    /// converted writer's existing unconditional-admission behavior.
    pub fn internal(source_id: impl Into<String>) -> MemoryResult<Self> {
        let partition = crate::model::PolicyPartition::new("core", "internal", 0)?;
        let caller = CallerContext::local_desktop("kria-core", partition)?;
        Ok(Self {
            caller,
            idempotency_key: IdempotencyKey::new(uuid::Uuid::now_v7().to_string())?,
            base_revision: GraphRevision::base(),
            invocation_id: InvocationId::new_v7(),
            source_id: source_id.into(),
            mode: MemoryMode::Permanent,
            deadline: Deadline::default_write(),
        })
    }
}

/// The typed semantic intent of a durable write, independent of caller context.
///
/// Build one with a per-category constructor (e.g.
/// [`CommandCandidate::native_fact`]), then finalize it with
/// [`into_envelope`](Self::into_envelope) and submit it through the
/// [`AuthorityCommandBus`](super::bus::AuthorityCommandBus).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandCandidate {
    kind: CommandKind,
    source_kind: SourceKind,
    trust: SourceTrust,
    payload: Value,
}

impl CommandCandidate {
    /// Construct a raw candidate. Prefer the per-category constructors; this is
    /// the shared builder they delegate to (and an escape hatch for a category
    /// not yet enumerated). `category` is stamped as the payload's stable
    /// `"candidate"` discriminator so the F2 semantic builder can dispatch.
    pub fn new(
        kind: CommandKind,
        source_kind: SourceKind,
        trust: SourceTrust,
        category: &str,
        mut body: Value,
    ) -> Self {
        // Ensure the payload is an object carrying the discriminator (the
        // validator requires a JSON object for policy inputs).
        if !body.is_object() {
            body = json!({ "value": body });
        }
        if let Value::Object(map) = &mut body {
            map.insert("candidate".to_string(), Value::from(category));
        }
        Self {
            kind,
            source_kind,
            trust,
            payload: body,
        }
    }

    // ── core / native ────────────────────────────────────────────────────

    /// A native/core observation of a durable fact (the in-process cognitive
    /// subsystem asserting a new claim). Trusted local system provenance.
    pub fn native_fact(text: impl Into<String>, category: Option<&str>) -> Self {
        Self::new(
            CommandKind::Observe,
            SourceKind::Native,
            SourceTrust::System,
            "native_fact",
            json!({ "text": text.into(), "category": category }),
        )
    }

    /// A tool/MCP/skill/sidecar meaningful-outcome observation (design §46.1).
    /// The provenance `source_kind`/`trust` are supplied by the caller since a
    /// tool outcome can originate from any invocation source.
    pub fn tool_outcome(
        source_kind: SourceKind,
        trust: SourceTrust,
        content: impl Into<String>,
    ) -> Self {
        Self::new(
            CommandKind::Observe,
            source_kind,
            trust,
            "tool_outcome",
            json!({ "content": content.into() }),
        )
    }

    // ── conversation ──────────────────────────────────────────────────────

    /// A conversation-turn observation. Conversational user/model content is
    /// treated as untrusted provenance until independently verified (design
    /// §7.3).
    pub fn conversation_turn(role: impl Into<String>, content: impl Into<String>) -> Self {
        Self::new(
            CommandKind::Observe,
            SourceKind::Conversation,
            SourceTrust::Untrusted,
            "conversation_turn",
            json!({ "role": role.into(), "content": content.into() }),
        )
    }

    // ── library / document ingestion ───────────────────────────────────────

    /// A library document-chunk ingestion observation. Imported corpus content
    /// is untrusted provenance.
    pub fn library_chunk(
        item_id: impl Into<String>,
        chunk_index: u32,
        text: impl Into<String>,
    ) -> Self {
        Self::new(
            CommandKind::Observe,
            SourceKind::Library,
            SourceTrust::Untrusted,
            "library_chunk",
            json!({
                "item_id": item_id.into(),
                "chunk_index": chunk_index,
                "text": text.into(),
            }),
        )
    }

    // ── feedback ────────────────────────────────────────────────────────

    /// A feedback-signal observation against a target record (design §22.3).
    /// Recording the *signal* is an observation; the Memory-Worth counter
    /// *update* it drives is a correction (F1.7 preview/confirm + F2
    /// persistence).
    pub fn feedback_signal(
        target_id: impl Into<String>,
        target_kind: impl Into<String>,
        signal: impl Into<String>,
    ) -> Self {
        Self::new(
            CommandKind::Observe,
            SourceKind::Native,
            SourceTrust::System,
            "feedback_signal",
            json!({
                "target_id": target_id.into(),
                "target_kind": target_kind.into(),
                "signal": signal.into(),
            }),
        )
    }

    // ── goals ────────────────────────────────────────────────────────────

    /// A new-goal observation (design Priority 1). A goal *status transition* is
    /// a correction (F1.7) rather than an observation.
    pub fn goal(title: impl Into<String>, goal_kind: impl Into<String>) -> Self {
        Self::new(
            CommandKind::Observe,
            SourceKind::Native,
            SourceTrust::System,
            "goal",
            json!({ "title": title.into(), "goal_kind": goal_kind.into() }),
        )
    }

    // ── cognition (reasoning / planning / causal / knowledge gaps) ─────────

    /// A reasoning-trace observation (chains/hypotheses/counterexamples).
    pub fn reasoning_trace(task_label: impl Into<String>, content: impl Into<String>) -> Self {
        Self::new(
            CommandKind::Observe,
            SourceKind::Native,
            SourceTrust::System,
            "reasoning_trace",
            json!({ "task_label": task_label.into(), "content": content.into() }),
        )
    }

    /// A plan-outcome observation (plan-outcome learning).
    pub fn plan_outcome(signature: impl Into<String>, success: bool) -> Self {
        Self::new(
            CommandKind::Observe,
            SourceKind::Native,
            SourceTrust::System,
            "plan_outcome",
            json!({ "signature": signature.into(), "success": success }),
        )
    }

    /// A causal-link observation (cause→effect reasoning).
    pub fn causal_link(cause: impl Into<String>, effect: impl Into<String>, success: bool) -> Self {
        Self::new(
            CommandKind::Observe,
            SourceKind::Native,
            SourceTrust::System,
            "causal_link",
            json!({ "cause": cause.into(), "effect": effect.into(), "success": success }),
        )
    }

    /// A knowledge-gap observation (an unmet retrieval need).
    pub fn knowledge_gap(query: impl Into<String>, domain: Option<&str>) -> Self {
        Self::new(
            CommandKind::Observe,
            SourceKind::Native,
            SourceTrust::System,
            "knowledge_gap",
            json!({ "query": query.into(), "domain": domain }),
        )
    }

    // ── accessors ─────────────────────────────────────────────────────────

    /// The governed command kind.
    pub fn kind(&self) -> CommandKind {
        self.kind
    }

    /// The provenance source kind.
    pub fn source_kind(&self) -> SourceKind {
        self.source_kind
    }

    /// The provenance trust tier.
    pub fn trust(&self) -> SourceTrust {
        self.trust
    }

    /// The canonical payload (with its `"candidate"` discriminator).
    pub fn payload(&self) -> &Value {
        &self.payload
    }

    /// Stamp a canonical `observed_at` timestamp into an observation payload
    /// (deterministic provenance for the F2 builder). Returns `self` for
    /// chaining. No-op for a non-object payload.
    pub fn observed_at(mut self, ts: UtcTimestamp) -> Self {
        if let Value::Object(map) = &mut self.payload {
            map.insert("observed_at".to_string(), Value::from(ts.to_rfc3339()));
        }
        self
    }

    /// Finalize this candidate into a validated governed [`CommandEnvelope`]
    /// using the caller-supplied [`WriteContext`].
    ///
    /// `preview_token` MUST be `None` for an observation and `Some` for a
    /// preview-gated corrective/lifecycle kind — [`CommandEnvelope::new`]
    /// enforces that pairing. Since the constructors here build observations,
    /// callers pass `None`.
    pub fn into_envelope(
        self,
        ctx: WriteContext,
        preview_token: Option<PreviewToken>,
    ) -> MemoryResult<CommandEnvelope> {
        let source = SourceContext::new(
            ctx.invocation_id,
            self.source_kind,
            ctx.source_id,
            self.trust,
        )?;
        CommandEnvelope::new(
            ctx.caller,
            self.kind,
            ctx.idempotency_key,
            ctx.base_revision,
            source,
            ctx.mode,
            ctx.deadline,
            self.payload,
            preview_token,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::PolicyPartition;

    fn ctx(key: &str) -> WriteContext {
        let partition = PolicyPartition::new("user", "chat", 0).unwrap();
        WriteContext {
            caller: CallerContext::local_desktop("local-desktop", partition).unwrap(),
            idempotency_key: IdempotencyKey::new(key).unwrap(),
            base_revision: GraphRevision::base(),
            invocation_id: InvocationId::new_v7(),
            source_id: "core:test".to_string(),
            mode: MemoryMode::Permanent,
            deadline: Deadline::default_write(),
        }
    }

    #[test]
    fn every_category_builds_an_observe_object_payload() {
        let candidates = [
            CommandCandidate::native_fact("f", Some("c")),
            CommandCandidate::tool_outcome(SourceKind::Mcp, SourceTrust::Trusted, "ok"),
            CommandCandidate::conversation_turn("user", "hi"),
            CommandCandidate::library_chunk("item-1", 0, "chunk"),
            CommandCandidate::feedback_signal("rec-1", "memory", "positive"),
            CommandCandidate::goal("ship it", "task"),
            CommandCandidate::reasoning_trace("label", "because"),
            CommandCandidate::plan_outcome("sig", true),
            CommandCandidate::causal_link("a", "b", true),
            CommandCandidate::knowledge_gap("q", Some("d")),
        ];
        for c in candidates {
            assert_eq!(c.kind(), CommandKind::Observe);
            assert!(c.payload().is_object(), "payload must be a JSON object");
            assert!(
                c.payload()
                    .get("candidate")
                    .and_then(Value::as_str)
                    .is_some(),
                "payload carries a category discriminator"
            );
        }
    }

    #[test]
    fn into_envelope_preserves_kind_source_and_payload() {
        let cand = CommandCandidate::library_chunk("item-9", 3, "text");
        let source_kind = cand.source_kind();
        let env = cand.into_envelope(ctx("cmd-lib"), None).unwrap();
        assert_eq!(env.kind(), CommandKind::Observe);
        assert_eq!(env.source().source_kind(), source_kind);
        assert_eq!(
            env.payload().get("candidate").and_then(Value::as_str),
            Some("library_chunk")
        );
        assert_eq!(
            env.payload().get("chunk_index").and_then(Value::as_u64),
            Some(3)
        );
    }

    #[test]
    fn observed_at_stamps_timestamp() {
        let ts = UtcTimestamp::from_rfc3339_utc("2026-01-01T00:00:00Z").unwrap();
        let cand = CommandCandidate::native_fact("f", None).observed_at(ts);
        assert_eq!(
            cand.payload().get("observed_at").and_then(Value::as_str),
            Some("2026-01-01T00:00:00+00:00")
        );
    }
}
