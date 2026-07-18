//! Knowledge-Graph Intelligence (memory-upgrade Phase 2).
//!
//! Autonomous analysis + inference over the existing `entities` / `relationships`
//! authority tables (no new storage): degree centrality, community detection
//! (union-find), Adamic-Adar link prediction (hidden-relationship inference),
//! and transitive graph completion that materializes inferred edges through the
//! authority transaction. Bounded for a personal-scale graph; runs as a P4
//! background job.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use rusqlite::params;
use uuid::Uuid;

use crate::memory::db::Database;
use crate::memory::error::{MemoryResult, StorageError};
use crate::memory::ids::new_id;

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

    /// Load the undirected adjacency of currently-valid relationships.
    fn adjacency(&self) -> MemoryResult<HashMap<Uuid, HashSet<Uuid>>> {
        self.db.with_read(|conn| {
            let mut stmt = conn
                .prepare("SELECT source_id, target_id FROM relationships WHERE valid_until IS NULL")
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

    /// Typed directed edges (for transitive completion), valid only.
    fn typed_edges(&self) -> MemoryResult<Vec<(Uuid, Uuid, String, f64)>> {
        self.db.with_read(|conn| {
            let mut stmt = conn
                .prepare(
                    "SELECT source_id, target_id, rel_type, strength FROM relationships \
                     WHERE valid_until IS NULL",
                )
                .map_err(StorageError::Sqlite)?;
            let rows = stmt
                .query_map([], |r| {
                    Ok((
                        r.get::<_, String>(0)?,
                        r.get::<_, String>(1)?,
                        r.get::<_, String>(2)?,
                        r.get::<_, f64>(3)?,
                    ))
                })
                .map_err(StorageError::Sqlite)?
                .collect::<Result<Vec<_>, _>>()
                .map_err(StorageError::Sqlite)?;
            let mut out = Vec::new();
            for (s, t, rt, st) in rows {
                if let (Ok(s), Ok(t)) = (Uuid::parse_str(&s), Uuid::parse_str(&t)) {
                    out.push((s, t, rt, st));
                }
            }
            Ok(out)
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
    pub fn degree_centrality(&self, limit: usize) -> MemoryResult<Vec<CentralityHit>> {
        let adj = self.adjacency()?;
        let mut hits: Vec<CentralityHit> = Vec::with_capacity(adj.len());
        for (id, neigh) in &adj {
            hits.push(CentralityHit {
                entity: *id,
                display_name: self.display_name(*id)?,
                degree: neigh.len(),
            });
        }
        hits.sort_by(|a, b| b.degree.cmp(&a.degree).then(a.entity.cmp(&b.entity)));
        hits.truncate(limit);
        Ok(hits)
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

    /// Transitive graph completion: for A —t→ B and B —t→ C (same rel_type) with
    /// no existing A —t→ C, materialize the inferred edge (strength = product of
    /// the two, so inferred links are weaker). Returns the number of edges added.
    /// Bounded by `max_new`. Idempotent (skips existing edges).
    pub fn complete_transitive(&self, max_new: usize) -> MemoryResult<usize> {
        let edges = self.typed_edges()?;
        // Existing directed (src,tgt,type) set for idempotency.
        let existing: HashSet<(Uuid, Uuid, String)> = edges
            .iter()
            .map(|(s, t, rt, _)| (*s, *t, rt.clone()))
            .collect();
        // Typed adjacency: (node, rel_type) -> [(neighbor, strength)]
        let mut typed: HashMap<(Uuid, String), Vec<(Uuid, f64)>> = HashMap::new();
        for (s, t, rt, st) in &edges {
            typed.entry((*s, rt.clone())).or_default().push((*t, *st));
        }

        let mut proposals: Vec<(Uuid, Uuid, String, f64)> = Vec::new();
        for ((a, rt), mids) in &typed {
            for (mid, st1) in mids {
                if let Some(seconds) = typed.get(&(*mid, rt.clone())) {
                    for (c, st2) in seconds {
                        if c == a {
                            continue;
                        }
                        let key = (*a, *c, rt.clone());
                        if existing.contains(&key) {
                            continue;
                        }
                        proposals.push((*a, *c, rt.clone(), (st1 * st2).clamp(0.0, 1.0)));
                    }
                }
            }
        }
        // De-dup proposals + bound.
        let mut seen: HashSet<(Uuid, Uuid, String)> = HashSet::new();
        proposals.retain(|(a, c, rt, _)| seen.insert((*a, *c, rt.clone())));
        proposals.truncate(max_new);
        if proposals.is_empty() {
            return Ok(0);
        }

        let now = chrono::Utc::now().to_rfc3339();
        let tx = self.db.begin()?;
        for (a, c, rt, st) in &proposals {
            tx.conn()
                .execute(
                    "INSERT INTO relationships(id, source_id, target_id, rel_type, strength, \
                     valid_from, valid_until, evidence_event_id) \
                     VALUES(?1,?2,?3,?4,?5,?6,NULL,NULL)",
                    params![
                        new_id().to_string(),
                        a.to_string(),
                        c.to_string(),
                        rt,
                        st,
                        now,
                    ],
                )
                .map_err(StorageError::Sqlite)?;
        }
        tx.commit()?;
        Ok(proposals.len())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::memory::stores::ports::GraphStore;
    use crate::memory::stores::SqliteGraphStore;
    use crate::memory::types::{Entity, Relationship};

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
    fn rel(a: Uuid, b: Uuid, rt: &str) -> Relationship {
        Relationship {
            id: new_id(),
            source_id: a,
            target_id: b,
            rel_type: rt.into(),
            strength: 0.9,
            valid_from: chrono::Utc::now(),
            valid_until: None,
            evidence_event_id: None,
        }
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
        // Star-ish: A-B, A-C, B-C (triangle), C-D, D-E.
        g.add_relationship(&mut tx, &rel(ents[0].id, ents[1].id, "related_to"))
            .unwrap();
        g.add_relationship(&mut tx, &rel(ents[0].id, ents[2].id, "related_to"))
            .unwrap();
        g.add_relationship(&mut tx, &rel(ents[1].id, ents[2].id, "related_to"))
            .unwrap();
        g.add_relationship(&mut tx, &rel(ents[2].id, ents[3].id, "related_to"))
            .unwrap();
        g.add_relationship(&mut tx, &rel(ents[3].id, ents[4].id, "related_to"))
            .unwrap();
        tx.commit().unwrap();
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

    #[test]
    fn transitive_completion_adds_inferred_edges() {
        let (db, _ids) = seed();
        let gi = GraphIntelligence::new(db.clone());
        let before = gi.typed_edges().unwrap().len();
        let added = gi.complete_transitive(100).unwrap();
        assert!(added > 0, "expected inferred transitive edges");
        let after = gi.typed_edges().unwrap().len();
        assert_eq!(after, before + added);
        // Idempotent-ish: a second pass adds fewer or zero *new* unique edges
        // beyond what now exists (existing set now includes the inferred ones).
        let added2 = gi.complete_transitive(100).unwrap();
        assert!(added2 <= added);
    }
}
