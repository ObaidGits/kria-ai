-- Authority schema v2 — immutable event log (design §4.1, task 1.1.2).
--
-- This migration extends the v2 authority epoch opened by 0011 with the
-- redesigned append-only event log. It is a NEW table, `events_v2`, kept
-- distinct from the legacy `events` table (0001_init.sql) which still lives in
-- the same database until the reset/cutover work (task 1.1.7). At cutover the
-- legacy `events` is dropped and this table can be renamed; keeping both
-- compiling side-by-side is required for now.
--
-- Scope (task 1.1.2 only): the table itself plus its phase/outcome/payload
-- constraints and the UPDATE/DELETE immutability triggers. The broader
-- secondary index set (unique (source_kind,source_id,source_event_id),
-- HLC/session/invocation/policy/shred indexes) belongs to task 1.1.5; only the
-- inline UNIQUE on `hlc` and the primary key are added here.
--
-- Canonical encodings (design §4 preamble):
--   * UUIDs      → canonical lower-case TEXT
--   * timestamps → RFC3339 UTC TEXT, with explicit source offset (tz_offset_min)
--   * payload    → EXACTLY ONE of {payload_cipher, payload_plain} is non-null

-- ── v2 event log (immutable, append-only — design §4.1) ──────────────────
CREATE TABLE IF NOT EXISTS events_v2 (
    -- Identity / correlation
    id               TEXT PRIMARY KEY,          -- canonical lower-case UUID text
    source_event_id  TEXT,
    idempotency_key  TEXT,
    invocation_id    TEXT,

    -- Lifecycle phase + typed outcome
    phase            TEXT NOT NULL CHECK (phase IN ('start','completion','observation')),
    outcome          TEXT,                      -- typed outcome; null for phases without one

    -- Ordering / time
    hlc              TEXT UNIQUE NOT NULL,      -- hybrid logical clock, globally unique
    ts_utc           TEXT NOT NULL,             -- RFC3339 UTC
    tz_offset_min    INTEGER NOT NULL,          -- source timezone offset, minutes

    -- Provenance
    event_type       TEXT NOT NULL,
    source_kind      TEXT NOT NULL,
    source_id        TEXT NOT NULL,             -- also part of policy provenance
    actor_id         TEXT NOT NULL,
    session_id       TEXT,
    parent_event_id  TEXT REFERENCES events_v2(id),

    -- Policy columns (design §4.1): namespace/owner/scope/sensitivity/policy_version
    namespace        TEXT NOT NULL,
    owner_id         TEXT NOT NULL,
    scope            TEXT NOT NULL,
    sensitivity      INTEGER NOT NULL CHECK (sensitivity BETWEEN 0 AND 3),
    policy_version   TEXT NOT NULL,

    -- Payload — exactly one representation non-null (see table CHECK below)
    payload_cipher   BLOB,
    payload_plain    TEXT,
    payload_encoding TEXT NOT NULL,
    payload_checksum TEXT NOT NULL,

    -- Crypto-shred references
    shred_key_id     TEXT,
    key_version      INTEGER,

    -- Schema evolution
    schema_version   INTEGER NOT NULL,

    -- Exactly one of the two payload representations must be present.
    CHECK ((payload_cipher IS NOT NULL) + (payload_plain IS NOT NULL) = 1)
);

-- Enforce append-only immutability: forbid UPDATE/DELETE on events_v2
-- (mirrors the legacy `events` L1 triggers in 0001_init.sql).
CREATE TRIGGER IF NOT EXISTS trg_events_v2_no_update
    BEFORE UPDATE ON events_v2
    BEGIN SELECT RAISE(ABORT, 'events_v2 are immutable (append-only, L1)'); END;
CREATE TRIGGER IF NOT EXISTS trg_events_v2_no_delete
    BEFORE DELETE ON events_v2
    BEGIN SELECT RAISE(ABORT, 'events_v2 are immutable (append-only, L1)'); END;
