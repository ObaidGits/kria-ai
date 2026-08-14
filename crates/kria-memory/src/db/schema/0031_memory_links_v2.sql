-- Authority schema v2 — `memory_links` table for the five canonical cognitive
-- lineage and evidence relations (design §4.2/§7.3, task F3.6.4).
--
-- This migration delivers the `memory_links` table that migrations 0017–0019
-- explicitly deferred to a later F2.2 subtask. The `derived_from` canonical
-- relation (seeded in 0018) is the first relation authored here; `derive_record`
-- (task F3.6.4) writes a `derived_from` link from each parent record to the
-- newly derived record immediately after insertion.
--
-- ── Design invariants enforced here ──────────────────────────────────────
--   * `(link_type, link_version)` FKs `relation_registry` — only registered,
--     versioned relation names are permitted (design §4.2: "no parallel
--     untyped link table is permitted").
--   * Polymorphic source/target kinds are a closed CHECK set (design §4.2).
--   * The half-open valid interval must not be inverted (design §4 preamble).
--   * Unique ACTIVE semantic identity uses `(source_kind, source_id,
--     target_kind, target_id, link_type, link_version)` excluding superseded,
--     forgotten, and deleted truth states — the same "same edge" uniqueness
--     pattern as `relationships_v2` (design §4.2/§19.12).
--   * Endpoint existence (source_id / target_id) is NOT enforced via hard FK
--     because endpoints are polymorphic across mixed record kinds; AuthorityTx
--     (or the `derive_record` caller path) enforces existence instead.
--
-- Canonical encodings: timestamps RFC3339 UTC TEXT, booleans INTEGER,
-- canonical lower-case UUID TEXT ids.

CREATE TABLE IF NOT EXISTS memory_links (
    id              TEXT PRIMARY KEY,           -- canonical lower-case UUID text
    -- Source endpoint (polymorphic — no hard FK; caller checks existence).
    source_kind     TEXT NOT NULL CHECK (source_kind IN (
                        'event','memory','summary','skill','rule',
                        'episode','goal','evidence','relationship')),
    source_id       TEXT NOT NULL,
    -- Target endpoint (polymorphic — no hard FK).
    target_kind     TEXT NOT NULL CHECK (target_kind IN (
                        'event','memory','summary','skill','rule',
                        'episode','goal','evidence','relationship')),
    target_id       TEXT NOT NULL,
    -- Relation identity: FKs the versioned registry row (design §4.2).
    link_type       TEXT NOT NULL,
    link_version    INTEGER NOT NULL DEFAULT 1,
    truth_state     TEXT,
    -- Valid time (half-open [from, until)).
    valid_from      TEXT,
    valid_until     TEXT,
    -- Policy columns (design §4.1).
    namespace       TEXT NOT NULL,
    owner_id        TEXT NOT NULL,
    scope           TEXT NOT NULL,
    sensitivity     INTEGER NOT NULL CHECK (sensitivity BETWEEN 0 AND 3),
    source_policy_id TEXT NOT NULL,             -- policy-contributor source_id (distinct from source_id endpoint above)
    policy_version  TEXT NOT NULL,
    -- Provenance.
    created_event_id TEXT REFERENCES events_v2(id),
    revision        INTEGER,
    FOREIGN KEY (link_type, link_version)
        REFERENCES relation_registry (relation_name, version),
    CHECK (valid_from IS NULL OR valid_until IS NULL OR valid_until >= valid_from)
);

-- Unique ACTIVE semantic identity (excludes superseded/forgotten/deleted).
CREATE UNIQUE INDEX IF NOT EXISTS uq_active_memory_link_identity
    ON memory_links (source_kind, source_id, target_kind, target_id, link_type, link_version)
    WHERE truth_state IS NULL OR truth_state NOT IN ('superseded','forgotten','deleted');

-- Endpoint expansion indexes.
CREATE INDEX IF NOT EXISTS idx_memory_links_source
    ON memory_links (source_kind, source_id, link_type, truth_state);
CREATE INDEX IF NOT EXISTS idx_memory_links_target
    ON memory_links (target_kind, target_id, link_type, truth_state);
CREATE INDEX IF NOT EXISTS idx_memory_links_policy
    ON memory_links (namespace, scope, sensitivity);
