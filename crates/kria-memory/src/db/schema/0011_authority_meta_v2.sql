-- Authority schema v2 — meta scaffolding (design §4 / §4.1, task 1.1.1).
--
-- This migration opens the v2 authority schema epoch (schema_epoch = 2). It
-- introduces the two meta tables that anchor the redesigned authority:
--   * `schema_versions` (PLURAL, with a `name` column) — the v2 migration
--     ledger, immutable after insert. Distinct from the legacy singular
--     `schema_version` bootstrap ledger owned by migrations.rs, which is left
--     in place until the reset/cutover work (task 1.1.7).
--   * `authority_meta` — a singleton row (id = 1) holding the graph revision
--     counter, the last event HLC, and the current schema epoch.
--
-- Sibling tasks 1.1.2–1.1.7 extend THIS same epoch (events, idempotency,
-- revisions, audit, outbox/manifests/shred_keys, indexes, pragmas, reset).
--
-- Canonical encodings (design §4 preamble):
--   * UUIDs        → canonical lower-case TEXT
--   * timestamps   → RFC3339 UTC TEXT
--   * booleans     → INTEGER CHECK (... IN (0,1))

-- ── v2 migration ledger (immutable after insert) ─────────────────────────
CREATE TABLE IF NOT EXISTS schema_versions (
    version    INTEGER PRIMARY KEY,
    name       TEXT NOT NULL UNIQUE,
    checksum   TEXT NOT NULL,
    applied_at TEXT NOT NULL
);

-- Immutable after insert: forbid UPDATE and DELETE (mirrors the `events` L1
-- immutability triggers in 0001_init.sql).
CREATE TRIGGER IF NOT EXISTS trg_schema_versions_no_update
    BEFORE UPDATE ON schema_versions
    BEGIN SELECT RAISE(ABORT, 'schema_versions rows are immutable after insert'); END;
CREATE TRIGGER IF NOT EXISTS trg_schema_versions_no_delete
    BEFORE DELETE ON schema_versions
    BEGIN SELECT RAISE(ABORT, 'schema_versions rows are immutable after insert'); END;

-- ── Singleton authority meta (id = 1) ────────────────────────────────────
CREATE TABLE IF NOT EXISTS authority_meta (
    id             INTEGER PRIMARY KEY CHECK (id = 1),
    graph_revision INTEGER NOT NULL CHECK (graph_revision >= 0),
    event_hlc      TEXT NOT NULL,
    schema_epoch   INTEGER NOT NULL
);

-- Singleton invariant: the `CHECK (id = 1)` column constraint rejects any row
-- whose id <> 1, and an explicit BEFORE INSERT trigger makes the rejection
-- reason unambiguous. A BEFORE DELETE trigger forbids removing the singleton.
CREATE TRIGGER IF NOT EXISTS trg_authority_meta_no_extra
    BEFORE INSERT ON authority_meta
    WHEN NEW.id <> 1
    BEGIN SELECT RAISE(ABORT, 'authority_meta is a singleton (id must be 1)'); END;
CREATE TRIGGER IF NOT EXISTS trg_authority_meta_no_delete
    BEFORE DELETE ON authority_meta
    BEGIN SELECT RAISE(ABORT, 'authority_meta singleton cannot be deleted'); END;

-- Seed the singleton. `INSERT OR IGNORE` keeps re-open idempotent (the PK
-- collision on id = 1 is silently ignored on a second application).
INSERT OR IGNORE INTO authority_meta (id, graph_revision, event_hlc, schema_epoch)
VALUES (1, 0, '', 2);
