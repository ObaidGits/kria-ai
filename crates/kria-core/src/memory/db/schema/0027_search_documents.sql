-- F3.2 / task 3.2.1: search_documents projection table.
--
-- `search_documents` is the REBUILDABLE authority-derived projection that the
-- FTS5 external-content table (task 3.2.2) indexes over (design §4.4).  It
-- holds one row per searchable item — memories, summaries, skills, rules,
-- entity names/aliases, source records, goals, and relationship labels — with
-- all policy, truth, valid-time, content-hash, and revision columns required
-- for policy preselection and dedup (design §16, MGD-015/MGD-025).
--
-- DESIGN INVARIANTS
-- ─────────────────
-- * This table is a DERIVED PROJECTION (L4 / §A8 Rebuildability): it may be
--   truncated and rebuilt at any time.  No authority data originates here.
-- * Policy columns (namespace, owner_id, scope, sensitivity) are present on
--   EVERY row and are the primary gating dimensions for FTS5 queries.
-- * Deleted/Forgotten records remain with their truth_state intact so that
--   FTS5 queries can exclude them via the truth_state column without needing
--   to fully rebuild the projection.
-- * `content_hash` is the SHA-256 of the source record's searchable content
--   (not the raw authority payload) — used for dedup/rebuild comparison.
-- * `revision` tracks which graph_revision was current when the projection row
--   was written, enabling cursor-based incremental rebuilds.
-- * PRIMARY KEY (record_kind, record_id) ensures exactly one projection row
--   per searchable item, and ON CONFLICT DO UPDATE upserts replace it cleanly.
-- * `record_kind` covers all searchable categories: memory/summary/skill/rule
--   (cognitive records), entity (entities_v2 + aliases), source (sources),
--   goal (goals_v2), relationship (relationships_v2).
-- * `aliases`, `source_text`, and `relation_text` are NULL for record kinds
--   that do not carry those fields — FTS5 external-content tokenizes NULL as
--   empty and does not error.
--
-- POLICY / SCOPE NOTE (MGR-004, MGR-025)
-- Columns are populated only from authority rows the caller is authorized to
-- read.  The projection builder (sqlite_search_documents.rs) must never copy
-- content across scope/namespace boundaries; policy_source_id is intentionally
-- NOT stored here (the full policy provenance is in the authority tables).
--
-- Canonical encodings (design §4 preamble):
--   * UUIDs       → canonical lower-case TEXT
--   * timestamps  → RFC3339 UTC TEXT (valid_from / valid_until are nullable)
--   * sensitivity → INTEGER 0..3

CREATE TABLE IF NOT EXISTS search_documents (
    -- Identity
    record_kind     TEXT    NOT NULL,   -- 'memory'|'summary'|'skill'|'rule'|'entity'|'source'|'goal'|'relationship'
    record_id       TEXT    NOT NULL,   -- stable UUID (canonical lower-case text)
    -- Searchable fields
    title           TEXT,               -- primary searchable name / label
    body            TEXT,               -- main searchable content / description
    aliases         TEXT,               -- space/comma-joined aliases (entities)
    source_text     TEXT,               -- source name/description/kind (sources)
    relation_text   TEXT,               -- relation label(s) (relationships)
    -- Policy columns (design §4.1)
    namespace       TEXT    NOT NULL,
    owner_id        TEXT    NOT NULL,
    scope           TEXT    NOT NULL,
    sensitivity     INTEGER NOT NULL CHECK (sensitivity >= 0 AND sensitivity <= 3),
    -- Truth / time
    truth_state     TEXT    NOT NULL,   -- 'Current'|'Stale'|'Unverified'|'Superseded'|'Forgotten'|'Deleted'
    valid_from      TEXT,               -- RFC3339 UTC or NULL
    valid_until     TEXT,               -- RFC3339 UTC or NULL
    -- Projection integrity
    content_hash    TEXT    NOT NULL,   -- SHA-256 of the source record's searchable content
    revision        INTEGER NOT NULL CHECK (revision >= 0),
    -- Composite primary key (design §4.4: one row per searchable item)
    PRIMARY KEY (record_kind, record_id),
    -- Valid time must not be inverted (design §4 preamble).
    CHECK (valid_from IS NULL OR valid_until IS NULL OR valid_until >= valid_from)
);

-- ── Policy prefilter index ────────────────────────────────────────────────────
-- Supports the FTS5 preselection query pattern:
--   WHERE namespace = ? AND scope = ? AND sensitivity <= ? AND truth_state = ?
-- (design §16, MGD-025 "policy/truth/time preselection").
CREATE INDEX IF NOT EXISTS ix_sd_policy
    ON search_documents (namespace, scope, sensitivity, truth_state);

-- ── Rebuild cursor index ──────────────────────────────────────────────────────
-- Supports incremental rebuild scans that advance a cursor over
--   (revision, record_kind)
-- to identify rows that need re-projection after a graph revision bump.
CREATE INDEX IF NOT EXISTS ix_sd_revision
    ON search_documents (revision, record_kind);

-- ── Dedup / rebuild comparison index ─────────────────────────────────────────
-- Fast lookup to compare the stored `content_hash` against a freshly computed
-- one before deciding whether to write an updated projection row.
-- Matches the dedup pattern used by mem_vectors_v2 (0025).
CREATE INDEX IF NOT EXISTS ix_sd_content_hash
    ON search_documents (record_kind, record_id, content_hash);
