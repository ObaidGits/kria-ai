//! Configurable morning briefing (Phase 1.5).
//!
//! The briefing is defined by a [`BriefingConfig`] (sections + schedule),
//! persisted via [`BriefingStore`] in `kria.db`, and consumed by the
//! `gw_morning_briefing` tool which renders each enabled section.

pub mod config;
pub mod store;

pub use config::{BriefingConfig, BriefingSchedule, BriefingSection};
pub use store::BriefingStore;
