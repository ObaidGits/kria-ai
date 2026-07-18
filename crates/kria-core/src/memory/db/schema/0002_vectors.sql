-- Vector store backing table (memory-upgrade task 9).
-- MVP VectorStore backend: brute-force cosine over vectors stored as BLOBs
-- (little-endian f32). Version-partitioned by `model_version` so vectors from
-- different embedding models never mix (architecture §9 / C4). This is a
-- derived, rebuildable index (L4) behind the `VectorStore` trait; LanceDB is
-- the approved swap-in backend when scale demands it (D-1).
CREATE TABLE IF NOT EXISTS mem_vectors (
    model_version TEXT NOT NULL,
    id            TEXT NOT NULL,
    vector        BLOB NOT NULL,
    namespace     TEXT NOT NULL,
    scope         TEXT NOT NULL,
    sensitivity   TEXT NOT NULL,
    memory_type   TEXT NOT NULL,
    content_hash  TEXT NOT NULL,
    created_at    TEXT NOT NULL,
    PRIMARY KEY(model_version, id)
);
CREATE INDEX IF NOT EXISTS ix_vec_model_ns ON mem_vectors(model_version, namespace);
