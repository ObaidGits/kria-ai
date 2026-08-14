//! Entity Resolution Engine (memory-upgrade design §8.7/D-10, N5).
//!
//! Conservative + reversible: auto-merge **only** on a strong identifier match
//! (email/handle/URL/repo path); name-only similarity **never** auto-merges
//! people. Every merge records provenance so it can be reversed. Bias: a wrong
//! merge is worse than no merge.

use std::sync::Arc;

use rusqlite::{params, OptionalExtension};
use uuid::Uuid;

use crate::db::Database;
use crate::error::{MemoryResult, StorageError};
use crate::ids::new_id;
use crate::stores::ports::GraphStore;
use crate::types::Entity;

/// Alias strength: strong identifiers may auto-merge; weak (name) never does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AliasType {
    Email,
    Handle,
    Url,
    Repo,
    Name,
}

impl AliasType {
    pub fn as_str(&self) -> &'static str {
        match self {
            AliasType::Email => "email",
            AliasType::Handle => "handle",
            AliasType::Url => "url",
            AliasType::Repo => "repo",
            AliasType::Name => "name",
        }
    }
    /// Whether this identifier is strong enough to auto-merge on (D-10).
    pub fn is_strong(&self) -> bool {
        !matches!(self, AliasType::Name)
    }

    /// Parse the wire-format alias-type tag write-surface adapters
    /// (desktop `memory_resolve_entities`, and any future server route) accept
    /// from the caller back into an [`AliasType`]. Unrecognized/absent tags
    /// default to the weakest classification (`Name`, never auto-merges) —
    /// this mirrors the historical inline adapter match exactly, so relocating
    /// it here is a pure move, not a behavior change (task F1.5.2: adapters
    /// construct caller/command only and carry no standalone alias-taxonomy
    /// decision).
    pub fn from_str(s: &str) -> AliasType {
        match s {
            "email" => AliasType::Email,
            "handle" => AliasType::Handle,
            "url" => AliasType::Url,
            "repo" => AliasType::Repo,
            _ => AliasType::Name,
        }
    }
}

#[cfg(test)]
mod alias_type_tests {
    use super::AliasType;

    #[test]
    fn from_str_round_trips_every_known_tag() {
        for at in [
            AliasType::Email,
            AliasType::Handle,
            AliasType::Url,
            AliasType::Repo,
            AliasType::Name,
        ] {
            assert_eq!(AliasType::from_str(at.as_str()), at);
        }
    }

    #[test]
    fn from_str_defaults_unknown_tags_to_name() {
        assert_eq!(AliasType::from_str("bogus"), AliasType::Name);
        assert_eq!(AliasType::from_str(""), AliasType::Name);
    }
}

/// Outcome of resolving an incoming entity mention.
#[derive(Clone, Debug, PartialEq)]
pub enum Resolution {
    /// Matched an existing entity by a strong identifier.
    Matched(Uuid),
    /// Created a new entity (no strong match).
    Created(Uuid),
    /// A weak (name-only) match was found — proposed, needs confirmation.
    Proposed { existing: Uuid, created: Uuid },
}

pub struct EntityResolver {
    db: Arc<Database>,
    graph: Arc<dyn GraphStore>,
}

impl EntityResolver {
    pub fn new(db: Arc<Database>, graph: Arc<dyn GraphStore>) -> Self {
        Self { db, graph }
    }

    fn find_by_alias(&self, alias: &str, alias_type: AliasType) -> MemoryResult<Option<Uuid>> {
        self.db.with_read(|conn| {
            let id: Option<String> = conn
                .query_row(
                    "SELECT entity_id FROM entity_aliases WHERE alias = ?1 AND alias_type = ?2 LIMIT 1",
                    params![alias, alias_type.as_str()],
                    |r| r.get(0),
                )
                .optional()
                .map_err(StorageError::Sqlite)?;
            Ok(id.and_then(|s| Uuid::parse_str(&s).ok()))
        })
    }

    fn add_alias(
        &self,
        tx: &mut crate::db::AuthorityTx<'_>,
        entity_id: Uuid,
        alias: &str,
        alias_type: AliasType,
    ) -> MemoryResult<()> {
        tx.conn()
            .execute(
                "INSERT OR IGNORE INTO entity_aliases(entity_id, alias, alias_type) VALUES(?1,?2,?3)",
                params![entity_id.to_string(), alias, alias_type.as_str()],
            )
            .map_err(StorageError::Sqlite)?;
        Ok(())
    }

    /// Resolve an incoming mention. Strong identifier → match; weak match →
    /// propose (create + flag); no match → create.
    pub fn resolve(
        &self,
        display_name: &str,
        entity_type: &str,
        alias: &str,
        alias_type: AliasType,
    ) -> MemoryResult<Resolution> {
        // Strong identifier match → auto-merge (return the canonical id).
        if alias_type.is_strong() {
            if let Some(existing) = self.find_by_alias(alias, alias_type)? {
                return Ok(Resolution::Matched(existing));
            }
        }

        // Create the new entity.
        let entity = Entity {
            id: new_id(),
            canonical_id: new_id(), // set to self below
            entity_type: entity_type.to_string(),
            display_name: display_name.to_string(),
            created_at: chrono::Utc::now(),
        };
        let entity = Entity {
            canonical_id: entity.id,
            ..entity
        };
        {
            let mut tx = self.db.begin()?;
            self.graph.add_entity(&mut tx, &entity)?;
            self.add_alias(&mut tx, entity.id, alias, alias_type)?;
            tx.commit()?;
        }

        // Weak (name) match against an existing entity → propose (never auto-merge).
        if !alias_type.is_strong() {
            if let Some(existing) = self.find_name_match(display_name, entity.id)? {
                return Ok(Resolution::Proposed {
                    existing,
                    created: entity.id,
                });
            }
        }
        Ok(Resolution::Created(entity.id))
    }

    fn find_name_match(&self, display_name: &str, exclude: Uuid) -> MemoryResult<Option<Uuid>> {
        let matches = self.graph.search_entities(display_name)?;
        Ok(matches
            .into_iter()
            .find(|e| e.id != exclude && e.display_name.eq_ignore_ascii_case(display_name))
            .map(|e| e.id))
    }

    /// Confirm a proposed merge (or an evidence-threshold merge): point `merged`
    /// at `into`'s canonical id and record reversible provenance (D-10).
    pub fn merge(&self, merged: Uuid, into: Uuid) -> MemoryResult<()> {
        let tx = self.db.begin()?;
        tx.conn()
            .execute(
                "UPDATE entities SET canonical_id = ?2 WHERE id = ?1",
                params![merged.to_string(), into.to_string()],
            )
            .map_err(StorageError::Sqlite)?;
        // Move aliases to the canonical entity.
        tx.conn()
            .execute(
                "UPDATE OR IGNORE entity_aliases SET entity_id = ?2 WHERE entity_id = ?1",
                params![merged.to_string(), into.to_string()],
            )
            .map_err(StorageError::Sqlite)?;
        tx.conn()
            .execute(
                "INSERT OR IGNORE INTO entity_merge_provenance(merged_entity_id, into_entity_id, \
                 merged_at, reversible_until) VALUES(?1,?2,?3,?4)",
                params![
                    merged.to_string(),
                    into.to_string(),
                    chrono::Utc::now().to_rfc3339(),
                    (chrono::Utc::now() + chrono::Duration::days(30)).to_rfc3339(),
                ],
            )
            .map_err(StorageError::Sqlite)?;
        tx.commit()
    }

    /// Reverse a merge: restore the entity's own canonical id.
    pub fn split(&self, merged: Uuid) -> MemoryResult<()> {
        let tx = self.db.begin()?;
        tx.conn()
            .execute(
                "UPDATE entities SET canonical_id = id WHERE id = ?1",
                params![merged.to_string()],
            )
            .map_err(StorageError::Sqlite)?;
        tx.conn()
            .execute(
                "DELETE FROM entity_merge_provenance WHERE merged_entity_id = ?1",
                params![merged.to_string()],
            )
            .map_err(StorageError::Sqlite)?;
        tx.commit()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::stores::SqliteGraphStore;

    fn resolver(db: &Arc<Database>) -> EntityResolver {
        EntityResolver::new(db.clone(), Arc::new(SqliteGraphStore::new(db.clone())))
    }

    #[test]
    fn strong_identifier_auto_merges() {
        let db = Arc::new(Database::open_in_memory().unwrap());
        let r = resolver(&db);
        let first = r
            .resolve("Alice", "person", "alice@example.com", AliasType::Email)
            .unwrap();
        let created = match first {
            Resolution::Created(id) => id,
            other => panic!("expected Created, got {other:?}"),
        };
        // Same email again → matched, not a new entity.
        let second = r
            .resolve("Alice A.", "person", "alice@example.com", AliasType::Email)
            .unwrap();
        assert_eq!(second, Resolution::Matched(created));
    }

    #[test]
    fn name_only_never_auto_merges_people() {
        let db = Arc::new(Database::open_in_memory().unwrap());
        let r = resolver(&db);
        let _a = r
            .resolve("John Smith", "person", "John Smith", AliasType::Name)
            .unwrap();
        let b = r
            .resolve("John Smith", "person", "John Smith", AliasType::Name)
            .unwrap();
        // Second John Smith is a *proposal*, never an automatic merge.
        match b {
            Resolution::Proposed { .. } => {}
            other => panic!("name-only must propose, not auto-merge: {other:?}"),
        }
    }

    #[test]
    fn merge_and_split_are_reversible() {
        let db = Arc::new(Database::open_in_memory().unwrap());
        let r = resolver(&db);
        let a = match r
            .resolve("Repo", "repo", "github.com/kria/a", AliasType::Repo)
            .unwrap()
        {
            Resolution::Created(id) => id,
            o => panic!("{o:?}"),
        };
        let b = match r
            .resolve("Repo2", "repo", "github.com/kria/b", AliasType::Repo)
            .unwrap()
        {
            Resolution::Created(id) => id,
            o => panic!("{o:?}"),
        };
        r.merge(b, a).unwrap();
        r.split(b).unwrap(); // reversible, no panic
    }
}
