-- Runtime backend tables (memory-upgrade cutover): facts, snippets, and
-- document chunks, so the new `KriaMemoryRuntime` backend can serve the desktop
-- `MemoryManager`/`MemoryReader` surface (plus the RAG chunk store) over the
-- single authority DB — enabling deletion of the legacy `MemoryStore` SQLite
-- implementation. Schemas mirror the legacy shapes so consumer behavior is
-- unchanged. FTS5 tables use explicit rowids (no external-content) to match the
-- conversation store convention (0004).
CREATE TABLE IF NOT EXISTS memory_facts (
    id             INTEGER PRIMARY KEY AUTOINCREMENT,
    text           TEXT NOT NULL,
    category       TEXT NOT NULL DEFAULT 'general',
    source         TEXT NOT NULL DEFAULT 'inferred',
    created_at     TEXT NOT NULL DEFAULT (datetime('now')),
    last_accessed  TEXT NOT NULL DEFAULT (datetime('now')),
    access_count   INTEGER NOT NULL DEFAULT 0,
    decay_score    REAL NOT NULL DEFAULT 1.0
);
CREATE INDEX IF NOT EXISTS ix_memory_facts_decay ON memory_facts(decay_score);
CREATE VIRTUAL TABLE IF NOT EXISTS facts_fts USING fts5(text);

CREATE TABLE IF NOT EXISTS snippets (
    name     TEXT PRIMARY KEY,
    content  TEXT NOT NULL,
    language TEXT NOT NULL DEFAULT 'text',
    tags     TEXT NOT NULL DEFAULT '[]',
    created  TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE TABLE IF NOT EXISTS document_chunks (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    doc_id      TEXT NOT NULL,
    doc_name    TEXT NOT NULL,
    doc_type    TEXT NOT NULL DEFAULT 'text',
    chunk_index INTEGER NOT NULL DEFAULT 0,
    content     TEXT NOT NULL,
    char_offset INTEGER NOT NULL DEFAULT 0,
    created_at  TEXT NOT NULL DEFAULT (datetime('now'))
);
CREATE INDEX IF NOT EXISTS ix_document_chunks_doc ON document_chunks(doc_id);
CREATE VIRTUAL TABLE IF NOT EXISTS chunks_fts USING fts5(content);
