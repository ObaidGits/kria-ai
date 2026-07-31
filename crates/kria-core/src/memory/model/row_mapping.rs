//! SQLite row → typed cognitive-record projection (design §4.2/§4.3; tasks
//! F2.1.5, seeds F3 read projection).
//!
//! Every function here reconstructs a validated model value object from a
//! `records` / `entities_v2` / … row read back out of the authority. They are
//! the **read** half of the SQL↔Rust boundary: each column is projected through
//! the same validated value objects the write boundary uses
//! ([`RecordId`]/[`UtcTimestamp`]/[`PolicyPartition`]/[`ValidInterval`]/…), so a
//! row that cannot form a valid typed value fails with a typed
//! [`StorageError::Encoding`] rather than silently yielding a corrupt struct.
//!
//! ## Policy-column ↔ [`PolicyPartition`] mapping
//!
//! The schema stores the policy partition as four columns
//! (`namespace`/`owner_id`/`scope`/`sensitivity`) with `owner_id` declared
//! `NOT NULL`, whereas [`PolicyPartition`]'s owner is optional. The lossless
//! bijection used on both read and write is: an absent owner (`None`) ↔ the
//! empty string `""`. This is unambiguous because [`PolicyPartition`] rejects an
//! empty-but-present owner, so a non-empty column value is always `Some` and
//! `""` is always `None`.
//!
//! ## Malformed-row isolation (MGR-034)
//!
//! [`read_isolated`] maps each row independently and collects a per-row
//! `Result`, so one malformed row (a bad UUID, a non-UTC timestamp, an
//! unrecognized closed-set value) surfaces as an `Err` for *that row only* and
//! never aborts the batch or corrupts the valid rows around it — no panic, no
//! silent data loss.

use rusqlite::{Connection, Row};

use super::entity::Span;
use super::record::RecordPayload;
use super::relation_registry::{
    DirectionClass, EndpointKind, EvidencePolicy, RelationDefinition, RelationName, ValidityPolicy,
};
use super::truth::TruthState;
use super::{
    encoding_err, Alias, AliasId, ConsolidationLevel, ConsolidationRun, ConsolidationRunId, Entity,
    EntityId, Episode, EpisodeId, EventId, Evidence, EvidenceId, EvidencePolarity, Feedback,
    FeedbackId, Goal, GoalId, GoalProgress, GoalProgressId, GoalStatus, GraphRevision, Mention,
    MentionId, PolicyPartition, Record, RecordId, RecordKind, RetrievalTrace, RetrievalTraceId,
    RetrievalTraceItem, SchemaVersion, SourceId, SourceRecord, ToolObservation, ToolObservationId,
    UtcTimestamp, ValidInterval, Version,
};
use crate::memory::authority::command::SourceKind;
use crate::memory::error::{MemoryError, MemoryResult, StorageError};
use crate::memory::types::StalenessClass;

// ── small column helpers ─────────────────────────────────────────────────

/// Wrap a rusqlite column fetch into the memory error taxonomy.
#[inline]
fn sql<T>(r: rusqlite::Result<T>) -> MemoryResult<T> {
    r.map_err(|e| StorageError::Sqlite(e).into())
}

/// A required RFC3339-UTC timestamp column.
fn ts_req(row: &Row<'_>, col: &str) -> MemoryResult<UtcTimestamp> {
    let s: String = sql(row.get(col))?;
    UtcTimestamp::from_rfc3339_utc(&s)
}

/// An optional RFC3339-UTC timestamp column.
fn ts_opt(row: &Row<'_>, col: &str) -> MemoryResult<Option<UtcTimestamp>> {
    match sql(row.get::<_, Option<String>>(col))? {
        Some(s) => Ok(Some(UtcTimestamp::from_rfc3339_utc(&s)?)),
        None => Ok(None),
    }
}

/// A required canonical-UUID id column, wrapped by `ctor`.
fn id_req<T>(
    row: &Row<'_>,
    col: &str,
    ctor: impl FnOnce(String) -> MemoryResult<T>,
) -> MemoryResult<T> {
    let s: String = sql(row.get(col))?;
    ctor(s)
}

/// An optional canonical-UUID id column, wrapped by `ctor`.
fn id_opt<T>(
    row: &Row<'_>,
    col: &str,
    ctor: impl FnOnce(String) -> MemoryResult<T>,
) -> MemoryResult<Option<T>> {
    match sql(row.get::<_, Option<String>>(col))? {
        Some(s) => Ok(Some(ctor(s)?)),
        None => Ok(None),
    }
}

/// An optional forward-compatible [`TruthState`] column (unknown → `Other`).
fn truth_opt(row: &Row<'_>, col: &str) -> MemoryResult<Option<TruthState>> {
    Ok(sql(row.get::<_, Option<String>>(col))?.map(|s| s.parse::<TruthState>().unwrap()))
}

/// The `namespace`/`owner_id`/`scope`/`sensitivity` policy columns →
/// [`PolicyPartition`]. `owner_id = ""` maps to an absent owner (`None`).
fn policy(row: &Row<'_>) -> MemoryResult<PolicyPartition> {
    let namespace: String = sql(row.get("namespace"))?;
    let owner_id: String = sql(row.get("owner_id"))?;
    let scope: String = sql(row.get("scope"))?;
    let sensitivity: i64 = sql(row.get("sensitivity"))?;
    if !(0..=i64::from(u8::MAX)).contains(&sensitivity) {
        return Err(encoding_err(format!(
            "policy sensitivity {sensitivity} out of byte range"
        )));
    }
    let owner = if owner_id.is_empty() {
        None
    } else {
        Some(owner_id)
    };
    PolicyPartition::with_owner(namespace, scope, sensitivity as u8, owner)
}

/// The half-open `valid_from`/`valid_until` interval columns → [`ValidInterval`].
fn interval(row: &Row<'_>) -> MemoryResult<ValidInterval> {
    ValidInterval::new(ts_opt(row, "valid_from")?, ts_opt(row, "valid_until")?)
}

/// An optional non-negative integer column read as `u32`.
fn u32_opt(row: &Row<'_>, col: &str) -> MemoryResult<Option<u32>> {
    match sql(row.get::<_, Option<i64>>(col))? {
        Some(v) if (0..=i64::from(u32::MAX)).contains(&v) => Ok(Some(v as u32)),
        Some(v) => Err(encoding_err(format!("{col} value {v} out of u32 range"))),
        None => Ok(None),
    }
}

/// An optional non-negative integer column read as `u64`.
fn u64_opt(row: &Row<'_>, col: &str) -> MemoryResult<Option<u64>> {
    match sql(row.get::<_, Option<i64>>(col))? {
        Some(v) if v >= 0 => Ok(Some(v as u64)),
        Some(v) => Err(encoding_err(format!("{col} value {v} out of u64 range"))),
        None => Ok(None),
    }
}

/// A boolean stored as `INTEGER CHECK IN (0,1)`. Any other value is a canonical
/// encoding fault.
fn bool_col(row: &Row<'_>, col: &str) -> MemoryResult<bool> {
    match sql(row.get::<_, i64>(col))? {
        0 => Ok(false),
        1 => Ok(true),
        v => Err(encoding_err(format!(
            "{col} value {v} is not a canonical boolean (0/1)"
        ))),
    }
}

// ── records ────────────────────────────────────────────────────────────────

/// Reconstruct a [`Record`] from a `records` row (`SELECT *`).
pub fn record(row: &Row<'_>) -> MemoryResult<Record> {
    let content: Option<String> = sql(row.get("content"))?;
    let cipher: Option<Vec<u8>> = sql(row.get("content_cipher"))?;
    let payload = match (content, cipher) {
        (Some(c), None) => RecordPayload::Plaintext(c),
        (None, Some(b)) => RecordPayload::Ciphertext(b),
        (Some(_), Some(_)) => {
            return Err(encoding_err(
                "records row has both content and content_cipher (payload exclusivity)",
            ))
        }
        (None, None) => {
            return Err(encoding_err(
                "records row has neither content nor content_cipher",
            ))
        }
    };
    let schema_version: i64 = sql(row.get("schema_version"))?;
    let staleness: Option<StalenessClass> =
        sql(row.get::<_, Option<String>>("staleness_class"))?.map(|s| s.parse().unwrap());
    Ok(Record {
        id: id_req(row, "id", RecordId::new)?,
        record_kind: {
            let k: String = sql(row.get("record_kind"))?;
            k.parse::<RecordKind>()?
        },
        schema_version: SchemaVersion::new(u32::try_from(schema_version).map_err(|_| {
            encoding_err(format!("schema_version {schema_version} out of u32 range"))
        })?),
        payload,
        content_hash: sql(row.get("content_hash"))?,
        truth_state: truth_opt(row, "truth_state")?,
        staleness_class: staleness,
        valid_interval: interval(row)?,
        policy: policy(row)?,
        source_id: id_req(row, "source_id", SourceId::new)?,
        policy_version: sql(row.get("policy_version"))?,
        created_event_id: id_req(row, "created_event_id", EventId::new)?,
        created_at: ts_req(row, "created_at")?,
        superseded_by: id_opt(row, "superseded_by", RecordId::new)?,
        episode_id: id_opt(row, "episode_id", EpisodeId::new)?,
        goal_context_id: id_opt(row, "goal_context_id", GoalId::new)?,
        estimated_tokens: u32_opt(row, "estimated_tokens")?,
        shred_key_id: sql(row.get("shred_key_id"))?,
        key_version: u32_opt(row, "key_version")?,
    })
}

// ── entities ─────────────────────────────────────────────────────────────

/// Reconstruct an [`Entity`] from an `entities_v2` row.
pub fn entity(row: &Row<'_>) -> MemoryResult<Entity> {
    Ok(Entity {
        id: id_req(row, "id", EntityId::new)?,
        canonical_id: id_opt(row, "canonical_id", EntityId::new)?,
        entity_type: sql(row.get("entity_type"))?,
        display_name: sql(row.get("display_name"))?,
        normalized_name: sql(row.get("normalized_name"))?,
        truth_state: truth_opt(row, "truth_state")?,
        policy: policy(row)?,
        source_id: id_req(row, "source_id", SourceId::new)?,
        policy_version: sql(row.get("policy_version"))?,
        created_event_id: id_req(row, "created_event_id", EventId::new)?,
        created_at: ts_req(row, "created_at")?,
        revision: u64_opt(row, "revision")?,
    })
}

// ── aliases ────────────────────────────────────────────────────────────────

/// Reconstruct an [`Alias`] from an `aliases` row.
pub fn alias(row: &Row<'_>) -> MemoryResult<Alias> {
    Ok(Alias {
        id: id_req(row, "id", AliasId::new)?,
        entity_id: id_req(row, "entity_id", EntityId::new)?,
        alias: sql(row.get("alias"))?,
        normalized_alias: sql(row.get("normalized_alias"))?,
        alias_type: sql(row.get("alias_type"))?,
        truth_state: truth_opt(row, "truth_state")?,
        policy: policy(row)?,
        source_id: id_req(row, "source_id", SourceId::new)?,
        policy_version: sql(row.get("policy_version"))?,
        created_event_id: id_opt(row, "created_event_id", EventId::new)?,
        created_at: ts_opt(row, "created_at")?,
        valid_interval: interval(row)?,
    })
}

// ── mentions ─────────────────────────────────────────────────────────────

/// Reconstruct a [`Mention`] from a `mentions` row.
pub fn mention(row: &Row<'_>) -> MemoryResult<Mention> {
    let span_start: Option<i64> = sql(row.get("span_start"))?;
    let span_end: Option<i64> = sql(row.get("span_end"))?;
    let span = match (span_start, span_end) {
        (Some(a), Some(b)) => {
            let a = u32::try_from(a)
                .map_err(|_| encoding_err(format!("span_start {a} out of u32 range")))?;
            let b = u32::try_from(b)
                .map_err(|_| encoding_err(format!("span_end {b} out of u32 range")))?;
            Some(Span::new(a, b)?)
        }
        (None, None) => None,
        _ => {
            return Err(encoding_err(
                "mention has a partial span (exactly one of span_start/span_end is NULL)",
            ))
        }
    };
    Ok(Mention {
        id: id_req(row, "id", MentionId::new)?,
        record_id: sql(row.get("record_id"))?,
        record_kind: sql(row.get("record_kind"))?,
        entity_id: id_req(row, "entity_id", EntityId::new)?,
        locator_json: sql(row.get("locator_json"))?,
        span,
        role: sql(row.get("role"))?,
        extractor: sql(row.get("extractor"))?,
        extractor_version: sql(row.get("extractor_version"))?,
        score: sql(row.get("score"))?,
        score_semantics: sql(row.get("score_semantics"))?,
        policy: policy(row)?,
        source_id: id_req(row, "source_id", SourceId::new)?,
        policy_version: sql(row.get("policy_version"))?,
        observed_at: ts_opt(row, "observed_at")?,
        created_event_id: id_opt(row, "created_event_id", EventId::new)?,
    })
}

// ── evidence ─────────────────────────────────────────────────────────────

/// Reconstruct an [`Evidence`] from an `evidence_v2` row.
pub fn evidence(row: &Row<'_>) -> MemoryResult<Evidence> {
    let polarity = {
        let p: String = sql(row.get("polarity"))?;
        p.parse::<EvidencePolarity>()?
    };
    Ok(Evidence {
        id: id_req(row, "id", EvidenceId::new)?,
        subject_kind: sql(row.get("subject_kind"))?,
        subject_id: sql(row.get("subject_id"))?,
        source_record_kind: sql(row.get("source_record_kind"))?,
        source_record_id: sql(row.get("source_record_id"))?,
        source_event_id: id_opt(row, "source_event_id", EventId::new)?,
        locator_json: sql(row.get("locator_json"))?,
        actor_id: sql(row.get("actor_id"))?,
        method: sql(row.get("method"))?,
        method_version: sql(row.get("method_version"))?,
        polarity,
        score: sql(row.get("score"))?,
        score_semantics: sql(row.get("score_semantics"))?,
        policy: policy(row)?,
        source_id: id_req(row, "source_id", SourceId::new)?,
        policy_version: sql(row.get("policy_version"))?,
        observed_at: ts_opt(row, "observed_at")?,
        removed_at: ts_opt(row, "removed_at")?,
        created_event_id: id_opt(row, "created_event_id", EventId::new)?,
    })
}

// ── episodes ─────────────────────────────────────────────────────────────

/// Reconstruct an [`Episode`] from an `episodes_v2` row.
pub fn episode(row: &Row<'_>) -> MemoryResult<Episode> {
    Ok(Episode {
        id: id_req(row, "id", EpisodeId::new)?,
        session_id: sql(row.get("session_id"))?,
        task_id: sql(row.get("task_id"))?,
        policy: policy(row)?,
        source_id: id_req(row, "source_id", SourceId::new)?,
        policy_version: sql(row.get("policy_version"))?,
        opened_at: ts_opt(row, "opened_at")?,
        closed_at: ts_opt(row, "closed_at")?,
        boundary_reason: sql(row.get("boundary_reason"))?,
        cursor_event_id: id_opt(row, "cursor_event_id", EventId::new)?,
        truth_state: truth_opt(row, "truth_state")?,
        revision: u64_opt(row, "revision")?,
    })
}

// ── goals ──────────────────────────────────────────────────────────────────

/// Reconstruct a [`Goal`] from a `goals_v2` row.
pub fn goal(row: &Row<'_>) -> MemoryResult<Goal> {
    let status = {
        let s: String = sql(row.get("status"))?;
        s.parse::<GoalStatus>()?
    };
    let priority = match sql(row.get::<_, Option<i64>>("priority"))? {
        Some(p) => {
            let p = u8::try_from(p)
                .map_err(|_| encoding_err(format!("goal priority {p} out of byte range")))?;
            Some(Goal::validate_priority(p)?)
        }
        None => None,
    };
    Ok(Goal {
        id: id_req(row, "id", GoalId::new)?,
        kind: sql(row.get("kind"))?,
        title: sql(row.get("title"))?,
        status,
        priority,
        score: sql(row.get("score"))?,
        score_semantics: sql(row.get("score_semantics"))?,
        resumption_context: sql(row.get("resumption_context"))?,
        policy: policy(row)?,
        source_id: id_req(row, "source_id", SourceId::new)?,
        policy_version: sql(row.get("policy_version"))?,
        created_event_id: id_opt(row, "created_event_id", EventId::new)?,
        created_at: ts_opt(row, "created_at")?,
        updated_at: ts_opt(row, "updated_at")?,
        revision: u64_opt(row, "revision")?,
    })
}

// ── goal progress ──────────────────────────────────────────────────────────

/// Reconstruct a [`GoalProgress`] from a `goal_progress` row.
pub fn goal_progress(row: &Row<'_>) -> MemoryResult<GoalProgress> {
    Ok(GoalProgress {
        id: id_req(row, "id", GoalProgressId::new)?,
        goal_id: id_req(row, "goal_id", GoalId::new)?,
        event_id: id_opt(row, "event_id", EventId::new)?,
        state: sql(row.get("state"))?,
        summary: sql(row.get("summary"))?,
        observed_at: ts_opt(row, "observed_at")?,
        revision: u64_opt(row, "revision")?,
    })
}

// ── consolidation runs ─────────────────────────────────────────────────────

/// Reconstruct a [`ConsolidationRun`] from a `consolidation_runs` row.
pub fn consolidation_run(row: &Row<'_>) -> MemoryResult<ConsolidationRun> {
    let level = {
        let l: String = sql(row.get("level"))?;
        l.parse::<ConsolidationLevel>()?
    };
    Ok(ConsolidationRun {
        id: id_req(row, "id", ConsolidationRunId::new)?,
        algorithm: sql(row.get("algorithm"))?,
        version: sql(row.get("version"))?,
        input_set_hash: sql(row.get("input_set_hash"))?,
        level,
        cursor: sql(row.get("cursor"))?,
        status: sql(row.get("status"))?,
        started_at: ts_opt(row, "started_at")?,
        completed_at: ts_opt(row, "completed_at")?,
        output_id: sql(row.get("output_id"))?,
        error_code: sql(row.get("error_code"))?,
    })
}

// ── sources ────────────────────────────────────────────────────────────────

/// Reconstruct a [`SourceRecord`] from a `sources` row.
pub fn source(row: &Row<'_>) -> MemoryResult<SourceRecord> {
    let source_kind = match sql(row.get::<_, Option<String>>("source_kind"))? {
        Some(s) => s.parse::<SourceKind>()?,
        None => return Err(encoding_err("sources row missing source_kind")),
    };
    Ok(SourceRecord {
        id: id_req(row, "id", SourceId::new)?,
        source_kind,
        external_identity: sql(row.get("external_identity"))?,
        version: sql(row.get("version"))?,
        trust_class: sql(row.get("trust_class"))?,
        policy: policy(row)?,
        policy_version: sql(row.get("policy_version"))?,
        consent_state: sql(row.get("consent_state"))?,
        content_hash: sql(row.get("content_hash"))?,
        lifecycle_state: sql(row.get("lifecycle_state"))?,
        cursor_json: sql(row.get("cursor_json"))?,
        created_at: ts_opt(row, "created_at")?,
        updated_at: ts_opt(row, "updated_at")?,
    })
}

// ── tool observations ──────────────────────────────────────────────────────

/// Reconstruct a [`ToolObservation`] from a `tool_observations` row.
pub fn tool_observation(row: &Row<'_>) -> MemoryResult<ToolObservation> {
    Ok(ToolObservation {
        id: id_req(row, "id", ToolObservationId::new)?,
        invocation_id: sql(row.get("invocation_id"))?,
        tool_kind: sql(row.get("tool_kind"))?,
        tool_id: sql(row.get("tool_id"))?,
        tool_version: sql(row.get("tool_version"))?,
        capability_id: sql(row.get("capability_id"))?,
        outcome: sql(row.get("outcome"))?,
        goal_id: id_opt(row, "goal_id", GoalId::new)?,
        environment_class: sql(row.get("environment_class"))?,
        input_fingerprint: sql(row.get("input_fingerprint"))?,
        result_summary: sql(row.get("result_summary"))?,
        error_class: sql(row.get("error_class"))?,
        latency_ms: u64_opt(row, "latency_ms")?,
        retry_count: u32_opt(row, "retry_count")?,
        recovery_action: sql(row.get("recovery_action"))?,
        policy: policy(row)?,
        source_id: id_req(row, "source_id", SourceId::new)?,
        policy_version: sql(row.get("policy_version"))?,
        start_event_id: id_opt(row, "start_event_id", EventId::new)?,
        completion_event_id: id_opt(row, "completion_event_id", EventId::new)?,
        created_at: ts_opt(row, "created_at")?,
    })
}

// ── retrieval traces ───────────────────────────────────────────────────────

/// Reconstruct a [`RetrievalTrace`] from a `retrieval_traces` row.
pub fn retrieval_trace(row: &Row<'_>) -> MemoryResult<RetrievalTrace> {
    Ok(RetrievalTrace {
        id: id_req(row, "id", RetrievalTraceId::new)?,
        response_id: sql(row.get("response_id"))?,
        task_id: sql(row.get("task_id"))?,
        query_hash: sql(row.get("query_hash"))?,
        query_class: sql(row.get("query_class"))?,
        classifier_version: sql(row.get("classifier_version"))?,
        profile_id: sql(row.get("profile_id"))?,
        graph_revision: u64_opt(row, "graph_revision")?.map(GraphRevision::new),
        policy_hash: sql(row.get("policy_hash"))?,
        token_budget: u32_opt(row, "token_budget")?,
        status: sql(row.get("status"))?,
        degradation_json: sql(row.get("degradation_json"))?,
        embed_model_version: sql(row.get("embed_model_version"))?,
        rerank_model_version: sql(row.get("rerank_model_version"))?,
        created_at: ts_opt(row, "created_at")?,
    })
}

/// Reconstruct a [`RetrievalTraceItem`] from a `retrieval_trace_items` row.
pub fn retrieval_trace_item(row: &Row<'_>) -> MemoryResult<RetrievalTraceItem> {
    Ok(RetrievalTraceItem {
        trace_id: id_req(row, "trace_id", RetrievalTraceId::new)?,
        record_id: sql(row.get("record_id"))?,
        strategy: sql(row.get("strategy"))?,
        strategy_rank: u32_opt(row, "strategy_rank")?,
        strategy_score: sql(row.get("strategy_score"))?,
        weight: sql(row.get("weight"))?,
        rrf_contribution: sql(row.get("rrf_contribution"))?,
        gate_disposition: sql(row.get("gate_disposition"))?,
        reason_code: sql(row.get("reason_code"))?,
        token_cost: u32_opt(row, "token_cost")?,
        allocated_tokens: u32_opt(row, "allocated_tokens")?,
        injected_order: u32_opt(row, "injected_order")?,
        goal_id: id_opt(row, "goal_id", GoalId::new)?,
    })
}

// ── feedback ─────────────────────────────────────────────────────────────

/// Reconstruct a [`Feedback`] from a `feedback` row.
pub fn feedback(row: &Row<'_>) -> MemoryResult<Feedback> {
    Ok(Feedback {
        id: id_req(row, "id", FeedbackId::new)?,
        target_kind: sql(row.get("target_kind"))?,
        target_id: sql(row.get("target_id"))?,
        signal: sql(row.get("signal"))?,
        payload_json: sql(row.get("payload_json"))?,
        policy: policy(row)?,
        source_id: id_req(row, "source_id", SourceId::new)?,
        policy_version: sql(row.get("policy_version"))?,
        actor_id: sql(row.get("actor_id"))?,
        event_id: id_opt(row, "event_id", EventId::new)?,
        created_at: ts_opt(row, "created_at")?,
        revision: u64_opt(row, "revision")?,
    })
}

// ── relation registry (design §4.2, task F2.2.1) ───────────────────────────

/// Parse a `json_valid`-guarded JSON array-of-strings column.
fn json_str_array(row: &Row<'_>, col: &str) -> MemoryResult<Vec<String>> {
    let raw: String = sql(row.get(col))?;
    serde_json::from_str::<Vec<String>>(&raw)
        .map_err(|e| encoding_err(format!("{col} is not a JSON string array: {e}")))
}

/// Reconstruct a [`RelationDefinition`] from a `relation_registry` row
/// (`SELECT *`). Every closed-enum column is validated, so a row carrying an
/// out-of-set direction class / endpoint kind / validity policy fails as a typed
/// [`StorageError::Encoding`] rather than yielding a corrupt definition.
pub fn relation_definition(row: &Row<'_>) -> MemoryResult<RelationDefinition> {
    let version: i64 = sql(row.get("version"))?;
    let version = u32::try_from(version)
        .map_err(|_| encoding_err(format!("relation version {version} out of u32 range")))?;

    let direction_class: DirectionClass = {
        let s: String = sql(row.get("direction_class"))?;
        s.parse()?
    };
    let validity_policy: ValidityPolicy = {
        let s: String = sql(row.get("validity_policy"))?;
        s.parse()?
    };

    let parse_kinds = |col: &str| -> MemoryResult<Vec<EndpointKind>> {
        json_str_array(row, col)?
            .into_iter()
            .map(|k| k.parse::<EndpointKind>())
            .collect()
    };

    let evidence_policy: EvidencePolicy = {
        let raw: String = sql(row.get("evidence_policy_json"))?;
        serde_json::from_str(&raw)
            .map_err(|e| encoding_err(format!("evidence_policy_json is malformed: {e}")))?
    };

    let inverse_name = match sql(row.get::<_, Option<String>>("inverse_name"))? {
        Some(s) => Some(RelationName::new(s)?),
        None => None,
    };

    RelationDefinition {
        relation_name: RelationName::new(sql(row.get::<_, String>("relation_name"))?)?,
        version: Version::new(version),
        display_forward: sql(row.get("display_forward"))?,
        display_inverse: sql(row.get("display_inverse"))?,
        aliases: json_str_array(row, "aliases_json")?,
        direction_class,
        inverse_name,
        reflexive: bool_col(row, "reflexive")?,
        source_kinds: parse_kinds("source_kinds_json")?,
        target_kinds: parse_kinds("target_kinds_json")?,
        validity_policy,
        evidence_policy,
        policy_rule_version: sql(row.get("policy_rule_version"))?,
        writable: bool_col(row, "writable")?,
    }
    .validate()
}

// ── malformed-row isolation (MGR-034) ──────────────────────────────────────

/// Run `sql_query` and project **each** row independently through `from_row`,
/// returning one `Result` per row in row order.
///
/// A row that fails to project (bad UUID, non-UTC timestamp, unrecognized
/// closed-set value, …) yields an `Err` for *that row only*; it does not abort
/// the batch, drop the surrounding valid rows, or panic. The outer `Result`
/// covers only failures to prepare/execute the query itself (a schema-level
/// fault), never a single bad row. This is the read-side isolation guarantee of
/// MGR-034 that the F3 read projection builds on.
pub fn read_isolated<T>(
    conn: &Connection,
    sql_query: &str,
    from_row: impl Fn(&Row<'_>) -> MemoryResult<T>,
) -> MemoryResult<Vec<MemoryResult<T>>> {
    let mut stmt = conn.prepare(sql_query).map_err(StorageError::Sqlite)?;
    let mut rows = stmt.query([]).map_err(StorageError::Sqlite)?;
    let mut out: Vec<MemoryResult<T>> = Vec::new();
    loop {
        // Advancing the cursor is a query-level operation: a failure here is a
        // schema/IO fault, not a single malformed row, so it aborts the batch.
        let next = rows.next().map_err(StorageError::Sqlite)?;
        match next {
            Some(row) => out.push(from_row(row)),
            None => break,
        }
    }
    Ok(out)
}

/// Whether a projection error is a per-row canonical-encoding fault (the typed
/// error a malformed row produces), as opposed to any other failure. Used by
/// callers/tests that must confirm a bad row was *isolated as a typed error*.
pub fn is_encoding_error(err: &MemoryError) -> bool {
    matches!(err, MemoryError::Storage(StorageError::Encoding(_)))
}
