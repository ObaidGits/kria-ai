//! The [`Episode`] value object (design §4.3, task F2.1.1).
//!
//! An episode is a bounded span of session/task activity (`episodes_v2` row).
//! `cursor_event_id` marks the authority event boundary; `truth_state` is
//! forward-compatible.

use serde::{Deserialize, Serialize};

use super::{EpisodeId, EventId, PolicyPartition, SourceId, UtcTimestamp};
use crate::memory::model::truth::TruthState;

/// A bounded episode of activity (`episodes_v2` row — design §4.3).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Episode {
    /// Stable identity (`episodes_v2.id`).
    pub id: EpisodeId,
    /// The originating session, if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    /// The originating task, if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub task_id: Option<String>,
    /// Policy partition.
    pub policy: PolicyPartition,
    /// Contributing source id.
    pub source_id: SourceId,
    /// Effective policy version tag.
    pub policy_version: String,
    /// When the episode opened.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub opened_at: Option<UtcTimestamp>,
    /// When the episode closed, if it has.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub closed_at: Option<UtcTimestamp>,
    /// Why the episode boundary was drawn.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub boundary_reason: Option<String>,
    /// The event marking the episode cursor.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cursor_event_id: Option<EventId>,
    /// Truth/lifecycle disposition.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub truth_state: Option<TruthState>,
    /// Authority revision at last write.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub revision: Option<u64>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn episode_serde_roundtrips() {
        let e = Episode {
            id: EpisodeId::new_v7(),
            session_id: Some("sess-1".into()),
            task_id: None,
            policy: PolicyPartition::new("user", "chat", 0).unwrap(),
            source_id: SourceId::new_v7(),
            policy_version: "p1".into(),
            opened_at: Some(UtcTimestamp::now()),
            closed_at: None,
            boundary_reason: Some("session_start".into()),
            cursor_event_id: Some(EventId::new_v7()),
            truth_state: Some(TruthState::Current),
            revision: Some(1),
        };
        let back: Episode = serde_json::from_str(&serde_json::to_string(&e).unwrap()).unwrap();
        assert_eq!(back, e);
    }
}
