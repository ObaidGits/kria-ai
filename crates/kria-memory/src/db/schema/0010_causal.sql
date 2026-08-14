-- Causal Memory (memory-upgrade Phase 2, research). Directed cause→effect edges
-- with observation counts and success/failure attribution, so KRIA can reason
-- about what leads to what: success/failure causality, multi-hop causal chains,
-- and counterfactuals. Cause/effect are normalized labels; confidence is the
-- observed success ratio. One authority DB; no parallel store.
CREATE TABLE IF NOT EXISTS causal_links (
    cause        TEXT NOT NULL,
    effect       TEXT NOT NULL,
    observations INTEGER NOT NULL DEFAULT 0,
    successes    INTEGER NOT NULL DEFAULT 0,
    updated_at   TEXT NOT NULL,
    PRIMARY KEY (cause, effect)
);
CREATE INDEX IF NOT EXISTS ix_causal_cause ON causal_links(cause);
CREATE INDEX IF NOT EXISTS ix_causal_effect ON causal_links(effect);
