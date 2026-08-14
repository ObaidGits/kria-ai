-- Authority schema v2 — audited declassification provenance (design §4.1,
-- MGR-004 AC3, task 1.4.3).
--
-- An authorized declassification relaxes/changes the Effective Policy for a
-- target record or source. Per MGR-004 AC3 it MUST "create a new audited
-- provenance record rather than mutate source policy": this table holds those
-- new, immutable provenance records. The contributing `sources` row is never
-- touched — declassification only ever INSERTs here, so the original policy is
-- preserved verbatim for audit.
--
-- Each row captures the target, the prior Effective Policy (its provenance hash
-- + a serialized snapshot), the new declassified policy (hash + snapshot), the
-- authorizing actor, the justification, an integrity `provenance_hash` over the
-- semantic content, the invocation that carried the governed command (for
-- correlation with `events_v2` / `audit_records` of the same transaction), and
-- a `reverses` self-link for a compensating declassification.
--
-- Immutability: like `events_v2` / `audit_records` / `graph_revisions`, the row
-- is append-only (UPDATE/DELETE abort triggers). The self-FK resolves within
-- this single CREATE.
--
-- Canonical encodings (design §4 preamble):
--   * UUIDs      → canonical lower-case TEXT
--   * timestamps → RFC3339 UTC TEXT

CREATE TABLE IF NOT EXISTS declassifications (
    id                TEXT PRIMARY KEY,          -- canonical lower-case UUID text
    target_kind       TEXT NOT NULL CHECK (target_kind IN ('record','source')),
    target_id         TEXT NOT NULL,
    -- Prior Effective Policy (captured by value; never a live reference).
    prior_policy_hash TEXT NOT NULL,
    prior_policy_json TEXT NOT NULL,
    -- New declassified policy.
    new_policy_hash   TEXT NOT NULL,
    new_policy_json   TEXT NOT NULL,
    -- Governance provenance.
    actor_id          TEXT NOT NULL,
    reason            TEXT NOT NULL,
    provenance_hash   TEXT NOT NULL,             -- integrity digest of semantic content
    invocation_id     TEXT NOT NULL,             -- correlates with events_v2/audit of the tx
    reverses          TEXT REFERENCES declassifications(id),  -- compensating self-link
    created_at        TEXT NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_declassifications_target
    ON declassifications (target_kind, target_id);
CREATE INDEX IF NOT EXISTS idx_declassifications_reverses
    ON declassifications (reverses);

CREATE TRIGGER IF NOT EXISTS trg_declassifications_no_update
    BEFORE UPDATE ON declassifications
    BEGIN SELECT RAISE(ABORT, 'declassifications are append-only (immutable, L1)'); END;
CREATE TRIGGER IF NOT EXISTS trg_declassifications_no_delete
    BEFORE DELETE ON declassifications
    BEGIN SELECT RAISE(ABORT, 'declassifications are append-only (immutable, L1)'); END;
