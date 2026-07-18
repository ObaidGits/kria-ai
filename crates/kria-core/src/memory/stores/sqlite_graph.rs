//! SQLite-backed [`GraphStore`] (memory-upgrade design §16, ADR-004).
//!
//! Graph is adjacency tables + traversal. `neighbors` is a **cycle-safe,
//! visited-set, depth-capped BFS** (design §16, Issue 12): a `HashSet` of
//! visited entities guarantees termination even on cyclic graphs, and the hop
//! cap (`<= 3`) bounds breadth. Implemented in Rust over `relationships_for`
//! rather than a recursive CTE for clarity and provable termination.

use std::collections::{HashSet, VecDeque};
use std::sync::Arc;

use rusqlite::{params, Connection, OptionalExtension, Row};
use uuid::Uuid;

use crate::memory::db::{AuthorityTx, Database};
use crate::memory::error::{MemoryResult, StorageError};
use crate::memory::types::{Entity, GraphHit, Relationship};

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

fn row_to_relationship(row: &Row<'_>) -> MemoryResult<Relationship> {
    let id: String = row.get(0).map_err(StorageError::Sqlite)?;
    let source_id: String = row.get(1).map_err(StorageError::Sqlite)?;
    let target_id: String = row.get(2).map_err(StorageError::Sqlite)?;
    let rel_type: String = row.get(3).map_err(StorageError::Sqlite)?;
    let strength: f64 = row.get(4).map_err(StorageError::Sqlite)?;
    let valid_from: String = row.get(5).map_err(StorageError::Sqlite)?;
    let valid_until: Option<String> = row.get(6).map_err(StorageError::Sqlite)?;
    let evidence_event_id: Option<String> = row.get(7).map_err(StorageError::Sqlite)?;
    Ok(Relationship {
        id: parse_uuid(&id)?,
        source_id: parse_uuid(&source_id)?,
        target_id: parse_uuid(&target_id)?,
        rel_type,
        strength: strength as f32,
        valid_from: parse_ts(&valid_from)?,
        valid_until: match valid_until {
            Some(s) => Some(parse_ts(&s)?),
            None => None,
        },
        evidence_event_id: match evidence_event_id {
            Some(s) => Some(parse_uuid(&s)?),
            None => None,
        },
    })
}

const REL_COLS: &str =
    "id, source_id, target_id, rel_type, strength, valid_from, valid_until, evidence_event_id";
const ENT_COLS: &str = "id, canonical_id, entity_type, display_name, created_at";

pub struct SqliteGraphStore {
    db: Arc<Database>,
}

impl SqliteGraphStore {
    pub fn new(db: Arc<Database>) -> Self {
        Self { db }
    }

    fn get_entity(&self, id: Uuid) -> MemoryResult<Option<Entity>> {
        self.db.with_read(|conn: &Connection| {
            conn.query_row(
                &format!("SELECT {ENT_COLS} FROM entities WHERE id = ?1"),
                params![id.to_string()],
                |r| Ok(row_to_entity(r)),
            )
            .optional()
            .map_err(StorageError::Sqlite)?
            .transpose()
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

    fn add_relationship(&self, tx: &mut AuthorityTx<'_>, r: &Relationship) -> MemoryResult<()> {
        tx.conn()
            .execute(
                "INSERT INTO relationships(id, source_id, target_id, rel_type, strength, \
                 valid_from, valid_until, evidence_event_id) VALUES(?1,?2,?3,?4,?5,?6,?7,?8) \
                 ON CONFLICT(id) DO UPDATE SET strength=excluded.strength, \
                 valid_until=excluded.valid_until",
                params![
                    r.id.to_string(),
                    r.source_id.to_string(),
                    r.target_id.to_string(),
                    r.rel_type,
                    r.strength as f64,
                    r.valid_from.to_rfc3339(),
                    r.valid_until.map(|t| t.to_rfc3339()),
                    r.evidence_event_id.map(|u| u.to_string()),
                ],
            )
            .map_err(StorageError::Sqlite)?;
        Ok(())
    }

    fn neighbors(&self, root: Uuid, max_hops: u8) -> MemoryResult<Vec<GraphHit>> {
        let cap = max_hops.min(MAX_HOPS_CAP);
        let mut visited: HashSet<Uuid> = HashSet::new();
        visited.insert(root);
        let mut queue: VecDeque<(Uuid, u8, Vec<Uuid>)> = VecDeque::new();
        queue.push_back((root, 0, vec![root]));
        let mut hits: Vec<GraphHit> = Vec::new();

        while let Some((node, depth, path)) = queue.pop_front() {
            if depth >= cap {
                continue;
            }
            for rel in self.relationships_for(node)? {
                let other = if rel.source_id == node {
                    rel.target_id
                } else {
                    rel.source_id
                };
                if visited.contains(&other) {
                    continue; // visited-set → cycle-safe termination
                }
                visited.insert(other);
                let mut next_path = path.clone();
                next_path.push(other);
                if let Some(entity) = self.get_entity(other)? {
                    hits.push(GraphHit {
                        entity,
                        distance: depth + 1,
                        path: next_path.clone(),
                    });
                }
                queue.push_back((other, depth + 1, next_path));
            }
        }
        Ok(hits)
    }

    fn relationships_for(&self, entity: Uuid) -> MemoryResult<Vec<Relationship>> {
        self.db.with_read(|conn: &Connection| {
            let mut stmt = conn
                .prepare(&format!(
                    "SELECT {REL_COLS} FROM relationships WHERE source_id = ?1 OR target_id = ?1"
                ))
                .map_err(StorageError::Sqlite)?;
            let rows = stmt
                .query_map(params![entity.to_string()], |r| Ok(row_to_relationship(r)))
                .map_err(StorageError::Sqlite)?;
            let mut out = Vec::new();
            for r in rows {
                out.push(r.map_err(StorageError::Sqlite)??);
            }
            Ok(out)
        })
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

    fn rel(a: Uuid, b: Uuid) -> Relationship {
        Relationship {
            id: crate::memory::ids::new_id(),
            source_id: a,
            target_id: b,
            rel_type: "related_to".into(),
            strength: 1.0,
            valid_from: chrono::Utc::now(),
            valid_until: None,
            evidence_event_id: None,
        }
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
        g.add_relationship(&mut tx, &rel(a.id, b.id)).unwrap();
        g.add_relationship(&mut tx, &rel(b.id, c.id)).unwrap();
        tx.commit().unwrap();

        // From Alice, 2 hops reaches Bob (d=1) and Carol (d=2).
        let hits = g.neighbors(a.id, 2).unwrap();
        let names: Vec<_> = hits.iter().map(|h| h.entity.display_name.clone()).collect();
        assert!(names.contains(&"Bob".to_string()));
        assert!(names.contains(&"Carol".to_string()));
        let carol = hits
            .iter()
            .find(|h| h.entity.display_name == "Carol")
            .unwrap();
        assert_eq!(carol.distance, 2);

        // 1 hop reaches only Bob.
        let one = g.neighbors(a.id, 1).unwrap();
        assert_eq!(one.len(), 1);
        assert_eq!(one[0].entity.display_name, "Bob");

        assert_eq!(g.search_entities("Ali").unwrap().len(), 1);
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
            for (a, b) in edges {
                if a < n && b < n && a != b {
                    // Intentionally allow both directions → cycles.
                    g.add_relationship(&mut tx, &rel(ids[a], ids[b])).unwrap();
                }
            }
            tx.commit().unwrap();

            // Must terminate and never exceed the hop cap or revisit the root.
            let hits = g.neighbors(ids[0], max_hops).unwrap();
            for h in &hits {
                prop_assert!(h.distance as u8 <= max_hops.min(MAX_HOPS_CAP));
                prop_assert_ne!(h.entity.id, ids[0]);
            }
            // No entity appears twice (visited-set guarantee).
            let mut seen = std::collections::HashSet::new();
            for h in &hits {
                prop_assert!(seen.insert(h.entity.id), "entity visited twice");
            }
        }
    }
}
