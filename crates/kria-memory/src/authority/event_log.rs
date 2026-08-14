//! The transaction-scoped immutable event log (task **F1.3.3**, design §4.1
//! `events_v2`, §5.1 "AuthorityTx … appends start/completion events for
//! invocations").
//!
//! [`TxEventLog`] is the **transaction-scoped repository** that appends rows to
//! the append-only `events_v2` log *using only the serialized-writer
//! transaction connection* handed to it — it never opens or touches a separate
//! connection or the read pool. That is the F1.3 non-negotiable ("the
//! serialized writer owns the transaction"; "all writes must occur on the
//! transaction connection"). Because the type carries no [`Database`] handle at
//! all, mis-wiring a write onto a second connection is unrepresentable.
//!
//! ## Scope of this module (F1.3.3 only)
//!
//! This implements the **invocation start event** append (`phase = 'start'`):
//! HLC allocation from `authority_meta.event_hlc`, canonical time / source /
//! policy / payload columns, payload checksum, and advancing the authority's
//! last-HLC watermark — all inside the caller's transaction so it commits (or
//! rolls back) atomically with the rest of the command.
//!
//! The private [`TxEventLog::append`] helper is phase-generic on purpose: the
//! **completion / command** event and the **observation** event (F1.3.4) slot in
//! as thin wrappers over it without re-deriving HLC/column logic. This module
//! deliberately does **not** write audit records, revisions, changes, outbox,
//! or idempotency results — those are F1.3.4–F1.3.6.
//!
//! [`Database`]: crate::db::Database

use rusqlite::params;
use serde_json::{json, Value};

use crate::db::AuthorityTx;
use crate::error::{MemoryResult, StorageError};
use crate::ids::{blake3_hex, Hlc, HlcGenerator};
use crate::model::{EventId, UtcTimestamp};

use super::command::CommandEnvelope;

/// The event schema version stamped on rows this build appends
/// (`events_v2.schema_version`). Bump when the appended column/payload shape
/// changes so a reader can branch on it.
pub const EVENT_SCHEMA_VERSION: i64 = 1;

/// The `payload_encoding` tag for a plaintext, UTF-8, canonical-JSON payload
/// (`events_v2.payload_plain`). Encrypted payloads (`payload_cipher`) use a
/// different tag introduced with crypto-shred wiring (later gate).
pub const PAYLOAD_ENCODING_PLAIN_JSON: &str = "json/utf8";

/// Placeholder `events_v2.policy_version` until the Effective-Policy layer
/// (F1.4) computes and stamps the real policy version. The column is NOT NULL,
/// so a stable, honest sentinel is written meanwhile — never faked as a real
/// resolved policy version.
pub const PENDING_POLICY_VERSION: &str = "pending-f1.4";

// ─────────────────────────────────────────────────────────────────────────
// EventPhase — the lifecycle phase column (events_v2.phase CHECK)
// ─────────────────────────────────────────────────────────────────────────

/// The lifecycle phase of an appended event, mirroring the schema
/// `events_v2.phase CHECK (phase IN ('start','completion','observation'))`
/// (design §4.1). A closed set so `phase` can never be a raw unchecked string.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EventPhase {
    /// The beginning of an invocation (`phase = 'start'`), appended by F1.3.3.
    Start,
    /// The terminal command/completion event (`phase = 'completion'`, F1.3.4).
    Completion,
    /// A standalone observation (`phase = 'observation'`, F1.3.4).
    Observation,
}

impl EventPhase {
    /// The canonical text stored in `events_v2.phase`.
    pub fn as_str(self) -> &'static str {
        match self {
            EventPhase::Start => "start",
            EventPhase::Completion => "completion",
            EventPhase::Observation => "observation",
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────
// AppendedEvent — the outcome of an event append
// ─────────────────────────────────────────────────────────────────────────

/// The identity and ordering key assigned to a freshly appended event. Returned
/// so the transaction stage can correlate later phases / audit rows to it and
/// echo the completion event id in the command outcome (F1.3.4+).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppendedEvent {
    /// The new event's canonical id (`events_v2.id`).
    pub event_id: EventId,
    /// The HLC allocated to the event (`events_v2.hlc`); strictly greater than
    /// every previously appended event's HLC.
    pub hlc: Hlc,
    /// The wall-clock instant stamped as `events_v2.ts_utc`.
    pub ts_utc: UtcTimestamp,
}

// ─────────────────────────────────────────────────────────────────────────
// TxEventLog — the transaction-scoped event-log repository
// ─────────────────────────────────────────────────────────────────────────

/// The transaction-scoped append surface over `events_v2`.
///
/// A zero-sized handle: every method takes the `&mut AuthorityTx` it must write
/// through, so it is **structurally impossible** for this repository to write to
/// anything other than the serialized-writer transaction (F1.3 invariant). It
/// owns no [`Database`](crate::db::Database) / connection / pool.
#[derive(Debug, Default, Clone, Copy)]
pub struct TxEventLog;

impl TxEventLog {
    /// Construct the (stateless) event-log repository.
    pub fn new() -> Self {
        TxEventLog
    }

    /// Append the immutable **invocation start** event for `env` (`phase =
    /// 'start'`). Allocates the next HLC, writes canonical time / source /
    /// policy / payload columns, checksums the payload, and advances
    /// `authority_meta.event_hlc` — all on `tx`'s connection, so it is part of
    /// the same atomic commit as the rest of the command.
    ///
    /// The start payload is a small provenance marker (command kind, invocation,
    /// source, base revision, idempotency key) — **not** the semantic command
    /// body, which travels on the completion event (F1.3.4).
    pub fn append_start(
        &self,
        tx: &mut AuthorityTx<'_>,
        env: &CommandEnvelope,
    ) -> MemoryResult<AppendedEvent> {
        let payload = start_payload(env);
        self.append(tx, env, EventPhase::Start, None, &payload)
    }

    /// Append the immutable **completion / command** event for `env` (`phase =
    /// 'completion'`, F1.3.4) with a typed `outcome` (the command disposition —
    /// `accepted` / `rejected` / `deferred`). Unlike the start marker, the
    /// completion event carries the *semantic command body* (see
    /// [`command_payload`]) so the log records what was actually decided.
    ///
    /// Thin wrapper over the phase-generic [`append`](Self::append): HLC / time
    /// / source / policy / checksum columns are derived identically to the start
    /// event, so the two phases stay in lock-step.
    pub fn append_completion(
        &self,
        tx: &mut AuthorityTx<'_>,
        env: &CommandEnvelope,
        outcome: &str,
    ) -> MemoryResult<AppendedEvent> {
        let payload = command_payload(env);
        self.append(tx, env, EventPhase::Completion, Some(outcome), &payload)
    }

    /// Append the immutable **observation** event for `env` (`phase =
    /// 'observation'`, F1.3.4). This is the single terminal event for a passive
    /// ingestion / turn source (design §5.1: ingestion/turn sources record one
    /// observation event with no separate start marker), carrying the same
    /// semantic command body as a completion event. `outcome` is the command
    /// disposition, or `None` when the schema records the observation without a
    /// typed outcome.
    pub fn append_observation(
        &self,
        tx: &mut AuthorityTx<'_>,
        env: &CommandEnvelope,
        outcome: Option<&str>,
    ) -> MemoryResult<AppendedEvent> {
        let payload = command_payload(env);
        self.append(tx, env, EventPhase::Observation, outcome, &payload)
    }

    /// Phase-generic append shared by every phase. Allocates the HLC, inserts
    /// the row, and advances the authority's HLC watermark. `outcome` is stored
    /// verbatim in `events_v2.outcome` (null for phases without one).
    fn append(
        &self,
        tx: &mut AuthorityTx<'_>,
        env: &CommandEnvelope,
        phase: EventPhase,
        outcome: Option<&str>,
        payload: &Value,
    ) -> MemoryResult<AppendedEvent> {
        let event_id = EventId::new_v7();
        let hlc = allocate_hlc(tx)?;
        let ts_utc = UtcTimestamp::now();
        let tz_offset_min = local_offset_minutes();

        // Canonical UTF-8 JSON payload + BLAKE3 checksum (design §14 hashing).
        let payload_bytes =
            serde_json::to_vec(payload).map_err(|e| StorageError::Serde(e.to_string()))?;
        let payload_plain = String::from_utf8(payload_bytes.clone())
            .map_err(|e| StorageError::Serde(e.to_string()))?;
        let payload_checksum = blake3_hex(&payload_bytes);

        let caller = env.caller();
        let partition = caller.partition();
        // owner_id is NOT NULL; fall back to the authenticated actor when the
        // partition carries no explicit owner.
        let owner_id = partition.owner_id().unwrap_or_else(|| caller.actor_id());

        tx.conn()
            .execute(
                "INSERT INTO events_v2(
                     id, source_event_id, idempotency_key, invocation_id,
                     phase, outcome,
                     hlc, ts_utc, tz_offset_min,
                     event_type, source_kind, source_id, actor_id, session_id, parent_event_id,
                     namespace, owner_id, scope, sensitivity, policy_version,
                     payload_cipher, payload_plain, payload_encoding, payload_checksum,
                     shred_key_id, key_version,
                     schema_version)
                 VALUES (
                     ?1, NULL, ?2, ?3,
                     ?4, ?5,
                     ?6, ?7, ?8,
                     ?9, ?10, ?11, ?12, NULL, NULL,
                     ?13, ?14, ?15, ?16, ?17,
                     NULL, ?18, ?19, ?20,
                     NULL, NULL,
                     ?21)",
                params![
                    event_id.as_str(),
                    env.idempotency_key().as_str(),
                    env.source().invocation_id().as_str(),
                    phase.as_str(),
                    outcome,
                    hlc.encode(),
                    ts_utc.to_rfc3339(),
                    tz_offset_min,
                    env.kind().as_str(),
                    env.source().source_kind().as_str(),
                    env.source().source_id(),
                    caller.actor_id(),
                    partition.namespace(),
                    owner_id,
                    partition.scope(),
                    partition.sensitivity() as i64,
                    PENDING_POLICY_VERSION,
                    payload_plain,
                    PAYLOAD_ENCODING_PLAIN_JSON,
                    payload_checksum,
                    EVENT_SCHEMA_VERSION,
                ],
            )
            .map_err(StorageError::Sqlite)?;

        Ok(AppendedEvent {
            event_id,
            hlc,
            ts_utc,
        })
    }
}

/// Allocate the next strictly-increasing HLC for an event and persist it as the
/// authority's last-HLC watermark (`authority_meta.event_hlc`) — all on `tx`.
///
/// Reads the current watermark, ticks it forward with the monotonic
/// [`HlcGenerator`] (drift/DST/backward-jump safe), and writes the new value
/// back. Because this runs inside the serialized writer transaction, no two
/// events can ever share an HLC (also guarded by the `events_v2.hlc UNIQUE`
/// constraint), and a rollback restores the prior watermark.
fn allocate_hlc(tx: &mut AuthorityTx<'_>) -> MemoryResult<Hlc> {
    let last_enc: String = tx
        .conn()
        .query_row(
            "SELECT event_hlc FROM authority_meta WHERE id = 1",
            [],
            |r| r.get(0),
        )
        .map_err(StorageError::Sqlite)?;

    let last = if last_enc.is_empty() {
        Hlc::ZERO
    } else {
        Hlc::decode(&last_enc).ok_or_else(|| {
            StorageError::Corruption(format!(
                "authority_meta.event_hlc is malformed: {last_enc:?}"
            ))
        })?
    };

    let next = HlcGenerator::from_last(last).now();

    tx.conn()
        .execute(
            "UPDATE authority_meta SET event_hlc = ?1 WHERE id = 1",
            params![next.encode()],
        )
        .map_err(StorageError::Sqlite)?;

    Ok(next)
}

/// The originating source's UTC offset in minutes (design §14: store UTC plus
/// the source offset). The authority itself is the source of a start event, so
/// this is the local machine offset at append time.
fn local_offset_minutes() -> i64 {
    (chrono::Local::now().offset().local_minus_utc() / 60) as i64
}

/// The provenance marker payload for an invocation start event. Deliberately
/// small and metadata-only — the semantic command body is carried by the
/// completion event (F1.3.4), never duplicated here.
fn start_payload(env: &CommandEnvelope) -> Value {
    json!({
        "marker": "invocation_start",
        "command_kind": env.kind().as_str(),
        "invocation_id": env.source().invocation_id().as_str(),
        "source_kind": env.source().source_kind().as_str(),
        "source_id": env.source().source_id(),
        "source_trust": env.source().trust().as_str(),
        "base_revision": env.base_revision().get(),
        "idempotency_key": env.idempotency_key().as_str(),
    })
}

/// The payload for a completion / observation event: the *semantic command
/// body* wrapped in a small provenance envelope. Unlike [`start_payload`] this
/// carries `env.payload()` verbatim so the immutable log records exactly what
/// was decided, checksummed by the phase-generic append.
fn command_payload(env: &CommandEnvelope) -> Value {
    json!({
        "marker": "command_completion",
        "command_kind": env.kind().as_str(),
        "invocation_id": env.source().invocation_id().as_str(),
        "command_hash": env.command_hash().as_str(),
        "command": env.payload(),
    })
}
