-- Authority schema v2 — idempotency, revision history, and audit (design §4.1,
-- task 1.1.3).
--
-- This migration extends the v2 authority epoch (opened by 0011, event log
-- added by 0012) with the four tables that make command execution
-- deterministic, replayable, and auditable:
--   * `idempotency_results` — dedup ledger keyed by (caller_partition,
--     idempotency_key); the composite PK enforces caller-partition key
--     uniqueness. The "replay with a different command_hash → conflict" rule is
--     enforced by the write path in code (later tasks); the schema anchors the
--     uniqueness the write path relies on.
--   * `graph_revisions` — append-only revision ledger. Contiguity is enforced
--     structurally by `CHECK (base_revision = revision - 1)`, and immutability
--     by BEFORE UPDATE/DELETE abort triggers.
--   * `graph_changes` — append-only per-revision change set. FK ties each row
--     back to its `graph_revisions` parent; change kinds are constrained.
--   * `audit_records` — append-only disposition record for every command, with
--     a self-FK for reversal linkage.
--
-- FK targets must precede referrers because foreign_keys=ON (see configure()):
-- `events_v2` already exists from 0012; `graph_revisions` is created before
-- `graph_changes`; `audit_records`' self-FK resolves within one CREATE.
--
-- Scope (task 1.1.3 only): these four tables plus the FKs/CHECKs/PKs, the
-- append-only triggers, the `(record_kind,record_id,revision)` index on
-- `graph_changes`, and the event/revision/actor indexes on `audit_records`.
-- Broader index work is task 1.1.5.
--
-- Canonical encodings (design §4 preamble):
--   * UUIDs      → canonical lower-case TEXT
--   * timestamps → RFC3339 UTC TEXT

-- ── Idempotency dedup ledger (design §4.1) ───────────────────────────────
-- Composite PK (caller_partition, idempotency_key) enforces caller-partition
-- key uniqueness. Conflict-on-different-hash is a write-path rule (later tasks).
CREATE TABLE IF NOT EXISTS idempotency_results (
    caller_partition   TEXT NOT NULL,
    idempotency_key    TEXT NOT NULL,
    command_hash       TEXT NOT NULL,
    result_json        TEXT NOT NULL,
    committed_revision INTEGER,
    event_id           TEXT REFERENCES events_v2(id),
    created_at         TEXT NOT NULL,
    PRIMARY KEY (caller_partition, idempotency_key)
);

-- ── Append-only revision ledger (design §4.1) ────────────────────────────
-- Contiguity: base_revision must be exactly the prior revision. Immutability:
-- UPDATE/DELETE abort triggers (mirrors the events_v2 L1 triggers in 0012).
CREATE TABLE IF NOT EXISTS graph_revisions (
    revision      INTEGER PRIMARY KEY,
    base_revision INTEGER NOT NULL,
    tx_id         TEXT NOT NULL UNIQUE,
    committed_at  TEXT NOT NULL,
    actor_id      TEXT NOT NULL,
    policy_hash   TEXT NOT NULL,
    change_count  INTEGER NOT NULL CHECK (change_count >= 0),
    CHECK (base_revision = revision - 1)
);

CREATE TRIGGER IF NOT EXISTS trg_graph_revisions_no_update
    BEFORE UPDATE ON graph_revisions
    BEGIN SELECT RAISE(ABORT, 'graph_revisions are append-only (immutable, L1)'); END;
CREATE TRIGGER IF NOT EXISTS trg_graph_revisions_no_delete
    BEFORE DELETE ON graph_revisions
    BEGIN SELECT RAISE(ABORT, 'graph_revisions are append-only (immutable, L1)'); END;

-- ── Append-only per-revision change set (design §4.1) ────────────────────
CREATE TABLE IF NOT EXISTS graph_changes (
    revision         INTEGER NOT NULL,
    ordinal          INTEGER NOT NULL,
    record_kind      TEXT,
    record_id        TEXT,
    change_kind      TEXT CHECK (change_kind IN ('insert','update','state','delete','invalidate')),
    before_hash      TEXT,
    after_hash       TEXT,
    policy_partition TEXT NOT NULL,
    payload_json     TEXT,
    PRIMARY KEY (revision, ordinal),
    FOREIGN KEY (revision) REFERENCES graph_revisions(revision)
);

CREATE INDEX IF NOT EXISTS idx_graph_changes_record
    ON graph_changes (record_kind, record_id, revision);

CREATE TRIGGER IF NOT EXISTS trg_graph_changes_no_update
    BEFORE UPDATE ON graph_changes
    BEGIN SELECT RAISE(ABORT, 'graph_changes are append-only (immutable, L1)'); END;
CREATE TRIGGER IF NOT EXISTS trg_graph_changes_no_delete
    BEFORE DELETE ON graph_changes
    BEGIN SELECT RAISE(ABORT, 'graph_changes are append-only (immutable, L1)'); END;

-- ── Append-only audit ledger (design §4.1) ───────────────────────────────
-- `reversal_of` is a self-FK linking a reversal audit row to the row it undoes.
CREATE TABLE IF NOT EXISTS audit_records (
    id                TEXT PRIMARY KEY,
    event_id          TEXT REFERENCES events_v2(id),
    command_kind      TEXT,
    disposition       TEXT CHECK (disposition IN ('accepted','rejected','deferred')),
    policy_version    TEXT,
    actor_id          TEXT,
    caller_partition  TEXT,
    reason_codes_json TEXT,
    authority_revision INTEGER,
    created_at        TEXT,
    reversal_of       TEXT REFERENCES audit_records(id)
);

CREATE INDEX IF NOT EXISTS idx_audit_records_event
    ON audit_records (event_id);
CREATE INDEX IF NOT EXISTS idx_audit_records_revision
    ON audit_records (authority_revision);
CREATE INDEX IF NOT EXISTS idx_audit_records_actor
    ON audit_records (actor_id);

CREATE TRIGGER IF NOT EXISTS trg_audit_records_no_update
    BEFORE UPDATE ON audit_records
    BEGIN SELECT RAISE(ABORT, 'audit_records are append-only (immutable, L1)'); END;
CREATE TRIGGER IF NOT EXISTS trg_audit_records_no_delete
    BEFORE DELETE ON audit_records
    BEGIN SELECT RAISE(ABORT, 'audit_records are append-only (immutable, L1)'); END;
