-- Library tables (memory-upgrade design §14, task 31).
CREATE TABLE IF NOT EXISTS library_items (
    id              TEXT PRIMARY KEY,
    sha256          TEXT NOT NULL,
    title           TEXT,
    author          TEXT,
    version         INTEGER NOT NULL DEFAULT 1,
    prev_version_id TEXT REFERENCES library_items(id),
    path            TEXT NOT NULL,
    ingested_at     TEXT NOT NULL,
    shred_key_id    TEXT REFERENCES shred_keys(subject_id)
);
CREATE INDEX IF NOT EXISTS ix_lib_sha ON library_items(sha256, version);

CREATE TABLE IF NOT EXISTS library_collections (
    item_id    TEXT NOT NULL REFERENCES library_items(id),
    collection TEXT NOT NULL,
    PRIMARY KEY(item_id, collection)
);

CREATE TABLE IF NOT EXISTS library_chunks (
    id                      TEXT PRIMARY KEY,
    item_id                 TEXT NOT NULL REFERENCES library_items(id),
    chunk_index             INTEGER NOT NULL,
    text                    TEXT NOT NULL,
    embedding_id            TEXT,
    modality                TEXT NOT NULL DEFAULT 'text',
    embedding_model_version TEXT,
    page                    INTEGER
);
CREATE INDEX IF NOT EXISTS ix_lib_chunks_item ON library_chunks(item_id);
