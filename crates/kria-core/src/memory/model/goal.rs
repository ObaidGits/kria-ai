//! Goal value objects: [`Goal`], [`GoalStatus`], and [`GoalProgress`] (design
//! §4.3, task F2.1.1).
//!
//! The v2 `goals_v2` status set is a **closed** set (schema
//! `CHECK(candidate/active/paused/completed/conflicted/stale/superseded/
//! deleted)`) — distinct from the legacy [`crate::memory::goals::GoalStatus`]
//! (which used `failed`/`abandoned`). This is the canonical v2 status; the
//! legacy type is retained only as the live `goals`-table representation until
//! the F1.5 write cutover, with the status remap and gate recorded in
//! [`crate::memory::model::legacy_mapping`] (task F2.1.6). `priority` is `0..=10`.

use std::str::FromStr;

use serde::{Deserialize, Serialize};

use super::{
    encoding_err, EventId, GoalId, GoalProgressId, PolicyPartition, SourceId, UtcTimestamp,
};
use crate::memory::error::MemoryResult;

/// The v2 goal lifecycle status (`goals_v2.status` — design §4.3). A **closed**
/// set matching the schema `CHECK`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum GoalStatus {
    /// Proposed, not yet actively pursued.
    Candidate,
    /// Actively being pursued.
    Active,
    /// Temporarily suspended (resumable).
    Paused,
    /// Achieved.
    Completed,
    /// In conflict with another goal.
    Conflicted,
    /// Possibly out of date; awaiting re-verification.
    Stale,
    /// Replaced by a newer goal.
    Superseded,
    /// Governed-deleted.
    Deleted,
}

impl GoalStatus {
    /// The canonical text form stored in `status`.
    pub fn as_str(self) -> &'static str {
        match self {
            GoalStatus::Candidate => "candidate",
            GoalStatus::Active => "active",
            GoalStatus::Paused => "paused",
            GoalStatus::Completed => "completed",
            GoalStatus::Conflicted => "conflicted",
            GoalStatus::Stale => "stale",
            GoalStatus::Superseded => "superseded",
            GoalStatus::Deleted => "deleted",
        }
    }

    /// All known variants.
    pub fn all() -> &'static [GoalStatus] {
        &[
            GoalStatus::Candidate,
            GoalStatus::Active,
            GoalStatus::Paused,
            GoalStatus::Completed,
            GoalStatus::Conflicted,
            GoalStatus::Stale,
            GoalStatus::Superseded,
            GoalStatus::Deleted,
        ]
    }

    /// Whether this status is still "open" (relevant to planning).
    pub fn is_open(self) -> bool {
        matches!(
            self,
            GoalStatus::Candidate | GoalStatus::Active | GoalStatus::Paused
        )
    }

    /// Whether this status contributes to retrieval (design §6.5 / task F3.6.2).
    ///
    /// Only `Active` goals are allowed to contribute retrieval candidates.
    /// All other statuses — `Candidate`, `Paused`, `Completed`, `Conflicted`,
    /// `Stale`, `Superseded`, `Deleted` — contribute **zero** and are excluded.
    /// The moment a goal transitions away from `Active`, its retrieval
    /// contribution stops immediately (no grace period, no cached use).
    pub fn contributes_to_retrieval(self) -> bool {
        self == GoalStatus::Active
    }
}

impl FromStr for GoalStatus {
    type Err = crate::memory::error::MemoryError;
    fn from_str(s: &str) -> MemoryResult<Self> {
        Ok(match s {
            "candidate" => GoalStatus::Candidate,
            "active" => GoalStatus::Active,
            "paused" => GoalStatus::Paused,
            "completed" => GoalStatus::Completed,
            "conflicted" => GoalStatus::Conflicted,
            "stale" => GoalStatus::Stale,
            "superseded" => GoalStatus::Superseded,
            "deleted" => GoalStatus::Deleted,
            other => return Err(encoding_err(format!("unknown goal status {other:?}"))),
        })
    }
}

impl std::fmt::Display for GoalStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for GoalStatus {
    fn deserialize<D: serde::Deserializer<'de>>(de: D) -> Result<Self, D::Error> {
        let s = String::deserialize(de)?;
        s.parse().map_err(serde::de::Error::custom)
    }
}

/// The inclusive maximum goal priority (design §4.3 `priority 0..10`).
pub const GOAL_PRIORITY_MAX: u8 = 10;

/// A goal (`goals_v2` row — design §4.3). Priority is validated `0..=10` via
/// [`Goal::new`] / [`Goal::with_priority`].
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Goal {
    /// Stable identity (`goals_v2.id`).
    pub id: GoalId,
    /// Free-text goal kind.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
    /// Human-facing title.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    /// Lifecycle status (closed set).
    pub status: GoalStatus,
    /// Priority `0..=10` (validated on construction).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub priority: Option<u8>,
    /// Ranking score (semantics named separately).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub score: Option<f64>,
    /// What `score` means.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub score_semantics: Option<String>,
    /// Context needed to resume the goal.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resumption_context: Option<String>,
    /// Policy partition.
    pub policy: PolicyPartition,
    /// Contributing source id.
    pub source_id: SourceId,
    /// Effective policy version tag.
    pub policy_version: String,
    /// The creating authority event.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub created_event_id: Option<EventId>,
    /// Transaction-time creation instant.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub created_at: Option<UtcTimestamp>,
    /// Last-update instant.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub updated_at: Option<UtcTimestamp>,
    /// Authority revision at last write.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub revision: Option<u64>,
}

impl Goal {
    /// Validate a priority is within `0..=10` (design §4.3).
    pub fn validate_priority(priority: u8) -> MemoryResult<u8> {
        if priority > GOAL_PRIORITY_MAX {
            return Err(encoding_err(format!(
                "goal priority {priority} out of range 0..={GOAL_PRIORITY_MAX}"
            )));
        }
        Ok(priority)
    }
}

/// An immutable progress observation against a goal (`goal_progress` row —
/// design §4.3, append-only).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct GoalProgress {
    /// Stable identity (`goal_progress.id`).
    pub id: GoalProgressId,
    /// The goal this progress belongs to.
    pub goal_id: GoalId,
    /// The authority event that recorded the progress.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub event_id: Option<EventId>,
    /// Free-text progress state.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub state: Option<String>,
    /// Progress summary.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
    /// When the progress was observed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub observed_at: Option<UtcTimestamp>,
    /// Authority revision.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub revision: Option<u64>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn goal_status_roundtrip_and_rejects_unknown() {
        for s in GoalStatus::all() {
            assert_eq!(GoalStatus::from_str(s.as_str()).unwrap(), *s);
        }
        // Legacy-only values are rejected by the v2 closed set.
        assert!(GoalStatus::from_str("failed").is_err());
        assert!(GoalStatus::from_str("abandoned").is_err());
        assert!(serde_json::from_str::<GoalStatus>("\"failed\"").is_err());
    }

    /// F3.6.2: only Active contributes to retrieval; all other statuses contribute zero.
    #[test]
    fn only_active_contributes_to_retrieval() {
        assert!(GoalStatus::Active.contributes_to_retrieval());
        for s in GoalStatus::all() {
            if *s != GoalStatus::Active {
                assert!(
                    !s.contributes_to_retrieval(),
                    "{:?} must NOT contribute to retrieval",
                    s
                );
            }
        }
    }

    #[test]
    fn priority_range_is_validated() {
        assert!(Goal::validate_priority(0).is_ok());
        assert!(Goal::validate_priority(10).is_ok());
        assert!(Goal::validate_priority(11).is_err());
    }

    #[test]
    fn goal_serde_roundtrips() {
        let g = Goal {
            id: GoalId::new_v7(),
            kind: Some("task".into()),
            title: Some("ship F2".into()),
            status: GoalStatus::Active,
            priority: Some(7),
            score: None,
            score_semantics: None,
            resumption_context: None,
            policy: PolicyPartition::new("user", "chat", 0).unwrap(),
            source_id: SourceId::new_v7(),
            policy_version: "p1".into(),
            created_event_id: Some(EventId::new_v7()),
            created_at: Some(UtcTimestamp::now()),
            updated_at: None,
            revision: Some(2),
        };
        let back: Goal = serde_json::from_str(&serde_json::to_string(&g).unwrap()).unwrap();
        assert_eq!(back, g);
    }
}
