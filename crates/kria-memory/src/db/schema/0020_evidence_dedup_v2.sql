-- Partial UNIQUE index enforcing at most one `evidence_v2` observation per
-- (subject, source_event) pair (task F2.2.4, design §4.2, MGR-005 AC3-style
-- idempotency applied to Evidence append).
--
-- ── Why this exists ───────────────────────────────────────────────────────
-- A single authority event is the durable record of one concrete observation.
-- The F1.3 governed-command layer already deduplicates a *replayed identical
-- command* via `idempotency_results (caller_partition, idempotency_key)`
-- (MGR-005 AC3) — a second submission under the same idempotency key never
-- re-executes the semantic mutation at all. This index is a **structural,
-- defense-in-depth** backstop *inside* the evidence-append repository
-- (`TxRelationshipEvidence`, task 2.2.4) for the narrower case where the same
-- authority event is appended as evidence more than once (e.g. a caller that
-- retries the append step under a *different* idempotency key that bypasses
-- the F1.3 replay guard): the same `(subject_kind, subject_id,
-- source_event_id)` triple can never occupy two rows, independent of caller
-- discipline.
--
-- `source_event_id IS NOT NULL` scopes the index to *event-linked* evidence
-- only. A manually authored evidence row with no linked event
-- (`source_event_id IS NULL`) has no event-level identity to deduplicate on;
-- that case is deliberately left to the outer F1.3 idempotency_key/
-- command_hash layer instead — a documented task 2.2.4 decision (see
-- `crates/kria-core/src/memory/authority/relationship_evidence.rs` module
-- docs), not an oversight.
CREATE UNIQUE INDEX IF NOT EXISTS uq_evidence_v2_subject_event
    ON evidence_v2 (subject_kind, subject_id, source_event_id)
    WHERE source_event_id IS NOT NULL;
