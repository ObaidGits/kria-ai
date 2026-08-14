-- Goal Memory (memory-upgrade Phase 2). Extends the existing authority `goals`
-- table (defined in 0001) with hierarchy support so goals can be decomposed into
-- sub-goals (goal graph / dependencies). Additive-only (L-additive): a single
-- ADD COLUMN plus supporting indexes. Goals remain first-class authority
-- entities that `memories.goal_context_id` already references — one substrate.
ALTER TABLE goals ADD COLUMN parent_id TEXT;
CREATE INDEX IF NOT EXISTS ix_goals_parent ON goals(parent_id);
CREATE INDEX IF NOT EXISTS ix_goals_status_priority ON goals(status, priority);
