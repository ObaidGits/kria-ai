-- Authority schema v2 — cognitive records + semantic base tables (design
-- §4.2/§4.3, task F2.1.1).
--
-- This migration extends the v2 authority epoch (opened by 0011; events_v2 in
-- 0012; revisions/idempotency/audit in 0013; outbox/manifests/shred/sources in
-- 0014; indexes in 0015; declassifications in 0016) with the typed cognitive
-- record model and its supporting semantic/observation tables.
--
-- Scope (task 2.1.1 only): the record/entity/alias/mention/evidence tables
-- (§4.2 minus the relation registry / relationships / memory_links /
-- entity_resolution_* tables, which are F2.2 — task 2.2), plus episodes, goals
-- + goal_progress, consolidation_runs, tool_observations, retrieval_traces +
-- retrieval_trace_items, and feedback (§4.3), plus the deferred `sources`
-- indexes. Provenance encoding (2.1.2), content-hash/token/staleness/truth/
-- lifecycle wiring (2.1.3), unknown-field preservation (2.1.4), round-trip
-- property tests (2.1.5), and legacy-struct removal (2.1.6) are out of scope.
--
-- ── Naming: v2 coexistence suffix ────────────────────────────────────────
-- Four target names collide with legacy tables from 0001_init.sql/0006_goals:
-- `episodes`, `goals`, `entities`, `evidence`. A plain `CREATE TABLE IF NOT
-- EXISTS <name>` would silently no-op against the legacy schema, so those four
-- take the `_v2` suffix — `episodes_v2`, `goals_v2`, `entities_v2`,
-- `evidence_v2` — mirroring the established events → events_v2 / shred_keys →
-- shred_keys_v2 precedent. The legacy tables are left untouched and are dropped
-- at the F1.5 writer cutover / F2.1.6 legacy-struct removal. The remaining
-- design names are collision-free and used verbatim: `records`, `aliases`,
-- `mentions`, `goal_progress`, `consolidation_runs`, `tool_observations`,
-- `retrieval_traces`, `retrieval_trace_items`, `feedback`. `sources` already
-- exists as the v2 base row (0014); this migration only adds its deferred
-- identity/version/policy/lifecycle indexes.
--
-- ── FK ordering (foreign_keys=ON, see configure()) ───────────────────────
-- FK targets must precede referrers. `events_v2` exists from 0012. Tables are
-- created in dependency order: episodes_v2 → goals_v2 → entities_v2 → records
-- → aliases → mentions → evidence_v2 → goal_progress → consolidation_runs →
-- tool_observations → retrieval_traces → retrieval_trace_items → feedback.
-- `records.superseded_by` and `entities_v2.canonical_id` are self-FKs that
-- resolve within their own CREATE. Polymorphic endpoints across mixed record
-- kinds (mentions.record_id, evidence_v2.subject_id, feedback.target_id,
-- retrieval_trace_items.record_id) carry NO hard FK — SQLite cannot express a
-- polymorphic FK; WritePolicyEngine/AuthorityTx enforce endpoint existence
-- (design §4.2). `retrieval_traces.graph_revision` is a plain INTEGER (no hard
-- FK to graph_revisions), matching the outbox/manifest revision-reference
-- convention from 0014.
--
-- ── Policy columns (design §4.1) ─────────────────────────────────────────
-- namespace/owner_id/scope/sensitivity(0..3)/source_id/policy_version, matching
-- events_v2 (0012) / sources (0014). Cognitive-record tables include source_id
-- as a policy contributor.
--
-- Canonical encodings (design §4 preamble):
--   * UUIDs      → canonical lower-case TEXT
--   * timestamps → RFC3339 UTC TEXT (half-open valid intervals [from, until))
--   * booleans   → INTEGER CHECK (... IN (0,1))
--   * JSON       → TEXT with `json_valid` guard (JSON1 is bundled)

-- ── Episodes (design §4.3) ───────────────────────────────────────────────
-- A bounded span of session/task activity. `cursor_event_id` FKs events_v2.
CREATE TABLE IF NOT EXISTS episodes_v2 (
    id              TEXT PRIMARY KEY,           -- canonical lower-case UUID text
    session_id      TEXT,
    task_id         TEXT,
    -- Policy columns (design §4.1)
    namespace       TEXT NOT NULL,
    owner_id        TEXT NOT NULL,
    scope           TEXT NOT NULL,
    sensitivity     INTEGER NOT NULL CHECK (sensitivity BETWEEN 0 AND 3),
    source_id       TEXT NOT NULL,
    policy_version  TEXT NOT NULL,
    -- Lifecycle
    opened_at       TEXT,
    closed_at       TEXT,
    boundary_reason TEXT,
    cursor_event_id TEXT REFERENCES events_v2(id),
    truth_state     TEXT,
    revision        INTEGER,
    CHECK (closed_at IS NULL OR opened_at IS NULL OR closed_at >= opened_at)
);

CREATE INDEX IF NOT EXISTS idx_episodes_v2_session ON episodes_v2 (session_id);
CREATE INDEX IF NOT EXISTS idx_episodes_v2_task    ON episodes_v2 (task_id);
CREATE INDEX IF NOT EXISTS idx_episodes_v2_time    ON episodes_v2 (opened_at, closed_at);

-- ── Goals (design §4.3) ──────────────────────────────────────────────────
-- Status is a closed set (schema CHECK); priority is 0..10.
CREATE TABLE IF NOT EXISTS goals_v2 (
    id                 TEXT PRIMARY KEY,
    kind               TEXT,
    title              TEXT,
    status             TEXT CHECK (status IN
                           ('candidate','active','paused','completed',
                            'conflicted','stale','superseded','deleted')),
    priority           INTEGER CHECK (priority BETWEEN 0 AND 10),
    score              REAL,
    score_semantics    TEXT,
    resumption_context TEXT,
    -- Policy columns (design §4.1)
    namespace          TEXT NOT NULL,
    owner_id           TEXT NOT NULL,
    scope              TEXT NOT NULL,
    sensitivity        INTEGER NOT NULL CHECK (sensitivity BETWEEN 0 AND 3),
    source_id          TEXT NOT NULL,
    policy_version     TEXT NOT NULL,
    -- Provenance / event / times
    created_event_id   TEXT REFERENCES events_v2(id),
    created_at         TEXT,
    updated_at         TEXT,
    revision           INTEGER
);

CREATE INDEX IF NOT EXISTS idx_goals_v2_status   ON goals_v2 (status);
CREATE INDEX IF NOT EXISTS idx_goals_v2_priority ON goals_v2 (priority);
CREATE INDEX IF NOT EXISTS idx_goals_v2_policy   ON goals_v2 (namespace, scope, sensitivity);

-- ── Entities (design §4.2) ───────────────────────────────────────────────
-- `canonical_id` self-FK links an alias entity to its canonical form.
CREATE TABLE IF NOT EXISTS entities_v2 (
    id               TEXT PRIMARY KEY,
    canonical_id     TEXT REFERENCES entities_v2(id),
    entity_type      TEXT,
    display_name     TEXT,
    normalized_name  TEXT,                       -- normalized display name (index target)
    truth_state      TEXT,
    -- Policy columns (design §4.1)
    namespace        TEXT NOT NULL,
    owner_id         TEXT NOT NULL,
    scope            TEXT NOT NULL,
    sensitivity      INTEGER NOT NULL CHECK (sensitivity BETWEEN 0 AND 3),
    source_id        TEXT NOT NULL,
    policy_version   TEXT NOT NULL,
    -- Provenance / event / time / revision
    created_event_id TEXT NOT NULL REFERENCES events_v2(id),
    created_at       TEXT NOT NULL,
    revision         INTEGER
);

CREATE INDEX IF NOT EXISTS idx_entities_v2_canonical  ON entities_v2 (canonical_id);
CREATE INDEX IF NOT EXISTS idx_entities_v2_type       ON entities_v2 (entity_type);
CREATE INDEX IF NOT EXISTS idx_entities_v2_policy     ON entities_v2 (namespace, scope, sensitivity);
CREATE INDEX IF NOT EXISTS idx_entities_v2_normalized ON entities_v2 (normalized_name);

-- ── Records (design §4.2) ────────────────────────────────────────────────
-- The core cognitive record. record_kind is a closed set; exactly one payload
-- representation (content | content_cipher) is present; the valid interval is
-- half-open and non-inverted; estimated_tokens is non-negative.
CREATE TABLE IF NOT EXISTS records (
    id               TEXT PRIMARY KEY,
    record_kind      TEXT NOT NULL CHECK (record_kind IN ('memory','summary','skill','rule')),
    schema_version   INTEGER NOT NULL,
    content          TEXT,
    content_cipher   BLOB,
    content_hash     TEXT,
    truth_state      TEXT,
    staleness_class  TEXT,
    valid_from       TEXT,
    valid_until      TEXT,
    -- Policy columns (design §4.1)
    namespace        TEXT NOT NULL,
    owner_id         TEXT NOT NULL,
    scope            TEXT NOT NULL,
    sensitivity      INTEGER NOT NULL CHECK (sensitivity BETWEEN 0 AND 3),
    source_id        TEXT NOT NULL,
    policy_version   TEXT NOT NULL,
    -- Provenance / lifecycle / lineage
    created_event_id TEXT NOT NULL REFERENCES events_v2(id),
    created_at       TEXT NOT NULL,
    superseded_by    TEXT REFERENCES records(id),
    episode_id       TEXT REFERENCES episodes_v2(id),
    goal_context_id  TEXT REFERENCES goals_v2(id),
    estimated_tokens INTEGER CHECK (estimated_tokens IS NULL OR estimated_tokens >= 0),
    shred_key_id     TEXT,
    key_version      INTEGER,
    -- Exactly one payload representation must be present (design §4.2).
    CHECK ((content IS NOT NULL) + (content_cipher IS NOT NULL) = 1),
    -- Half-open valid interval must not be inverted (design §4 preamble).
    CHECK (valid_from IS NULL OR valid_until IS NULL OR valid_until >= valid_from)
);

CREATE INDEX IF NOT EXISTS idx_records_kind_state   ON records (record_kind, truth_state);
CREATE INDEX IF NOT EXISTS idx_records_policy       ON records (namespace, scope, sensitivity);
CREATE INDEX IF NOT EXISTS idx_records_content_hash ON records (content_hash);
CREATE INDEX IF NOT EXISTS idx_records_validity     ON records (valid_from, valid_until);
CREATE INDEX IF NOT EXISTS idx_records_superseded   ON records (superseded_by);
CREATE INDEX IF NOT EXISTS idx_records_episode      ON records (episode_id);
CREATE INDEX IF NOT EXISTS idx_records_goal         ON records (goal_context_id);

-- ── Aliases (design §4.2) ────────────────────────────────────────────────
-- An alternative surface form for an entity. Unique active identity is
-- (normalized_alias, alias_type, namespace, scope).
CREATE TABLE IF NOT EXISTS aliases (
    id               TEXT PRIMARY KEY,
    entity_id        TEXT NOT NULL REFERENCES entities_v2(id),
    alias            TEXT NOT NULL,
    normalized_alias TEXT NOT NULL,
    alias_type       TEXT NOT NULL,
    truth_state      TEXT,
    -- Policy columns (design §4.1)
    namespace        TEXT NOT NULL,
    owner_id         TEXT NOT NULL,
    scope            TEXT NOT NULL,
    sensitivity      INTEGER NOT NULL CHECK (sensitivity BETWEEN 0 AND 3),
    source_id        TEXT NOT NULL,
    policy_version   TEXT NOT NULL,
    -- Provenance fields
    created_event_id TEXT REFERENCES events_v2(id),
    created_at       TEXT,
    valid_from       TEXT,
    valid_until      TEXT,
    CHECK (valid_from IS NULL OR valid_until IS NULL OR valid_until >= valid_from)
);

-- Required uniqueness key (design §4.2).
CREATE UNIQUE INDEX IF NOT EXISTS idx_aliases_identity
    ON aliases (normalized_alias, alias_type, namespace, scope);
CREATE INDEX IF NOT EXISTS idx_aliases_entity ON aliases (entity_id);

-- ── Mentions (design §4.2) ───────────────────────────────────────────────
-- A provenance-bearing link from a source span/locator to an entity.
-- record_id/record_kind are a polymorphic endpoint (no hard FK).
CREATE TABLE IF NOT EXISTS mentions (
    id                TEXT PRIMARY KEY,
    record_id         TEXT,
    record_kind       TEXT,
    entity_id         TEXT NOT NULL REFERENCES entities_v2(id),
    locator_json      TEXT CHECK (locator_json IS NULL OR json_valid(locator_json)),
    span_start        INTEGER,
    span_end          INTEGER,
    role              TEXT,
    extractor         TEXT,
    extractor_version TEXT,
    score             REAL,
    score_semantics   TEXT,
    -- Policy columns (design §4.1)
    namespace         TEXT NOT NULL,
    owner_id          TEXT NOT NULL,
    scope             TEXT NOT NULL,
    sensitivity       INTEGER NOT NULL CHECK (sensitivity BETWEEN 0 AND 3),
    source_id         TEXT NOT NULL,
    policy_version    TEXT NOT NULL,
    observed_at       TEXT,
    created_event_id  TEXT REFERENCES events_v2(id),
    -- Span order (design §4.2 "span order check").
    CHECK (span_start IS NULL OR span_end IS NULL OR span_end >= span_start)
);

CREATE INDEX IF NOT EXISTS idx_mentions_record ON mentions (record_kind, record_id);
CREATE INDEX IF NOT EXISTS idx_mentions_entity ON mentions (entity_id);
CREATE INDEX IF NOT EXISTS idx_mentions_policy ON mentions (namespace, scope, sensitivity);

-- ── Evidence (design §4.2) ───────────────────────────────────────────────
-- A supporting/contradicting observation about a subject. polarity is a closed
-- set. subject/source_record are polymorphic endpoints (no hard FK).
CREATE TABLE IF NOT EXISTS evidence_v2 (
    id                 TEXT PRIMARY KEY,
    subject_kind       TEXT,
    subject_id         TEXT,
    source_record_kind TEXT,
    source_record_id   TEXT,
    source_event_id    TEXT REFERENCES events_v2(id),
    locator_json       TEXT CHECK (locator_json IS NULL OR json_valid(locator_json)),
    actor_id           TEXT,
    method             TEXT,
    method_version     TEXT,
    polarity           TEXT CHECK (polarity IN ('supports','contradicts')),
    score              REAL,
    score_semantics    TEXT,
    -- Policy columns (design §4.1)
    namespace          TEXT NOT NULL,
    owner_id           TEXT NOT NULL,
    scope              TEXT NOT NULL,
    sensitivity        INTEGER NOT NULL CHECK (sensitivity BETWEEN 0 AND 3),
    source_id          TEXT NOT NULL,
    policy_version     TEXT NOT NULL,
    observed_at        TEXT,
    removed_at         TEXT,
    created_event_id   TEXT REFERENCES events_v2(id)
);

CREATE INDEX IF NOT EXISTS idx_evidence_v2_subject  ON evidence_v2 (subject_kind, subject_id);
CREATE INDEX IF NOT EXISTS idx_evidence_v2_source   ON evidence_v2 (source_record_kind, source_record_id);
CREATE INDEX IF NOT EXISTS idx_evidence_v2_polarity ON evidence_v2 (polarity);
CREATE INDEX IF NOT EXISTS idx_evidence_v2_policy   ON evidence_v2 (namespace, scope, sensitivity);

-- ── Goal progress (design §4.3, append-only) ─────────────────────────────
-- Immutable progress observations against a goal.
CREATE TABLE IF NOT EXISTS goal_progress (
    id          TEXT PRIMARY KEY,
    goal_id     TEXT NOT NULL REFERENCES goals_v2(id),
    event_id    TEXT REFERENCES events_v2(id),
    state       TEXT,
    summary     TEXT,
    observed_at TEXT,
    revision    INTEGER
);

CREATE INDEX IF NOT EXISTS idx_goal_progress_goal ON goal_progress (goal_id, observed_at);

CREATE TRIGGER IF NOT EXISTS trg_goal_progress_no_update
    BEFORE UPDATE ON goal_progress
    BEGIN SELECT RAISE(ABORT, 'goal_progress is append-only (immutable, L1)'); END;
CREATE TRIGGER IF NOT EXISTS trg_goal_progress_no_delete
    BEFORE DELETE ON goal_progress
    BEGIN SELECT RAISE(ABORT, 'goal_progress is append-only (immutable, L1)'); END;

-- ── Consolidation runs (design §4.3) ─────────────────────────────────────
-- level is a closed set. A run is uniquely identified by
-- (algorithm, version, input_set_hash, level).
CREATE TABLE IF NOT EXISTS consolidation_runs (
    id             TEXT PRIMARY KEY,
    algorithm      TEXT NOT NULL,
    version        TEXT NOT NULL,
    input_set_hash TEXT NOT NULL,
    level          TEXT CHECK (level IN ('episode','summary','skill','rule')),
    cursor         TEXT,
    status         TEXT,
    started_at     TEXT,
    completed_at   TEXT,
    output_id      TEXT,
    error_code     TEXT
);

CREATE UNIQUE INDEX IF NOT EXISTS idx_consolidation_runs_identity
    ON consolidation_runs (algorithm, version, input_set_hash, level);

-- ── Sources: deferred indexes (design §4.3) ──────────────────────────────
-- The `sources` table itself was created in 0014 (base fields). Its
-- identity/version/policy/lifecycle indexes were deferred to F2 and are added
-- here.
CREATE INDEX IF NOT EXISTS idx_sources_identity  ON sources (source_kind, external_identity);
CREATE INDEX IF NOT EXISTS idx_sources_version   ON sources (version);
CREATE INDEX IF NOT EXISTS idx_sources_policy    ON sources (namespace, scope, sensitivity);
CREATE INDEX IF NOT EXISTS idx_sources_lifecycle ON sources (lifecycle_state);

-- ── Tool observations (design §4.3) ──────────────────────────────────────
-- A start/completion-linked outcome record for a tool invocation. NEVER an
-- authorization grant (glossary). Unique per invocation completion.
CREATE TABLE IF NOT EXISTS tool_observations (
    id                  TEXT PRIMARY KEY,
    invocation_id       TEXT NOT NULL,
    tool_kind           TEXT,
    tool_id             TEXT,
    tool_version        TEXT,
    capability_id       TEXT,
    outcome             TEXT,
    goal_id             TEXT REFERENCES goals_v2(id),
    environment_class   TEXT,
    input_fingerprint   TEXT,
    result_summary      TEXT,
    error_class         TEXT,
    latency_ms          INTEGER,
    retry_count         INTEGER,
    recovery_action     TEXT,
    -- Policy columns (design §4.1)
    namespace           TEXT NOT NULL,
    owner_id            TEXT NOT NULL,
    scope               TEXT NOT NULL,
    sensitivity         INTEGER NOT NULL CHECK (sensitivity BETWEEN 0 AND 3),
    source_id           TEXT NOT NULL,
    policy_version      TEXT NOT NULL,
    start_event_id      TEXT REFERENCES events_v2(id),
    completion_event_id TEXT REFERENCES events_v2(id),
    created_at          TEXT
);

-- Unique invocation completion (design §4.3).
CREATE UNIQUE INDEX IF NOT EXISTS idx_tool_observations_invocation
    ON tool_observations (invocation_id);
CREATE INDEX IF NOT EXISTS idx_tool_observations_tool
    ON tool_observations (tool_id, tool_version, outcome);
CREATE INDEX IF NOT EXISTS idx_tool_observations_window
    ON tool_observations (created_at);

-- ── Retrieval traces (design §4.3) ───────────────────────────────────────
-- The provenance of one retrieval response. graph_revision is a plain INTEGER
-- (no hard FK — matches outbox/manifest revision-reference convention).
CREATE TABLE IF NOT EXISTS retrieval_traces (
    id                   TEXT PRIMARY KEY,
    response_id          TEXT,
    task_id              TEXT,
    query_hash           TEXT,
    query_class          TEXT,
    classifier_version   TEXT,
    profile_id           TEXT,
    graph_revision       INTEGER,
    policy_hash          TEXT,
    token_budget         INTEGER,
    status               TEXT,
    degradation_json     TEXT CHECK (degradation_json IS NULL OR json_valid(degradation_json)),
    embed_model_version  TEXT,
    rerank_model_version TEXT,
    created_at           TEXT
);

CREATE INDEX IF NOT EXISTS idx_retrieval_traces_response ON retrieval_traces (response_id);
CREATE INDEX IF NOT EXISTS idx_retrieval_traces_task     ON retrieval_traces (task_id);
CREATE INDEX IF NOT EXISTS idx_retrieval_traces_revision ON retrieval_traces (graph_revision);
CREATE INDEX IF NOT EXISTS idx_retrieval_traces_policy   ON retrieval_traces (policy_hash);

-- ── Retrieval trace items (design §4.3) ──────────────────────────────────
-- One candidate row per (trace, record, strategy). Unauthorized items use
-- opaque reason rows without hidden record IDs (design §4.3) — that redaction
-- is a write-path concern; the schema anchors the PK it relies on.
CREATE TABLE IF NOT EXISTS retrieval_trace_items (
    trace_id         TEXT NOT NULL REFERENCES retrieval_traces(id),
    record_id        TEXT NOT NULL,
    strategy         TEXT NOT NULL,
    strategy_rank    INTEGER,
    strategy_score   REAL,
    weight           REAL,
    rrf_contribution REAL,
    gate_disposition TEXT,
    reason_code      TEXT,
    token_cost       INTEGER,
    allocated_tokens INTEGER,
    injected_order   INTEGER,
    goal_id          TEXT REFERENCES goals_v2(id),
    PRIMARY KEY (trace_id, record_id, strategy)
);

CREATE INDEX IF NOT EXISTS idx_retrieval_trace_items_disposition
    ON retrieval_trace_items (trace_id, gate_disposition);

-- ── Feedback (design §4.3) ───────────────────────────────────────────────
-- A signal about a target record/link/etc. target_kind/target_id is a
-- polymorphic endpoint (no hard FK).
CREATE TABLE IF NOT EXISTS feedback (
    id             TEXT PRIMARY KEY,
    target_kind    TEXT,
    target_id      TEXT,
    signal         TEXT,
    payload_json   TEXT CHECK (payload_json IS NULL OR json_valid(payload_json)),
    -- Policy columns (design §4.1)
    namespace      TEXT NOT NULL,
    owner_id       TEXT NOT NULL,
    scope          TEXT NOT NULL,
    sensitivity    INTEGER NOT NULL CHECK (sensitivity BETWEEN 0 AND 3),
    source_id      TEXT NOT NULL,
    policy_version TEXT NOT NULL,
    -- Actor / event / time / revision
    actor_id       TEXT,
    event_id       TEXT REFERENCES events_v2(id),
    created_at     TEXT,
    revision       INTEGER
);

CREATE INDEX IF NOT EXISTS idx_feedback_target ON feedback (target_kind, target_id);
CREATE INDEX IF NOT EXISTS idx_feedback_time   ON feedback (created_at);
CREATE INDEX IF NOT EXISTS idx_feedback_policy ON feedback (namespace, scope, sensitivity);
