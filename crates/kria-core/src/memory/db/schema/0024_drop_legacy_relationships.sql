-- F2.2.7 legacy cutover: drop the free-text `relationships` table and its
-- associated indexes and 2-hop cache.
--
-- Context
-- -------
-- The `relationships` table (created in 0001_init.sql) was the legacy graph
-- edge store with a free-text `rel_type` column.  Its replacement is
-- `relationships_v2` (0019), which enforces registry-governed relation
-- identity, typed polymorphic endpoints, and the full v2 evidence/truth/
-- policy model.
--
-- By the time this migration runs, the LegacyRelationshipMigrator (task
-- F2.2.6) has already migrated or rejected every legacy row: resolved rows
-- were committed to `relationships_v2`; unresolvable rows were recorded in
-- the deterministic migration report.  There is no live data to preserve.
--
-- The `graph_2hop_cache` table (also from 0001) stored pre-computed BFS
-- results keyed on the legacy table — it is no longer valid and is dropped
-- here as well.
--
-- Rust callers
-- ------------
-- All Rust code that wrote to `relationships` has been redirected to
-- `relationships_v2` or removed in task F2.2.7.  The only remaining reader,
-- `LegacyRelationshipMigrator::read_legacy_rows`, guards itself against a
-- missing table and returns an empty list when this migration has been
-- applied.
--
-- Additive-only policy
-- --------------------
-- The migrations.rs comment says "never drop or rename columns; only add."
-- That comment describes the general policy for *column* changes (to keep
-- older binaries readable).  Dropping an entire obsolete *table* is a hard
-- cutover that the spec explicitly authorises for this pre-production,
-- single-user codebase (tasks.md §2.2.7: "Add a new SQL migration that DROPs
-- the legacy relationships table").  No older binary needs to read data from
-- this table after the migration reconciler has run.

DROP INDEX IF EXISTS ix_rel_source;
DROP INDEX IF EXISTS ix_rel_target;
DROP TABLE IF EXISTS relationships;
DROP TABLE IF EXISTS graph_2hop_cache;
