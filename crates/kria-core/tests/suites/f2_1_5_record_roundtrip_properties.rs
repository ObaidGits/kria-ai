//! F2.1.5 — SQL↔Rust↔API round-trip property tests for every v2 cognitive
//! record type, plus malformed-row isolation (design §4.2/§4.3; MGR-001,
//! MGR-002, MGR-034; MGD-002–005).
//!
//! For every record kind and cognitive-record type this proves the three
//! representations agree:
//!
//!   * **SQL↔Rust** — a value built in Rust → INSERTed into the 0017 columns →
//!     SELECTed back → projected via [`row_mapping`] equals the original.
//!   * **Rust↔API** — the same value → API JSON → back equals the original.
//!
//! The generators push **extreme inputs**: extreme Unicode (multi-byte, CJK,
//! emoji + ZWJ sequences, combining marks, RTL overrides, zero-width joiners),
//! empty optional fields (`None` everywhere optional), and time boundaries
//! (min/max representable RFC3339 UTC instants; open, half-open, and empty
//! `from == until` valid intervals).
//!
//! Finally, [`malformed_row_is_isolated_*`] proves MGR-034: a malformed row in a
//! table (bad UUID, non-UTC timestamp) is isolated as a *typed error for that
//! row only*, while every valid row in the same batch still reads back — no
//! panic, no silent data loss.
//!
//! These tests INSERT directly via SQL (the write path F1.5 is not cut over
//! yet), which is the correct way to exercise the SQL↔Rust column mapping at
//! this stage. Foreign-key enforcement is disabled on the test connection so a
//! single-table round trip needs no unrelated parent rows — referential
//! integrity is the write-path's concern (F1.5), not this mapping test's.

use chrono::{Duration, TimeZone, Utc};
use proptest::prelude::*;
use rusqlite::{params, Connection};

use kria_core::memory::authority::command::SourceKind;
use kria_core::memory::db::Database;
use kria_core::memory::error::MemoryResult;
use kria_core::memory::model::entity::Span;
use kria_core::memory::model::record::RecordPayload;
use kria_core::memory::model::row_mapping;
use kria_core::memory::model::{
    Alias, AliasId, ConsolidationLevel, ConsolidationRun, ConsolidationRunId, Entity, EntityId,
    Episode, EpisodeId, EventId, Evidence, EvidenceId, EvidencePolarity, Feedback, FeedbackId,
    Goal, GoalId, GoalProgress, GoalProgressId, GoalStatus, GraphRevision, Mention, MentionId,
    PolicyPartition, Record, RecordId, RecordKind, RetrievalTrace, RetrievalTraceId,
    RetrievalTraceItem, SchemaVersion, SourceId, SourceRecord, ToolObservation, ToolObservationId,
    TruthState, UtcTimestamp, ValidInterval,
};
use kria_core::memory::types::StalenessClass;

const CASES: u32 = 128;

/// A fresh in-memory authority with foreign-key enforcement disabled (see
/// module docs). Each proptest case gets its own DB so PK/unique indexes never
/// collide across cases.
fn fresh_db() -> Database {
    let db = Database::open_in_memory().expect("open in-memory authority");
    db.write()
        .execute_batch("PRAGMA foreign_keys=OFF;")
        .expect("disable FK enforcement for the mapping test");
    db
}

// ── shared generators ───────────────────────────────────────────────────────

/// An `Option<T>` that produces `None` about half the time (so the "empty
/// optional fields" boundary is exercised heavily).
fn opt<T: std::fmt::Debug>(s: impl Strategy<Value = T>) -> impl Strategy<Value = Option<T>> {
    proptest::option::of(s)
}

/// Extreme-Unicode free text: random Unicode scalar sequences (which include
/// CJK, emoji, combining marks, RTL, and format chars) mixed with curated
/// pathological samples. Embedded NUL is stripped (SQLite TEXT is NUL-sensitive
/// at the C-API boundary; NUL is not part of the "extreme Unicode" surface this
/// task targets).
fn arb_text() -> impl Strategy<Value = String> {
    prop_oneof![
        6 => proptest::collection::vec(any::<char>(), 0..24)
            .prop_map(|cs| cs.into_iter().collect::<String>().replace('\0', "")),
        1 => Just(String::new()),
        1 => Just("👩‍👩‍👧‍👦".to_string()),                    // ZWJ family emoji
        1 => Just("日本語テキスト漢字".to_string()),             // CJK
        1 => Just("e\u{0301}a\u{0300}o\u{0308}".to_string()),   // combining diacritics
        1 => Just("\u{202E}dlrow olleh\u{202C}".to_string()),   // RTL override
        1 => Just("a\u{200B}b\u{200D}c\u{FEFF}".to_string()),   // zero-width + BOM
        1 => Just("🇯🇵🏳️‍🌈🧑🏽‍💻".to_string()),                   // flags + skin-tone ZWJ
        1 => Just("  padded whitespace  \t ".to_string()),
    ]
}

fn arb_text_opt() -> impl Strategy<Value = Option<String>> {
    opt(arb_text())
}

/// A control-char-free structural reference (namespace/scope/owner suffix, JSON
/// value). Includes some non-ASCII so policy columns also carry Unicode.
fn arb_ref() -> impl Strategy<Value = String> {
    prop_oneof![
        4 => "[A-Za-z0-9_:/.\\-]{1,24}".prop_map(|s| s),
        1 => Just("café".to_string()),
        1 => Just("проект".to_string()),
        1 => Just("プロジェクト".to_string()),
    ]
}

/// A finite `f64` score. SQLite `REAL` is an 8-byte IEEE-754 double so it
/// stores *any* finite `f64` bit-exactly; serde_json's default float parser,
/// however, is not guaranteed bit-exact for arbitrary doubles (that needs the
/// `float_roundtrip` feature). Since a semantic score only needs to round-trip
/// for values it can actually take, this generator draws exactly-representable
/// doubles (integers and dyadic fractions) that round-trip losslessly through
/// *both* SQLite and JSON — covering zero, ±, fractional, and large magnitudes.
fn arb_score() -> impl Strategy<Value = f64> {
    prop_oneof![
        Just(0.0f64),
        Just(1.0f64),
        Just(-1.0f64),
        Just(0.5f64),
        Just(0.25f64),
        Just(-0.75f64),
        // Exact integer-valued doubles across a wide magnitude range.
        (-1_000_000_000i64..1_000_000_000i64).prop_map(|n| n as f64),
        // Dyadic fraction (denominator a power of two) → exact in binary.
        (-1_000_000i64..1_000_000i64).prop_map(|n| n as f64 / 64.0),
    ]
}

fn arb_score_opt() -> impl Strategy<Value = Option<f64>> {
    opt(arb_score())
}

/// A UTC instant covering the representable RFC3339 range (year 1..=9999),
/// including the exact min/max boundaries and sub-second precision.
fn arb_ts() -> impl Strategy<Value = UtcTimestamp> {
    prop_oneof![
        8 => (1i32..=9999, 1u32..=12, 1u32..=28, 0u32..=23, 0u32..=59, 0u32..=59, 0i64..=999)
            .prop_map(|(y, mo, d, h, mi, s, ms)| {
                let dt = Utc.with_ymd_and_hms(y, mo, d, h, mi, s).unwrap()
                    + Duration::milliseconds(ms);
                UtcTimestamp::from_datetime(dt)
            }),
        1 => Just(UtcTimestamp::from_rfc3339_utc("0001-01-01T00:00:00Z").unwrap()),
        1 => Just(UtcTimestamp::from_rfc3339_utc("9999-12-31T23:59:59.999Z").unwrap()),
    ]
}

fn arb_ts_opt() -> impl Strategy<Value = Option<UtcTimestamp>> {
    opt(arb_ts())
}

/// Two optional timestamps with `a <= b` when both present (satisfies the
/// `closed_at >= opened_at` schema CHECK on episodes).
fn arb_ordered_ts_opt_pair() -> impl Strategy<Value = (Option<UtcTimestamp>, Option<UtcTimestamp>)>
{
    (arb_ts_opt(), arb_ts_opt()).prop_map(|(a, b)| match (a, b) {
        (Some(x), Some(y)) if x > y => (Some(y), Some(x)),
        other => other,
    })
}

/// A half-open valid interval: open, half-open each way, the empty
/// `from == until` boundary, and a general non-inverted interval.
fn arb_interval() -> impl Strategy<Value = ValidInterval> {
    prop_oneof![
        Just(ValidInterval::open()),
        arb_ts().prop_map(|f| ValidInterval::new(Some(f), None).unwrap()),
        arb_ts().prop_map(|u| ValidInterval::new(None, Some(u)).unwrap()),
        arb_ts().prop_map(|t| ValidInterval::new(Some(t), Some(t)).unwrap()),
        (arb_ts(), arb_ts()).prop_map(|(a, b)| {
            let (lo, hi) = if a <= b { (a, b) } else { (b, a) };
            ValidInterval::new(Some(lo), Some(hi)).unwrap()
        }),
    ]
}

/// A validated policy partition. `namespace`/`scope`/`owner` are prefixed with a
/// non-whitespace marker so they stay valid while still carrying extreme
/// Unicode; owner is absent about half the time (empty-optional boundary).
fn arb_policy() -> impl Strategy<Value = PolicyPartition> {
    (arb_ref(), arb_ref(), 0u8..=3, opt(arb_ref())).prop_map(|(ns, sc, sens, owner)| {
        PolicyPartition::with_owner(
            format!("n\u{00B7}{ns}"),
            format!("s\u{00B7}{sc}"),
            sens,
            owner.map(|o| format!("o\u{00B7}{o}")),
        )
        .unwrap()
    })
}

fn arb_truth_opt() -> impl Strategy<Value = Option<TruthState>> {
    opt(prop_oneof![
        Just(TruthState::Current),
        Just(TruthState::Unverified),
        Just(TruthState::Stale),
        Just(TruthState::Contradicted),
        Just(TruthState::Superseded),
        Just(TruthState::Inferred),
        Just(TruthState::Confirmed),
        Just(TruthState::Forgotten),
        Just(TruthState::Deleted),
        Just(TruthState::Unavailable),
        // Forward-compatible unknown value (design §40) must also round-trip.
        arb_ref().prop_map(TruthState::Other),
    ])
}

fn arb_staleness_opt() -> impl Strategy<Value = Option<StalenessClass>> {
    opt(prop_oneof![
        Just(StalenessClass::Immutable),
        Just(StalenessClass::Permanent),
        Just(StalenessClass::Slow),
        Just(StalenessClass::VolatileVerifiable),
        Just(StalenessClass::VolatileUnverifiable),
        arb_ref().prop_map(StalenessClass::Other),
    ])
}

/// A valid JSON string for a `json_valid`-guarded column, preserved verbatim.
fn arb_json_opt() -> impl Strategy<Value = Option<String>> {
    opt(arb_text().prop_map(|t| serde_json::to_string(&serde_json::json!({ "note": t })).unwrap()))
}

fn arb_u32_opt() -> impl Strategy<Value = Option<u32>> {
    opt(any::<u32>())
}

fn arb_u64_opt() -> impl Strategy<Value = Option<u64>> {
    opt(0u64..=(i64::MAX as u64))
}

/// Both-or-neither span (the schema stores `span_start`/`span_end` as a pair;
/// the model's `Span` requires `end >= start`).
fn arb_span_opt() -> impl Strategy<Value = Option<Span>> {
    opt((any::<u32>(), any::<u32>()).prop_map(|(a, b)| {
        let (lo, hi) = if a <= b { (a, b) } else { (b, a) };
        Span::new(lo, hi).unwrap()
    }))
}

// ── generic round-trip drivers ──────────────────────────────────────────────

/// Insert `original`, read the single row back through `from_row`, and return
/// the reconstructed value (SQL↔Rust half).
fn sql_roundtrip<T>(
    insert: impl FnOnce(&Connection, &T) -> rusqlite::Result<usize>,
    select: &str,
    from_row: impl Fn(&rusqlite::Row<'_>) -> MemoryResult<T>,
    original: &T,
) -> T {
    let db = fresh_db();
    let conn = db.write();
    insert(&conn, original).expect("insert row");
    let mut rows = row_mapping::read_isolated(&conn, select, from_row).expect("read batch");
    assert_eq!(rows.len(), 1, "expected exactly one row back");
    rows.remove(0).expect("row must project to a typed value")
}

/// Serialize to API JSON and back, asserting equality (Rust↔API half).
fn api_roundtrip<T>(original: &T)
where
    T: serde::Serialize + serde::de::DeserializeOwned + PartialEq + std::fmt::Debug,
{
    let json = serde_json::to_string(original).expect("serialize to API JSON");
    let back: T = serde_json::from_str(&json).expect("deserialize from API JSON");
    assert_eq!(original, &back, "API JSON round trip mismatch");
}

// ── records ─────────────────────────────────────────────────────────────────

fn arb_record_kind() -> impl Strategy<Value = RecordKind> {
    prop_oneof![
        Just(RecordKind::Memory),
        Just(RecordKind::Summary),
        Just(RecordKind::Skill),
        Just(RecordKind::Rule),
    ]
}

fn arb_payload() -> impl Strategy<Value = RecordPayload> {
    prop_oneof![
        arb_text().prop_map(RecordPayload::Plaintext),
        proptest::collection::vec(any::<u8>(), 0..32).prop_map(RecordPayload::Ciphertext),
    ]
}

fn arb_record() -> impl Strategy<Value = Record> {
    let core = (
        arb_record_kind(),
        arb_payload(),
        any::<u32>(),
        arb_text_opt(),
        arb_truth_opt(),
        arb_staleness_opt(),
        arb_interval(),
        arb_policy(),
        arb_text(),
    );
    let extra = (
        arb_ts(),
        any::<bool>(),
        any::<bool>(),
        any::<bool>(),
        arb_u32_opt(),
        arb_text_opt(),
        arb_u32_opt(),
    );
    (core, extra).prop_map(
        |(
            (kind, payload, sv, content_hash, truth, stale, valid_interval, policy, policy_version),
            (created_at, has_sup, has_ep, has_goal, estimated_tokens, shred_key_id, key_version),
        )| Record {
            id: RecordId::new_v7(),
            record_kind: kind,
            schema_version: SchemaVersion::new(sv),
            payload,
            content_hash,
            truth_state: truth,
            staleness_class: stale,
            valid_interval,
            policy,
            source_id: SourceId::new_v7(),
            policy_version,
            created_event_id: EventId::new_v7(),
            created_at,
            superseded_by: has_sup.then(RecordId::new_v7),
            episode_id: has_ep.then(EpisodeId::new_v7),
            goal_context_id: has_goal.then(GoalId::new_v7),
            estimated_tokens,
            shred_key_id,
            key_version,
        },
    )
}

fn insert_record(conn: &Connection, r: &Record) -> rusqlite::Result<usize> {
    let (content, cipher): (Option<&str>, Option<&[u8]>) = match &r.payload {
        RecordPayload::Plaintext(s) => (Some(s.as_str()), None),
        RecordPayload::Ciphertext(b) => (None, Some(b.as_slice())),
    };
    let truth = r.truth_state.as_ref().map(|t| t.as_str());
    let stale = r.staleness_class.as_ref().map(|s| s.as_str());
    let vf = r.valid_interval.valid_from().map(|t| t.to_rfc3339());
    let vu = r.valid_interval.valid_until().map(|t| t.to_rfc3339());
    let created = r.created_at.to_rfc3339();
    let owner = r.policy.owner_id().unwrap_or("");
    conn.execute(
        "INSERT INTO records(
            id, record_kind, schema_version, content, content_cipher, content_hash,
            truth_state, staleness_class, valid_from, valid_until,
            namespace, owner_id, scope, sensitivity, source_id, policy_version,
            created_event_id, created_at, superseded_by, episode_id, goal_context_id,
            estimated_tokens, shred_key_id, key_version)
         VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17,?18,?19,?20,?21,?22,?23,?24)",
        params![
            r.id.as_str(),
            r.record_kind.as_str(),
            i64::from(r.schema_version.get()),
            content,
            cipher,
            r.content_hash,
            truth,
            stale,
            vf,
            vu,
            r.policy.namespace(),
            owner,
            r.policy.scope(),
            i64::from(r.policy.sensitivity()),
            r.source_id.as_str(),
            r.policy_version,
            r.created_event_id.as_str(),
            created,
            r.superseded_by.as_ref().map(|x| x.as_str()),
            r.episode_id.as_ref().map(|x| x.as_str()),
            r.goal_context_id.as_ref().map(|x| x.as_str()),
            r.estimated_tokens.map(i64::from),
            r.shred_key_id,
            r.key_version.map(i64::from),
        ],
    )
}

// ── entities ────────────────────────────────────────────────────────────────

fn arb_entity() -> impl Strategy<Value = Entity> {
    (
        any::<bool>(),
        arb_text_opt(),
        arb_text_opt(),
        arb_text_opt(),
        arb_truth_opt(),
        arb_policy(),
        arb_text(),
        arb_ts(),
        arb_u64_opt(),
    )
        .prop_map(
            |(
                has_canonical,
                entity_type,
                display_name,
                normalized_name,
                truth_state,
                policy,
                policy_version,
                created_at,
                revision,
            )| Entity {
                id: EntityId::new_v7(),
                canonical_id: has_canonical.then(EntityId::new_v7),
                entity_type,
                display_name,
                normalized_name,
                truth_state,
                policy,
                source_id: SourceId::new_v7(),
                policy_version,
                created_event_id: EventId::new_v7(),
                created_at,
                revision,
            },
        )
}

fn insert_entity(conn: &Connection, e: &Entity) -> rusqlite::Result<usize> {
    let truth = e.truth_state.as_ref().map(|t| t.as_str());
    let created = e.created_at.to_rfc3339();
    let owner = e.policy.owner_id().unwrap_or("");
    conn.execute(
        "INSERT INTO entities_v2(
            id, canonical_id, entity_type, display_name, normalized_name, truth_state,
            namespace, owner_id, scope, sensitivity, source_id, policy_version,
            created_event_id, created_at, revision)
         VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15)",
        params![
            e.id.as_str(),
            e.canonical_id.as_ref().map(|x| x.as_str()),
            e.entity_type,
            e.display_name,
            e.normalized_name,
            truth,
            e.policy.namespace(),
            owner,
            e.policy.scope(),
            i64::from(e.policy.sensitivity()),
            e.source_id.as_str(),
            e.policy_version,
            e.created_event_id.as_str(),
            created,
            e.revision.map(|v| v as i64),
        ],
    )
}

// ── aliases ─────────────────────────────────────────────────────────────────

fn arb_alias() -> impl Strategy<Value = Alias> {
    (
        arb_text(),
        arb_text(),
        arb_ref(),
        arb_truth_opt(),
        arb_policy(),
        arb_text(),
        any::<bool>(),
        arb_ts_opt(),
        arb_interval(),
    )
        .prop_map(
            |(
                alias,
                normalized_alias,
                alias_type,
                truth_state,
                policy,
                policy_version,
                has_event,
                created_at,
                valid_interval,
            )| Alias {
                id: AliasId::new_v7(),
                entity_id: EntityId::new_v7(),
                alias,
                normalized_alias,
                alias_type,
                truth_state,
                policy,
                source_id: SourceId::new_v7(),
                policy_version,
                created_event_id: has_event.then(EventId::new_v7),
                created_at,
                valid_interval,
            },
        )
}

fn insert_alias(conn: &Connection, a: &Alias) -> rusqlite::Result<usize> {
    let truth = a.truth_state.as_ref().map(|t| t.as_str());
    let created = a.created_at.map(|t| t.to_rfc3339());
    let vf = a.valid_interval.valid_from().map(|t| t.to_rfc3339());
    let vu = a.valid_interval.valid_until().map(|t| t.to_rfc3339());
    let owner = a.policy.owner_id().unwrap_or("");
    conn.execute(
        "INSERT INTO aliases(
            id, entity_id, alias, normalized_alias, alias_type, truth_state,
            namespace, owner_id, scope, sensitivity, source_id, policy_version,
            created_event_id, created_at, valid_from, valid_until)
         VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16)",
        params![
            a.id.as_str(),
            a.entity_id.as_str(),
            a.alias,
            a.normalized_alias,
            a.alias_type,
            truth,
            a.policy.namespace(),
            owner,
            a.policy.scope(),
            i64::from(a.policy.sensitivity()),
            a.source_id.as_str(),
            a.policy_version,
            a.created_event_id.as_ref().map(|x| x.as_str()),
            created,
            vf,
            vu,
        ],
    )
}

// ── mentions ────────────────────────────────────────────────────────────────

fn arb_mention() -> impl Strategy<Value = Mention> {
    let a = (
        arb_text_opt(),
        arb_text_opt(),
        arb_json_opt(),
        arb_span_opt(),
        arb_text_opt(),
        arb_text_opt(),
    );
    let b = (
        arb_text_opt(),
        arb_score_opt(),
        arb_text_opt(),
        arb_policy(),
        arb_text(),
        arb_ts_opt(),
        any::<bool>(),
    );
    (a, b).prop_map(
        |(
            (record_id, record_kind, locator_json, span, role, extractor),
            (
                extractor_version,
                score,
                score_semantics,
                policy,
                policy_version,
                observed_at,
                has_ev,
            ),
        )| Mention {
            id: MentionId::new_v7(),
            record_id,
            record_kind,
            entity_id: EntityId::new_v7(),
            locator_json,
            span,
            role,
            extractor,
            extractor_version,
            score,
            score_semantics,
            policy,
            source_id: SourceId::new_v7(),
            policy_version,
            observed_at,
            created_event_id: has_ev.then(EventId::new_v7),
        },
    )
}

fn insert_mention(conn: &Connection, m: &Mention) -> rusqlite::Result<usize> {
    let span_start = m.span.map(|s| i64::from(s.start()));
    let span_end = m.span.map(|s| i64::from(s.end()));
    let observed = m.observed_at.map(|t| t.to_rfc3339());
    let owner = m.policy.owner_id().unwrap_or("");
    conn.execute(
        "INSERT INTO mentions(
            id, record_id, record_kind, entity_id, locator_json, span_start, span_end,
            role, extractor, extractor_version, score, score_semantics,
            namespace, owner_id, scope, sensitivity, source_id, policy_version,
            observed_at, created_event_id)
         VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17,?18,?19,?20)",
        params![
            m.id.as_str(),
            m.record_id,
            m.record_kind,
            m.entity_id.as_str(),
            m.locator_json,
            span_start,
            span_end,
            m.role,
            m.extractor,
            m.extractor_version,
            m.score,
            m.score_semantics,
            m.policy.namespace(),
            owner,
            m.policy.scope(),
            i64::from(m.policy.sensitivity()),
            m.source_id.as_str(),
            m.policy_version,
            observed,
            m.created_event_id.as_ref().map(|x| x.as_str()),
        ],
    )
}

// ── evidence ────────────────────────────────────────────────────────────────

fn arb_polarity() -> impl Strategy<Value = EvidencePolarity> {
    prop_oneof![
        Just(EvidencePolarity::Supports),
        Just(EvidencePolarity::Contradicts),
    ]
}

fn arb_evidence() -> impl Strategy<Value = Evidence> {
    let a = (
        arb_text_opt(),
        arb_text_opt(),
        arb_text_opt(),
        arb_text_opt(),
        any::<bool>(),
        arb_json_opt(),
        arb_text_opt(),
    );
    let b = (
        arb_text_opt(),
        arb_text_opt(),
        arb_polarity(),
        arb_score_opt(),
        arb_text_opt(),
        arb_policy(),
    );
    let c = (arb_text(), arb_ts_opt(), arb_ts_opt(), any::<bool>());
    (a, b, c).prop_map(
        |(
            (
                subject_kind,
                subject_id,
                source_record_kind,
                source_record_id,
                has_src_ev,
                locator_json,
                actor_id,
            ),
            (method, method_version, polarity, score, score_semantics, policy),
            (policy_version, observed_at, removed_at, has_ev),
        )| Evidence {
            id: EvidenceId::new_v7(),
            subject_kind,
            subject_id,
            source_record_kind,
            source_record_id,
            source_event_id: has_src_ev.then(EventId::new_v7),
            locator_json,
            actor_id,
            method,
            method_version,
            polarity,
            score,
            score_semantics,
            policy,
            source_id: SourceId::new_v7(),
            policy_version,
            observed_at,
            removed_at,
            created_event_id: has_ev.then(EventId::new_v7),
        },
    )
}

fn insert_evidence(conn: &Connection, e: &Evidence) -> rusqlite::Result<usize> {
    let observed = e.observed_at.map(|t| t.to_rfc3339());
    let removed = e.removed_at.map(|t| t.to_rfc3339());
    let owner = e.policy.owner_id().unwrap_or("");
    conn.execute(
        "INSERT INTO evidence_v2(
            id, subject_kind, subject_id, source_record_kind, source_record_id, source_event_id,
            locator_json, actor_id, method, method_version, polarity, score, score_semantics,
            namespace, owner_id, scope, sensitivity, source_id, policy_version,
            observed_at, removed_at, created_event_id)
         VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17,?18,?19,?20,?21,?22)",
        params![
            e.id.as_str(),
            e.subject_kind,
            e.subject_id,
            e.source_record_kind,
            e.source_record_id,
            e.source_event_id.as_ref().map(|x| x.as_str()),
            e.locator_json,
            e.actor_id,
            e.method,
            e.method_version,
            e.polarity.as_str(),
            e.score,
            e.score_semantics,
            e.policy.namespace(),
            owner,
            e.policy.scope(),
            i64::from(e.policy.sensitivity()),
            e.source_id.as_str(),
            e.policy_version,
            observed,
            removed,
            e.created_event_id.as_ref().map(|x| x.as_str()),
        ],
    )
}

// ── episodes ────────────────────────────────────────────────────────────────

fn arb_episode() -> impl Strategy<Value = Episode> {
    (
        arb_text_opt(),
        arb_text_opt(),
        arb_policy(),
        arb_text(),
        arb_ordered_ts_opt_pair(),
        arb_text_opt(),
        any::<bool>(),
        arb_truth_opt(),
        arb_u64_opt(),
    )
        .prop_map(
            |(
                session_id,
                task_id,
                policy,
                policy_version,
                (opened_at, closed_at),
                boundary_reason,
                has_cursor,
                truth_state,
                revision,
            )| Episode {
                id: EpisodeId::new_v7(),
                session_id,
                task_id,
                policy,
                source_id: SourceId::new_v7(),
                policy_version,
                opened_at,
                closed_at,
                boundary_reason,
                cursor_event_id: has_cursor.then(EventId::new_v7),
                truth_state,
                revision,
            },
        )
}

fn insert_episode(conn: &Connection, e: &Episode) -> rusqlite::Result<usize> {
    let opened = e.opened_at.map(|t| t.to_rfc3339());
    let closed = e.closed_at.map(|t| t.to_rfc3339());
    let truth = e.truth_state.as_ref().map(|t| t.as_str());
    let owner = e.policy.owner_id().unwrap_or("");
    conn.execute(
        "INSERT INTO episodes_v2(
            id, session_id, task_id, namespace, owner_id, scope, sensitivity,
            source_id, policy_version, opened_at, closed_at, boundary_reason,
            cursor_event_id, truth_state, revision)
         VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15)",
        params![
            e.id.as_str(),
            e.session_id,
            e.task_id,
            e.policy.namespace(),
            owner,
            e.policy.scope(),
            i64::from(e.policy.sensitivity()),
            e.source_id.as_str(),
            e.policy_version,
            opened,
            closed,
            e.boundary_reason,
            e.cursor_event_id.as_ref().map(|x| x.as_str()),
            truth,
            e.revision.map(|v| v as i64),
        ],
    )
}

// ── goals ───────────────────────────────────────────────────────────────────

fn arb_goal_status() -> impl Strategy<Value = GoalStatus> {
    prop_oneof![
        Just(GoalStatus::Candidate),
        Just(GoalStatus::Active),
        Just(GoalStatus::Paused),
        Just(GoalStatus::Completed),
        Just(GoalStatus::Conflicted),
        Just(GoalStatus::Stale),
        Just(GoalStatus::Superseded),
        Just(GoalStatus::Deleted),
    ]
}

fn arb_goal() -> impl Strategy<Value = Goal> {
    let a = (
        arb_text_opt(),
        arb_text_opt(),
        arb_goal_status(),
        opt(0u8..=10u8),
        arb_score_opt(),
        arb_text_opt(),
    );
    let b = (
        arb_text_opt(),
        arb_policy(),
        arb_text(),
        any::<bool>(),
        arb_ts_opt(),
        arb_ts_opt(),
        arb_u64_opt(),
    );
    (a, b).prop_map(
        |(
            (kind, title, status, priority, score, score_semantics),
            (resumption_context, policy, policy_version, has_ev, created_at, updated_at, revision),
        )| Goal {
            id: GoalId::new_v7(),
            kind,
            title,
            status,
            priority,
            score,
            score_semantics,
            resumption_context,
            policy,
            source_id: SourceId::new_v7(),
            policy_version,
            created_event_id: has_ev.then(EventId::new_v7),
            created_at,
            updated_at,
            revision,
        },
    )
}

fn insert_goal(conn: &Connection, g: &Goal) -> rusqlite::Result<usize> {
    let created = g.created_at.map(|t| t.to_rfc3339());
    let updated = g.updated_at.map(|t| t.to_rfc3339());
    let owner = g.policy.owner_id().unwrap_or("");
    conn.execute(
        "INSERT INTO goals_v2(
            id, kind, title, status, priority, score, score_semantics, resumption_context,
            namespace, owner_id, scope, sensitivity, source_id, policy_version,
            created_event_id, created_at, updated_at, revision)
         VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17,?18)",
        params![
            g.id.as_str(),
            g.kind,
            g.title,
            g.status.as_str(),
            g.priority.map(i64::from),
            g.score,
            g.score_semantics,
            g.resumption_context,
            g.policy.namespace(),
            owner,
            g.policy.scope(),
            i64::from(g.policy.sensitivity()),
            g.source_id.as_str(),
            g.policy_version,
            g.created_event_id.as_ref().map(|x| x.as_str()),
            created,
            updated,
            g.revision.map(|v| v as i64),
        ],
    )
}

// ── goal progress ────────────────────────────────────────────────────────────

fn arb_goal_progress() -> impl Strategy<Value = GoalProgress> {
    (
        any::<bool>(),
        arb_text_opt(),
        arb_text_opt(),
        arb_ts_opt(),
        arb_u64_opt(),
    )
        .prop_map(
            |(has_ev, state, summary, observed_at, revision)| GoalProgress {
                id: GoalProgressId::new_v7(),
                goal_id: GoalId::new_v7(),
                event_id: has_ev.then(EventId::new_v7),
                state,
                summary,
                observed_at,
                revision,
            },
        )
}

fn insert_goal_progress(conn: &Connection, p: &GoalProgress) -> rusqlite::Result<usize> {
    let observed = p.observed_at.map(|t| t.to_rfc3339());
    conn.execute(
        "INSERT INTO goal_progress(id, goal_id, event_id, state, summary, observed_at, revision)
         VALUES (?1,?2,?3,?4,?5,?6,?7)",
        params![
            p.id.as_str(),
            p.goal_id.as_str(),
            p.event_id.as_ref().map(|x| x.as_str()),
            p.state,
            p.summary,
            observed,
            p.revision.map(|v| v as i64),
        ],
    )
}

// ── consolidation runs ────────────────────────────────────────────────────────

fn arb_level() -> impl Strategy<Value = ConsolidationLevel> {
    prop_oneof![
        Just(ConsolidationLevel::Episode),
        Just(ConsolidationLevel::Summary),
        Just(ConsolidationLevel::Skill),
        Just(ConsolidationLevel::Rule),
    ]
}

fn arb_consolidation_run() -> impl Strategy<Value = ConsolidationRun> {
    (
        arb_text(),
        arb_text(),
        arb_text(),
        arb_level(),
        arb_text_opt(),
        arb_text_opt(),
        arb_ts_opt(),
        arb_ts_opt(),
        arb_text_opt(),
        arb_text_opt(),
    )
        .prop_map(
            |(
                algorithm,
                version,
                input_set_hash,
                level,
                cursor,
                status,
                started_at,
                completed_at,
                output_id,
                error_code,
            )| ConsolidationRun {
                id: ConsolidationRunId::new_v7(),
                algorithm,
                version,
                input_set_hash,
                level,
                cursor,
                status,
                started_at,
                completed_at,
                output_id,
                error_code,
            },
        )
}

fn insert_consolidation_run(conn: &Connection, r: &ConsolidationRun) -> rusqlite::Result<usize> {
    let started = r.started_at.map(|t| t.to_rfc3339());
    let completed = r.completed_at.map(|t| t.to_rfc3339());
    conn.execute(
        "INSERT INTO consolidation_runs(
            id, algorithm, version, input_set_hash, level, cursor, status,
            started_at, completed_at, output_id, error_code)
         VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11)",
        params![
            r.id.as_str(),
            r.algorithm,
            r.version,
            r.input_set_hash,
            r.level.as_str(),
            r.cursor,
            r.status,
            started,
            completed,
            r.output_id,
            r.error_code,
        ],
    )
}

// ── sources ─────────────────────────────────────────────────────────────────

fn arb_source_kind() -> impl Strategy<Value = SourceKind> {
    prop_oneof![
        Just(SourceKind::Native),
        Just(SourceKind::Mcp),
        Just(SourceKind::OpenClaw),
        Just(SourceKind::Sidecar),
        Just(SourceKind::Import),
        Just(SourceKind::Library),
        Just(SourceKind::Conversation),
    ]
}

fn arb_source() -> impl Strategy<Value = SourceRecord> {
    let a = (
        arb_source_kind(),
        arb_text_opt(),
        arb_text_opt(),
        arb_text_opt(),
        arb_policy(),
        arb_text(),
    );
    let b = (
        arb_text_opt(),
        arb_text_opt(),
        arb_text_opt(),
        arb_json_opt(),
        arb_ts_opt(),
        arb_ts_opt(),
    );
    (a, b).prop_map(
        |(
            (source_kind, external_identity, version, trust_class, policy, policy_version),
            (consent_state, content_hash, lifecycle_state, cursor_json, created_at, updated_at),
        )| SourceRecord {
            id: SourceId::new_v7(),
            source_kind,
            external_identity,
            version,
            trust_class,
            policy,
            policy_version,
            consent_state,
            content_hash,
            lifecycle_state,
            cursor_json,
            created_at,
            updated_at,
        },
    )
}

fn insert_source(conn: &Connection, s: &SourceRecord) -> rusqlite::Result<usize> {
    let created = s.created_at.map(|t| t.to_rfc3339());
    let updated = s.updated_at.map(|t| t.to_rfc3339());
    let owner = s.policy.owner_id().unwrap_or("");
    conn.execute(
        "INSERT INTO sources(
            id, source_kind, external_identity, version, trust_class,
            namespace, owner_id, scope, sensitivity, policy_version,
            consent_state, content_hash, lifecycle_state, cursor_json, created_at, updated_at)
         VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16)",
        params![
            s.id.as_str(),
            s.source_kind.as_str(),
            s.external_identity,
            s.version,
            s.trust_class,
            s.policy.namespace(),
            owner,
            s.policy.scope(),
            i64::from(s.policy.sensitivity()),
            s.policy_version,
            s.consent_state,
            s.content_hash,
            s.lifecycle_state,
            s.cursor_json,
            created,
            updated,
        ],
    )
}

// ── tool observations ────────────────────────────────────────────────────────

fn arb_tool_observation() -> impl Strategy<Value = ToolObservation> {
    let a = (
        arb_text(),
        arb_text_opt(),
        arb_text_opt(),
        arb_text_opt(),
        arb_text_opt(),
        arb_text_opt(),
        any::<bool>(),
    );
    let b = (
        arb_text_opt(),
        arb_text_opt(),
        arb_text_opt(),
        arb_text_opt(),
        arb_u64_opt(),
        arb_u32_opt(),
        arb_text_opt(),
    );
    let c = (
        arb_policy(),
        arb_text(),
        any::<bool>(),
        any::<bool>(),
        arb_ts_opt(),
    );
    (a, b, c).prop_map(
        |(
            (invocation_id, tool_kind, tool_id, tool_version, capability_id, outcome, has_goal),
            (
                environment_class,
                input_fingerprint,
                result_summary,
                error_class,
                latency_ms,
                retry_count,
                recovery_action,
            ),
            (policy, policy_version, has_start, has_completion, created_at),
        )| ToolObservation {
            id: ToolObservationId::new_v7(),
            invocation_id,
            tool_kind,
            tool_id,
            tool_version,
            capability_id,
            outcome,
            goal_id: has_goal.then(GoalId::new_v7),
            environment_class,
            input_fingerprint,
            result_summary,
            error_class,
            latency_ms,
            retry_count,
            recovery_action,
            policy,
            source_id: SourceId::new_v7(),
            policy_version,
            start_event_id: has_start.then(EventId::new_v7),
            completion_event_id: has_completion.then(EventId::new_v7),
            created_at,
        },
    )
}

fn insert_tool_observation(conn: &Connection, t: &ToolObservation) -> rusqlite::Result<usize> {
    let created = t.created_at.map(|x| x.to_rfc3339());
    let owner = t.policy.owner_id().unwrap_or("");
    conn.execute(
        "INSERT INTO tool_observations(
            id, invocation_id, tool_kind, tool_id, tool_version, capability_id, outcome, goal_id,
            environment_class, input_fingerprint, result_summary, error_class, latency_ms,
            retry_count, recovery_action, namespace, owner_id, scope, sensitivity, source_id,
            policy_version, start_event_id, completion_event_id, created_at)
         VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17,?18,?19,?20,?21,?22,?23,?24)",
        params![
            t.id.as_str(),
            t.invocation_id,
            t.tool_kind,
            t.tool_id,
            t.tool_version,
            t.capability_id,
            t.outcome,
            t.goal_id.as_ref().map(|x| x.as_str()),
            t.environment_class,
            t.input_fingerprint,
            t.result_summary,
            t.error_class,
            t.latency_ms.map(|v| v as i64),
            t.retry_count.map(i64::from),
            t.recovery_action,
            t.policy.namespace(),
            owner,
            t.policy.scope(),
            i64::from(t.policy.sensitivity()),
            t.source_id.as_str(),
            t.policy_version,
            t.start_event_id.as_ref().map(|x| x.as_str()),
            t.completion_event_id.as_ref().map(|x| x.as_str()),
            created,
        ],
    )
}

// ── retrieval traces ──────────────────────────────────────────────────────────

fn arb_graph_revision_opt() -> impl Strategy<Value = Option<GraphRevision>> {
    opt(0u64..=(i64::MAX as u64)).prop_map(|o| o.map(GraphRevision::new))
}

fn arb_retrieval_trace() -> impl Strategy<Value = RetrievalTrace> {
    let a = (
        arb_text_opt(),
        arb_text_opt(),
        arb_text_opt(),
        arb_text_opt(),
        arb_text_opt(),
        arb_text_opt(),
        arb_graph_revision_opt(),
    );
    let b = (
        arb_text_opt(),
        arb_u32_opt(),
        arb_text_opt(),
        arb_json_opt(),
        arb_text_opt(),
        arb_text_opt(),
        arb_ts_opt(),
    );
    (a, b).prop_map(
        |(
            (
                response_id,
                task_id,
                query_hash,
                query_class,
                classifier_version,
                profile_id,
                graph_revision,
            ),
            (
                policy_hash,
                token_budget,
                status,
                degradation_json,
                embed_model_version,
                rerank_model_version,
                created_at,
            ),
        )| RetrievalTrace {
            id: RetrievalTraceId::new_v7(),
            response_id,
            task_id,
            query_hash,
            query_class,
            classifier_version,
            profile_id,
            graph_revision,
            policy_hash,
            token_budget,
            status,
            degradation_json,
            embed_model_version,
            rerank_model_version,
            created_at,
        },
    )
}

fn insert_retrieval_trace(conn: &Connection, r: &RetrievalTrace) -> rusqlite::Result<usize> {
    let created = r.created_at.map(|t| t.to_rfc3339());
    conn.execute(
        "INSERT INTO retrieval_traces(
            id, response_id, task_id, query_hash, query_class, classifier_version, profile_id,
            graph_revision, policy_hash, token_budget, status, degradation_json,
            embed_model_version, rerank_model_version, created_at)
         VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15)",
        params![
            r.id.as_str(),
            r.response_id,
            r.task_id,
            r.query_hash,
            r.query_class,
            r.classifier_version,
            r.profile_id,
            r.graph_revision.map(|g| g.get() as i64),
            r.policy_hash,
            r.token_budget.map(i64::from),
            r.status,
            r.degradation_json,
            r.embed_model_version,
            r.rerank_model_version,
            created,
        ],
    )
}

// ── retrieval trace items ──────────────────────────────────────────────────────

fn arb_retrieval_trace_item() -> impl Strategy<Value = RetrievalTraceItem> {
    (
        arb_text(),
        arb_text(),
        arb_u32_opt(),
        arb_score_opt(),
        arb_score_opt(),
        arb_score_opt(),
        arb_text_opt(),
        arb_text_opt(),
        arb_u32_opt(),
        arb_u32_opt(),
        arb_u32_opt(),
        any::<bool>(),
    )
        .prop_map(
            |(
                record_id,
                strategy,
                strategy_rank,
                strategy_score,
                weight,
                rrf_contribution,
                gate_disposition,
                reason_code,
                token_cost,
                allocated_tokens,
                injected_order,
                has_goal,
            )| RetrievalTraceItem {
                trace_id: RetrievalTraceId::new_v7(),
                record_id,
                strategy,
                strategy_rank,
                strategy_score,
                weight,
                rrf_contribution,
                gate_disposition,
                reason_code,
                token_cost,
                allocated_tokens,
                injected_order,
                goal_id: has_goal.then(GoalId::new_v7),
            },
        )
}

fn insert_retrieval_trace_item(
    conn: &Connection,
    i: &RetrievalTraceItem,
) -> rusqlite::Result<usize> {
    conn.execute(
        "INSERT INTO retrieval_trace_items(
            trace_id, record_id, strategy, strategy_rank, strategy_score, weight,
            rrf_contribution, gate_disposition, reason_code, token_cost, allocated_tokens,
            injected_order, goal_id)
         VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13)",
        params![
            i.trace_id.as_str(),
            i.record_id,
            i.strategy,
            i.strategy_rank.map(i64::from),
            i.strategy_score,
            i.weight,
            i.rrf_contribution,
            i.gate_disposition,
            i.reason_code,
            i.token_cost.map(i64::from),
            i.allocated_tokens.map(i64::from),
            i.injected_order.map(i64::from),
            i.goal_id.as_ref().map(|x| x.as_str()),
        ],
    )
}

// ── feedback ─────────────────────────────────────────────────────────────────

fn arb_feedback() -> impl Strategy<Value = Feedback> {
    (
        arb_text_opt(),
        arb_text_opt(),
        arb_text_opt(),
        arb_json_opt(),
        arb_policy(),
        arb_text(),
        arb_text_opt(),
        any::<bool>(),
        arb_ts_opt(),
        arb_u64_opt(),
    )
        .prop_map(
            |(
                target_kind,
                target_id,
                signal,
                payload_json,
                policy,
                policy_version,
                actor_id,
                has_ev,
                created_at,
                revision,
            )| Feedback {
                id: FeedbackId::new_v7(),
                target_kind,
                target_id,
                signal,
                payload_json,
                policy,
                source_id: SourceId::new_v7(),
                policy_version,
                actor_id,
                event_id: has_ev.then(EventId::new_v7),
                created_at,
                revision,
            },
        )
}

fn insert_feedback(conn: &Connection, f: &Feedback) -> rusqlite::Result<usize> {
    let created = f.created_at.map(|t| t.to_rfc3339());
    let owner = f.policy.owner_id().unwrap_or("");
    conn.execute(
        "INSERT INTO feedback(
            id, target_kind, target_id, signal, payload_json,
            namespace, owner_id, scope, sensitivity, source_id, policy_version,
            actor_id, event_id, created_at, revision)
         VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15)",
        params![
            f.id.as_str(),
            f.target_kind,
            f.target_id,
            f.signal,
            f.payload_json,
            f.policy.namespace(),
            owner,
            f.policy.scope(),
            i64::from(f.policy.sensitivity()),
            f.source_id.as_str(),
            f.policy_version,
            f.actor_id,
            f.event_id.as_ref().map(|x| x.as_str()),
            created,
            f.revision.map(|v| v as i64),
        ],
    )
}

// ── properties: SQL↔Rust↔API round trip for every type ───────────────────────

proptest! {
    #![proptest_config(ProptestConfig::with_cases(CASES))]

    /// Records — all four record kinds (memory/summary/skill/rule) and both
    /// payload representations. Validates: MGR-002, MGR-034.
    #[test]
    fn record_roundtrip(v in arb_record()) {
        let back = sql_roundtrip(insert_record, "SELECT * FROM records", row_mapping::record, &v);
        prop_assert_eq!(&v, &back);
        api_roundtrip(&v);
    }

    #[test]
    fn entity_roundtrip(v in arb_entity()) {
        let back = sql_roundtrip(insert_entity, "SELECT * FROM entities_v2", row_mapping::entity, &v);
        prop_assert_eq!(&v, &back);
        api_roundtrip(&v);
    }

    #[test]
    fn alias_roundtrip(v in arb_alias()) {
        let back = sql_roundtrip(insert_alias, "SELECT * FROM aliases", row_mapping::alias, &v);
        prop_assert_eq!(&v, &back);
        api_roundtrip(&v);
    }

    #[test]
    fn mention_roundtrip(v in arb_mention()) {
        let back = sql_roundtrip(insert_mention, "SELECT * FROM mentions", row_mapping::mention, &v);
        prop_assert_eq!(&v, &back);
        api_roundtrip(&v);
    }

    #[test]
    fn evidence_roundtrip(v in arb_evidence()) {
        let back = sql_roundtrip(insert_evidence, "SELECT * FROM evidence_v2", row_mapping::evidence, &v);
        prop_assert_eq!(&v, &back);
        api_roundtrip(&v);
    }

    #[test]
    fn episode_roundtrip(v in arb_episode()) {
        let back = sql_roundtrip(insert_episode, "SELECT * FROM episodes_v2", row_mapping::episode, &v);
        prop_assert_eq!(&v, &back);
        api_roundtrip(&v);
    }

    #[test]
    fn goal_roundtrip(v in arb_goal()) {
        let back = sql_roundtrip(insert_goal, "SELECT * FROM goals_v2", row_mapping::goal, &v);
        prop_assert_eq!(&v, &back);
        api_roundtrip(&v);
    }

    #[test]
    fn goal_progress_roundtrip(v in arb_goal_progress()) {
        let back = sql_roundtrip(insert_goal_progress, "SELECT * FROM goal_progress", row_mapping::goal_progress, &v);
        prop_assert_eq!(&v, &back);
        api_roundtrip(&v);
    }

    #[test]
    fn consolidation_run_roundtrip(v in arb_consolidation_run()) {
        let back = sql_roundtrip(insert_consolidation_run, "SELECT * FROM consolidation_runs", row_mapping::consolidation_run, &v);
        prop_assert_eq!(&v, &back);
        api_roundtrip(&v);
    }

    #[test]
    fn source_roundtrip(v in arb_source()) {
        let back = sql_roundtrip(insert_source, "SELECT * FROM sources", row_mapping::source, &v);
        prop_assert_eq!(&v, &back);
        api_roundtrip(&v);
    }

    #[test]
    fn tool_observation_roundtrip(v in arb_tool_observation()) {
        let back = sql_roundtrip(insert_tool_observation, "SELECT * FROM tool_observations", row_mapping::tool_observation, &v);
        prop_assert_eq!(&v, &back);
        api_roundtrip(&v);
    }

    #[test]
    fn retrieval_trace_roundtrip(v in arb_retrieval_trace()) {
        let back = sql_roundtrip(insert_retrieval_trace, "SELECT * FROM retrieval_traces", row_mapping::retrieval_trace, &v);
        prop_assert_eq!(&v, &back);
        api_roundtrip(&v);
    }

    #[test]
    fn retrieval_trace_item_roundtrip(v in arb_retrieval_trace_item()) {
        let back = sql_roundtrip(insert_retrieval_trace_item, "SELECT * FROM retrieval_trace_items", row_mapping::retrieval_trace_item, &v);
        prop_assert_eq!(&v, &back);
        api_roundtrip(&v);
    }

    #[test]
    fn feedback_roundtrip(v in arb_feedback()) {
        let back = sql_roundtrip(insert_feedback, "SELECT * FROM feedback", row_mapping::feedback, &v);
        prop_assert_eq!(&v, &back);
        api_roundtrip(&v);
    }
}

// ── explicit boundary examples (empty-optional + time boundaries) ─────────────

fn simple_record() -> Record {
    Record {
        id: RecordId::new_v7(),
        record_kind: RecordKind::Memory,
        schema_version: SchemaVersion::new(1),
        payload: RecordPayload::Plaintext(String::new()),
        content_hash: None,
        truth_state: None,
        staleness_class: None,
        valid_interval: ValidInterval::open(),
        policy: PolicyPartition::new("ns", "sc", 0).unwrap(),
        source_id: SourceId::new_v7(),
        policy_version: String::new(),
        created_event_id: EventId::new_v7(),
        created_at: UtcTimestamp::from_rfc3339_utc("0001-01-01T00:00:00Z").unwrap(),
        superseded_by: None,
        episode_id: None,
        goal_context_id: None,
        estimated_tokens: None,
        shred_key_id: None,
        key_version: None,
    }
}

#[test]
fn record_all_optionals_none_min_timestamp_roundtrips() {
    // Every optional field None, empty payload, and the minimum representable
    // RFC3339 UTC instant — the "empty optional + time boundary" corner.
    let rec = simple_record();
    let back = sql_roundtrip(
        insert_record,
        "SELECT * FROM records",
        row_mapping::record,
        &rec,
    );
    assert_eq!(rec, back);
    api_roundtrip(&rec);
}

#[test]
fn record_all_optionals_some_max_timestamp_roundtrips() {
    // Every optional field populated and the maximum representable RFC3339 UTC
    // instant, with an empty (from == until) half-open interval boundary.
    let ts = UtcTimestamp::from_rfc3339_utc("9999-12-31T23:59:59.999Z").unwrap();
    let rec = Record {
        id: RecordId::new_v7(),
        record_kind: RecordKind::Rule,
        schema_version: SchemaVersion::new(u32::MAX),
        payload: RecordPayload::Ciphertext(vec![0, 255, 7, 42]),
        content_hash: Some("deadbeef".into()),
        truth_state: Some(TruthState::Confirmed),
        staleness_class: Some(StalenessClass::Permanent),
        valid_interval: ValidInterval::new(Some(ts), Some(ts)).unwrap(),
        policy: PolicyPartition::with_owner("日本語", "🌐scope", 3, Some("owner-x".into()))
            .unwrap(),
        source_id: SourceId::new_v7(),
        policy_version: "p-∞".into(),
        created_event_id: EventId::new_v7(),
        created_at: ts,
        superseded_by: Some(RecordId::new_v7()),
        episode_id: Some(EpisodeId::new_v7()),
        goal_context_id: Some(GoalId::new_v7()),
        estimated_tokens: Some(u32::MAX),
        shred_key_id: Some("key-1".into()),
        key_version: Some(7),
    };
    let back = sql_roundtrip(
        insert_record,
        "SELECT * FROM records",
        row_mapping::record,
        &rec,
    );
    assert_eq!(rec, back);
    api_roundtrip(&rec);
}

// ── malformed-row isolation (MGR-034) ────────────────────────────────────────

/// Insert a `records` row with caller-chosen `id` / `created_at` strings but
/// otherwise-valid, CHECK-satisfying columns. Used to plant malformed rows that
/// bypass schema CHECKs (which do not guard UUID/timestamp *shape*).
fn insert_raw_record(conn: &Connection, id: &str, created_at: &str) -> rusqlite::Result<usize> {
    let src = SourceId::new_v7().into_string();
    let ev = EventId::new_v7().into_string();
    conn.execute(
        "INSERT INTO records(
            id, record_kind, schema_version, content, namespace, owner_id, scope,
            sensitivity, source_id, policy_version, created_event_id, created_at)
         VALUES (?1,'memory',1,'payload','ns','','sc',0,?2,'pv',?3,?4)",
        params![id, src, ev, created_at],
    )
}

#[test]
fn malformed_row_is_isolated_from_valid_rows() {
    let db = fresh_db();
    let conn = db.write();

    // Three valid records first.
    let valid_head: Vec<Record> = (0..3).map(|_| simple_record()).collect();
    for r in &valid_head {
        insert_record(&conn, r).unwrap();
    }

    // Malformed #1: a bad UUID in `id` (no schema CHECK guards UUID shape).
    insert_raw_record(&conn, "not-a-uuid", "2026-06-01T00:00:00Z").unwrap();
    // Malformed #2: a valid id but a non-RFC3339 `created_at` (TEXT, no shape CHECK).
    let good_id = RecordId::new_v7().into_string();
    insert_raw_record(&conn, &good_id, "2026-01-01 00:00:00").unwrap();

    // A valid record AFTER the malformed rows — proves the failure does not cascade.
    let tail = simple_record();
    insert_record(&conn, &tail).unwrap();

    let rows = row_mapping::read_isolated(
        &conn,
        "SELECT * FROM records ORDER BY rowid",
        row_mapping::record,
    )
    .expect("batch read itself must not fail");

    // Every physical row is accounted for — nothing silently dropped.
    assert_eq!(rows.len(), 6, "all six rows are returned (no silent loss)");

    let ok_count = rows.iter().filter(|r| r.is_ok()).count();
    let errs: Vec<_> = rows.iter().filter_map(|r| r.as_ref().err()).collect();
    assert_eq!(
        ok_count, 4,
        "all four valid rows survive alongside the malformed ones"
    );
    assert_eq!(errs.len(), 2, "both malformed rows are isolated as errors");
    for e in &errs {
        assert!(
            row_mapping::is_encoding_error(e),
            "malformed row must surface a typed encoding error, got {e:?}"
        );
    }
    // The trailing valid row must still project cleanly — isolation, not cascade.
    assert!(
        rows.last().unwrap().is_ok(),
        "a valid row after a malformed one must still read back"
    );
}

#[test]
fn malformed_entity_row_isolated_among_valid() {
    // Generality: the same isolation holds for another table/type.
    let db = fresh_db();
    let conn = db.write();

    let valid = Entity {
        id: EntityId::new_v7(),
        canonical_id: None,
        entity_type: Some("person".into()),
        display_name: Some("Ada".into()),
        normalized_name: Some("ada".into()),
        truth_state: Some(TruthState::Current),
        policy: PolicyPartition::new("ns", "sc", 0).unwrap(),
        source_id: SourceId::new_v7(),
        policy_version: "pv".into(),
        created_event_id: EventId::new_v7(),
        created_at: UtcTimestamp::now(),
        revision: Some(1),
    };
    insert_entity(&conn, &valid).unwrap();

    // Malformed: bad UUID in `id`, everything else valid & NOT NULL satisfied.
    let src = SourceId::new_v7().into_string();
    let ev = EventId::new_v7().into_string();
    conn.execute(
        "INSERT INTO entities_v2(
            id, namespace, owner_id, scope, sensitivity, source_id, policy_version,
            created_event_id, created_at)
         VALUES ('%%bad-uuid%%','ns','','sc',0,?1,'pv',?2,?3)",
        params![src, ev, UtcTimestamp::now().to_rfc3339()],
    )
    .unwrap();

    let rows = row_mapping::read_isolated(
        &conn,
        "SELECT * FROM entities_v2 ORDER BY rowid",
        row_mapping::entity,
    )
    .expect("batch read must not fail");
    assert_eq!(rows.len(), 2);
    assert!(rows[0].is_ok(), "valid entity reads back");
    assert!(rows[1].is_err(), "malformed entity is isolated");
    assert!(row_mapping::is_encoding_error(
        rows[1].as_ref().err().unwrap()
    ));
}
