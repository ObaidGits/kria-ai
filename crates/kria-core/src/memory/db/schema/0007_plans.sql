-- Planning Memory (memory-upgrade Phase 2, Priority 1). Records the outcomes of
-- plans (tool/step sequences) per normalized task class so the planner can
-- prefer historically successful approaches and avoid failed ones. Worth is
-- tracked inline (success/failure/samples) exactly like Memory-Worth, so plan
-- ranking uses the same min-sample-gated scoring. One authority DB; no parallel
-- store. `signature` = stable hash of (task_label + step sequence).
CREATE TABLE IF NOT EXISTS plans (
    signature   TEXT PRIMARY KEY,
    task_label  TEXT NOT NULL,
    steps       TEXT NOT NULL,
    success     INTEGER NOT NULL DEFAULT 0,
    failure     INTEGER NOT NULL DEFAULT 0,
    samples     INTEGER NOT NULL DEFAULT 0,
    confidence  REAL NOT NULL DEFAULT 0.5,
    created_at  TEXT NOT NULL,
    last_used   TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS ix_plans_task ON plans(task_label, samples);
