//! Entity extraction pipeline (memory-upgrade Phase 2 wiring).
//!
//! Turns memory content into knowledge-graph data:
//! `content → NER (deterministic) → entity resolution → mention links →
//! co-mention relationships`. This is the sensory input that populates
//! `entities`/`relationships`/`memory_mentions_entity`, giving the graph
//! intelligence (centrality, communities, link prediction, causal/analogical)
//! real data to work on. LLM-free (L8): regex-based detectors for strong
//! identifiers + multiword proper nouns. Reuses [`EntityResolver`] (conservative,
//! reversible) and [`GraphStore`] — no parallel entity store.

use std::collections::HashSet;
use std::sync::Arc;

use once_cell::sync::Lazy;
use regex::Regex;
use rusqlite::params;
use uuid::Uuid;

use crate::memory::db::Database;
use crate::memory::entity_resolution::{AliasType, EntityResolver, Resolution};
use crate::memory::error::{MemoryResult, StorageError};
use crate::memory::ids::new_id;
use crate::memory::stores::ports::GraphStore;
use crate::memory::stores::SqliteGraphStore;
use crate::memory::types::Relationship;

/// Max entities linked per memory (bounds co-mention edge explosion).
const MAX_ENTITIES_PER_MEMORY: usize = 6;

static RE_EMAIL: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"[A-Za-z0-9._%+\-]+@[A-Za-z0-9.\-]+\.[A-Za-z]{2,}").unwrap());
static RE_URL: Lazy<Regex> = Lazy::new(|| Regex::new(r"https?://[^\s)]+").unwrap());
static RE_REPO: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"(?:github|gitlab)\.com/[A-Za-z0-9_.\-]+/[A-Za-z0-9_.\-]+").unwrap());
static RE_HANDLE: Lazy<Regex> = Lazy::new(|| Regex::new(r"@[A-Za-z0-9_]{2,}").unwrap());
static RE_PATH: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"(?:/[A-Za-z0-9_.\-]+){2,}|[A-Za-z0-9_.\-]+\.[a-z]{1,4}\b").unwrap());
// 2+ consecutive Capitalized words → a proper-noun name (bounded, weak signal).
static RE_PROPER: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"\b([A-Z][a-z]{1,}(?:\s+[A-Z][a-z]{1,}){1,3})\b").unwrap());

/// A candidate entity extracted from text.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExtractedEntity {
    pub display_name: String,
    pub entity_type: String,
    pub alias: String,
    pub alias_type: AliasType,
}

/// Deterministic entity extractor (NER-lite).
pub struct EntityExtractor;

impl EntityExtractor {
    /// Extract candidate entities from `content`, de-duplicated by alias.
    /// Ordered strong→weak so strong identifiers win resolution.
    pub fn extract(content: &str) -> Vec<ExtractedEntity> {
        let mut out = Vec::new();
        let mut seen: HashSet<String> = HashSet::new();
        let mut push = |name: String, etype: &str, alias: String, at: AliasType| {
            let key = format!("{}:{}", at.as_str(), alias.to_lowercase());
            if seen.insert(key) {
                out.push(ExtractedEntity {
                    display_name: name,
                    entity_type: etype.to_string(),
                    alias,
                    alias_type: at,
                });
            }
        };

        for m in RE_EMAIL.find_iter(content) {
            push(
                m.as_str().to_string(),
                "person",
                m.as_str().to_string(),
                AliasType::Email,
            );
        }
        for m in RE_REPO.find_iter(content) {
            push(
                m.as_str().to_string(),
                "repo",
                m.as_str().to_string(),
                AliasType::Repo,
            );
        }
        for m in RE_URL.find_iter(content) {
            // Skip URLs already captured as repos.
            if RE_REPO.is_match(m.as_str()) {
                continue;
            }
            push(
                m.as_str().to_string(),
                "resource",
                m.as_str().to_string(),
                AliasType::Url,
            );
        }
        for m in RE_HANDLE.find_iter(content) {
            push(
                m.as_str().to_string(),
                "person",
                m.as_str().to_string(),
                AliasType::Handle,
            );
        }
        // Proper nouns before generic paths — higher graph value under the cap.
        for c in RE_PROPER.captures_iter(content) {
            let name = c.get(1).map(|m| m.as_str().trim()).unwrap_or("");
            if name.len() >= 3 {
                push(
                    name.to_string(),
                    "concept",
                    name.to_string(),
                    AliasType::Name,
                );
            }
        }
        for m in RE_PATH.find_iter(content) {
            let s = m.as_str();
            // Avoid re-capturing emails/urls as paths.
            if s.contains('@') || s.contains("://") {
                continue;
            }
            push(s.to_string(), "artifact", s.to_string(), AliasType::Url);
        }
        out.truncate(MAX_ENTITIES_PER_MEMORY);
        out
    }
}

/// The extraction pipeline: resolve entities, link them to a memory, and add
/// co-mention relationships between entities appearing together.
pub struct EntityExtractionPipeline {
    db: Arc<Database>,
    resolver: EntityResolver,
    graph: Arc<dyn GraphStore>,
}

impl EntityExtractionPipeline {
    pub fn new(db: Arc<Database>) -> Self {
        let graph: Arc<dyn GraphStore> = Arc::new(SqliteGraphStore::new(db.clone()));
        Self {
            resolver: EntityResolver::new(db.clone(), graph.clone()),
            graph,
            db,
        }
    }

    fn resolved_id(res: Resolution) -> Uuid {
        match res {
            Resolution::Matched(id) => id,
            Resolution::Created(id) => id,
            Resolution::Proposed { created, .. } => created,
        }
    }

    fn link_mention(&self, memory_id: Uuid, entity_id: Uuid) -> MemoryResult<()> {
        let tx = self.db.begin()?;
        tx.conn()
            .execute(
                "INSERT OR IGNORE INTO memory_mentions_entity(memory_id, entity_id) VALUES(?1,?2)",
                params![memory_id.to_string(), entity_id.to_string()],
            )
            .map_err(StorageError::Sqlite)?;
        tx.commit()
    }

    /// Process one memory: extract → resolve → link → co-mention edges. Returns
    /// the number of distinct entities linked.
    pub fn process_memory(&self, memory_id: Uuid, content: &str) -> MemoryResult<usize> {
        let extracted = EntityExtractor::extract(content);
        if extracted.is_empty() {
            return Ok(0);
        }
        let mut entity_ids = Vec::new();
        for e in extracted {
            let res =
                self.resolver
                    .resolve(&e.display_name, &e.entity_type, &e.alias, e.alias_type)?;
            let id = Self::resolved_id(res);
            self.link_mention(memory_id, id)?;
            entity_ids.push(id);
        }
        // Co-mention relationships (undirected pairs) — the raw signal graph
        // intelligence later refines (link prediction, communities).
        self.add_comention_edges(&entity_ids)?;
        Ok(entity_ids.len())
    }

    fn add_comention_edges(&self, entity_ids: &[Uuid]) -> MemoryResult<()> {
        if entity_ids.len() < 2 {
            return Ok(());
        }
        let now = chrono::Utc::now();
        let mut tx = self.db.begin()?;
        for i in 0..entity_ids.len() {
            for j in (i + 1)..entity_ids.len() {
                let (a, b) = (entity_ids[i], entity_ids[j]);
                if a == b {
                    continue;
                }
                let rel = Relationship {
                    id: new_id(),
                    source_id: a,
                    target_id: b,
                    rel_type: "co_mentioned_with".into(),
                    strength: 0.5,
                    valid_from: now,
                    valid_until: None,
                    evidence_event_id: None,
                };
                self.graph.add_relationship(&mut tx, &rel)?;
            }
        }
        tx.commit()
    }

    /// Process active memories that have no entity mentions yet (background
    /// enrichment). Bounded by `limit`. Returns `(memories_processed, entities_linked)`.
    pub fn process_pending(&self, limit: usize) -> MemoryResult<(usize, usize)> {
        let pending: Vec<(Uuid, String)> = self.db.with_read(|conn| {
            let mut stmt = conn
                .prepare(
                    "SELECT m.id, m.content FROM memories m \
                     WHERE m.state = 'active' \
                     AND NOT EXISTS (SELECT 1 FROM memory_mentions_entity me WHERE me.memory_id = m.id) \
                     ORDER BY m.created_at DESC LIMIT ?1",
                )
                .map_err(StorageError::Sqlite)?;
            let rows = stmt
                .query_map(params![limit as i64], |r| {
                    Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
                })
                .map_err(StorageError::Sqlite)?
                .collect::<Result<Vec<_>, _>>()
                .map_err(StorageError::Sqlite)?;
            let mut out = Vec::new();
            for (id, content) in rows {
                if let Ok(u) = Uuid::parse_str(&id) {
                    out.push((u, content));
                }
            }
            Ok(out)
        })?;

        let mut processed = 0usize;
        let mut linked = 0usize;
        for (id, content) in pending {
            linked += self.process_memory(id, &content)?;
            processed += 1;
        }
        Ok((processed, linked))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::memory::graph_intel::GraphIntelligence;
    use crate::memory::stores::ports::{EventStore, RelationalStore};
    use crate::memory::stores::{SqliteEventStore, SqliteRelationalStore};
    use crate::memory::types::{
        Event, EventType, Memory, MemoryState, MemoryType, MemoryWorth, Modality, Scope,
        Sensitivity, Source, StalenessClass,
    };

    #[test]
    fn extracts_strong_and_weak_entities() {
        let ents = EntityExtractor::extract(
            "email alice@example.com about github.com/kria/core with @bob — see New York",
        );
        let kinds: Vec<AliasType> = ents.iter().map(|e| e.alias_type).collect();
        assert!(kinds.contains(&AliasType::Email));
        assert!(kinds.contains(&AliasType::Repo));
        assert!(kinds.contains(&AliasType::Handle));
        assert!(kinds.contains(&AliasType::Name)); // "New York"
    }

    fn seed(db: &Arc<Database>, content: &str, hash: &str) -> Uuid {
        let events = SqliteEventStore::new(db.clone());
        let rel = SqliteRelationalStore::new(db.clone());
        let ev = Event {
            id: new_id(),
            hlc: crate::memory::ids::HlcGenerator::new().now(),
            ts_utc: chrono::Utc::now(),
            tz_offset_min: 0,
            event_type: EventType::UserMessage,
            source: Source::User,
            session_id: None,
            parent_event_id: None,
            shred_key_id: None,
            payload: serde_json::json!({}),
            encrypted: false,
            checksum: "c".into(),
        };
        let now = chrono::Utc::now();
        let m = Memory {
            id: new_id(),
            content: content.into(),
            memory_type: MemoryType::Semantic,
            compression_level: 0,
            source_event_id: ev.id,
            namespace: "core".into(),
            owner_id: "user".into(),
            device_id: "d".into(),
            scope: Scope::Global,
            confidence: 0.8,
            importance: 5.0,
            access_count: 0,
            decay_score: 1.0,
            staleness_class: StalenessClass::Slow,
            sensitivity: Sensitivity::Private,
            state: MemoryState::Active,
            created_at: now,
            last_accessed: None,
            valid_from: now,
            valid_until: None,
            embedding_id: None,
            embedding_model_version: None,
            estimated_tokens: 3,
            content_hash: hash.into(),
            shred_key_id: None,
            verify_against: None,
            superseded_by: None,
            episode_id: None,
            goal_context_id: None,
            worth: MemoryWorth::default(),
            modality: Modality::Text,
            preference_pair_id: None,
            training_eligible: false,
        };
        let mut tx = db.begin().unwrap();
        events.append(&mut tx, &ev).unwrap();
        rel.upsert_memory(&mut tx, &m).unwrap();
        tx.commit().unwrap();
        m.id
    }

    #[test]
    fn pipeline_populates_graph_from_memories() {
        let db = Arc::new(Database::open_in_memory().unwrap());
        seed(&db, "alice@example.com works on github.com/kria/core", "h1");
        let pipe = EntityExtractionPipeline::new(db.clone());
        let (processed, linked) = pipe.process_pending(50).unwrap();
        assert_eq!(processed, 1);
        assert!(linked >= 2, "expected ≥2 entities linked, got {linked}");

        // The graph now has data → centrality is non-empty.
        let gi = GraphIntelligence::new(db.clone());
        assert!(!gi.degree_centrality(10).unwrap().is_empty());

        // Idempotent: already-linked memories are skipped on a second pass.
        let (processed2, _) = pipe.process_pending(50).unwrap();
        assert_eq!(processed2, 0);
    }
}
