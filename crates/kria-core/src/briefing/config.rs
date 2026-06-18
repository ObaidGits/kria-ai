//! User-configurable morning-briefing definition (Phase 1.5).
//!
//! A briefing is an ordered list of [`BriefingSection`]s (gmail / calendar /
//! github / tasks) plus an optional auto schedule. Stored as JSON in `kria.db`
//! so the frontend "Briefing Builder" can edit it.

use serde::{Deserialize, Serialize};

fn default_true() -> bool {
    true
}

/// One source block in the briefing.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BriefingSection {
    /// gmail | calendar | github | tasks
    pub source: String,
    #[serde(default = "default_true")]
    pub enabled: bool,
    /// Gmail search query (e.g. "is:unread", "subject:urgent OR from:boss").
    #[serde(default)]
    pub query: Option<String>,
    /// Max items (gmail).
    #[serde(default)]
    pub max: Option<u64>,
    /// Google account override (gmail/calendar).
    #[serde(default)]
    pub account: Option<String>,
    /// Calendar window: "today" | "next24h".
    #[serde(default)]
    pub window: Option<String>,
    /// Include calendar conflict detection.
    #[serde(default)]
    pub include_conflicts: Option<bool>,
    /// GitHub MCP tool name (default list_notifications).
    #[serde(default)]
    pub tool: Option<String>,
    /// Tasks filter: "urgent_and_overdue" | "active" | "all".
    #[serde(default)]
    pub filter: Option<String>,
}

/// Optional daily auto-run schedule.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BriefingSchedule {
    #[serde(default)]
    pub auto: bool,
    /// HH:MM (local) to deliver the briefing.
    #[serde(default = "default_time")]
    pub time: String,
    /// Delivery channels: notification | chat | tts.
    #[serde(default)]
    pub delivery: Vec<String>,
}

fn default_time() -> String {
    "08:00".to_string()
}

impl Default for BriefingSchedule {
    fn default() -> Self {
        Self {
            auto: false,
            time: default_time(),
            delivery: vec!["notification".into()],
        }
    }
}

/// Full briefing configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BriefingConfig {
    pub sections: Vec<BriefingSection>,
    #[serde(default)]
    pub schedule: BriefingSchedule,
}

impl Default for BriefingConfig {
    fn default() -> Self {
        Self {
            sections: vec![
                BriefingSection {
                    source: "gmail".into(),
                    enabled: true,
                    query: Some("is:unread".into()),
                    max: Some(10),
                    account: None,
                    window: None,
                    include_conflicts: None,
                    tool: None,
                    filter: None,
                },
                BriefingSection {
                    source: "calendar".into(),
                    enabled: true,
                    query: None,
                    max: None,
                    account: None,
                    window: Some("today".into()),
                    include_conflicts: Some(true),
                    tool: None,
                    filter: None,
                },
                BriefingSection {
                    source: "github".into(),
                    enabled: true,
                    query: None,
                    max: None,
                    account: None,
                    window: None,
                    include_conflicts: None,
                    tool: Some("list_notifications".into()),
                    filter: None,
                },
                BriefingSection {
                    source: "tasks".into(),
                    enabled: true,
                    query: None,
                    max: None,
                    account: None,
                    window: None,
                    include_conflicts: None,
                    tool: None,
                    filter: Some("urgent_and_overdue".into()),
                },
            ],
            schedule: BriefingSchedule::default(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_has_four_sections() {
        let c = BriefingConfig::default();
        assert_eq!(c.sections.len(), 4);
        assert!(c.sections.iter().all(|s| s.enabled));
        assert_eq!(c.schedule.time, "08:00");
    }

    #[test]
    fn json_roundtrip() {
        let c = BriefingConfig::default();
        let json = serde_json::to_string(&c).unwrap();
        let back: BriefingConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(back.sections.len(), 4);
    }

    #[test]
    fn partial_section_uses_defaults() {
        // A minimal gmail section (missing optional fields) parses fine.
        let s: BriefingSection =
            serde_json::from_str(r#"{"source":"gmail","query":"is:starred"}"#).unwrap();
        assert!(s.enabled);
        assert_eq!(s.query.as_deref(), Some("is:starred"));
        assert!(s.max.is_none());
    }
}
