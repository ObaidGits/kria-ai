//! MVP brute-force cosine [`VectorStore`] over SQLite (memory-upgrade task 9).
//!
//! Vectors are stored as little-endian f32 BLOBs, version-partitioned by
//! `model_version` (architecture §9/C4 — vectors from different models never
//! mix). Search loads the candidate partition (optionally namespace-filtered),
//! computes cosine similarity in Rust, and returns the top-k. At single-laptop
//! MVP scale (<~50k vectors) this is well within the retrieval latency budget;
//! the `VectorStore` trait keeps LanceDB a caller-transparent swap (D-1) when
//! scale demands ANN.

use std::sync::Arc;

use async_trait::async_trait;
use rusqlite::{params, Connection};
use uuid::Uuid;

use crate::memory::db::Database;
use crate::memory::error::{MemoryResult, StorageError};
use crate::memory::types::{
    ModelVersion, Scope, ScopeFilter, Sensitivity, VectorHit, VectorPayload,
};

use super::ports::VectorStore;

pub(crate) fn encode_vector(v: &[f32]) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(v.len() * 4);
    for f in v {
        bytes.extend_from_slice(&f.to_le_bytes());
    }
    bytes
}

pub(crate) fn decode_vector(bytes: &[u8]) -> Vec<f32> {
    bytes
        .chunks_exact(4)
        .map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]]))
        .collect()
}

/// Cosine similarity of two equal-length vectors. Returns 0 for degenerate
/// (zero-norm or mismatched-length) inputs.
pub(crate) fn cosine(a: &[f32], b: &[f32]) -> f32 {
    if a.len() != b.len() || a.is_empty() {
        return 0.0;
    }
    let mut dot = 0.0f32;
    let mut na = 0.0f32;
    let mut nb = 0.0f32;
    for i in 0..a.len() {
        dot += a[i] * b[i];
        na += a[i] * a[i];
        nb += b[i] * b[i];
    }
    let denom = na.sqrt() * nb.sqrt();
    if denom == 0.0 {
        0.0
    } else {
        dot / denom
    }
}

pub struct SqliteVectorStore {
    db: Arc<Database>,
}

impl SqliteVectorStore {
    pub fn new(db: Arc<Database>) -> Self {
        Self { db }
    }
}

#[async_trait]
impl VectorStore for SqliteVectorStore {
    async fn create_partition(&self, _model: &ModelVersion, _dim: usize) -> MemoryResult<()> {
        // Partitioning is by the `model_version` column; the table already
        // exists (migration 0002). No-op for the brute-force backend.
        Ok(())
    }

    async fn upsert(
        &self,
        model: &ModelVersion,
        id: Uuid,
        vector: &[f32],
        payload: &VectorPayload,
    ) -> MemoryResult<()> {
        let blob = encode_vector(vector);
        let tx = self.db.begin()?;
        tx.conn()
            .execute(
                "INSERT INTO mem_vectors(model_version, id, vector, namespace, scope, \
                 sensitivity, memory_type, content_hash, created_at) \
                 VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9) \
                 ON CONFLICT(model_version, id) DO UPDATE SET vector=excluded.vector, \
                 namespace=excluded.namespace, scope=excluded.scope, \
                 sensitivity=excluded.sensitivity, memory_type=excluded.memory_type, \
                 content_hash=excluded.content_hash",
                params![
                    model.as_str(),
                    id.to_string(),
                    blob,
                    payload.namespace,
                    payload.scope.as_str(),
                    payload.sensitivity.as_str(),
                    payload.memory_type.as_str(),
                    payload.content_hash,
                    payload.created_at.to_rfc3339(),
                ],
            )
            .map_err(StorageError::Sqlite)?;
        tx.commit()
    }

    async fn search(
        &self,
        model: &ModelVersion,
        query: &[f32],
        k: usize,
        filter: &ScopeFilter,
    ) -> MemoryResult<Vec<VectorHit>> {
        let model = model.clone();
        let query = query.to_vec();
        let filter = filter.clone();
        self.db.with_read(move |conn: &Connection| {
            let mut stmt = conn
                .prepare(
                    "SELECT id, vector, namespace, scope, sensitivity FROM mem_vectors \
                     WHERE model_version = ?1",
                )
                .map_err(StorageError::Sqlite)?;
            let rows = stmt
                .query_map(params![model.as_str()], |r| {
                    Ok((
                        r.get::<_, String>(0)?,
                        r.get::<_, Vec<u8>>(1)?,
                        r.get::<_, String>(2)?,
                        r.get::<_, String>(3)?,
                        r.get::<_, String>(4)?,
                    ))
                })
                .map_err(StorageError::Sqlite)?;

            let mut scored: Vec<VectorHit> = Vec::new();
            for row in rows {
                let (id, blob, ns, scope, sens) = row.map_err(StorageError::Sqlite)?;
                let scope: Scope = scope.parse().unwrap();
                let sens: Sensitivity = sens.parse().unwrap();
                // Enforce scope/namespace/sensitivity at the source (L7/D-20).
                if !filter.allows(&ns, &scope, &sens) {
                    continue;
                }
                let v = decode_vector(&blob);
                let score = cosine(&query, &v);
                let uuid = Uuid::parse_str(&id)
                    .map_err(|e| StorageError::Serde(format!("bad uuid: {e}")))?;
                scored.push(VectorHit { id: uuid, score });
            }
            scored.sort_by(|a, b| {
                b.score
                    .partial_cmp(&a.score)
                    .unwrap_or(std::cmp::Ordering::Equal)
            });
            scored.truncate(k);
            Ok(scored)
        })
    }

    async fn delete(&self, model: &ModelVersion, ids: &[Uuid]) -> MemoryResult<()> {
        let tx = self.db.begin()?;
        for id in ids {
            tx.conn()
                .execute(
                    "DELETE FROM mem_vectors WHERE model_version = ?1 AND id = ?2",
                    params![model.as_str(), id.to_string()],
                )
                .map_err(StorageError::Sqlite)?;
        }
        tx.commit()
    }

    async fn all_ids(&self, model: &ModelVersion) -> MemoryResult<Vec<Uuid>> {
        let model = model.clone();
        self.db.with_read(move |conn: &Connection| {
            let mut stmt = conn
                .prepare("SELECT id FROM mem_vectors WHERE model_version = ?1")
                .map_err(StorageError::Sqlite)?;
            let rows = stmt
                .query_map(params![model.as_str()], |r| r.get::<_, String>(0))
                .map_err(StorageError::Sqlite)?;
            let mut out = Vec::new();
            for r in rows {
                let s = r.map_err(StorageError::Sqlite)?;
                out.push(
                    Uuid::parse_str(&s)
                        .map_err(|e| StorageError::Serde(format!("bad uuid: {e}")))?,
                );
            }
            Ok(out)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn payload() -> VectorPayload {
        VectorPayload {
            namespace: "core".into(),
            scope: Scope::Global,
            sensitivity: Sensitivity::Private,
            memory_type: crate::memory::types::MemoryType::Semantic,
            content_hash: "h".into(),
            created_at: chrono::Utc::now(),
        }
    }

    #[tokio::test]
    async fn upsert_search_ranks_by_cosine() {
        let db = Arc::new(Database::open_in_memory().unwrap());
        let vs = SqliteVectorStore::new(db.clone());
        let model = ModelVersion("minilm_v1".into());
        let a = Uuid::now_v7();
        let b = Uuid::now_v7();
        // a aligns with the query, b is orthogonal.
        vs.upsert(&model, a, &[1.0, 0.0, 0.0], &payload())
            .await
            .unwrap();
        vs.upsert(&model, b, &[0.0, 1.0, 0.0], &payload())
            .await
            .unwrap();

        let hits = vs
            .search(&model, &[1.0, 0.0, 0.0], 10, &ScopeFilter::default())
            .await
            .unwrap();
        assert_eq!(hits.len(), 2);
        assert_eq!(hits[0].id, a);
        assert!(hits[0].score > hits[1].score);
    }

    #[tokio::test]
    async fn secret_is_filtered_and_delete_works() {
        let db = Arc::new(Database::open_in_memory().unwrap());
        let vs = SqliteVectorStore::new(db.clone());
        let model = ModelVersion("minilm_v1".into());
        let secret_id = Uuid::now_v7();
        let mut p = payload();
        p.sensitivity = Sensitivity::Secret;
        vs.upsert(&model, secret_id, &[1.0, 0.0], &p).await.unwrap();

        // Default filter excludes secret.
        let hits = vs
            .search(&model, &[1.0, 0.0], 10, &ScopeFilter::default())
            .await
            .unwrap();
        assert!(hits.is_empty());

        assert_eq!(vs.all_ids(&model).await.unwrap().len(), 1);
        vs.delete(&model, &[secret_id]).await.unwrap();
        assert!(vs.all_ids(&model).await.unwrap().is_empty());
    }
}
