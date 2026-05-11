//! World Model — Persistent facts about the user's systems.
//!
//! # Design: Conflict Resolution with SQLite
//!
//! The World Model stores facts as (subject, predicate, object) triples
//! with confidence scores and evidence chains. When a new fact contradicts
//! an existing one, the old fact is deprecated and the new one takes its place.
//!
//! ## Conflict Resolution Rules
//!
//! 1. **Same (subject, predicate)** → The new fact overwrites the old one.
//!    - Old fact is moved to `world_facts_archive` with `deprecated_by` pointing to the new fact.
//!    - Confidence of the new fact is computed via Bayesian update from the old and new evidence.
//!
//! 2. **Same (subject, predicate, object)** → Evidence is merged (no conflict).
//!    - Confidence is updated: `1 - (1 - old) * (1 - new)` (independent evidence).
//!
//! 3. **Staleness decay** → Facts lose confidence over time without re-verification.
//!    - Decay rate: 0.05/hour (configurable).
//!    - Facts below 0.1 confidence are auto-archived.
//!
//! ## Storage
//!
//! All facts are persisted in SQLite via the existing `MemoryStore` connection.
//! The World Model adds two tables:
//! - `world_facts` — active facts
//! - `world_facts_archive` — deprecated facts (for audit trail)

mod store;
mod types;

pub use store::WorldModelStore;
pub use types::{ConflictResolution, FactSource, WorldFact, WorldModelStats};
