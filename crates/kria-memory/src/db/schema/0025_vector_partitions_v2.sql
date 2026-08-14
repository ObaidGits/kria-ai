-- F3.1 / task 3.1.2: embedding_partitions + mem_vectors v2 schema.
--
-- DESIGN INVARIANTS
-- ─────────────────
-- * embedding_partitions is the single source of truth for what model/revision
--   combination owns a given partition_id.  Every vector row in mem_vectors
--   foreign-keys to it so orphan vectors are impossible at the DB layer.
-- * mem_vectors is a REBUILDABLE derived projection (design §14 / L4).  The
--   table may be truncated and rebuilt at any time; no authority data lives here.
-- * Policy columns (namespace, owner_id, scope, sensitivity) are present on
--   EVERY row — policy-prefiltered search runs the (partition_id, namespace,
--   scope, sensitivity, truth_state) index before touching any vector BLOB.
-- * The PRIMARY KEY (partition_id, record_id) enables exact per-partition
--   per-record upsert with ON CONFLICT DO UPDATE (no separate SELECT).
-- * Dimension and vector_byte_length are enforced at BOTH the schema level
--   (SQLite CHECK constraint) AND the Rust decoding layer (task 3.1.3).
-- * The old mem_vectors table (from 0002_vectors.sql) is kept alive alongside
--   this new schema until the F3.1 cutover.  The old table uses
--   (model_version, id) as PK; the new one uses (partition_id, record_id).
--   They coexist until all Rust write paths have been redirected.

-- ── embedding_partitions ─────────────────────────────────────────────────────
CREATE TABLE IF NOT EXISTS embedding_partitions (
    partition_id               TEXT    NOT NULL,
    model_id                   TEXT    NOT NULL,
    model_source_revision      TEXT    NOT NULL,
    onnx_sha256                TEXT    NOT NULL,
    tokenizer_sha256           TEXT    NOT NULL,
    license_spdx               TEXT    NOT NULL,
    license_disposition_id     TEXT    NOT NULL,
    ort_version                TEXT    NOT NULL,
    fastembed_version          TEXT    NOT NULL,
    dimension                  INTEGER NOT NULL CHECK (dimension = 384),
    dtype                      TEXT    NOT NULL CHECK (dtype = 'f32le'),
    normalized                 INTEGER NOT NULL CHECK (normalized IN (0, 1)),
    max_tokens                 INTEGER NOT NULL CHECK (max_tokens = 256),
    pooling                    TEXT    NOT NULL CHECK (pooling = 'mean'),
    vector_byte_length         INTEGER NOT NULL CHECK (vector_byte_length = 1536),
    status                     TEXT    NOT NULL CHECK (status IN ('active', 'migrating', 'deprecated', 'deleted')),
    build_time                 TEXT    NOT NULL,
    manifest_checksum          TEXT    NOT NULL,
    PRIMARY KEY (partition_id)
);

-- Prevent two active (non-deleted) partitions claiming the same model+revision.
-- Deleted partitions may coexist to preserve audit history.
CREATE UNIQUE INDEX IF NOT EXISTS ix_ep_model_rev_active
    ON embedding_partitions (model_id, model_source_revision)
    WHERE status != 'deleted';

-- ── mem_vectors (v2) ─────────────────────────────────────────────────────────
CREATE TABLE IF NOT EXISTS mem_vectors_v2 (
    partition_id    TEXT    NOT NULL REFERENCES embedding_partitions (partition_id),
    record_id       TEXT    NOT NULL,
    vector          BLOB    NOT NULL CHECK (length(vector) = 1536),
    content_hash    TEXT    NOT NULL,   -- SHA-256 of the embedded text (dedup/rebuild key)
    namespace       TEXT    NOT NULL,   -- policy column
    owner_id        TEXT    NOT NULL,   -- policy column
    scope           TEXT    NOT NULL,   -- policy column
    sensitivity     INTEGER NOT NULL CHECK (sensitivity >= 0 AND sensitivity <= 3),
    truth_state     TEXT    NOT NULL,   -- e.g. 'Current', 'Stale', 'Unverified'
    revision        INTEGER NOT NULL CHECK (revision >= 0),
    PRIMARY KEY (partition_id, record_id),
    FOREIGN KEY (partition_id) REFERENCES embedding_partitions (partition_id)
);

-- Policy-prefiltered search index: allows the planner to pre-filter by
-- partition+namespace+scope+sensitivity+truth before scanning any BLOBs.
CREATE INDEX IF NOT EXISTS ix_mv2_policy
    ON mem_vectors_v2 (partition_id, namespace, scope, sensitivity, truth_state);

-- Dedup/rebuild index: fast lookup by content hash within a partition.
CREATE INDEX IF NOT EXISTS ix_mv2_content_hash
    ON mem_vectors_v2 (partition_id, content_hash);
