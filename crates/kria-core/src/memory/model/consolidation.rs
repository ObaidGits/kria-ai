//! Consolidation-run value objects: [`ConsolidationRun`] and
//! [`ConsolidationLevel`] (design §4.3, task F2.1.1).
//!
//! A consolidation run is the governed, deterministic, idempotent
//! Episode→Summary→Skill→Rule derivation record (`consolidation_runs` row). Its
//! `level` is a **closed** set (schema `CHECK(episode/summary/skill/rule)`), and
//! a run is uniquely identified by `(algorithm, version, input_set_hash,
//! level)`.

use std::str::FromStr;

use serde::{Deserialize, Serialize};

use super::{encoding_err, ConsolidationRunId, UtcTimestamp};
use crate::memory::error::MemoryResult;

/// The consolidation level produced by a run (`consolidation_runs.level` —
/// design §4.3). A **closed** set matching the schema `CHECK`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ConsolidationLevel {
    /// Episode-level consolidation.
    Episode,
    /// Summary-level consolidation.
    Summary,
    /// Skill-level consolidation.
    Skill,
    /// Rule-level consolidation (terminal).
    Rule,
}

impl ConsolidationLevel {
    /// The canonical text form stored in `level`.
    pub fn as_str(self) -> &'static str {
        match self {
            ConsolidationLevel::Episode => "episode",
            ConsolidationLevel::Summary => "summary",
            ConsolidationLevel::Skill => "skill",
            ConsolidationLevel::Rule => "rule",
        }
    }

    /// All known variants.
    pub fn all() -> &'static [ConsolidationLevel] {
        &[
            ConsolidationLevel::Episode,
            ConsolidationLevel::Summary,
            ConsolidationLevel::Skill,
            ConsolidationLevel::Rule,
        ]
    }
}

impl FromStr for ConsolidationLevel {
    type Err = crate::memory::error::MemoryError;
    fn from_str(s: &str) -> MemoryResult<Self> {
        Ok(match s {
            "episode" => ConsolidationLevel::Episode,
            "summary" => ConsolidationLevel::Summary,
            "skill" => ConsolidationLevel::Skill,
            "rule" => ConsolidationLevel::Rule,
            other => {
                return Err(encoding_err(format!(
                    "unknown consolidation level {other:?}"
                )))
            }
        })
    }
}

impl std::fmt::Display for ConsolidationLevel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for ConsolidationLevel {
    fn deserialize<D: serde::Deserializer<'de>>(de: D) -> Result<Self, D::Error> {
        let s = String::deserialize(de)?;
        s.parse().map_err(serde::de::Error::custom)
    }
}

/// A governed consolidation run (`consolidation_runs` row — design §4.3).
/// `(algorithm, version, input_set_hash, level)` is the uniqueness key.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ConsolidationRun {
    /// Stable identity (`consolidation_runs.id`).
    pub id: ConsolidationRunId,
    /// The consolidation algorithm identity.
    pub algorithm: String,
    /// The algorithm version.
    pub version: String,
    /// A stable hash over the run's input set (uniqueness key component).
    pub input_set_hash: String,
    /// The consolidation level (closed set).
    pub level: ConsolidationLevel,
    /// Resumable cursor position.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cursor: Option<String>,
    /// Run status.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
    /// When the run started.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub started_at: Option<UtcTimestamp>,
    /// When the run completed, if it has.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub completed_at: Option<UtcTimestamp>,
    /// The produced output record id, if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_id: Option<String>,
    /// A terminal error code, if the run failed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error_code: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn level_roundtrip_and_rejects_unknown() {
        for l in ConsolidationLevel::all() {
            assert_eq!(ConsolidationLevel::from_str(l.as_str()).unwrap(), *l);
        }
        assert!(ConsolidationLevel::from_str("meta").is_err());
        assert!(serde_json::from_str::<ConsolidationLevel>("\"meta\"").is_err());
    }

    #[test]
    fn run_serde_roundtrips() {
        let r = ConsolidationRun {
            id: ConsolidationRunId::new_v7(),
            algorithm: "episodic_summary".into(),
            version: "v1".into(),
            input_set_hash: "abc123".into(),
            level: ConsolidationLevel::Summary,
            cursor: None,
            status: Some("running".into()),
            started_at: Some(UtcTimestamp::now()),
            completed_at: None,
            output_id: None,
            error_code: None,
        };
        let back: ConsolidationRun =
            serde_json::from_str(&serde_json::to_string(&r).unwrap()).unwrap();
        assert_eq!(back, r);
    }
}
