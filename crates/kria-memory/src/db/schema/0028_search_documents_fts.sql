-- F3.2 / task 3.2.2: search_documents_fts external-content FTS5 table.
--
-- `search_documents_fts` is the **rebuildable FTS5 projection** that indexes
-- over the `search_documents` content table (created by migration 0027).
-- This is an EXTERNAL-CONTENT FTS5 table: the text is stored in
-- `search_documents`; the FTS5 index provides ranked full-text retrieval.
--
-- DESIGN INVARIANTS (design §4.4, §A1, §A8)
-- ─────────────────────────────────────────
-- * FTS5 is NEVER the semantic authority.  `search_documents_fts` is a
--   disposable derived projection; it may be dropped and rebuilt from
--   `search_documents` (the `content` table) at any time without data loss.
-- * Indexed columns: title, body, aliases, source_text, relation_text —
--   exactly the five text fields that carry user-searchable content.
-- * UNINDEXED columns: record_kind, record_id, namespace, scope, sensitivity,
--   truth_state, revision — carried through for policy prefiltering and result
--   construction without adding to the inverted index.
-- * content="search_documents" + content_rowid="rowid" links the FTS5 index
--   to the content table.  SQLite uses the rowid to retrieve content columns
--   when the table is queried; the content MUST remain in sync with the index
--   (enforced by the three triggers below).
-- * Tokenizer: `unicode61 remove_diacritics 2` — Unicode-aware with full
--   diacritic removal (e.g. "café" → "cafe").
-- * Prefix sizes 2 3 4: enables 2, 3, and 4-character prefix matching so that
--   searching "mem" finds "memory", "prog" finds "programming", etc.
--
-- TRIGGER SEMANTICS (SQLite FTS5 external-content update protocol)
-- ─────────────────────────────────────────────────────────────────
-- For an external-content FTS5 table, content-row changes must be reflected
-- manually via triggers.  The FTS5 deletion protocol requires inserting a
-- special 'delete' command row (not a plain DELETE) because the inverted index
-- tracks term→rowid mappings that must be explicitly removed.
--
-- INSERT trigger:  inserts the new FTS row.
-- UPDATE trigger:  deletes the OLD FTS entry first (using the 'delete' command),
--                  then inserts the NEW entry.
-- DELETE trigger:  removes the FTS entry using the 'delete' command.

-- ── FTS5 external-content table ──────────────────────────────────────────────

CREATE VIRTUAL TABLE IF NOT EXISTS search_documents_fts USING fts5(
    -- Indexed columns (full-text searchable)
    title,
    body,
    aliases,
    source_text,
    relation_text,
    -- Unindexed columns (carried for prefiltering / result construction)
    record_kind UNINDEXED,
    record_id   UNINDEXED,
    namespace   UNINDEXED,
    scope       UNINDEXED,
    sensitivity UNINDEXED,
    truth_state UNINDEXED,
    revision    UNINDEXED,
    -- External-content link
    content     = "search_documents",
    content_rowid = "rowid",
    -- Tokenizer: Unicode-aware, full diacritic removal
    tokenize    = "unicode61 remove_diacritics 2",
    -- Prefix index for 2-, 3-, and 4-character prefixes
    prefix      = "2 3 4"
);

-- ── AFTER INSERT trigger ──────────────────────────────────────────────────────
-- Keeps the FTS5 index in sync when a new row is added to search_documents.

CREATE TRIGGER IF NOT EXISTS trg_sd_fts_insert
AFTER INSERT ON search_documents
BEGIN
    INSERT INTO search_documents_fts(
        rowid,
        title, body, aliases, source_text, relation_text,
        record_kind, record_id, namespace, scope, sensitivity, truth_state, revision
    ) VALUES (
        new.rowid,
        new.title, new.body, new.aliases, new.source_text, new.relation_text,
        new.record_kind, new.record_id, new.namespace, new.scope,
        new.sensitivity, new.truth_state, new.revision
    );
END;

-- ── AFTER UPDATE trigger ──────────────────────────────────────────────────────
-- Deletes the OLD FTS5 entry (using the FTS5 'delete' command protocol) then
-- inserts the NEW entry.  The 'delete' protocol is required by FTS5 external-
-- content tables because the inverted index must explicitly untag the old rowid.

CREATE TRIGGER IF NOT EXISTS trg_sd_fts_update
AFTER UPDATE ON search_documents
BEGIN
    -- Step 1: Remove the OLD FTS5 entry using the external-content delete protocol.
    INSERT INTO search_documents_fts(
        search_documents_fts, rowid,
        title, body, aliases, source_text, relation_text,
        record_kind, record_id, namespace, scope, sensitivity, truth_state, revision
    ) VALUES (
        'delete', old.rowid,
        old.title, old.body, old.aliases, old.source_text, old.relation_text,
        old.record_kind, old.record_id, old.namespace, old.scope,
        old.sensitivity, old.truth_state, old.revision
    );
    -- Step 2: Insert the NEW FTS5 entry.
    INSERT INTO search_documents_fts(
        rowid,
        title, body, aliases, source_text, relation_text,
        record_kind, record_id, namespace, scope, sensitivity, truth_state, revision
    ) VALUES (
        new.rowid,
        new.title, new.body, new.aliases, new.source_text, new.relation_text,
        new.record_kind, new.record_id, new.namespace, new.scope,
        new.sensitivity, new.truth_state, new.revision
    );
END;

-- ── AFTER DELETE trigger ──────────────────────────────────────────────────────
-- Removes the FTS5 entry using the external-content delete protocol when a row
-- is physically deleted from search_documents.

CREATE TRIGGER IF NOT EXISTS trg_sd_fts_delete
AFTER DELETE ON search_documents
BEGIN
    INSERT INTO search_documents_fts(
        search_documents_fts, rowid,
        title, body, aliases, source_text, relation_text,
        record_kind, record_id, namespace, scope, sensitivity, truth_state, revision
    ) VALUES (
        'delete', old.rowid,
        old.title, old.body, old.aliases, old.source_text, old.relation_text,
        old.record_kind, old.record_id, old.namespace, old.scope,
        old.sensitivity, old.truth_state, old.revision
    );
END;
