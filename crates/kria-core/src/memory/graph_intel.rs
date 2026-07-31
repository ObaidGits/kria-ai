//! Knowledge-Graph Intelligence (memory-upgrade Phase 2).
//!
//! Autonomous analysis + inference over the `entities` / `relationships_v2`
//! tables (no new storage): degree centrality, community detection
//! (union-find), Adamic-Adar link prediction (hidden-relationship inference),
//! and transitive graph completion that materializes inferred edges through the
//! v2 governed path. Bounded for a personal-scale graph; runs as a P4
//! background job.
//!
//! After task F2.2.7 all graph intelligence queries target `relationships_v2`
//! (entity-endpoint rows only); the legacy `relationships` table has been
//! dropped.  The `complete_transitive` helper is removed — inferred edge
//! materialization for v2 is part of F3.3.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use rusqlite::params;
use uuid::Uuid;

use crate::memory::db::Database;
use crate::memory::error::{MemoryResult, StorageError};

/// A centrality result: entity + degree + display name.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CentralityHit {
    pub entity: Uuid,
    pub display_name: String,
    pub degree: usize,
}

/// A predicted (currently-absent) link and its Adamic-Adar score.
#[derive(Clone, Debug, PartialEq)]
pub struct LinkPrediction {
    pub target: Uuid,
    pub display_name: String,
    pub score: f64,
    pub shared_neighbors: usize,
}

/// Graph Intelligence engine over the authority database.
#[derive(Clone)]
pub struct GraphIntelligence {
    db: Arc<Database>,
}

impl GraphIntelligence {
    pub fn new(db: Arc<Database>) -> Self {
        Self { db }
    }

    /// Load the undirected adjacency of currently-valid entity-endpoint relationships
    /// from `relationships_v2`.
    fn adjacency(&self) -> MemoryResult<HashMap<Uuid, HashSet<Uuid>>> {
        self.db.with_read(|conn| {
            let mut stmt = conn
                .prepare(
                    "SELECT source_id, target_id FROM relationships_v2 \
                     WHERE source_kind = 'entity' AND target_kind = 'entity' \
                       AND valid_until IS NULL \
                       AND (truth_state IS NULL \
                            OR truth_state NOT IN ('superseded','forgotten','deleted'))",
                )
                .map_err(StorageError::Sqlite)?;
            let rows = stmt
                .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))
                .map_err(StorageError::Sqlite)?
                .collect::<Result<Vec<_>, _>>()
                .map_err(StorageError::Sqlite)?;
            let mut adj: HashMap<Uuid, HashSet<Uuid>> = HashMap::new();
            for (s, t) in rows {
                if let (Ok(s), Ok(t)) = (Uuid::parse_str(&s), Uuid::parse_str(&t)) {
                    if s == t {
                        continue;
                    }
                    adj.entry(s).or_default().insert(t);
                    adj.entry(t).or_default().insert(s);
                }
            }
            Ok(adj)
        })
    }

    fn display_name(&self, id: Uuid) -> MemoryResult<String> {
        self.db.with_read(|conn| {
            let name: Option<String> = conn
                .query_row(
                    "SELECT display_name FROM entities WHERE id = ?1",
                    params![id.to_string()],
                    |r| r.get(0),
                )
                .ok();
            Ok(name.unwrap_or_default())
        })
    }

    /// Degree centrality (most-connected entities first), top `limit`.
    /// Standalone entities are included with degree 0 so newly extracted
    /// knowledge appears before it gains relationships.
    pub fn degree_centrality(&self, limit: usize) -> MemoryResult<Vec<CentralityHit>> {
        self.db.with_read(|conn| {
            let mut stmt = conn
                .prepare(
                    "SELECT e.id, e.display_name, \
                     COUNT(DISTINCT CASE \
                       WHEN r.source_id = e.id THEN r.target_id \
                       ELSE r.source_id \
                     END) AS degree \
                     FROM entities e \
                     LEFT JOIN relationships_v2 r \
                       ON r.source_kind = 'entity' AND r.target_kind = 'entity' \
                      AND r.valid_until IS NULL \
                      AND (r.truth_state IS NULL \
                           OR r.truth_state NOT IN ('superseded','forgotten','deleted')) \
                      AND r.source_id <> r.target_id \
                      AND (r.source_id = e.id OR r.target_id = e.id) \
                     GROUP BY e.id, e.display_name \
                     ORDER BY degree DESC, e.id ASC \
                     LIMIT ?1",
                )
                .map_err(StorageError::Sqlite)?;
            let rows = stmt
                .query_map(params![limit as i64], |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, i64>(2)?,
                    ))
                })
                .map_err(StorageError::Sqlite)?
                .collect::<Result<Vec<_>, _>>()
                .map_err(StorageError::Sqlite)?;

            Ok(rows
                .into_iter()
                .filter_map(|(id, display_name, degree)| {
                    Uuid::parse_str(&id).ok().map(|entity| CentralityHit {
                        entity,
                        display_name,
                        degree: degree.max(0) as usize,
                    })
                })
                .collect())
        })
    }

    /// Community detection via union-find over the undirected graph. Returns
    /// each community as a sorted list of entity ids (singletons excluded).
    pub fn communities(&self) -> MemoryResult<Vec<Vec<Uuid>>> {
        let adj = self.adjacency()?;
        let mut parent: HashMap<Uuid, Uuid> = adj.keys().map(|k| (*k, *k)).collect();

        fn find(parent: &mut HashMap<Uuid, Uuid>, x: Uuid) -> Uuid {
            let mut root = x;
            while parent[&root] != root {
                root = parent[&root];
            }
            // Path compression.
            let mut cur = x;
            while parent[&cur] != root {
                let next = parent[&cur];
                parent.insert(cur, root);
                cur = next;
            }
            root
        }

        for (a, neigh) in &adj {
            for b in neigh {
                let ra = find(&mut parent, *a);
                let rb = find(&mut parent, *b);
                if ra != rb {
                    parent.insert(ra, rb);
                }
            }
        }
        let mut groups: HashMap<Uuid, Vec<Uuid>> = HashMap::new();
        let keys: Vec<Uuid> = parent.keys().copied().collect();
        for k in keys {
            let root = find(&mut parent, k);
            groups.entry(root).or_default().push(k);
        }
        let mut out: Vec<Vec<Uuid>> = groups
            .into_values()
            .filter(|g| g.len() > 1)
            .map(|mut g| {
                g.sort();
                g
            })
            .collect();
        out.sort_by(|a, b| b.len().cmp(&a.len()).then(a[0].cmp(&b[0])));
        Ok(out)
    }

    /// Adamic-Adar link prediction: score candidate non-neighbors of `entity`
    /// by shared-neighbor rarity `Σ 1/ln(deg(z))`. Higher = more likely a real
    /// but currently-missing link (hidden relationship inference).
    pub fn predict_links(&self, entity: Uuid, limit: usize) -> MemoryResult<Vec<LinkPrediction>> {
        let adj = self.adjacency()?;
        let Some(neighbors) = adj.get(&entity) else {
            return Ok(Vec::new());
        };
        let mut scores: HashMap<Uuid, (f64, usize)> = HashMap::new();
        for z in neighbors {
            let Some(z_neigh) = adj.get(z) else { continue };
            let deg = z_neigh.len().max(2);
            let w = 1.0 / (deg as f64).ln();
            for c in z_neigh {
                if *c == entity || neighbors.contains(c) {
                    continue; // already linked or self
                }
                let e = scores.entry(*c).or_insert((0.0, 0));
                e.0 += w;
                e.1 += 1;
            }
        }
        let mut preds: Vec<LinkPrediction> = Vec::with_capacity(scores.len());
        for (target, (score, shared)) in scores {
            preds.push(LinkPrediction {
                target,
                display_name: self.display_name(target)?,
                score,
                shared_neighbors: shared,
            });
        }
        preds.sort_by(|a, b| {
            b.score
                .partial_cmp(&a.score)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then(a.target.cmp(&b.target))
        });
        preds.truncate(limit);
        Ok(preds)
    }

    // NOTE: `complete_transitive` was removed in task F2.2.7. Transitive edge
    // materialization over the v2 governed path is part of F3.3.
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::memory::ids::new_id;
    use crate::memory::stores::ports::GraphStore;
    use crate::memory::stores::SqliteGraphStore;
    use crate::memory::types::Entity;

    fn entity(name: &str) -> Entity {
        let id = new_id();
        Entity {
            id,
            canonical_id: id,
            entity_type: "concept".into(),
            display_name: name.into(),
            created_at: chrono::Utc::now(),
        }
    }

    /// Insert a `relationships_v2` entity-to-entity edge.
    fn insert_v2_rel(db: &Arc<Database>, source: Uuid, target: Uuid, rel_name: &str) {
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
                rusqlite::params![
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

    fn seed() -> (Arc<Database>, Vec<Uuid>) {
        let db = Arc::new(Database::open_in_memory().unwrap());
        let g = SqliteGraphStore::new(db.clone());
        let ents: Vec<Entity> = ["A", "B", "C", "D", "E"]
            .iter()
            .map(|n| entity(n))
            .collect();
        let mut tx = db.begin().unwrap();
        for e in &ents {
            g.add_entity(&mut tx, e).unwrap();
        }
        tx.commit().unwrap();
        // Star-ish: A-B, A-C, B-C (triangle), C-D, D-E.
        insert_v2_rel(&db, ents[0].id, ents[1].id, "related_to");
        insert_v2_rel(&db, ents[0].id, ents[2].id, "related_to");
        insert_v2_rel(&db, ents[1].id, ents[2].id, "related_to");
        insert_v2_rel(&db, ents[2].id, ents[3].id, "related_to");
        insert_v2_rel(&db, ents[3].id, ents[4].id, "related_to");
        (db, ents.iter().map(|e| e.id).collect())
    }

    #[test]
    fn centrality_and_communities() {
        let (db, ids) = seed();
        let gi = GraphIntelligence::new(db);
        let cent = gi.degree_centrality(10).unwrap();
        // C has degree 3 (A,B,D) → top.
        assert_eq!(cent[0].entity, ids[2]);
        assert_eq!(cent[0].degree, 3);
        // All 5 are one connected component.
        let comms = gi.communities().unwrap();
        assert_eq!(comms.len(), 1);
        assert_eq!(comms[0].len(), 5);
    }

    #[test]
    fn link_prediction_finds_missing_edge() {
        let (db, ids) = seed();
        let gi = GraphIntelligence::new(db);
        // D's non-neighbors sharing neighbors: A and B share C with D → predicted.
        let preds = gi.predict_links(ids[3], 10).unwrap();
        let targets: Vec<Uuid> = preds.iter().map(|p| p.target).collect();
        assert!(targets.contains(&ids[0]) || targets.contains(&ids[1]));
        assert!(preds.iter().all(|p| p.score > 0.0));
    }
}
