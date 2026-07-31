-- Retrieval trace v2 extension (design §4.3/§6.4, task F3.4.4).
-- Adds RRF replay fields (k, availability, weights) and separate evidence/goal/
-- Memory-Worth contribution columns to retrieval_traces and retrieval_trace_items.
-- These fields enable exact offline RRF replay from stored one-based ranks.

ALTER TABLE retrieval_traces ADD COLUMN k_value REAL;
ALTER TABLE retrieval_traces ADD COLUMN availability_json TEXT CHECK (availability_json IS NULL OR json_valid(availability_json));
ALTER TABLE retrieval_traces ADD COLUMN weights_json TEXT CHECK (weights_json IS NULL OR json_valid(weights_json));
ALTER TABLE retrieval_traces ADD COLUMN evidence_contribution REAL;
ALTER TABLE retrieval_traces ADD COLUMN memory_worth_contribution REAL;
ALTER TABLE retrieval_traces ADD COLUMN goal_contribution_total REAL;

ALTER TABLE retrieval_trace_items ADD COLUMN evidence_contribution REAL;
ALTER TABLE retrieval_trace_items ADD COLUMN memory_worth_contribution REAL;
