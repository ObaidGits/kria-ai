-- F2.2.7 follow-up: seed `co_mentioned_with` into the relation registry so
-- that the extraction pipeline's direct `relationships_v2` INSERT OR IGNORE
-- does not violate the `(relation_name, relation_version) REFERENCES
-- relation_registry (relation_name, version)` FK.
--
-- `co_mentioned_with` is the symmetric raw co-occurrence signal written by
-- `EntityExtractionPipeline::add_comention_edges` when two entities appear in
-- the same memory.  It is NOT a governed claim (writable = 1 so that the
-- extraction pipeline can insert it directly) and requires no evidence.
--
-- `INSERT OR IGNORE` keeps this migration idempotent: if a DB was freshly
-- created after this migration was first added (so migration 0018 already
-- contains the row via an updated seed) the ignore prevents a dup-PK error.

INSERT OR IGNORE INTO relation_registry
    (relation_name, version, display_forward, display_inverse, aliases_json,
     direction_class, inverse_name, reflexive, source_kinds_json,
     target_kinds_json, validity_policy, evidence_policy_json,
     policy_rule_version, writable)
VALUES
    ('co_mentioned_with', 1, 'co-mentioned with', 'co-mentioned with',
     json('["co_mentioned_with","co_occurs_with"]'),
     'symmetric', NULL, 0,
     json('["entity"]'),
     json('["entity"]'),
     'optional',
     json('{"min_evidence":0,"required_attributes":[]}'),
     '1', 1);

INSERT OR IGNORE INTO relation_aliases (alias, version, relation_name) VALUES
    ('co_mentioned_with', 1, 'co_mentioned_with'),
    ('co_occurs_with',    1, 'co_mentioned_with');
