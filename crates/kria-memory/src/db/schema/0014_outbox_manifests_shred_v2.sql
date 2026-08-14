-- Authority schema v2 — outbox, manifests, recovery, crypto-shred, and
-- source/policy/lifecycle base (design §4.1/§4.3/§4.4, task 1.1.4).
--
-- This migration extends the v2 authority epoch (opened by 0011; events_v2 in
-- 0012; revisions/idempotency/audit in 0013) with the derived-projection
-- delivery + integrity + recovery + crypto-shred base infra, plus the minimal
-- `sources` base row that anchors lifecycle/policy at the authority layer.
--
-- Scope (task 1.1.4 only):
--   * `derived_outbox`      — outbox rows with retry/dead-letter state and the
--                             semantic-uniqueness key (COALESCE model_partition).
--   * `derived_manifests`   — per-target integrity/rebuild-comparison manifests.
--   * `recovery_snapshots`  — metadata-only snapshot catalog (no validity claim
--                             until verified_at is set).
--   * `shred_keys_v2`       — crypto-shred key CATALOG: references only, NEVER
--                             secret key bytes (MGR-041). Destroyed is terminal.
--                             `_v2` suffix because a legacy `shred_keys`
--                             (0001_init.sql) still lives in the DB until cutover
--                             (task 1.1.7), mirroring the events → events_v2
--                             precedent; the legacy table is left untouched.
--   * `sources`             — BASE fields only (id/kind/identity/policy/lifecycle
--                             /cursor/times). Full indexes/expansion are F2 (2.x).
--   * `interchange_imports` — import package catalog (authority/recovery base).
-- Siblings 1.1.5–1.1.7 (broader index set, cognitive/derived tables, cutover)
-- are out of scope here.
--
-- FK targets must precede referrers because foreign_keys=ON (see configure()):
-- `events_v2` already exists from 0012, so `interchange_imports.event_id` FK
-- resolves. `authority_revision` columns are PLAIN INTEGERs by design intent:
-- outbox/manifest/import rows REFERENCE the authority revision but do NOT
-- enforce a hard FK to `graph_revisions` (a hard FK would over-constrain
-- outbox/manifest writes that race ahead of / behind the revision ledger).
--
-- Canonical encodings (design §4 preamble):
--   * UUIDs      → canonical lower-case TEXT
--   * timestamps → RFC3339 UTC TEXT

-- ── Crypto-shred key CATALOG (design §4.1) ───────────────────────────────
-- HONESTY INVARIANT (MGR-041): this table stores key METADATA only. `key_ref`
-- is an EXTERNAL locator/reference (e.g. an OS keyring handle or keystore path)
-- — it is NEVER the secret key material itself. No column in this table ever
-- holds secret key bytes; destroying the key means the referenced external
-- material is gone and the ciphertext is unrecoverable.
-- NOTE: named `shred_keys_v2` (not `shred_keys`) because a legacy `shred_keys`
-- table already exists (0001_init.sql). A plain `CREATE TABLE IF NOT EXISTS
-- shred_keys` would silently no-op against the legacy schema. This mirrors the
-- events → events_v2 coexistence pattern; the legacy table is untouched and is
-- dropped at cutover (task 1.1.7).
CREATE TABLE IF NOT EXISTS shred_keys_v2 (
    subject_id         TEXT NOT NULL,
    key_version        INTEGER NOT NULL,
    key_ref            TEXT NOT NULL,   -- external reference/locator ONLY (never secret bytes)
    algorithm          TEXT,
    status             TEXT CHECK (status IN ('active','destroyed','unavailable')),
    created_at         TEXT,
    destroyed_at       TEXT,
    destruction_method TEXT,
    proof_hash         TEXT,
    PRIMARY KEY (subject_id, key_version)
);

-- Destroyed is TERMINAL: a destroyed key can never be resurrected (status must
-- stay 'destroyed'). Transitions active→destroyed and active→unavailable remain
-- allowed; broader immutability is intentionally NOT enforced here.
CREATE TRIGGER IF NOT EXISTS trg_shred_keys_v2_destroyed_terminal
    BEFORE UPDATE ON shred_keys_v2
    WHEN OLD.status = 'destroyed' AND NEW.status <> 'destroyed'
    BEGIN SELECT RAISE(ABORT, 'shred_keys_v2: destroyed status is terminal (cannot resurrect)'); END;

-- ── Derived projection outbox (design §4.4) ──────────────────────────────
-- Retry/dead-letter state: attempts / status / next_attempt_at / error_code.
-- `authority_revision` is a plain INTEGER (see header: no hard FK).
CREATE TABLE IF NOT EXISTS derived_outbox (
    id                 INTEGER PRIMARY KEY AUTOINCREMENT,
    target             TEXT NOT NULL,
    op                 TEXT NOT NULL,
    record_kind        TEXT,
    record_id          TEXT,
    content_hash       TEXT,
    model_partition    TEXT,
    authority_revision INTEGER,
    attempts           INTEGER NOT NULL DEFAULT 0,
    status             TEXT NOT NULL DEFAULT 'pending',
    next_attempt_at    TEXT,
    error_code         TEXT,
    created_at         TEXT NOT NULL
);

-- Semantic uniqueness key (design §4.4): SQLite cannot embed COALESCE in a
-- table-level UNIQUE constraint, so it is a UNIQUE INDEX over the expression.
-- NULL model_partition collapses to '' so two NULL-model rows that otherwise
-- match are duplicates, while a NULL-model and a 'p'-model row are distinct.
CREATE UNIQUE INDEX IF NOT EXISTS idx_derived_outbox_semantic
    ON derived_outbox (
        target, op, record_kind, record_id, content_hash, COALESCE(model_partition, '')
    );

-- ── Derived manifests (design §4.4) ──────────────────────────────────────
-- Per-target integrity / rebuild-comparison snapshot. PK (target, version).
-- `authority_revision` is a plain INTEGER (see header: no hard FK).
CREATE TABLE IF NOT EXISTS derived_manifests (
    target             TEXT NOT NULL,
    version            INTEGER NOT NULL,
    authority_revision INTEGER,
    member_count       INTEGER,
    membership_hash    TEXT,
    algorithm_version  TEXT,
    model_version      TEXT,
    completed_cursor   TEXT,
    completed_at       TEXT,
    status             TEXT,
    PRIMARY KEY (target, version)
);

-- ── Recovery snapshot catalog (design §4.4) ──────────────────────────────
-- METADATA ONLY. `verified_at` is nullable: a snapshot makes NO claim of
-- validity until verification passes and sets `verified_at`.
CREATE TABLE IF NOT EXISTS recovery_snapshots (
    id             TEXT PRIMARY KEY,
    path_ref       TEXT,
    schema_version INTEGER,
    revision       INTEGER,
    checksum       TEXT,
    verified_at    TEXT   -- NULL until verification passes (no validity claim)
);

-- ── Source/policy/lifecycle BASE row (design §4.3, base fields only) ──────
-- Only the base fields that anchor lifecycle/policy at the authority layer plus
-- the source_kind CHECK. Full identity/version/policy/lifecycle indexes and the
-- rest of the F2 expansion are task 2.x.
CREATE TABLE IF NOT EXISTS sources (
    id                TEXT PRIMARY KEY,
    source_kind       TEXT CHECK (source_kind IN
                          ('native','mcp','openclaw','sidecar','import','library','conversation')),
    external_identity TEXT,
    version           TEXT,
    trust_class       TEXT,
    -- Policy columns (design §4.1)
    namespace         TEXT NOT NULL,
    owner_id          TEXT NOT NULL,
    scope             TEXT NOT NULL,
    sensitivity       INTEGER NOT NULL CHECK (sensitivity BETWEEN 0 AND 3),
    policy_version    TEXT NOT NULL,
    -- Lifecycle
    consent_state     TEXT,
    content_hash      TEXT,
    lifecycle_state   TEXT,
    cursor_json       TEXT,
    created_at        TEXT,
    updated_at        TEXT
);

-- ── Interchange import catalog (design §4.4, authority/recovery base) ─────
-- `event_id` FKs `events_v2` (present from 0012). `authority_revision` is a
-- plain INTEGER (see header: no hard FK).
CREATE TABLE IF NOT EXISTS interchange_imports (
    id                 TEXT PRIMARY KEY,
    package_ref        TEXT,
    checksum           TEXT,
    schema_version     INTEGER,
    status             TEXT,
    idempotency_key    TEXT,
    report_json        TEXT,
    event_id           TEXT REFERENCES events_v2(id),
    authority_revision INTEGER,
    created_at         TEXT
);
