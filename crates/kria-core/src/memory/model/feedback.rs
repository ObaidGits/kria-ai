//! The [`Feedback`] value object (design §4.3, task F2.1.1).
//!
//! A feedback signal targets a record/link/entity/etc. (`feedback` row). The
//! `target_kind`/`target_id` pair is a polymorphic endpoint (no hard FK);
//! endpoint existence is enforced at the write boundary in later tasks.

use serde::{Deserialize, Serialize};

use super::{EventId, FeedbackId, PolicyPartition, SourceId, UtcTimestamp};

/// A feedback signal about a target (`feedback` row — design §4.3).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Feedback {
    /// Stable identity (`feedback.id`).
    pub id: FeedbackId,
    /// The kind of the target (polymorphic endpoint).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target_kind: Option<String>,
    /// The id of the target (polymorphic endpoint).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target_id: Option<String>,
    /// The feedback signal.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub signal: Option<String>,
    /// Structured feedback payload (validated JSON).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub payload_json: Option<String>,
    /// Policy partition.
    pub policy: PolicyPartition,
    /// Contributing source id.
    pub source_id: SourceId,
    /// Effective policy version tag.
    pub policy_version: String,
    /// The actor that gave the feedback.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub actor_id: Option<String>,
    /// The authority event that recorded the feedback.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub event_id: Option<EventId>,
    /// Transaction-time creation instant.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub created_at: Option<UtcTimestamp>,
    /// Authority revision.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub revision: Option<u64>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn feedback_serde_roundtrips() {
        let f = Feedback {
            id: FeedbackId::new_v7(),
            target_kind: Some("record".into()),
            target_id: Some("r-1".into()),
            signal: Some("thumbs_up".into()),
            payload_json: None,
            policy: PolicyPartition::new("user", "chat", 0).unwrap(),
            source_id: SourceId::new_v7(),
            policy_version: "p1".into(),
            actor_id: Some("user-1".into()),
            event_id: Some(EventId::new_v7()),
            created_at: Some(UtcTimestamp::now()),
            revision: Some(1),
        };
        let back: Feedback = serde_json::from_str(&serde_json::to_string(&f).unwrap()).unwrap();
        assert_eq!(back, f);
    }
}
