-- Outbox relay hardening (task 1.8.4, MGR-042).
--
-- Adds two columns to `embedding_outbox` that the enhanced relay needs:
--   * `next_attempt_at`  TEXT (RFC3339 UTC) — nil = eligible immediately;
--     set by the relay when a delivery fails so the entry is suppressed until
--     the backoff window expires.  The existing pending-pull query in
--     `sqlite_memory.rs` already returns entries WHERE status='pending'; this
--     column adds the time gate.
--   * `error_code`       TEXT — the last failure reason, for observability
--     (dead-letter inspection, Health panel).
--
-- Both columns are additive with DEFAULT NULL so existing rows are unaffected.
-- The existing `ix_outbox_pending` index is on (index_target, status, id);
-- a new index adds the time gate so the relay can skip backoff-suppressed
-- entries without a full table scan.

ALTER TABLE embedding_outbox ADD COLUMN next_attempt_at TEXT;
ALTER TABLE embedding_outbox ADD COLUMN error_code      TEXT;

-- Covering index for the enhanced pending-pull query:
-- WHERE index_target = ? AND status = 'pending'
--       AND (next_attempt_at IS NULL OR next_attempt_at <= ?)
-- ORDER BY id ASC LIMIT ?
CREATE INDEX IF NOT EXISTS ix_outbox_pending_v2
    ON embedding_outbox (index_target, status, next_attempt_at, id);
