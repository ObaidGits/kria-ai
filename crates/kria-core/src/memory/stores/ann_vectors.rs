//! ANN [`VectorStore`] (memory-upgrade H2): an in-process HNSW index over
//! model-partitioned vectors, with SQLite as the durable authority and a
//! brute-force fallback for tiny partitions.
//!
//! Why: the brute-force [`SqliteVectorStore`](super::sqlite_vectors::SqliteVectorStore)
//! reloads *every* vector BLOB from SQLite and scores it on *every* search —
//! fine below ~50k vectors, seconds at 100k/1M. Grounding runs a vector search
//! on every turn, so that cost is on the hot path.
//!
//! Design (architecture §9/C4 — vectors from different models never mix):
//! - SQLite (`mem_vectors`) stays the **durable authority** — `upsert`/`delete`/
//!   `all_ids` behave exactly like the brute-force store, so persistence,
//!   restart recovery, and reconciliation (D-16) are unchanged.
//! - Each `model_version` gets an in-memory [`Partition`] holding an HNSW index
//!   (`hnsw_rs`, cosine) + decoded vectors + scope metadata, lazily built from
//!   SQLite on first touch (so a restart rebuilds the index from the authority).
//! - A partition with `<= BRUTE_FORCE_MAX` live vectors is scored by exact
//!   in-RAM brute force (no ANN error, still no SQLite reload); larger
//!   partitions use HNSW with over-fetch + exact re-rank so results stay
//!   ordered and scope-filtered.
//! - HNSW has no native delete/update, so removals/overwrites tombstone the old
//!   node; the partition compacts (rebuilds from live metadata) once tombstones
//!   dominate.

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, RwLock};

use async_trait::async_trait;
use dashmap::DashMap;
use hnsw_rs::prelude::{DistCosine, Hnsw};
use rusqlite::{params, Connection};
use uuid::Uuid;

use crate::memory::db::Database;
use crate::memory::error::{MemoryResult, StorageError};
use crate::memory::types::{
    ModelVersion, Scope, ScopeFilter, Sensitivity, VectorHit, VectorPayload,
};

use super::ports::VectorStore;
use super::sqlite_vectors::{cosine, decode_vector, encode_vector};

/// Partitions with at most this many live vectors are scored by exact brute
/// force (ANN overhead/error is not worth it at this size).
const BRUTE_FORCE_MAX: usize = 256;
/// HNSW graph degree (`max_nb_connection`) — must be `<= 256` for `hnsw_rs`.
const HNSW_M: usize = 16;
/// HNSW `ef_construction` (build-time candidate list).
const HNSW_EF_CONSTRUCTION: usize = 200;
/// HNSW maximum layer count.
const HNSW_MAX_LAYER: usize = 16;
/// Query-time candidate list floor (`ef_search`); over-fetch guards recall
/// under scope filtering + tombstones.
const HNSW_EF_SEARCH: usize = 64;

/// Per-vector metadata retained in RAM for filtering + exact re-rank/brute force.
struct VecMeta {
    id: Uuid,
    vector: Vec<f32>,
    namespace: String,
    scope: Scope,
    sensitivity: Sensitivity,
}

/// In-memory index for a single `model_version` partition.
struct Partition {
    hnsw: Hnsw<'static, f32, DistCosine>,
    capacity: usize,
    /// Next internal HNSW id to assign.
    next_idx: usize,
    /// Uuid → live internal idx.
    id_to_idx: HashMap<Uuid, usize>,
    /// Live internal idx → metadata.
    meta: HashMap<usize, VecMeta>,
    /// Internal ids that were deleted/overwritten (HNSW can't remove nodes).
    tombstones: HashSet<usize>,
}

impl Partition {
    fn new_hnsw(capacity: usize) -> Hnsw<'static, f32, DistCosine> {
        Hnsw::new(
            HNSW_M,
            capacity.max(HNSW_EF_CONSTRUCTION),
            HNSW_MAX_LAYER,
            HNSW_EF_CONSTRUCTION,
            DistCosine {},
        )
    }

    fn empty(capacity: usize) -> Self {
        Self {
            hnsw: Self::new_hnsw(capacity),
            capacity,
            next_idx: 0,
            id_to_idx: HashMap::new(),
            meta: HashMap::new(),
            tombstones: HashSet::new(),
        }
    }

    #[inline]
    fn live(&self) -> usize {
        self.meta.len()
    }

    /// Insert (or overwrite) a vector. Overwrite tombstones the previous node.
    fn insert(
        &mut self,
        id: Uuid,
        vector: Vec<f32>,
        namespace: String,
        scope: Scope,
        sensitivity: Sensitivity,
    ) {
        if let Some(old) = self.id_to_idx.remove(&id) {
            self.meta.remove(&old);
            self.tombstones.insert(old);
        }
        let idx = self.next_idx;
        self.next_idx += 1;
        self.hnsw.insert((vector.as_slice(), idx));
        self.id_to_idx.insert(id, idx);
        self.meta.insert(
            idx,
            VecMeta {
                id,
                vector,
                namespace,
                scope,
                sensitivity,
            },
        );
        // Compact once dead nodes dominate the live set (keeps HNSW healthy).
        if self.tombstones.len() > self.live().max(BRUTE_FORCE_MAX) {
            self.rebuild();
        }
    }

    fn delete(&mut self, id: &Uuid) {
        if let Some(idx) = self.id_to_idx.remove(id) {
            self.meta.remove(&idx);
            self.tombstones.insert(idx);
        }
    }

    /// Rebuild the HNSW graph from the live metadata (drops all tombstones).
    fn rebuild(&mut self) {
        let cap = self.live().next_power_of_two().max(HNSW_EF_CONSTRUCTION);
        let hnsw = Self::new_hnsw(cap);
        let mut id_to_idx = HashMap::with_capacity(self.live());
        let mut meta = HashMap::with_capacity(self.live());
        let mut next_idx = 0usize;
        // Drain the old meta so we can move vectors without cloning.
        let old_meta = std::mem::take(&mut self.meta);
        for (_, m) in old_meta {
            let idx = next_idx;
            next_idx += 1;
            hnsw.insert((m.vector.as_slice(), idx));
            id_to_idx.insert(m.id, idx);
            meta.insert(idx, m);
        }
        self.hnsw = hnsw;
        self.capacity = cap;
        self.next_idx = next_idx;
        self.id_to_idx = id_to_idx;
        self.meta = meta;
        self.tombstones.clear();
    }

    /// Exact brute-force top-k over the live set (used for small partitions and
    /// as the ANN re-rank / recall guard).
    fn brute_force(&self, query: &[f32], k: usize, filter: &ScopeFilter) -> Vec<VectorHit> {
        let mut scored: Vec<VectorHit> = self
            .meta
            .values()
            .filter(|m| filter.allows(&m.namespace, &m.scope, &m.sensitivity))
            .map(|m| VectorHit {
                id: m.id,
                score: cosine(query, &m.vector),
            })
            .collect();
        sort_truncate(&mut scored, k);
        scored
    }

    fn ann_search(&self, query: &[f32], k: usize, filter: &ScopeFilter) -> Vec<VectorHit> {
        // Over-fetch so scope filtering + tombstones don't starve the top-k.
        let ef = HNSW_EF_SEARCH.max(k * 4);
        let want = (k * 4).max(k + 32);
        let neighbours = self.hnsw.search(query, want, ef);
        let mut scored: Vec<VectorHit> = Vec::with_capacity(neighbours.len());
        for n in neighbours {
            let idx = n.get_origin_id();
            if self.tombstones.contains(&idx) {
                continue;
            }
            let Some(m) = self.meta.get(&idx) else {
                continue;
            };
            if !filter.allows(&m.namespace, &m.scope, &m.sensitivity) {
                continue;
            }
            // `DistCosine` is `1 - cosine_similarity`; recover the similarity so
            // scores match the brute-force store (higher = closer).
            scored.push(VectorHit {
                id: m.id,
                score: 1.0 - n.distance,
            });
        }
        sort_truncate(&mut scored, k);
        scored
    }
}

fn sort_truncate(scored: &mut Vec<VectorHit>, k: usize) {
    scored.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    scored.truncate(k);
}

/// ANN-backed vector store. Cheap to clone (shares the partition map + db).
pub struct AnnVectorStore {
    db: Arc<Database>,
    partitions: Arc<DashMap<String, Arc<RwLock<Partition>>>>,
}

impl AnnVectorStore {
    pub fn new(db: Arc<Database>) -> Self {
        Self {
            db,
            partitions: Arc::new(DashMap::new()),
        }
    }

    /// Get (or lazily build from the SQLite authority) the in-memory partition
    /// for a model version.
    fn partition(&self, model: &ModelVersion) -> MemoryResult<Arc<RwLock<Partition>>> {
        let key = model.as_str().to_string();
        if let Some(p) = self.partitions.get(&key) {
            return Ok(p.clone());
        }
        let built = self.load_partition(model)?;
        // Another thread may have inserted concurrently — keep the first winner.
        let entry = self
            .partitions
            .entry(key)
            .or_insert_with(|| Arc::new(RwLock::new(built)));
        Ok(entry.clone())
    }

    /// Build a partition by streaming the model's rows out of SQLite.
    fn load_partition(&self, model: &ModelVersion) -> MemoryResult<Partition> {
        let model = model.clone();
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
            let mut collected: Vec<(Uuid, Vec<f32>, String, Scope, Sensitivity)> = Vec::new();
            for row in rows {
                let (id, blob, ns, scope, sens) = row.map_err(StorageError::Sqlite)?;
                let id = Uuid::parse_str(&id)
                    .map_err(|e| StorageError::Serde(format!("bad uuid: {e}")))?;
                let scope: Scope = scope.parse().unwrap_or(Scope::Global);
                let sens: Sensitivity = sens.parse().unwrap_or(Sensitivity::Private);
                collected.push((id, decode_vector(&blob), ns, scope, sens));
            }
            let mut part = Partition::empty(collected.len().next_power_of_two());
            for (id, vector, ns, scope, sens) in collected {
                part.insert(id, vector, ns, scope, sens);
            }
            Ok(part)
        })
    }
}

#[async_trait]
impl VectorStore for AnnVectorStore {
    async fn create_partition(&self, model: &ModelVersion, _dim: usize) -> MemoryResult<()> {
        // Allocate the in-memory partition eagerly; the SQLite table already
        // exists (migration 0002).
        self.partition(model)?;
        Ok(())
    }

    async fn upsert(
        &self,
        model: &ModelVersion,
        id: Uuid,
        vector: &[f32],
        payload: &VectorPayload,
    ) -> MemoryResult<()> {
        // 1) Durable authority write (identical to the brute-force store).
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
        tx.commit()?;

        // 2) Update the in-memory index.
        let part = self.partition(model)?;
        let mut guard = part.write().unwrap_or_else(|p| p.into_inner());
        guard.insert(
            id,
            vector.to_vec(),
            payload.namespace.clone(),
            payload.scope.clone(),
            payload.sensitivity.clone(),
        );
        Ok(())
    }

    async fn search(
        &self,
        model: &ModelVersion,
        query: &[f32],
        k: usize,
        filter: &ScopeFilter,
    ) -> MemoryResult<Vec<VectorHit>> {
        if k == 0 {
            return Ok(Vec::new());
        }
        let part = self.partition(model)?;
        let guard = part.read().unwrap_or_else(|p| p.into_inner());
        if guard.live() <= BRUTE_FORCE_MAX {
            return Ok(guard.brute_force(query, k, filter));
        }
        let mut hits = guard.ann_search(query, k, filter);
        // Recall guard: if scope filtering starved the ANN candidate set below
        // k while more live vectors exist, fall back to exact brute force so a
        // filtered query never silently loses results.
        if hits.len() < k && guard.live() > hits.len() {
            hits = guard.brute_force(query, k, filter);
        }
        Ok(hits)
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
        tx.commit()?;

        let part = self.partition(model)?;
        let mut guard = part.write().unwrap_or_else(|p| p.into_inner());
        for id in ids {
            guard.delete(id);
        }
        Ok(())
    }

    async fn all_ids(&self, model: &ModelVersion) -> MemoryResult<Vec<Uuid>> {
        // Authority = SQLite (reconciliation set-difference, D-16).
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
    use crate::memory::types::MemoryType;

    fn payload() -> VectorPayload {
        VectorPayload {
            namespace: "core".into(),
            scope: Scope::Global,
            sensitivity: Sensitivity::Private,
            memory_type: MemoryType::Semantic,
            content_hash: "h".into(),
            created_at: chrono::Utc::now(),
        }
    }

    /// Deterministic PRNG so recall/latency tests are reproducible (no `rand`).
    struct Lcg(u64);
    impl Lcg {
        fn next_unit(&mut self) -> f32 {
            self.0 = self
                .0
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            ((self.0 >> 40) as f32) / ((1u64 << 24) as f32) * 2.0 - 1.0
        }
        fn vector(&mut self, dim: usize) -> Vec<f32> {
            (0..dim).map(|_| self.next_unit()).collect()
        }
    }

    fn brute_force_topk(vectors: &[(Uuid, Vec<f32>)], query: &[f32], k: usize) -> Vec<Uuid> {
        let mut scored: Vec<(Uuid, f32)> = vectors
            .iter()
            .map(|(id, v)| (*id, cosine(query, v)))
            .collect();
        scored.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        scored.truncate(k);
        scored.into_iter().map(|(id, _)| id).collect()
    }

    #[tokio::test]
    async fn upsert_search_ranks_by_cosine_small_partition() {
        // Small partition → exact brute-force path (still no SQLite reload).
        let db = Arc::new(Database::open_in_memory().unwrap());
        let vs = AnnVectorStore::new(db.clone());
        let model = ModelVersion("minilm_v1".into());
        let a = Uuid::now_v7();
        let b = Uuid::now_v7();
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
        let vs = AnnVectorStore::new(db.clone());
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

    #[tokio::test]
    async fn overwrite_updates_vector_in_index() {
        let db = Arc::new(Database::open_in_memory().unwrap());
        let vs = AnnVectorStore::new(db.clone());
        let model = ModelVersion("m".into());
        let id = Uuid::now_v7();
        vs.upsert(&model, id, &[1.0, 0.0], &payload())
            .await
            .unwrap();
        // Overwrite with an orthogonal vector.
        vs.upsert(&model, id, &[0.0, 1.0], &payload())
            .await
            .unwrap();
        // Only one row (upsert, not insert-duplicate).
        assert_eq!(vs.all_ids(&model).await.unwrap().len(), 1);
        let hits = vs
            .search(&model, &[0.0, 1.0], 5, &ScopeFilter::default())
            .await
            .unwrap();
        assert_eq!(hits.len(), 1);
        assert!(hits[0].score > 0.9, "reflects the overwritten vector");
    }

    #[tokio::test]
    async fn survives_reload_from_sqlite_authority() {
        // A fresh store (simulating restart) rebuilds its index from SQLite.
        let db = Arc::new(Database::open_in_memory().unwrap());
        let model = ModelVersion("m".into());
        let a = Uuid::now_v7();
        {
            let vs = AnnVectorStore::new(db.clone());
            vs.upsert(&model, a, &[1.0, 0.0, 0.0], &payload())
                .await
                .unwrap();
        }
        // New instance, same durable DB.
        let vs2 = AnnVectorStore::new(db.clone());
        let hits = vs2
            .search(&model, &[1.0, 0.0, 0.0], 5, &ScopeFilter::default())
            .await
            .unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].id, a);
    }

    #[tokio::test]
    async fn ann_recall_matches_brute_force_at_scale() {
        // Large partition (> BRUTE_FORCE_MAX) → HNSW path. Assert its top-k
        // overlaps the exact brute-force top-k (approximate recall parity).
        let db = Arc::new(Database::open_in_memory().unwrap());
        let vs = AnnVectorStore::new(db.clone());
        let model = ModelVersion("scale".into());
        let dim = 48;
        let n = 1500usize;
        let mut rng = Lcg(0x1234_5678);
        let mut reference: Vec<(Uuid, Vec<f32>)> = Vec::with_capacity(n);
        for _ in 0..n {
            let id = Uuid::now_v7();
            let v = rng.vector(dim);
            vs.upsert(&model, id, &v, &payload()).await.unwrap();
            reference.push((id, v));
        }
        assert!(n > BRUTE_FORCE_MAX, "exercises the ANN path");

        let k = 10;
        let mut total_overlap = 0usize;
        let queries = 5;
        for _ in 0..queries {
            let q = rng.vector(dim);
            let expected: std::collections::HashSet<Uuid> =
                brute_force_topk(&reference, &q, k).into_iter().collect();
            let ann = vs
                .search(&model, &q, k, &ScopeFilter::default())
                .await
                .unwrap();
            assert_eq!(ann.len(), k, "ANN returns a full top-k");
            let overlap = ann.iter().filter(|h| expected.contains(&h.id)).count();
            total_overlap += overlap;
        }
        // HNSW is approximate; require strong recall (>= 70% averaged).
        let recall = total_overlap as f32 / (k * queries) as f32;
        assert!(recall >= 0.7, "ANN recall {recall} too low vs brute force");
    }

    #[tokio::test]
    async fn ann_scale_query_respects_scope_filter() {
        // AUD-05 regression: at ANN scale (> BRUTE_FORCE_MAX), a scope filter
        // must never leak excluded (secret) vectors — the recall-guard fallback
        // preserves correctness, not just latency.
        let db = Arc::new(Database::open_in_memory().unwrap());
        let vs = AnnVectorStore::new(db.clone());
        let model = ModelVersion("scope".into());
        let dim = 24;
        let mut rng = Lcg(7);
        let mut secret_ids = std::collections::HashSet::new();
        for i in 0..800 {
            let id = Uuid::now_v7();
            let mut p = payload();
            if i % 5 == 0 {
                p.sensitivity = Sensitivity::Secret;
                secret_ids.insert(id);
            }
            vs.upsert(&model, id, &rng.vector(dim), &p).await.unwrap();
        }
        // Default filter excludes Secret. Run several queries; NO secret id may
        // ever surface, and results stay within the non-secret population.
        for _ in 0..5 {
            let hits = vs
                .search(&model, &rng.vector(dim), 10, &ScopeFilter::default())
                .await
                .unwrap();
            assert!(
                hits.iter().all(|h| !secret_ids.contains(&h.id)),
                "scope filter must exclude secret vectors even on the ANN path"
            );
        }
    }

    #[tokio::test]
    async fn ann_path_returns_topk_after_deletes() {
        // Delete a chunk then confirm the ANN path still returns k live hits
        // (tombstones don't corrupt results).
        let db = Arc::new(Database::open_in_memory().unwrap());
        let vs = AnnVectorStore::new(db.clone());
        let model = ModelVersion("del".into());
        let dim = 16;
        let mut rng = Lcg(42);
        let mut ids = Vec::new();
        for _ in 0..600 {
            let id = Uuid::now_v7();
            vs.upsert(&model, id, &rng.vector(dim), &payload())
                .await
                .unwrap();
            ids.push(id);
        }
        // Delete the first 100.
        vs.delete(&model, &ids[..100]).await.unwrap();
        assert_eq!(vs.all_ids(&model).await.unwrap().len(), 500);
        let deleted: std::collections::HashSet<Uuid> = ids[..100].iter().copied().collect();
        let hits = vs
            .search(&model, &rng.vector(dim), 10, &ScopeFilter::default())
            .await
            .unwrap();
        assert_eq!(hits.len(), 10);
        assert!(
            hits.iter().all(|h| !deleted.contains(&h.id)),
            "no tombstoned id is ever returned"
        );
    }
}
