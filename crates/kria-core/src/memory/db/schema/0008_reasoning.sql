-- Reasoning Memory (memory-upgrade Phase 2, Priority 2). Stores reasoning traces
-- — chains, hypotheses, and counterexamples — per task class so past reasoning
-- becomes reusable: replayable history, hypothesis/counterexample recall, and
-- hallucination tracking (failed chains / counterexamples). One authority DB;
-- no parallel store.
CREATE TABLE IF NOT EXISTS reasoning_traces (
    id          TEXT PRIMARY KEY,
    session_id  TEXT,
    task_label  TEXT NOT NULL,
    kind        TEXT NOT NULL,          -- 'chain' | 'hypothesis' | 'counterexample'
    content     TEXT NOT NULL,
    confidence  REAL NOT NULL DEFAULT 0.5,
    success     INTEGER,                -- 1/0 for chains with a known outcome; NULL otherwise
    created_at  TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS ix_reasoning_task ON reasoning_traces(task_label, kind);
CREATE INDEX IF NOT EXISTS ix_reasoning_session ON reasoning_traces(session_id, created_at);
