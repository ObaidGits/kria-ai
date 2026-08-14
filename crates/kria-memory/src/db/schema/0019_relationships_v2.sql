-- Authority schema v2 — the `relationships_v2` semantic-link table (design
-- §4.2/§19.12, task F2.2.3).
--
-- Migration 0018 seeded the relation-identity registry but explicitly
-- DEFERRED the `relationships` table itself to a later F2.2 subtask. Task
-- 2.2.3 (the AuthorityTx validation gate) needs a physical table to check
-- polymorphic endpoint existence against for `EndpointKind::Relationship`
-- (a relation whose endpoint is itself another relationship claim, e.g.
-- `supports` → `relationship`), so this migration adds the table now, ahead
-- of the governed write commands (2.2.5) that will populate it.
--
-- ── Naming: v2 coexistence suffix ─────────────────────────────────────────
-- `relationships` (bare) already exists as the legacy free-text link table
-- from 0001_init.sql. This is a NEW, distinct table, so it takes the `_v2`
-- suffix, mirroring the events → events_v2 / entities → entities_v2
-- precedent from 0012/0017. The legacy table is left untouched until the
-- F2.2.7 legacy-table deletion.
--
-- ── Scope (task 2.2.3 only) ────────────────────────────────────────────────
-- Creates the table + the required active-identity uniqueness index and
-- endpoint-expansion indexes (design §19.12 representative SQL, adapted to
-- the `_v2` name). Does NOT insert any rows — governed writes are task 2.2.5.
-- `memory_links` remains out of scope (design §19.12: distinct table for the
-- five canonical cognitive lineage/evidence relations, not needed by 2.2.3's
-- endpoint-kind set).
--
-- ── Design invariants enforced here ────────────────────────────────────────
--   * `source_kind`/`target_kind` are the closed `EndpointKind` set (design
--     §4.2/§19.3). No hard FK on `source_id`/`target_id`: they are polymorphic
--     endpoints across mixed record kinds, which SQLite cannot express safely
--     (design §19.12) — AuthorityTx (this task) checks existence instead.
--   * `(relation_name, relation_version)` FKs `relation_registry` — an
--     unregistered/unversioned relation can never be stored.
--   * The half-open valid interval must not be inverted (design §4 preamble).
--   * Non-reflexivity, endpoint-kind legality, and evidence minimums are
--     relation-specific (depend on the registry row) and are therefore
--     AuthorityTx checks (task 2.2.3), never DB CHECKs (design §19.12: "checked
--     by AuthorityTx immediately before commit because polymorphic foreign
--     keys cannot express them safely").
--   * Unique **active** `identity_hash` (excluding superseded/forgotten/
--     deleted truth states) is the "same edge" key AuthorityTx (2.2.3+) and
--     Evidence append (2.2.4) rely on to distinguish a replay/additional
--     observation from a genuinely new semantic edge (design §4.2).
--
-- Canonical encodings (design §4 preamble): timestamps RFC3339 UTC TEXT,
-- booleans INTEGER, canonical lower-case UUID TEXT ids.

CREATE TABLE IF NOT EXISTS relationships_v2 (
    id                TEXT PRIMARY KEY,            -- canonical lower-case UUID text
    -- Polymorphic endpoints (design §4.2) — no hard FK; AuthorityTx checks
    -- existence in the owning table for the given kind.
    source_kind       TEXT NOT NULL CHECK (source_kind IN
                          ('entity','memory','summary','skill','rule','event',
                           'episode','goal','evidence','relationship')),
    source_id         TEXT NOT NULL,
    target_kind       TEXT NOT NULL CHECK (target_kind IN
                          ('entity','memory','summary','skill','rule','event',
                           'episode','goal','evidence','relationship')),
    target_id         TEXT NOT NULL,
    -- Relation identity (design §4.2) — FKs the versioned registry row.
    relation_name     TEXT NOT NULL,
    relation_version  INTEGER NOT NULL,
    direction_class   TEXT NOT NULL CHECK (direction_class IN ('directed','symmetric')),
    -- Valid Time (half-open [from, until)).
    valid_from        TEXT,
    valid_until       TEXT,
    truth_state       TEXT,
    authority_class   TEXT CHECK (authority_class IS NULL
                          OR authority_class IN ('stored','derived','inferred')),
    -- Policy columns (design §4.1). `policy_source_id` is the policy
    -- CONTRIBUTOR source (mirrors `records`/`entities_v2` `source_id`), kept
    -- distinct from this table's own polymorphic `source_id` endpoint column
    -- above (both named `source_id` in the design's prose; disambiguated here
    -- because a single row cannot have two same-named columns).
    namespace         TEXT NOT NULL,
    owner_id          TEXT NOT NULL,
    scope             TEXT NOT NULL,
    sensitivity       INTEGER NOT NULL CHECK (sensitivity BETWEEN 0 AND 3),
    policy_source_id  TEXT NOT NULL,
    policy_version    TEXT NOT NULL,
    -- Identity / algorithm / provenance / lineage.
    identity_hash     TEXT NOT NULL,               -- design §4.2 canonical semantic identity (task 2.2.2)
    algorithm         TEXT,
    algorithm_version TEXT,
    created_event_id  TEXT REFERENCES events_v2(id),
    revision          INTEGER,
    superseded_by     TEXT REFERENCES relationships_v2(id),
    FOREIGN KEY (relation_name, relation_version)
        REFERENCES relation_registry (relation_name, version),
    -- Half-open valid interval must not be inverted (design §4 preamble).
    CHECK (valid_from IS NULL OR valid_until IS NULL OR valid_until >= valid_from)
);

-- Unique ACTIVE semantic identity (design §4.2/§19.12 `uq_active_relationship_identity`).
CREATE UNIQUE INDEX IF NOT EXISTS uq_active_relationship_identity
    ON relationships_v2 (identity_hash)
    WHERE truth_state IS NULL OR truth_state NOT IN ('superseded','forgotten','deleted');

-- Endpoint-expansion indexes (design §19.12 `ix_relation_expand_source/target`).
CREATE INDEX IF NOT EXISTS idx_relationships_v2_expand_source
    ON relationships_v2 (source_kind, source_id, relation_name, truth_state, valid_until);
CREATE INDEX IF NOT EXISTS idx_relationships_v2_expand_target
    ON relationships_v2 (target_kind, target_id, relation_name, truth_state, valid_until);
CREATE INDEX IF NOT EXISTS idx_relationships_v2_superseded
    ON relationships_v2 (superseded_by);
CREATE INDEX IF NOT EXISTS idx_relationships_v2_policy
    ON relationships_v2 (namespace, scope, sensitivity);
