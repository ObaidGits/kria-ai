-- Authority schema v2 — relation registry + materialized alias lookup, with
-- the required canonical Memory Link seeds (design §4.2/§19.3, task F2.2.1).
--
-- This migration extends the F2 semantic epoch (opened by 0017 with the typed
-- cognitive record / entity / alias / mention / evidence tables) with the
-- **single source of relation identity**: the versioned `relation_registry`
-- and its materialized `relation_aliases` lookup. Migration 0017 explicitly
-- DEFERRED these tables (and `relationships` / `memory_links` /
-- `entity_resolution_*`) to F2.2; this file delivers the registry half.
--
-- ── Scope (task 2.2.1 only) ──────────────────────────────────────────────
-- Creates `relation_registry` + `relation_aliases`, their indexes, and SEEDS
-- the registry with the five required canonical Memory Link rows
-- (`derived_from`, `supports`, `contradicts`, `mentions_entity`,
-- `superseded_by` — design §4.2/§19.3) plus two representative domain-ontology
-- rows that establish the directed/symmetric contract (`related_to` symmetric,
-- `part_of` directed with a distinct inverse). All rows are version 1.
--
-- OUT OF SCOPE (later F2.2 subtasks, do NOT add here): the `relationships` and
-- `memory_links` tables + the canonical `identity_hash` (2.2.2); polymorphic
-- endpoint / kind / alias / direction / reflexivity / Valid Time / Evidence /
-- policy validation inside AuthorityTx (2.2.3); Evidence append (2.2.4);
-- governed create/edit/confirm/expire/delete/restore/undo commands (2.2.5);
-- legacy free-text relationship reconciliation (2.2.6); legacy relationship
-- table deletion (2.2.7).
--
-- ── Design invariants enforced here ──────────────────────────────────────
--   * `direction_class` is the closed set {directed, symmetric} (schema CHECK).
--   * `validity_policy` is the closed set {optional, required, forbidden}.
--   * Booleans (`reflexive`, `writable`) are INTEGER CHECK IN (0,1).
--   * JSON columns (`aliases_json`, `source_kinds_json`, `target_kinds_json`,
--     `evidence_policy_json`) are `json_valid`-guarded (JSON1 is bundled).
--   * The registry is the ONLY relation-identity authority; no parallel untyped
--     link table is permitted (design §4.2). Alias lookup is materialized in
--     `relation_aliases`, never a second registry.
--   * The five canonical links are directed, non-reflexive, and NOT writable by
--     raw clients (`writable = 0`); domain commands create them (design §19.3).
--
-- Canonical encodings (design §4 preamble): timestamps RFC3339 UTC TEXT,
-- booleans INTEGER, JSON TEXT with `json_valid`.

-- ── Relation registry (design §4.2) ──────────────────────────────────────
-- The versioned ontology of relation identities. PK is (relation_name,
-- version): a relation-version change adds a new row rather than mutating the
-- prior one, so historic relationships keep referring to their exact semantics.
CREATE TABLE IF NOT EXISTS relation_registry (
    relation_name       TEXT    NOT NULL,          -- canonical snake_case identity
    version             INTEGER NOT NULL,          -- registry version (>=1)
    display_forward     TEXT    NOT NULL,           -- human label, forward traversal
    display_inverse     TEXT,                       -- human label, reverse traversal (NULL ⇒ none)
    aliases_json        TEXT    NOT NULL            -- JSON array of alternate surface forms
                            CHECK (json_valid(aliases_json)),
    direction_class     TEXT    NOT NULL            -- closed set (design §4.2)
                            CHECK (direction_class IN ('directed','symmetric')),
    inverse_name        TEXT,                       -- relation_name of the paired inverse (directed only; NULL ⇒ none/self)
    reflexive           INTEGER NOT NULL            -- may an endpoint relate to itself?
                            CHECK (reflexive IN (0,1)),
    source_kinds_json   TEXT    NOT NULL            -- JSON array of legal source endpoint kinds
                            CHECK (json_valid(source_kinds_json)),
    target_kinds_json   TEXT    NOT NULL            -- JSON array of legal target endpoint kinds
                            CHECK (json_valid(target_kinds_json)),
    validity_policy     TEXT    NOT NULL            -- Valid Time disposition (closed set)
                            CHECK (validity_policy IN ('optional','required','forbidden')),
    evidence_policy_json TEXT   NOT NULL            -- JSON object: min evidence / required polarity / required attributes
                            CHECK (json_valid(evidence_policy_json)),
    policy_rule_version TEXT    NOT NULL,           -- version tag of the governing policy rule set
    writable            INTEGER NOT NULL            -- may raw clients create this relation directly?
                            CHECK (writable IN (0,1)),
    PRIMARY KEY (relation_name, version),
    CHECK (version >= 1),
    -- A symmetric relation is its own inverse: it must not name a distinct
    -- inverse relation (design §19.3 "A symmetric registry relation
    -- canonicalizes endpoints … directed relations retain order").
    CHECK (direction_class = 'directed' OR inverse_name IS NULL)
);

CREATE INDEX IF NOT EXISTS idx_relation_registry_direction
    ON relation_registry (direction_class);
CREATE INDEX IF NOT EXISTS idx_relation_registry_writable
    ON relation_registry (writable);

-- ── Relation aliases (design §4.2, "alias lookup is materialized") ────────
-- A materialized normalized surface-form → (relation_name, version) lookup so
-- free-text relation labels resolve to a single canonical registry row. This
-- is a projection of `relation_registry.aliases_json` + the canonical name; it
-- is NOT an independent relation authority. (alias, version) is unique so a
-- surface form resolves to exactly one relation within a registry version.
CREATE TABLE IF NOT EXISTS relation_aliases (
    alias         TEXT    NOT NULL,                 -- normalized alias (lower-case, snake)
    version       INTEGER NOT NULL,
    relation_name TEXT    NOT NULL,
    PRIMARY KEY (alias, version),
    FOREIGN KEY (relation_name, version)
        REFERENCES relation_registry (relation_name, version)
);

CREATE INDEX IF NOT EXISTS idx_relation_aliases_relation
    ON relation_aliases (relation_name, version);

-- ── Seed: required canonical Memory Link rows (design §4.2/§19.3) ─────────
-- All five are directed, non-reflexive, and writable = 0 (raw clients cannot
-- author them; only governed domain commands do). evidence_policy_json encodes
-- the §19.3 "policy and provenance" column as
-- {"min_evidence":N,"required_polarity":<supports|contradicts|null>,
--  "required_attributes":[...]}.

INSERT OR IGNORE INTO relation_registry
    (relation_name, version, display_forward, display_inverse, aliases_json,
     direction_class, inverse_name, reflexive, source_kinds_json,
     target_kinds_json, validity_policy, evidence_policy_json,
     policy_rule_version, writable)
VALUES
    -- derived_from: derived record → immediate Event/Memory/Episode/Summary/Skill.
    ('derived_from', 1, 'derived from', 'derives',
     json('["derived_from","derives_from"]'),
     'directed', NULL, 0,
     json('["memory","summary","skill","rule"]'),
     json('["event","memory","episode","summary","skill"]'),
     'optional',
     json('{"min_evidence":0,"required_attributes":["method","method_version","time"]}'),
     '1', 0),

    -- supports: Evidence → claim/relationship/goal; source locator required.
    ('supports', 1, 'supports', 'supported by',
     json('["supports","supported_by"]'),
     'directed', NULL, 0,
     json('["evidence"]'),
     json('["memory","summary","skill","rule","relationship","goal"]'),
     'optional',
     json('{"min_evidence":1,"required_polarity":"supports","required_attributes":["locator"]}'),
     '1', 0),

    -- contradicts: Evidence or claim → claim/relationship; polarity/rationale required.
    ('contradicts', 1, 'contradicts', 'contradicted by',
     json('["contradicts","contradicted_by"]'),
     'directed', NULL, 0,
     json('["evidence","memory","summary","skill","rule"]'),
     json('["memory","summary","skill","rule","relationship"]'),
     'optional',
     json('{"min_evidence":1,"required_polarity":"contradicts","required_attributes":["rationale"]}'),
     '1', 0),

    -- mentions_entity: Event/Memory/source span → Entity; locator/extractor/version required.
    ('mentions_entity', 1, 'mentions', 'mentioned in',
     json('["mentions_entity","mentions"]'),
     'directed', NULL, 0,
     json('["event","memory","summary","skill","rule"]'),
     json('["entity"]'),
     'optional',
     json('{"min_evidence":0,"required_attributes":["locator","extractor","extractor_version"]}'),
     '1', 0),

    -- superseded_by: predecessor → successor of compatible kind; decision evidence required.
    ('superseded_by', 1, 'superseded by', 'supersedes',
     json('["superseded_by","supersedes"]'),
     'directed', NULL, 0,
     json('["memory","summary","skill","rule","relationship","entity"]'),
     json('["memory","summary","skill","rule","relationship","entity"]'),
     'optional',
     json('{"min_evidence":1,"required_attributes":["decision_evidence"]}'),
     '1', 0);

-- ── Seed: representative domain-ontology rows (directed/symmetric contract) ──
-- Two governed domain relations that exercise both direction classes and the
-- writable disposition (writable = 1: clients may author these through the
-- governed write boundary). `related_to` is the canonical SYMMETRIC example
-- (its own inverse; endpoints canonicalized before identity — MGR-018);
-- `part_of` is a DIRECTED example with a distinct registered inverse label.

INSERT OR IGNORE INTO relation_registry
    (relation_name, version, display_forward, display_inverse, aliases_json,
     direction_class, inverse_name, reflexive, source_kinds_json,
     target_kinds_json, validity_policy, evidence_policy_json,
     policy_rule_version, writable)
VALUES
    ('related_to', 1, 'related to', 'related to',
     json('["related_to","associated_with","relates_to"]'),
     'symmetric', NULL, 0,
     json('["entity"]'),
     json('["entity"]'),
     'optional',
     json('{"min_evidence":0,"required_attributes":[]}'),
     '1', 1),

    ('part_of', 1, 'part of', 'has part',
     json('["part_of","belongs_to","is_part_of"]'),
     'directed', 'has_part', 0,
     json('["entity"]'),
     json('["entity"]'),
     'optional',
     json('{"min_evidence":0,"required_attributes":[]}'),
     '1', 1);

-- ── Seed: materialized alias lookup ──────────────────────────────────────
-- One row per (canonical name + each alias) → (relation_name, version). Alias
-- lists are disjoint across relations, so (alias, version) stays unique.
INSERT OR IGNORE INTO relation_aliases (alias, version, relation_name) VALUES
    ('derived_from',    1, 'derived_from'),
    ('derives_from',    1, 'derived_from'),
    ('supports',        1, 'supports'),
    ('supported_by',    1, 'supports'),
    ('contradicts',     1, 'contradicts'),
    ('contradicted_by', 1, 'contradicts'),
    ('mentions_entity', 1, 'mentions_entity'),
    ('mentions',        1, 'mentions_entity'),
    ('superseded_by',   1, 'superseded_by'),
    ('supersedes',      1, 'superseded_by'),
    ('related_to',      1, 'related_to'),
    ('associated_with', 1, 'related_to'),
    ('relates_to',      1, 'related_to'),
    ('part_of',         1, 'part_of'),
    ('belongs_to',      1, 'part_of'),
    ('is_part_of',      1, 'part_of');
