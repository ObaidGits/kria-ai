//! Additive-only schema migrations (memory-upgrade design §31.2, Issue 18).
//!
//! Migrations never drop or rename columns; they only add. Each migration is
//! applied once inside a transaction and recorded in `schema_version` with a
//! BLAKE3 checksum of its script. A newer binary can always read an older DB;
//! a downgrade that would require dropping schema is refused.

use rusqlite::Connection;

use crate::memory::error::{MemoryResult, MigrationError, StorageError};
use crate::memory::ids::blake3_hex;

/// A single ordered migration step.
struct Migration {
    version: u32,
    sql: &'static str,
}

/// The ordered migration set. Append new entries; never edit an applied one
/// (edits are caught by the checksum guard).
const MIGRATIONS: &[Migration] = &[
    Migration {
        version: 1,
        sql: include_str!("schema/0001_init.sql"),
    },
    Migration {
        version: 2,
        sql: include_str!("schema/0002_vectors.sql"),
    },
    Migration {
        version: 3,
        sql: include_str!("schema/0003_library.sql"),
    },
    Migration {
        version: 4,
        sql: include_str!("schema/0004_conversation.sql"),
    },
    Migration {
        version: 5,
        sql: include_str!("schema/0005_runtime_compat.sql"),
    },
    Migration {
        version: 6,
        sql: include_str!("schema/0006_goals.sql"),
    },
    Migration {
        version: 7,
        sql: include_str!("schema/0007_plans.sql"),
    },
    Migration {
        version: 8,
        sql: include_str!("schema/0008_reasoning.sql"),
    },
    Migration {
        version: 9,
        sql: include_str!("schema/0009_retrieval_weights.sql"),
    },
    Migration {
        version: 10,
        sql: include_str!("schema/0010_causal.sql"),
    },
    // ── v2 authority schema epoch (design §4) ──
    // 0011 opens schema_epoch = 2 with the meta scaffolding (schema_versions
    // + authority_meta singleton). Sibling tasks 1.1.2–1.1.7 extend this same
    // epoch; the legacy singular `schema_version` ledger stays until cutover.
    Migration {
        version: 11,
        sql: include_str!("schema/0011_authority_meta_v2.sql"),
    },
    // 0012 adds the redesigned immutable event log `events_v2` (design §4.1,
    // task 1.1.2): phase/outcome, source/invocation/time/policy/checksum/schema
    // fields, exactly-one payload representation, and UPDATE/DELETE abort
    // triggers. Legacy `events` (0001) is untouched until cutover (task 1.1.7).
    Migration {
        version: 12,
        sql: include_str!("schema/0012_events_v2.sql"),
    },
    // 0013 adds the idempotency dedup ledger, the append-only revision history
    // (`graph_revisions` + `graph_changes`), and the append-only `audit_records`
    // ledger (design §4.1, task 1.1.3). Contiguous base revision is enforced by
    // a table CHECK (base_revision = revision - 1); revision/change/audit rows
    // are immutable via UPDATE/DELETE abort triggers; the composite
    // (caller_partition, idempotency_key) PK enforces caller-partition key
    // uniqueness. FK targets precede referrers (events_v2 from 0012;
    // graph_revisions before graph_changes; audit_records self-FK within-CREATE).
    Migration {
        version: 13,
        sql: include_str!("schema/0013_revisions_audit_v2.sql"),
    },
    // 0014 adds the derived-projection delivery/integrity/recovery + crypto-shred
    // base infra (design §4.1/§4.3/§4.4, task 1.1.4): `derived_outbox` with
    // retry/dead-letter state and the semantic-uniqueness UNIQUE INDEX
    // (COALESCE(model_partition,'')); `derived_manifests` (PK target,version);
    // `recovery_snapshots` (metadata-only, verified_at nullable);
    // `shred_keys_v2` (reference-only catalog — never secret bytes, MGR-041 —
    // with a terminal "destroyed" BEFORE UPDATE trigger; `_v2` suffix because a
    // legacy `shred_keys` from 0001 coexists until cutover, mirroring
    // events→events_v2); the minimal `sources` base row
    // (source_kind CHECK + policy/lifecycle base fields); and
    // `interchange_imports` (event_id FKs events_v2 from 0012). Outbox/manifest/
    // import `authority_revision` columns are plain INTEGERs (no hard FK) by
    // design intent so derived writes are not over-constrained by the revision
    // ledger.
    Migration {
        version: 14,
        sql: include_str!("schema/0014_outbox_manifests_shred_v2.sql"),
    },
    // 0015 adds the v2 secondary index set that the table-creation tasks
    // (0012–0014) explicitly deferred (design §4.1 "Required indexes/triggers"
    // + startup/query paths, task 1.1.5): the partial UNIQUE source-identity
    // index on events_v2 (source_kind, source_id, source_event_id WHERE
    // source_event_id IS NOT NULL) plus events_v2 session/invocation/policy/
    // shred/idempotency indexes; idempotency_results revision/event indexes;
    // graph_revisions base/committed indexes; derived_outbox pending-pull
    // (target,status,next_attempt_at,id) + revision indexes; derived_manifests
    // revision index; shred_keys_v2 status index; sources identity/policy/
    // lifecycle indexes; and interchange_imports partial-UNIQUE idempotency +
    // status indexes. Indexes only — no tables/columns/triggers. The standalone
    // events_v2 (hlc) index is intentionally omitted (the inline UNIQUE already
    // provides the ordered btree). Duplicates of earlier-migration indexes
    // (graph_changes/audit_records/derived_outbox semantic) are not repeated.
    Migration {
        version: 15,
        sql: include_str!("schema/0015_authority_indexes_v2.sql"),
    },
    // 0016 adds the append-only `declassifications` provenance table (design
    // §4.1, MGR-004 AC3, task 1.4.3): an authorized declassification creates a
    // new immutable provenance record here rather than mutating the
    // contributing `sources` row. Captures target, prior/new policy (hash +
    // snapshot), authorizing actor, justification, integrity provenance hash,
    // the correlating invocation id, and a `reverses` self-link for a
    // compensating declassification. Immutable via UPDATE/DELETE abort triggers.
    Migration {
        version: 16,
        sql: include_str!("schema/0016_declassifications_v2.sql"),
    },
    // 0017 opens the F2 semantic epoch: the typed cognitive record model and
    // its supporting semantic/observation base tables (design §4.2/§4.3, task
    // F2.1.1). Adds `records` (the core cognitive record with record_kind CHECK,
    // exactly-one-payload CHECK, non-inverted valid-interval CHECK,
    // estimated_tokens>=0, policy columns, and FKs to events_v2/episodes_v2/
    // goals_v2 + self supersession), `entities_v2`/`aliases`/`mentions`/
    // `evidence_v2` (§4.2 cognitive-record subset; the relation_registry /
    // relationships / memory_links / entity_resolution_* tables are F2.2 —
    // task 2.2, not here), and `episodes_v2`/`goals_v2`/`goal_progress`
    // (append-only)/`consolidation_runs`/`tool_observations`/`retrieval_traces`/
    // `retrieval_trace_items`/`feedback` (§4.3) with their CHECKs, required
    // uniqueness keys ((normalized_alias,alias_type,namespace,scope);
    // (algorithm,version,input_set_hash,level); unique invocation completion),
    // json_valid guards, and required indexes. Four names take the `_v2`
    // coexistence suffix (episodes/goals/entities/evidence) because legacy
    // tables from 0001/0006 still live in the DB until the F1.5/F2.1.6 cutover,
    // mirroring the events→events_v2 precedent. The `sources` table (created in
    // 0014) is reconciled by adding its deferred identity/version/policy/
    // lifecycle indexes only. Provenance encoding, hash/token/staleness/truth/
    // lifecycle wiring, unknown-field preservation, round-trip property tests,
    // and legacy-struct removal are tasks 2.1.2–2.1.6.
    Migration {
        version: 17,
        sql: include_str!("schema/0017_cognitive_records_v2.sql"),
    },
    // 0018 delivers the F2.2 relation-identity authority that 0017 deferred
    // (design §4.2/§19.3, task F2.2.1): the versioned `relation_registry`
    // (PK (relation_name, version) with the direction_class {directed,
    // symmetric} CHECK, validity_policy {optional,required,forbidden} CHECK,
    // reflexive/writable INTEGER 0..1 CHECKs, and json_valid-guarded
    // aliases/source_kinds/target_kinds/evidence_policy JSON columns) plus its
    // materialized `relation_aliases` (normalized surface form → (relation_name,
    // version); (alias, version) unique). Seeds the five REQUIRED canonical
    // Memory Link rows (`derived_from`, `supports`, `contradicts`,
    // `mentions_entity`, `superseded_by` — directed, non-reflexive, writable=0)
    // and two representative domain-ontology rows establishing the
    // directed/symmetric contract (`related_to` symmetric, `part_of` directed
    // with a distinct inverse). All rows are version 1. The `relationships` /
    // `memory_links` / `entity_resolution_*` tables, the canonical identity
    // hash, AuthorityTx endpoint/kind/direction/evidence validation, and the
    // governed link write commands are the subsequent F2.2 subtasks
    // (2.2.2–2.2.7), not here.
    Migration {
        version: 18,
        sql: include_str!("schema/0018_relation_registry_v2.sql"),
    },
    // 0019 adds the `relationships_v2` semantic-link table that 0018 deferred
    // (design §4.2/§19.12, task F2.2.3): the polymorphic-endpoint relationship
    // row (source/target kind+id CHECKed against the closed EndpointKind set,
    // no hard FK — AuthorityTx checks existence), the `(relation_name,
    // relation_version)` FK to `relation_registry`, the non-inverted valid-
    // interval CHECK, the unique ACTIVE `identity_hash` index (excluding
    // superseded/forgotten/deleted), and the source/target endpoint-expansion
    // indexes. `_v2` suffix because the legacy free-text `relationships` table
    // from 0001 coexists until the F2.2.7 cutover. Non-reflexivity, endpoint-
    // kind legality, and evidence minimums are relation-specific and are
    // AuthorityTx checks, not DB CHECKs (design §19.12). No rows are inserted
    // here — governed writes are task 2.2.5.
    Migration {
        version: 19,
        sql: include_str!("schema/0019_relationships_v2.sql"),
    },
    // 0020 adds the structural, defense-in-depth partial UNIQUE index over
    // `evidence_v2 (subject_kind, subject_id, source_event_id) WHERE
    // source_event_id IS NOT NULL` (design §4.2, task F2.2.4): the same
    // authority event can never be appended as evidence for the same subject
    // twice, independent of caller idempotency-key discipline. See
    // `authority/relationship_evidence.rs` module docs for the full dedup-key
    // decision (this index vs. the outer F1.3 idempotency_key/command_hash
    // layer for event-less evidence).
    Migration {
        version: 20,
        sql: include_str!("schema/0020_evidence_dedup_v2.sql"),
    },
    // 0021 adds `restore_until TEXT` to `memories` (design §5.4, task F1.7.2).
    // Populated with `now() + 30 days` (RFC3339 UTC text) when a memory is
    // tombstoned to `Forgotten` via the governed `Lifecycle::forget()` commit.
    // NULL means the memory has never been forgotten. Additive ALTER TABLE ADD
    // COLUMN with DEFAULT NULL so existing rows are not affected.
    Migration {
        version: 21,
        sql: include_str!("schema/0021_forget_restore_until.sql"),
    },
    // 0022 hardens the `embedding_outbox` relay (task 1.8.4, MGR-042):
    // adds `next_attempt_at TEXT` (backoff gate: NULL = eligible immediately;
    // RFC3339 UTC = suppress until this time) and `error_code TEXT` (last
    // failure reason for dead-letter observability). Both are additive with
    // DEFAULT NULL. Also adds a covering index `ix_outbox_pending_v2` on
    // (index_target, status, next_attempt_at, id) for the enhanced pending-pull
    // query that filters on the time gate.
    Migration {
        version: 22,
        sql: include_str!("schema/0022_outbox_relay_v2.sql"),
    },
    // 0023 adds rebuild-tracking columns to `derived_manifests` (task 1.8.5,
    // MGR-042 / design §5.3): `rebuild_generation` (monotonically-increasing
    // generation counter; NULL = no active rebuild), `rebuild_cursor` (last
    // authority row id processed; NULL = not yet started), and
    // `rebuild_started_at` (RFC3339 UTC; observability). All additive with
    // DEFAULT NULL. Also adds a partial index `ix_derived_manifests_building`
    // on (target, status) WHERE status = 'building' for fast in-progress lookup.
    Migration {
        version: 23,
        sql: include_str!("schema/0023_rebuild_cursor.sql"),
    },
    // 0024 is the F2.2.7 hard cutover: drop the legacy free-text `relationships`
    // table (from 0001_init.sql) and its associated indexes plus the now-invalid
    // `graph_2hop_cache`.  By this point every resolvable legacy row has been
    // migrated to `relationships_v2` by the F2.2.6 LegacyRelationshipMigrator;
    // all Rust write paths have been redirected to `relationships_v2` or removed.
    // The `LegacyRelationshipMigrator::read_legacy_rows` caller guards against
    // a missing table and returns empty when this migration has been applied.
    Migration {
        version: 24,
        sql: include_str!("schema/0024_drop_legacy_relationships.sql"),
    },
    // 0025 delivers the F3.1 / task 3.1.2 vector partition schema:
    // `embedding_partitions` (partition registry keyed by partition_id, with
    // model/revision/checksum/license/runtime/encoding columns and a partial
    // UNIQUE index on (model_id, model_source_revision) WHERE status != 'deleted')
    // and `mem_vectors_v2` (rebuildable derived projection, (partition_id,
    // record_id) PK, policy columns on every row, vector BLOB length-checked to
    // 1536, policy-prefiltered composite index, and content_hash dedup index).
    // The legacy `mem_vectors` table (0002) coexists until the F3.1 write-path
    // cutover; this migration adds the new tables only.
    Migration {
        version: 25,
        sql: include_str!("schema/0025_vector_partitions_v2.sql"),
    },
    // 0026 adds a dedicated `rebuild_cursor` table for F3.1 / task 3.1.5
    // temporary-generation rebuild tracking (design §5.3): per-partition
    // `(run_id, last_record_id, status, migration_source_partition_id,
    // started_at, updated_at)`.  Distinct from the additive columns added to
    // `derived_manifests` in 0023 — this table has partition_id as PK so each
    // partition's in-progress / interrupted rebuild state is independently
    // queryable.  The `migration_source_partition_id` column records the old
    // partition being migrated FROM during a model-version upgrade.
    Migration {
        version: 26,
        sql: include_str!("schema/0026_rebuild_cursor_v2.sql"),
    },
    // 0027 delivers the F3.2 / task 3.2.1 `search_documents` projection table
    // (design §4.4): the authority-derived projection that the FTS5
    // external-content table (task 3.2.2) indexes over.  One row per
    // searchable item — memory/summary/skill/rule cognitive records, entity
    // names/aliases, source records, goals, and relationship labels — with
    // title/body/aliases/source_text/relation_text searchable columns, full
    // policy (namespace/owner_id/scope/sensitivity), truth_state,
    // valid_from/valid_until, content_hash (SHA-256 for dedup/rebuild), and
    // revision (graph revision cursor).  Three supporting indexes:
    // `ix_sd_policy` (namespace, scope, sensitivity, truth_state) for FTS5
    // prefilter; `ix_sd_revision` (revision, record_kind) for rebuild cursors;
    // `ix_sd_content_hash` (record_kind, record_id, content_hash) for dedup.
    // PRIMARY KEY is (record_kind, record_id); content is rebuildable — no
    // authority data originates here.
    Migration {
        version: 27,
        sql: include_str!("schema/0027_search_documents.sql"),
    },
    // 0028 delivers the F3.2 / task 3.2.2 `search_documents_fts` FTS5 external-
    // content table (design §4.4): the rebuildable full-text index that points at
    // `search_documents` (0027).  Indexed columns are title/body/aliases/
    // source_text/relation_text; UNINDEXED carry-through columns are record_kind,
    // record_id, namespace, scope, sensitivity, truth_state, revision (for policy
    // prefiltering and result construction).  Tokenizer is `unicode61
    // remove_diacritics 2`; prefix sizes `2 3 4`.  Three AFTER-triggers on
    // `search_documents` (INSERT / UPDATE / DELETE) keep the FTS5 index in sync
    // using the FTS5 external-content 'delete' command protocol.  FTS5 is NEVER
    // the semantic authority — it is a disposable derived projection.
    Migration {
        version: 28,
        sql: include_str!("schema/0028_search_documents_fts.sql"),
    },
    // 0029 extends the retrieval trace tables with RRF replay fields (design
    // §4.3/§6.4, task F3.4.4): `k_value`, `availability_json`, `weights_json`,
    // `evidence_contribution`, `memory_worth_contribution`, and
    // `goal_contribution_total` are added to `retrieval_traces`; per-item
    // `evidence_contribution` and `memory_worth_contribution` are added to
    // `retrieval_trace_items`.  These fields enable exact offline RRF replay
    // from stored one-based ranks and separately track Evidence/Memory-Worth
    // contributions (inert below 20 observations per design §6.4).
    Migration {
        version: 29,
        sql: include_str!("schema/0029_retrieval_trace_v2_ext.sql"),
    },
    // 0030 seeds the `co_mentioned_with` relation into `relation_registry` and
    // `relation_aliases` (task F2.2.7 follow-up). The extraction pipeline
    // (`EntityExtractionPipeline::add_comention_edges`) writes symmetric
    // co-occurrence edges directly to `relationships_v2` via INSERT OR IGNORE,
    // but `relationships_v2.(relation_name, relation_version)` FK-references
    // `relation_registry`; without this seed every co-mention insert would fail
    // with a foreign-key constraint violation. Migration 0018 only seeded the
    // five canonical links and the two representative domain rows; this additive
    // migration fills the gap without touching the 0018 checksum.
    Migration {
        version: 30,
        sql: include_str!("schema/0030_co_mentioned_with_registry.sql"),
    },
    // 0031 delivers the `memory_links` table (design §4.2/§7.3, task F3.6.4):
    // the distinct table for the five canonical cognitive lineage/evidence
    // relations (`derived_from`, `supports`, `contradicts`, `mentions_entity`,
    // `superseded_by`) that migrations 0017–0019 explicitly deferred.
    // `derive_record` (task F3.6.4) writes a `derived_from` link from each
    // parent record to the newly derived record immediately after insertion.
    // Polymorphic endpoints have a closed-set CHECK; `(link_type, link_version)`
    // FKs `relation_registry`; unique ACTIVE semantic identity index excludes
    // superseded/forgotten/deleted truth states.
    Migration {
        version: 31,
        sql: include_str!("schema/0031_memory_links_v2.sql"),
    },
    // 0032 extends `tool_observations` with two policy-safe rich-facts columns
    // (design §4.3, task F3.7.2): `server_id TEXT` (the MCP/sidecar/OpenClaw
    // server hosting the tool; NULL = native/unknown) and
    // `affected_records_json TEXT` (JSON array of authority `records.id` values
    // explicitly linked to the outcome; NULL = no record association; a
    // json_valid CHECK guards non-NULL values). Both are additive with
    // DEFAULT NULL. A partial index on `server_id` supports future per-server
    // capability_observations aggregations.
    Migration {
        version: 32,
        sql: include_str!("schema/0032_tool_observations_ext.sql"),
    },
];

/// The highest schema version this binary knows about.
pub fn latest_version() -> u32 {
    MIGRATIONS.iter().map(|m| m.version).max().unwrap_or(0)
}

/// Return a map from migration version → expected BLAKE3 checksum of the
/// compiled-in SQL script.  Used by the startup integrity checker
/// ([`crate::memory::authority::integrity::StartupIntegrityChecker`]) to
/// verify that the `schema_version` ledger has not been tampered with after
/// migrations were applied.
pub fn migration_checksums() -> std::collections::HashMap<u32, String> {
    MIGRATIONS
        .iter()
        .map(|m| (m.version, blake3_hex(m.sql.as_bytes())))
        .collect()
}

/// Apply all pending migrations to `conn`. Idempotent: already-applied
/// migrations are skipped. Returns the resulting schema version.
pub fn run(conn: &Connection) -> MemoryResult<u32> {
    // Bootstrap the version table so we can query applied versions.
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS schema_version (
            version INTEGER PRIMARY KEY,
            applied_at TEXT NOT NULL,
            checksum TEXT NOT NULL
        );",
    )
    .map_err(StorageError::Sqlite)?;

    let applied = applied_version(conn)?;
    let target = latest_version();

    // Refuse a silent downgrade: the DB is newer than this binary understands.
    if applied > target {
        return Err(MigrationError::SchemaTooOld {
            found: applied,
            required: target,
        }
        .into());
    }

    for m in MIGRATIONS {
        if m.version <= applied {
            verify_checksum(conn, m)?;
            continue;
        }
        apply_one(conn, m)?;
    }

    Ok(target)
}

/// Highest applied version, or 0 if none.
fn applied_version(conn: &Connection) -> MemoryResult<u32> {
    let v: Option<i64> = conn
        .query_row("SELECT MAX(version) FROM schema_version", [], |r| r.get(0))
        .map_err(StorageError::Sqlite)?;
    Ok(v.unwrap_or(0) as u32)
}

/// Apply a single migration inside a transaction and record it.
fn apply_one(conn: &Connection, m: &Migration) -> MemoryResult<()> {
    let checksum = blake3_hex(m.sql.as_bytes());
    conn.execute_batch("BEGIN;").map_err(StorageError::Sqlite)?;
    let result = (|| -> Result<(), StorageError> {
        conn.execute_batch(m.sql).map_err(StorageError::Sqlite)?;
        conn.execute(
            "INSERT INTO schema_version(version, applied_at, checksum) VALUES (?1, ?2, ?3)",
            rusqlite::params![m.version as i64, chrono::Utc::now().to_rfc3339(), checksum],
        )
        .map_err(StorageError::Sqlite)?;
        Ok(())
    })();

    match result {
        Ok(()) => {
            conn.execute_batch("COMMIT;")
                .map_err(StorageError::Sqlite)?;
            tracing::info!(version = m.version, "applied memory schema migration");
            Ok(())
        }
        Err(e) => {
            let _ = conn.execute_batch("ROLLBACK;");
            Err(e.into())
        }
    }
}

/// Guard against an applied migration being edited after the fact.
fn verify_checksum(conn: &Connection, m: &Migration) -> MemoryResult<()> {
    let recorded: Option<String> = conn
        .query_row(
            "SELECT checksum FROM schema_version WHERE version = ?1",
            [m.version as i64],
            |r| r.get(0),
        )
        .map_err(StorageError::Sqlite)?;
    let expected = blake3_hex(m.sql.as_bytes());
    match recorded {
        Some(c) if c == expected => Ok(()),
        Some(_) => Err(MigrationError::Script(format!(
            "migration {} checksum mismatch: script changed after being applied",
            m.version
        ))
        .into()),
        None => Ok(()),
    }
}
