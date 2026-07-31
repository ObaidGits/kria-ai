-- Authority schema v2 — secondary indexes for the v2 epoch (design §4.1
-- "Required indexes/triggers" + startup/query paths, task 1.1.5).
--
-- This migration extends the v2 authority epoch (opened by 0011; events_v2 in
-- 0012; revisions/idempotency/audit in 0013; outbox/manifests/recovery/shred/
-- sources/imports in 0014) with the secondary index set that the earlier
-- table-creation tasks explicitly DEFERRED to this task. No tables, columns, or
-- triggers are added here — indexes only. Earlier migrations are checksum-locked
-- and untouched; everything new lives in this file.
--
-- Indexes already created by earlier migrations are NOT duplicated here:
--   * idx_graph_changes_record            (0013)
--   * idx_audit_records_event/revision/actor (0013)
--   * idx_derived_outbox_semantic         (0014, UNIQUE COALESCE key)
-- as well as the implicit indexes SQLite creates for PRIMARY KEY and UNIQUE
-- constraints (events_v2.hlc UNIQUE, graph_revisions.tx_id UNIQUE, all PKs).
--
-- All statements use IF NOT EXISTS so the migration is safe to re-run and
-- coexists with the additive-only migration contract.

-- ── events_v2 secondary indexes (design §4.1) ────────────────────────────
-- Source identity: UNIQUE only "where source ID present" (design §4.1). The
-- partial predicate excludes NULL source_event_id rows so events without an
-- upstream source id are never forced unique against each other, while any two
-- events carrying the same (source_kind, source_id, source_event_id) are
-- rejected as duplicates of the same upstream event.
CREATE UNIQUE INDEX IF NOT EXISTS uq_events_v2_source_identity
    ON events_v2 (source_kind, source_id, source_event_id)
    WHERE source_event_id IS NOT NULL;

-- HLC ordering: a standalone `(hlc)` index is INTENTIONALLY OMITTED — the inline
-- UNIQUE NOT NULL on events_v2.hlc (0012) already materialises an ordered btree
-- that serves `ORDER BY hlc` startup/scan order checks, so a duplicate would be
-- dead weight. The session-scoped ordered scan path is covered by the composite
-- below (session_id, hlc), which the bare UNIQUE cannot serve.
CREATE INDEX IF NOT EXISTS idx_events_v2_session
    ON events_v2 (session_id, hlc);

-- Invocation correlation lookups.
CREATE INDEX IF NOT EXISTS idx_events_v2_invocation
    ON events_v2 (invocation_id);

-- Policy-partition scans (namespace/scope/sensitivity filters).
CREATE INDEX IF NOT EXISTS idx_events_v2_policy
    ON events_v2 (namespace, scope, sensitivity);

-- Crypto-shred transition/impact scans by key.
CREATE INDEX IF NOT EXISTS idx_events_v2_shred
    ON events_v2 (shred_key_id);

-- Idempotency/replay lookup path.
CREATE INDEX IF NOT EXISTS idx_events_v2_idempotency
    ON events_v2 (idempotency_key);

-- ── idempotency_results (startup/query + committed-revision lookups) ──────
CREATE INDEX IF NOT EXISTS idx_idempotency_results_revision
    ON idempotency_results (committed_revision);
CREATE INDEX IF NOT EXISTS idx_idempotency_results_event
    ON idempotency_results (event_id);

-- ── graph_revisions (revision chain / startup continuity) ────────────────
-- tx_id is already UNIQUE (0013); base/committed indexes support base-revision
-- chaining and time-ordered continuity checks at startup.
CREATE INDEX IF NOT EXISTS idx_graph_revisions_base
    ON graph_revisions (base_revision);
CREATE INDEX IF NOT EXISTS idx_graph_revisions_committed
    ON graph_revisions (committed_at);

-- ── derived_outbox (relay pull path) ─────────────────────────────────────
-- The pending-work query orders by target/status/next_attempt; id tie-breaks
-- for stable FIFO drain. authority_revision index supports revision-scoped
-- outbox inspection/rebuild.
CREATE INDEX IF NOT EXISTS idx_derived_outbox_pending
    ON derived_outbox (target, status, next_attempt_at, id);
CREATE INDEX IF NOT EXISTS idx_derived_outbox_revision
    ON derived_outbox (authority_revision);

-- ── derived_manifests (rebuild comparison by revision) ───────────────────
CREATE INDEX IF NOT EXISTS idx_derived_manifests_revision
    ON derived_manifests (authority_revision);

-- ── shred_keys_v2 (shred-transition / status scans) ──────────────────────
CREATE INDEX IF NOT EXISTS idx_shred_keys_v2_status
    ON shred_keys_v2 (status);

-- ── sources (identity / policy / lifecycle query paths — base) ───────────
CREATE INDEX IF NOT EXISTS idx_sources_identity
    ON sources (source_kind, external_identity);
CREATE INDEX IF NOT EXISTS idx_sources_policy
    ON sources (namespace, scope, sensitivity);
CREATE INDEX IF NOT EXISTS idx_sources_lifecycle
    ON sources (lifecycle_state);

-- ── interchange_imports (idempotency / status lookups) ───────────────────
-- Partial UNIQUE: only non-null idempotency_key must be unique; multiple NULL
-- (imports without an idempotency key) are allowed.
CREATE UNIQUE INDEX IF NOT EXISTS uq_interchange_imports_idem
    ON interchange_imports (idempotency_key)
    WHERE idempotency_key IS NOT NULL;
CREATE INDEX IF NOT EXISTS idx_interchange_imports_status
    ON interchange_imports (status);
