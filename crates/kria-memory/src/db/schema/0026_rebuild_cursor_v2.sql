-- F3.1 / task 3.1.5: standalone rebuild_cursor table for vector partition
-- temporary-generation tracking (design §5.3).
--
-- The earlier migration 0023 added `rebuild_generation`, `rebuild_cursor`, and
-- `rebuild_started_at` as additive columns on `derived_manifests`.  This
-- migration adds a SEPARATE dedicated `rebuild_cursor` table that tracks
-- in-progress and interrupted rebuilds with per-partition granularity and
-- supports the model migration cursor (migration_source_partition_id) required
-- by the model-upgrade path.
--
-- Columns:
--   partition_id                TEXT PK   — identifies the target partition
--   run_id                      TEXT      — a UUID assigned per rebuild attempt
--   last_record_id              TEXT      — last record_id successfully staged
--                                           (NULL = not yet started)
--   status                      TEXT      — 'running' | 'interrupted' | 'activated'
--   migration_source_partition_id TEXT    — when populating a NEW partition
--                                           alongside an OLD one (model version
--                                           change), this records which old
--                                           partition is being migrated FROM.
--                                           NULL = not a migration run.
--   started_at                  TEXT      — RFC3339 UTC timestamp
--   updated_at                  TEXT      — RFC3339 UTC, last checkpoint write

CREATE TABLE IF NOT EXISTS rebuild_cursor (
    partition_id                  TEXT    NOT NULL PRIMARY KEY,
    run_id                        TEXT    NOT NULL,
    last_record_id                TEXT,
    status                        TEXT    NOT NULL
        CHECK (status IN ('running', 'interrupted', 'activated')),
    migration_source_partition_id TEXT,
    started_at                    TEXT    NOT NULL,
    updated_at                    TEXT    NOT NULL
);
