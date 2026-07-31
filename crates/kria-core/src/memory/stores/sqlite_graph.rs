//! SQLite-backed [`GraphStore`] (memory-upgrade design §16, ADR-004).
//!
//! After task F2.2.7 the legacy `relationships` table has been dropped and the
//! `Relationship` / `GraphHit` structs have been deleted.  The graph store now:
//!
//! * **Writes entities** through the authority transaction (unchanged).
//! * **Traverses** the `relationships_v2` table (entity endpoints only) with
//!   the same cycle-safe, visited-set, depth-capped BFS algorithm.
//!
//! Full graph traversal over typed v2 relationships (with policy, evidence
//! minimums, hidden-intermediary omission, etc.) is implemented in F3.3; this
//! file provides the structural foundation.

use std::collections::HashSet;
use std::sync::Arc;

use rusqlite::{params, Connection, Row};
use uuid::Uuid;

use crate::memory::db::{AuthorityTx, Database};
use crate::memory::error::{MemoryResult, StorageError};
use crate::memory::types::Entity;

use super::ports::GraphStore;

/// Hard cap on traversal depth regardless of caller input (design §16).
const MAX_HOPS_CAP: u8 = 3;

fn parse_uuid(s: &str) -> Result<Uuid, StorageError> {
    Uuid::parse_str(s).map_err(|e| StorageError::Serde(format!("bad uuid {s:?}: {e}")))
}
fn parse_ts(s: &str) -> Result<chrono::DateTime<chrono::Utc>, StorageError> {
    Ok(chrono::DateTime::parse_from_rfc3339(s)
        .map_err(|e| StorageError::Serde(format!("bad timestamp {s:?}: {e}")))?
        .with_timezone(&chrono::Utc))
}

fn row_to_entity(row: &Row<'_>) -> MemoryResult<Entity> {
    let id: String = row.get(0).map_err(StorageError::Sqlite)?;
    let canonical_id: String = row.get(1).map_err(StorageError::Sqlite)?;
    let entity_type: String = row.get(2).map_err(StorageError::Sqlite)?;
    let display_name: String = row.get(3).map_err(StorageError::Sqlite)?;
    let created_at: String = row.get(4).map_err(StorageError::Sqlite)?;
    Ok(Entity {
        id: parse_uuid(&id)?,
        canonical_id: parse_uuid(&canonical_id)?,
        entity_type,
        display_name,
        created_at: parse_ts(&created_at)?,
    })
}

const ENT_COLS: &str = "id, canonical_id, entity_type, display_name, created_at";

pub struct SqliteGraphStore {
    db: Arc<Database>,
}

impl SqliteGraphStore {
    pub fn new(db: Arc<Database>) -> Self {
        Self { db }
    }

    /// Batch neighbor lookup for a frontier of entity IDs — one SQL query for
    /// the whole frontier instead of one per node (eliminates N+1 BFS overhead).
    ///
    /// Returns a map `node_id → Vec<neighbor_id>`. Nodes with no edges are
    /// absent from the map (callers should treat absent as empty).
    ///
    /// The query builds a dynamic `IN (…)` clause from `frontier`. When the
    /// frontier is empty the function returns immediately without a DB round-trip.
    fn batch_neighbors_v2(
        &self,
        frontier: &[Uuid],
    ) -> MemoryResult<std::collections::HashMap<Uuid, Vec<Uuid>>> {
        if frontier.is_empty() {
            return Ok(std::collections::HashMap::new());
        }
        let n = frontier.len();
        // Build `IN (?,?,…)` for the current frontier — one placeholder per node.
        // We expand the frontier IDs twice in all_params: first for the source IN,
        // then for the target IN. SQLite uses positional `?` bindings — position 1..n
        // maps to the source set and position n+1..2n maps to the target set.
        let placeholder = (0..n).map(|_| "?").collect::<Vec<_>>().join(",");
        let sql = format!(
            "SELECT source_id, target_id, source_kind, target_kind \
             FROM relationships_v2 \
             WHERE (truth_state IS NULL \
                    OR truth_state NOT IN ('superseded','forgotten','deleted')) \
               AND valid_until IS NULL \
               AND ((source_kind = 'entity' AND source_id IN ({ph})) \
                    OR (target_kind = 'entity' AND target_id IN ({ph})))",
            ph = placeholder,
        );
        let id_strings: Vec<String> = frontier.iter().map(|u| u.to_string()).collect();

        self.db.with_read(|conn: &Connection| {
            let mut stmt = conn.prepare(&sql).map_err(StorageError::Sqlite)?;

            // Bind all frontier IDs twice (once for source IN, once for target IN).
            let all_params: Vec<&dyn rusqlite::ToSql> = id_strings
                .iter()
                .chain(id_strings.iter())
                .map(|s| s as &dyn rusqlite::ToSql)
                .collect();

            let rows = stmt
                .query_map(all_params.as_slice(), |r| {
                    Ok((
                        r.get::<_, String>(0)?, // source_id
                        r.get::<_, String>(1)?, // target_id
                        r.get::<_, String>(2)?, // source_kind
                        r.get::<_, String>(3)?, // target_kind
                    ))
                })
                .map_err(StorageError::Sqlite)?;

            let frontier_set: std::collections::HashSet<&str> =
                id_strings.iter().map(|s| s.as_str()).collect();
            let mut map: std::collections::HashMap<Uuid, Vec<Uuid>> =
                std::collections::HashMap::new();

            for row in rows {
                let (src, tgt, src_kind, tgt_kind) = row.map_err(StorageError::Sqlite)?;
                // Each row represents one edge. Add an adjacency entry for the
                // frontier node(s) that appear in the row.
                if src_kind == "entity" && frontier_set.contains(src.as_str()) {
                    if let (Ok(fid), Ok(nid)) = (Uuid::parse_str(&src), Uuid::parse_str(&tgt)) {
                        map.entry(fid).or_default().push(nid);
                    }
                }
                if tgt_kind == "entity" && frontier_set.contains(tgt.as_str()) {
                    if let (Ok(fid), Ok(nid)) = (Uuid::parse_str(&tgt), Uuid::parse_str(&src)) {
                        map.entry(fid).or_default().push(nid);
                    }
                }
            }
            Ok(map)
        })
    }
}

impl GraphStore for SqliteGraphStore {
    fn add_entity(&self, tx: &mut AuthorityTx<'_>, e: &Entity) -> MemoryResult<()> {
        tx.conn()
            .execute(
                "INSERT INTO entities(id, canonical_id, entity_type, display_name, created_at) \
                 VALUES(?1,?2,?3,?4,?5) ON CONFLICT(id) DO UPDATE SET \
                 canonical_id=excluded.canonical_id, entity_type=excluded.entity_type, \
                 display_name=excluded.display_name",
                params![
                    e.id.to_string(),
                    e.canonical_id.to_string(),
                    e.entity_type,
                    e.display_name,
                    e.created_at.to_rfc3339(),
                ],
            )
            .map_err(StorageError::Sqlite)?;
        Ok(())
    }

    fn neighbors(&self, root: Uuid, max_hops: u8) -> MemoryResult<Vec<(Uuid, u8)>> {
        let cap = max_hops.min(MAX_HOPS_CAP);
        let mut visited: HashSet<Uuid> = HashSet::new();
        visited.insert(root);

        // Frontier-level batch BFS: one SQL query per hop level instead of one
        // per node (eliminates the N+1 pattern that caused the 427ms regression).
        let mut current_frontier: Vec<Uuid> = vec![root];
        let mut hits: Vec<(Uuid, u8)> = Vec::new();

        for depth in 0..cap {
            if current_frontier.is_empty() {
                break;
            }
            // Single batched query for the entire frontier at this depth.
            let neighbor_map = self.batch_neighbors_v2(&current_frontier)?;
            let mut next_frontier: Vec<Uuid> = Vec::new();
            for neighbors in neighbor_map.into_values() {
                for other in neighbors {
                    if visited.contains(&other) {
                        continue; // visited-set → cycle-safe
                    }
                    visited.insert(other);
                    hits.push((other, depth + 1));
                    next_frontier.push(other);
                }
            }
            current_frontier = next_frontier;
        }
        Ok(hits)
    }

    fn search_entities(&self, query: &str) -> MemoryResult<Vec<Entity>> {
        self.db.with_read(|conn: &Connection| {
            let like = format!("%{query}%");
            let mut stmt = conn
                .prepare(&format!(
                    "SELECT {ENT_COLS} FROM entities WHERE display_name LIKE ?1 \
                     OR id IN (SELECT entity_id FROM entity_aliases WHERE alias LIKE ?1) \
                     LIMIT 50"
                ))
                .map_err(StorageError::Sqlite)?;
            let rows = stmt
                .query_map(params![like], |r| Ok(row_to_entity(r)))
                .map_err(StorageError::Sqlite)?;
            let mut out = Vec::new();
            for r in rows {
                out.push(r.map_err(StorageError::Sqlite)??);
            }
            Ok(out)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    fn entity(name: &str) -> Entity {
        Entity {
            id: crate::memory::ids::new_id(),
            canonical_id: crate::memory::ids::new_id(),
            entity_type: "concept".into(),
            display_name: name.into(),
            created_at: chrono::Utc::now(),
        }
    }

    /// Insert a `relationships_v2` row linking two entity endpoints (minimal
    /// required columns; no evidence required for structural traversal tests).
    /// Uses `INSERT OR IGNORE` so that property-based tests generating duplicate
    /// (source, target, rel_name) triples don't fail on the unique identity_hash
    /// constraint — the graph-traversal invariant still holds whether the edge
    /// was freshly inserted or already present.
    fn insert_v2_rel(db: &Arc<Database>, source: Uuid, target: Uuid, rel_name: &str) {
        use crate::memory::ids::new_id;
        let id = new_id();
        let now = chrono::Utc::now().to_rfc3339();
        let identity = format!("{source}-{target}-{rel_name}");
        let tx = db.begin().unwrap();
        tx.conn()
            .execute(
                "INSERT OR IGNORE INTO relationships_v2(
                     id, source_kind, source_id, target_kind, target_id,
                     relation_name, relation_version, direction_class,
                     valid_from, valid_until, truth_state,
                     namespace, owner_id, scope, sensitivity,
                     policy_source_id, policy_version, identity_hash)
                 VALUES (?1,'entity',?2,'entity',?3,?4,1,'directed',?5,NULL,NULL,
                         'core','','global',0,'core','pending-f1.4',?6)",
                params![
                    id.to_string(),
                    source.to_string(),
                    target.to_string(),
                    rel_name,
                    now,
                    identity,
                ],
            )
            .unwrap();
        tx.commit().unwrap();
    }

    #[test]
    fn two_hop_traversal_and_search() {
        let db = Arc::new(Database::open_in_memory().unwrap());
        let g = SqliteGraphStore::new(db.clone());
        let (mut a, mut b, mut c) = (entity("Alice"), entity("Bob"), entity("Carol"));
        a.canonical_id = a.id;
        b.canonical_id = b.id;
        c.canonical_id = c.id;

        let mut tx = db.begin().unwrap();
        for e in [&a, &b, &c] {
            g.add_entity(&mut tx, e).unwrap();
        }
        tx.commit().unwrap();

        insert_v2_rel(&db, a.id, b.id, "related_to");
        insert_v2_rel(&db, b.id, c.id, "related_to");

        // From Alice, 2 hops reaches Bob (d=1) and Carol (d=2).
        let hits = g.neighbors(a.id, 2).unwrap();
        let ids: Vec<Uuid> = hits.iter().map(|(id, _)| *id).collect();
        assert!(ids.contains(&b.id));
        assert!(ids.contains(&c.id));
        let carol_dist = hits
            .iter()
            .find(|(id, _)| *id == c.id)
            .map(|(_, d)| *d)
            .unwrap();
        assert_eq!(carol_dist, 2);

        // 1 hop reaches only Bob.
        let one = g.neighbors(a.id, 1).unwrap();
        assert_eq!(one.len(), 1);
        assert_eq!(one[0].0, b.id);

        assert_eq!(g.search_entities("Ali").unwrap().len(), 1);
    }

    /// B-01: batch BFS returns same results as the old per-node BFS for a
    /// well-known linear chain of depth 3.
    #[test]
    fn batch_bfs_linear_chain_three_hops() {
        let db = Arc::new(Database::open_in_memory().unwrap());
        let g = SqliteGraphStore::new(db.clone());
        let nodes: Vec<_> = (0..5)
            .map(|i| {
                let mut e = entity(&format!("node{i}"));
                e.canonical_id = e.id;
                e
            })
            .collect();
        let mut tx = db.begin().unwrap();
        for n in &nodes {
            g.add_entity(&mut tx, n).unwrap();
        }
        tx.commit().unwrap();
        // 0→1→2→3→4 chain
        for i in 0..4 {
            insert_v2_rel(&db, nodes[i].id, nodes[i + 1].id, "related_to");
        }
        // 3 hops from node0 should reach node1(d=1), node2(d=2), node3(d=3)
        let hits = g.neighbors(nodes[0].id, 3).unwrap();
        let dist: std::collections::HashMap<Uuid, u8> = hits.iter().cloned().collect();
        assert_eq!(dist[&nodes[1].id], 1);
        assert_eq!(dist[&nodes[2].id], 2);
        assert_eq!(dist[&nodes[3].id], 3);
        // node4 is at d=4, beyond the cap=3 limit
        assert!(
            !dist.contains_key(&nodes[4].id),
            "node4 is beyond 3-hop cap"
        );
    }

    /// B-02: batch BFS is cycle-safe on a bidirectional triangle.
    #[test]
    fn batch_bfs_cycle_triangle() {
        let db = Arc::new(Database::open_in_memory().unwrap());
        let g = SqliteGraphStore::new(db.clone());
        let (mut a, mut b, mut c) = (entity("A"), entity("B"), entity("C"));
        a.canonical_id = a.id;
        b.canonical_id = b.id;
        c.canonical_id = c.id;
        let mut tx = db.begin().unwrap();
        for n in [&a, &b, &c] {
            g.add_entity(&mut tx, n).unwrap();
        }
        tx.commit().unwrap();
        insert_v2_rel(&db, a.id, b.id, "related_to");
        insert_v2_rel(&db, b.id, c.id, "related_to");
        insert_v2_rel(&db, c.id, a.id, "related_to"); // closes the cycle

        let hits = g.neighbors(a.id, 3).unwrap();
        // Each node appears at most once
        let seen: std::collections::HashSet<Uuid> = hits.iter().map(|(id, _)| *id).collect();
        assert_eq!(seen.len(), hits.len(), "duplicate node in BFS output");
        assert!(seen.contains(&b.id));
        assert!(seen.contains(&c.id));
        // Root (a) must NOT appear in output
        assert!(!seen.contains(&a.id));
    }

    /// B-03: batch BFS with fan-out 10 at depth 2 (100 nodes) — verifies
    /// that all 100 neighbors are reached with correct depths.
    #[test]
    fn batch_bfs_wide_fan_out() {
        let db = Arc::new(Database::open_in_memory().unwrap());
        let g = SqliteGraphStore::new(db.clone());
        let root = {
            let mut e = entity("root");
            e.canonical_id = e.id;
            e
        };
        let mut tx = db.begin().unwrap();
        g.add_entity(&mut tx, &root).unwrap();
        // 10 d=1 children each with 10 d=2 grandchildren
        let mut d1_nodes = Vec::new();
        for i in 0..10usize {
            let mut n = entity(&format!("d1_{i}"));
            n.canonical_id = n.id;
            g.add_entity(&mut tx, &n).unwrap();
            d1_nodes.push(n);
        }
        let mut d2_nodes = Vec::new();
        for i in 0..10usize {
            for j in 0..10usize {
                let mut n = entity(&format!("d2_{i}_{j}"));
                n.canonical_id = n.id;
                g.add_entity(&mut tx, &n).unwrap();
                d2_nodes.push((i, n));
            }
        }
        tx.commit().unwrap();
        for n in &d1_nodes {
            insert_v2_rel(&db, root.id, n.id, "related_to");
        }
        for (i, n) in &d2_nodes {
            insert_v2_rel(&db, d1_nodes[*i].id, n.id, "related_to");
        }
        let hits = g.neighbors(root.id, 2).unwrap();
        let dist: std::collections::HashMap<Uuid, u8> = hits.iter().cloned().collect();
        // All 10 d=1 children
        for n in &d1_nodes {
            assert_eq!(dist[&n.id], 1, "d=1 node not at depth 1");
        }
        // All 100 d=2 grandchildren
        for (_, n) in &d2_nodes {
            assert_eq!(dist[&n.id], 2, "d=2 node not at depth 2");
        }
        assert_eq!(hits.len(), 110, "expected 10 + 100 nodes in 2-hop result");
    }

    /// B-04: batch BFS with an isolated node returns empty (no edges).
    #[test]
    fn batch_bfs_isolated_node_returns_empty() {
        let db = Arc::new(Database::open_in_memory().unwrap());
        let g = SqliteGraphStore::new(db.clone());
        let mut n = entity("lone");
        n.canonical_id = n.id;
        let mut tx = db.begin().unwrap();
        g.add_entity(&mut tx, &n).unwrap();
        tx.commit().unwrap();
        let hits = g.neighbors(n.id, 3).unwrap();
        assert!(hits.is_empty(), "isolated node should have no neighbors");
    }

    /// B-05: superseded/forgotten/deleted relationships are excluded by BFS.
    #[test]
    fn batch_bfs_excludes_dead_relationships() {
        let db = Arc::new(Database::open_in_memory().unwrap());
        let g = SqliteGraphStore::new(db.clone());
        let (mut a, mut b, mut c) = (entity("X"), entity("Y"), entity("Z"));
        a.canonical_id = a.id;
        b.canonical_id = b.id;
        c.canonical_id = c.id;
        let mut tx = db.begin().unwrap();
        for n in [&a, &b, &c] {
            g.add_entity(&mut tx, n).unwrap();
        }
        tx.commit().unwrap();
        // Active edge A→B
        insert_v2_rel(&db, a.id, b.id, "related_to");
        // Dead edge A→C — manually insert with truth_state='deleted'
        {
            let id = crate::memory::ids::new_id();
            let now = chrono::Utc::now().to_rfc3339();
            let identity = format!("{}-{}-dead", a.id, c.id);
            let tx2 = db.begin().unwrap();
            tx2.conn()
                .execute(
                    "INSERT OR IGNORE INTO relationships_v2(
                     id, source_kind, source_id, target_kind, target_id,
                     relation_name, relation_version, direction_class,
                     valid_from, valid_until, truth_state,
                     namespace, owner_id, scope, sensitivity,
                     policy_source_id, policy_version, identity_hash)
                 VALUES (?1,'entity',?2,'entity',?3,'related_to',1,'directed',?4,NULL,'deleted',
                         'core','','global',0,'core','pending-f1.4',?5)",
                    rusqlite::params![
                        id.to_string(),
                        a.id.to_string(),
                        c.id.to_string(),
                        now,
                        identity
                    ],
                )
                .unwrap();
            tx2.commit().unwrap();
        }
        let hits = g.neighbors(a.id, 1).unwrap();
        let ids: std::collections::HashSet<Uuid> = hits.iter().map(|(id, _)| *id).collect();
        assert!(ids.contains(&b.id), "live edge B should be reachable");
        assert!(!ids.contains(&c.id), "deleted edge C must be excluded");
    }

    /// B-06: nodes with valid_until set (expired) are excluded.
    #[test]
    fn batch_bfs_excludes_expired_relationships() {
        let db = Arc::new(Database::open_in_memory().unwrap());
        let g = SqliteGraphStore::new(db.clone());
        let (mut a, mut b, mut c) = (entity("P"), entity("Q"), entity("R"));
        a.canonical_id = a.id;
        b.canonical_id = b.id;
        c.canonical_id = c.id;
        let mut tx = db.begin().unwrap();
        for n in [&a, &b, &c] {
            g.add_entity(&mut tx, n).unwrap();
        }
        tx.commit().unwrap();
        // Active edge A→B
        insert_v2_rel(&db, a.id, b.id, "related_to");
        // Expired edge A→C (valid_until set to past)
        {
            let id = crate::memory::ids::new_id();
            let now = chrono::Utc::now().to_rfc3339();
            let past = "2020-01-01T00:00:00Z";
            let identity = format!("{}-{}-expired", a.id, c.id);
            let tx2 = db.begin().unwrap();
            tx2.conn()
                .execute(
                    "INSERT OR IGNORE INTO relationships_v2(
                     id, source_kind, source_id, target_kind, target_id,
                     relation_name, relation_version, direction_class,
                     valid_from, valid_until, truth_state,
                     namespace, owner_id, scope, sensitivity,
                     policy_source_id, policy_version, identity_hash)
                 VALUES (?1,'entity',?2,'entity',?3,'related_to',1,'directed',?4,?5,NULL,
                         'core','','global',0,'core','pending-f1.4',?6)",
                    rusqlite::params![
                        id.to_string(),
                        a.id.to_string(),
                        c.id.to_string(),
                        past,
                        now,
                        identity
                    ],
                )
                .unwrap();
            tx2.commit().unwrap();
        }
        let hits = g.neighbors(a.id, 1).unwrap();
        let ids: std::collections::HashSet<Uuid> = hits.iter().map(|(id, _)| *id).collect();
        assert!(ids.contains(&b.id), "active edge B should be reachable");
        assert!(!ids.contains(&c.id), "expired edge C must be excluded");
    }

    proptest! {
        /// CP-15: `neighbors()` terminates on any graph including cycles.
        #[test]
        fn neighbors_terminates_on_cyclic_graphs(
            n in 2usize..12,
            edges in proptest::collection::vec((0usize..12, 0usize..12), 0..40),
            max_hops in 0u8..6,
        ) {
            let db = Arc::new(Database::open_in_memory().unwrap());
            let g = SqliteGraphStore::new(db.clone());
            let mut ids = Vec::new();
            let mut tx = db.begin().unwrap();
            for i in 0..n {
                let mut e = entity(&format!("e{i}"));
                e.canonical_id = e.id;
                g.add_entity(&mut tx, &e).unwrap();
                ids.push(e.id);
            }
            tx.commit().unwrap();

            for (a, b) in edges {
                if a < n && b < n && a != b {
                    // Intentionally allow both directions → cycles.
                    insert_v2_rel(&db, ids[a], ids[b], "related_to");
                }
            }

            // Must terminate and never exceed the hop cap or revisit the root.
            let hits = g.neighbors(ids[0], max_hops).unwrap();
            for (id, dist) in &hits {
                prop_assert!(*dist as u8 <= max_hops.min(MAX_HOPS_CAP));
                prop_assert_ne!(*id, ids[0]);
            }
            // No entity appears twice (visited-set guarantee).
            let mut seen = std::collections::HashSet::new();
            for (id, _) in &hits {
                prop_assert!(seen.insert(*id), "entity visited twice");
            }
        }
    }
}
