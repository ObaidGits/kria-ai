-- Cognitive Memory System — initial schema (memory-upgrade design §14).
-- SQLite is the sole transactional authority (invariant L2). Every other store
-- (vectors, FTS beyond this file) is a derived, rebuildable index (L4).
--
-- Representation notes (implementation choice, not an architecture change):
--   * IDs are stored as TEXT (hyphenated UUID v7) for debuggability; ordering
--     uses the `hlc` column, never the id (design §14 / N10).
--   * Timestamps are TEXT ISO-8601 UTC. `hlc` is the sortable HLC encoding.

-- ── Schema versioning (additive-only migrations, architecture Issue 18) ──
CREATE TABLE IF NOT EXISTS schema_version (
    version    INTEGER PRIMARY KEY,
    applied_at TEXT NOT NULL,
    checksum   TEXT NOT NULL
);

-- ── Crypto-shred keyring (L9). Minimal here; managed in task 23. ──
CREATE TABLE IF NOT EXISTS shred_keys (
    subject_id   TEXT PRIMARY KEY,
    subject_type TEXT NOT NULL,
    key_ref      TEXT NOT NULL,
    status       TEXT NOT NULL DEFAULT 'active',
    created_at   TEXT NOT NULL,
    destroyed_at TEXT
);

-- ── Event log (immutable, append-only — L1) ──
CREATE TABLE IF NOT EXISTS events (
    id              TEXT PRIMARY KEY,
    hlc             TEXT NOT NULL,
    ts_utc          TEXT NOT NULL,
    tz_offset_min   INTEGER NOT NULL,
    event_type      TEXT NOT NULL,
    source          TEXT NOT NULL,
    session_id      TEXT,
    parent_event_id TEXT REFERENCES events(id),
    shred_key_id    TEXT REFERENCES shred_keys(subject_id),
    payload         TEXT NOT NULL,
    encrypted       INTEGER NOT NULL DEFAULT 0,
    checksum        TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS ix_events_session_hlc ON events(session_id, hlc);
CREATE INDEX IF NOT EXISTS ix_events_type_hlc    ON events(event_type, hlc);
CREATE INDEX IF NOT EXISTS ix_events_hlc          ON events(hlc);
CREATE INDEX IF NOT EXISTS ix_events_shred        ON events(shred_key_id);

-- Enforce append-only (L1): forbid UPDATE/DELETE on events.
CREATE TRIGGER IF NOT EXISTS trg_events_no_update
    BEFORE UPDATE ON events
    BEGIN SELECT RAISE(ABORT, 'events are immutable (L1)'); END;
CREATE TRIGGER IF NOT EXISTS trg_events_no_delete
    BEFORE DELETE ON events
    BEGIN SELECT RAISE(ABORT, 'events are immutable (L1)'); END;

-- Per-consumer durable cursors (resumable pull, Issue 28).
CREATE TABLE IF NOT EXISTS event_consumer_cursor (
    consumer TEXT PRIMARY KEY,
    last_hlc TEXT NOT NULL DEFAULT ''
);

-- ── Sessions / episodes / goals ──
CREATE TABLE IF NOT EXISTS sessions (
    id         TEXT PRIMARY KEY,
    started_at TEXT NOT NULL,
    ended_at   TEXT,
    mode       TEXT NOT NULL DEFAULT 'permanent',
    state      TEXT NOT NULL DEFAULT 'open',
    device_id  TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS episodes (
    id                TEXT PRIMARY KEY,
    session_id        TEXT NOT NULL REFERENCES sessions(id),
    opened_at         TEXT NOT NULL,
    closed_at         TEXT,
    summary_memory_id TEXT,
    boundary_reason   TEXT
);
CREATE INDEX IF NOT EXISTS ix_episodes_session ON episodes(session_id);
CREATE TABLE IF NOT EXISTS goals (
    id                 TEXT PRIMARY KEY,
    kind               TEXT NOT NULL,
    title              TEXT NOT NULL,
    status             TEXT NOT NULL DEFAULT 'candidate',
    confidence         REAL NOT NULL DEFAULT 0.4,
    priority           INTEGER NOT NULL DEFAULT 5,
    resumption_context TEXT,
    created_at         TEXT NOT NULL,
    last_progress_at   TEXT
);

-- ── Memories (derived, durable, mutable — L4) ──
CREATE TABLE IF NOT EXISTS memories (
    id                      TEXT PRIMARY KEY,
    content                 TEXT NOT NULL,
    memory_type             TEXT NOT NULL,
    compression_level       INTEGER NOT NULL DEFAULT 0 CHECK(compression_level BETWEEN 0 AND 3),
    source_event_id         TEXT NOT NULL REFERENCES events(id),
    namespace               TEXT NOT NULL DEFAULT 'core',
    owner_id                TEXT NOT NULL DEFAULT 'user',
    device_id               TEXT NOT NULL,
    scope                   TEXT NOT NULL DEFAULT 'global',
    confidence              REAL NOT NULL DEFAULT 0.5 CHECK(confidence BETWEEN 0 AND 1),
    importance              REAL NOT NULL DEFAULT 5.0 CHECK(importance BETWEEN 0 AND 10),
    access_count            INTEGER NOT NULL DEFAULT 0,
    decay_score             REAL NOT NULL DEFAULT 1.0,
    staleness_class         TEXT NOT NULL DEFAULT 'slow',
    sensitivity             TEXT NOT NULL DEFAULT 'private',
    state                   TEXT NOT NULL DEFAULT 'active',
    created_at              TEXT NOT NULL,
    last_accessed           TEXT,
    valid_from              TEXT NOT NULL,
    valid_until             TEXT,
    embedding_id            TEXT,
    embedding_model_version TEXT,
    estimated_tokens        INTEGER NOT NULL DEFAULT 0,
    content_hash            TEXT NOT NULL,
    shred_key_id            TEXT REFERENCES shred_keys(subject_id),
    verify_against          TEXT,
    superseded_by           TEXT REFERENCES memories(id),
    episode_id              TEXT REFERENCES episodes(id),
    goal_context_id         TEXT REFERENCES goals(id),
    memory_worth_success    INTEGER NOT NULL DEFAULT 0,
    memory_worth_failure    INTEGER NOT NULL DEFAULT 0,
    memory_worth_samples    INTEGER NOT NULL DEFAULT 0,
    modality                TEXT NOT NULL DEFAULT 'text',
    preference_pair_id      TEXT,
    training_eligible       INTEGER NOT NULL DEFAULT 0
);
CREATE INDEX IF NOT EXISTS ix_mem_type_state  ON memories(memory_type, state);
CREATE INDEX IF NOT EXISTS ix_mem_ns_scope    ON memories(namespace, scope, state);
CREATE INDEX IF NOT EXISTS ix_mem_staleness   ON memories(staleness_class, last_accessed);
CREATE INDEX IF NOT EXISTS ix_mem_decay       ON memories(state, decay_score);
CREATE INDEX IF NOT EXISTS ix_mem_source_evt  ON memories(source_event_id);
CREATE INDEX IF NOT EXISTS ix_mem_shred       ON memories(shred_key_id);
-- Dedup + idempotent consolidation (N3): unique active content per (ns, type).
CREATE UNIQUE INDEX IF NOT EXISTS uq_mem_content
    ON memories(namespace, memory_type, content_hash) WHERE state = 'active';

-- M:N provenance / truth links.
CREATE TABLE IF NOT EXISTS memory_derived_from (
    parent_id TEXT NOT NULL REFERENCES memories(id),
    child_id  TEXT NOT NULL REFERENCES memories(id),
    PRIMARY KEY(parent_id, child_id)
);
CREATE TABLE IF NOT EXISTS memory_contradicts (
    a_id TEXT NOT NULL, b_id TEXT NOT NULL, PRIMARY KEY(a_id, b_id)
);
CREATE TABLE IF NOT EXISTS memory_supports (
    a_id TEXT NOT NULL, b_id TEXT NOT NULL, PRIMARY KEY(a_id, b_id)
);
CREATE TABLE IF NOT EXISTS memory_mentions_entity (
    memory_id TEXT NOT NULL, entity_id TEXT NOT NULL, PRIMARY KEY(memory_id, entity_id)
);

-- Evidence tracking (TMS, Issue 12/15).
CREATE TABLE IF NOT EXISTS evidence (
    id              TEXT PRIMARY KEY,
    memory_id       TEXT NOT NULL REFERENCES memories(id),
    kind            TEXT NOT NULL,
    source_event_id TEXT REFERENCES events(id),
    weight          REAL NOT NULL DEFAULT 1.0,
    observed_at     TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS ix_evidence_mem ON evidence(memory_id, kind);

-- ── Graph (adjacency + CTE, ADR-004) ──
CREATE TABLE IF NOT EXISTS entities (
    id           TEXT PRIMARY KEY,
    canonical_id TEXT NOT NULL,
    entity_type  TEXT NOT NULL,
    display_name TEXT NOT NULL,
    created_at   TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS entity_aliases (
    entity_id  TEXT NOT NULL REFERENCES entities(id),
    alias      TEXT NOT NULL,
    alias_type TEXT NOT NULL,
    PRIMARY KEY(entity_id, alias)
);
CREATE INDEX IF NOT EXISTS ix_alias_lookup ON entity_aliases(alias, alias_type);
CREATE TABLE IF NOT EXISTS entity_merge_provenance (
    merged_entity_id TEXT NOT NULL,
    into_entity_id   TEXT NOT NULL,
    merged_at        TEXT NOT NULL,
    reversible_until TEXT,
    PRIMARY KEY(merged_entity_id, into_entity_id)
);
CREATE TABLE IF NOT EXISTS relationships (
    id                TEXT PRIMARY KEY,
    source_id         TEXT NOT NULL REFERENCES entities(id),
    target_id         TEXT NOT NULL REFERENCES entities(id),
    rel_type          TEXT NOT NULL,
    strength          REAL NOT NULL DEFAULT 1.0,
    valid_from        TEXT NOT NULL,
    valid_until       TEXT,
    evidence_event_id TEXT REFERENCES events(id)
);
CREATE INDEX IF NOT EXISTS ix_rel_source ON relationships(source_id, rel_type);
CREATE INDEX IF NOT EXISTS ix_rel_target ON relationships(target_id, rel_type);
CREATE TABLE IF NOT EXISTS graph_2hop_cache (
    root_entity_id TEXT PRIMARY KEY,
    neighbors_json TEXT NOT NULL,
    refreshed_at   TEXT NOT NULL
);

-- ── Preferences ──
CREATE TABLE IF NOT EXISTS preferences (
    key          TEXT PRIMARY KEY,
    value        TEXT NOT NULL,
    vector_clock TEXT NOT NULL,
    updated_at   TEXT NOT NULL,
    device_id    TEXT NOT NULL
);

-- ── Transactional outbox (D-5, ADR-005) ──
CREATE TABLE IF NOT EXISTS embedding_outbox (
    id           INTEGER PRIMARY KEY AUTOINCREMENT,
    memory_id    TEXT NOT NULL,
    index_target TEXT NOT NULL,
    op           TEXT NOT NULL,
    content_hash TEXT NOT NULL,
    attempts     INTEGER NOT NULL DEFAULT 0,
    status       TEXT NOT NULL DEFAULT 'pending',
    created_at   TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS ix_outbox_pending ON embedding_outbox(index_target, status, id);
CREATE TABLE IF NOT EXISTS outbox_cursor (
    index_target TEXT PRIMARY KEY,
    last_done_id INTEGER NOT NULL DEFAULT 0
);

-- ── Governance / audit (D-19, 32.5, §30) ──
CREATE TABLE IF NOT EXISTS feedback_events (
    id          TEXT PRIMARY KEY,
    target_id   TEXT NOT NULL,
    target_kind TEXT NOT NULL,
    signal      TEXT NOT NULL,
    payload     TEXT,
    context     TEXT,
    ts          TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS knowledge_gaps (
    id             TEXT PRIMARY KEY,
    query          TEXT NOT NULL,
    domain         TEXT,
    times_missed   INTEGER NOT NULL DEFAULT 1,
    last_missed_at TEXT NOT NULL,
    resolved       INTEGER NOT NULL DEFAULT 0
);
CREATE TABLE IF NOT EXISTS memory_audit (
    id             TEXT PRIMARY KEY,
    ts             TEXT NOT NULL,
    decision       TEXT NOT NULL,
    reason         TEXT NOT NULL,
    candidate_hash TEXT,
    namespace      TEXT,
    mode           TEXT
);
CREATE INDEX IF NOT EXISTS ix_audit_ts ON memory_audit(ts);
CREATE TABLE IF NOT EXISTS enrichment_deadletter (
    event_id TEXT PRIMARY KEY,
    stage    TEXT NOT NULL,
    error    TEXT,
    attempts INTEGER NOT NULL DEFAULT 0,
    ts       TEXT NOT NULL
);

-- ── FTS5 full-text index (P1 keyword floor, D-2) ──
CREATE VIRTUAL TABLE IF NOT EXISTS memories_fts USING fts5(
    content,
    memory_id UNINDEXED,
    namespace UNINDEXED,
    tokenize = 'unicode61'
);
