-- Conversation history tables (memory-upgrade Step-1 cutover).
-- Chat/session replay is a distinct concern from cognitive memory (derived
-- knowledge). These tables live in the same authority DB (unified backup /
-- encryption / L14) and replace the legacy `MemoryStore` conversation surface.
CREATE TABLE IF NOT EXISTS conversations (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    session_id  TEXT NOT NULL,
    role        TEXT NOT NULL,
    content     TEXT NOT NULL,
    tool_name   TEXT,
    tool_result TEXT,
    tokens_used INTEGER,
    timestamp   TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS ix_conv_session ON conversations(session_id, id);

-- Standalone FTS5 (explicit rowid), mirroring the legacy conversation search.
CREATE VIRTUAL TABLE IF NOT EXISTS conversations_fts USING fts5(content);

CREATE TABLE IF NOT EXISTS chat_media (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    session_id  TEXT NOT NULL,
    media_type  TEXT NOT NULL,
    file_path   TEXT NOT NULL,
    sha256      TEXT,
    prompt      TEXT,
    width       INTEGER,
    height      INTEGER,
    style       TEXT,
    provenance  TEXT,
    created_at  TEXT NOT NULL DEFAULT (datetime('now'))
);
CREATE INDEX IF NOT EXISTS ix_chat_media_session ON chat_media(session_id, created_at);
