-- Retrieval self-optimization (memory-upgrade Phase 2, Priority 1). Persists
-- adaptive RRF fusion weights per query class, learned from turn outcomes: when
-- a retrieval that grounded a successful turn was surfaced by a given strategy
-- (vector / fts), that strategy's win count for the class increases, shifting
-- the class's fusion weights toward what works. One authority DB; no parallel
-- store. Weights are derived from win counts at read time (see RetrievalWeightStore).
CREATE TABLE IF NOT EXISTS retrieval_weights (
    query_class TEXT PRIMARY KEY,
    wins_vector INTEGER NOT NULL DEFAULT 0,
    wins_fts    INTEGER NOT NULL DEFAULT 0,
    samples     INTEGER NOT NULL DEFAULT 0,
    updated_at  TEXT NOT NULL
);
