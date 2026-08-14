//! The [`Source`] value object (design §4.3, task F2.1.1).
//!
//! A source is a consented origin of records/events (`sources` row, created as
//! a base row in migration 0014 and indexed in 0017). Its `source_kind` reuses
//! the existing **closed** [`crate::authority::command::SourceKind`]
//! (`CHECK(native/mcp/openclaw/sidecar/import/library/conversation)`) rather
//! than defining a parallel enum.
//!
//! Exposed as `SourceRecord` from the model root to avoid confusion with the
//! legacy provenance [`crate::types::Source`], whose canonical
//! replacement (structured [`crate::model::Provenance`] +
//! [`crate::model::SourceRef`]) and cutover gate are recorded in
//! [`crate::model::legacy_mapping`] (task F2.1.6).

use serde::{Deserialize, Serialize};

use super::{PolicyPartition, SourceId, UtcTimestamp};
use crate::authority::command::SourceKind;

/// A consented source of records/events (`sources` row — design §4.3).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Source {
    /// Stable identity (`sources.id`).
    pub id: SourceId,
    /// The kind of source (closed set, reused from the authority boundary).
    pub source_kind: SourceKind,
    /// External identity of the source (e.g. MCP server id, library corpus id).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub external_identity: Option<String>,
    /// Source version.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    /// Trust classification.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub trust_class: Option<String>,
    /// Policy partition.
    pub policy: PolicyPartition,
    /// Effective policy version tag.
    pub policy_version: String,
    /// Consent disposition.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub consent_state: Option<String>,
    /// Content hash of the source manifest.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content_hash: Option<String>,
    /// Lifecycle state.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lifecycle_state: Option<String>,
    /// Resumable ingestion cursor (validated JSON).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cursor_json: Option<String>,
    /// Transaction-time creation instant.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub created_at: Option<UtcTimestamp>,
    /// Last-update instant.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub updated_at: Option<UtcTimestamp>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn source_serde_roundtrips() {
        let s = Source {
            id: SourceId::new_v7(),
            source_kind: SourceKind::Mcp,
            external_identity: Some("github".into()),
            version: Some("1.0".into()),
            trust_class: Some("third_party".into()),
            policy: PolicyPartition::new("user", "chat", 0).unwrap(),
            policy_version: "p1".into(),
            consent_state: Some("granted".into()),
            content_hash: None,
            lifecycle_state: Some("active".into()),
            cursor_json: None,
            created_at: Some(UtcTimestamp::now()),
            updated_at: None,
        };
        let back: Source = serde_json::from_str(&serde_json::to_string(&s).unwrap()).unwrap();
        assert_eq!(back, s);
    }
}
