-- Derived-index rebuild state (task 1.8.5, MGR-042 / design §5.3).
--
-- Adds rebuild-tracking columns to `derived_manifests` so a temporary-
-- generation rebuild can be interrupted and resumed without starting over.
--
-- Columns added (all additive with DEFAULT NULL so existing rows are unaffected):
--
--   rebuild_generation  INTEGER  — monotonically-increasing generation counter
--                                   for the in-progress build.  NULL = no active
--                                   rebuild.  A row with status = 'building' has
--                                   an assigned generation; on atomic activation
--                                   the generation is promoted to the 'active'
--                                   entry and the previous 'active' entry moves
--                                   to 'superseded'.
--
--   rebuild_cursor      TEXT     — the last authority row id (UUID string) whose
--                                   content was successfully indexed into the
--                                   temporary generation.  NULL = rebuild has not
--                                   started processing rows yet.  On resume the
--                                   query filters `id > rebuild_cursor` in
--                                   creation order, so we continue exactly where
--                                   we left off.
--
--   rebuild_started_at  TEXT     — RFC3339 UTC timestamp when the current
--                                   rebuild generation was started.  NULL if no
--                                   build is active.  Purely informational /
--                                   observability.

ALTER TABLE derived_manifests ADD COLUMN rebuild_generation  INTEGER;
ALTER TABLE derived_manifests ADD COLUMN rebuild_cursor      TEXT;
ALTER TABLE derived_manifests ADD COLUMN rebuild_started_at  TEXT;

-- Index to find any 'building' row for a given target quickly.
CREATE INDEX IF NOT EXISTS ix_derived_manifests_building
    ON derived_manifests (target, status)
    WHERE status = 'building';
