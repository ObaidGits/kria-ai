//! World Model types — facts, sources, conflict resolution.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// Source of a world fact — determines initial confidence and update behavior.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum FactSource {
    /// Directly detected from system commands (highest trust).
    Detected,
    /// Explicitly stated by the user.
    UserStated,
    /// Inferred by the LLM from context.
    Inferred,
    /// Output from the Skill Compiler.
    Compiled,
}

impl FactSource {
    /// Initial confidence for facts from this source.
    pub fn base_confidence(&self) -> f64 {
        match self {
            Self::Detected => 0.95,
            Self::UserStated => 0.90,
            Self::Inferred => 0.60,
            Self::Compiled => 0.75,
        }
    }
}

impl std::fmt::Display for FactSource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Detected => write!(f, "detected"),
            Self::UserStated => write!(f, "user_stated"),
            Self::Inferred => write!(f, "inferred"),
            Self::Compiled => write!(f, "compiled"),
        }
    }
}

impl std::str::FromStr for FactSource {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "detected" => Ok(Self::Detected),
            "user_stated" => Ok(Self::UserStated),
            "inferred" => Ok(Self::Inferred),
            "compiled" => Ok(Self::Compiled),
            _ => Err(format!("unknown fact source: {}", s)),
        }
    }
}

/// A persisted world fact — (subject, predicate, object) triple with metadata.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorldFact {
    /// SQLite row id (None before first insert).
    pub id: Option<i64>,
    /// Subject (e.g., "VM1", "nginx", "user").
    pub subject: String,
    /// Predicate (e.g., "runs", "has_ip", "prefers").
    pub predicate: String,
    /// Object value (e.g., "Ubuntu 24.04", "192.168.122.240", "voice-first").
    pub object: String,
    /// Current confidence (Beta posterior mean).
    pub confidence: f64,
    /// Evidence that supports this fact.
    pub evidence: Vec<String>,
    /// How this fact was learned.
    pub source: FactSource,
    /// When the fact was last verified by direct observation.
    pub last_verified: DateTime<Utc>,
    /// When the fact was created.
    pub created_at: DateTime<Utc>,
    /// How many times this fact has been accessed.
    pub access_count: i64,
}

/// Result of a conflict resolution operation.
#[derive(Debug, Clone)]
pub enum ConflictResolution {
    /// No conflict — new fact inserted.
    Inserted { id: i64 },
    /// Same fact already exists — evidence merged, confidence updated.
    Merged { id: i64, new_confidence: f64 },
    /// Old fact deprecated, new fact inserted.
    Overwritten { new_id: i64, archived_id: i64 },
}

/// Aggregate statistics about the World Model.
#[derive(Debug, Clone, Serialize)]
pub struct WorldModelStats {
    pub total_facts: i64,
    pub archived_facts: i64,
    pub facts_by_source: HashMap<String, i64>,
    pub avg_confidence: f64,
    pub stale_facts: i64,
}

use std::collections::HashMap;

impl Default for WorldModelStats {
    fn default() -> Self {
        Self {
            total_facts: 0,
            archived_facts: 0,
            facts_by_source: HashMap::new(),
            avg_confidence: 0.0,
            stale_facts: 0,
        }
    }
}
