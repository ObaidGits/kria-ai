//! Outbound push notifications (Phase 4.5.5).
//!
//! Currently provides an [`ntfy`] client so the laptop can fire "task done" or
//! "approval needed" alerts to a paired phone. Notifications carry only short
//! human-readable summaries — never secret values or sensitive file contents
//! (see the Phase 4.5 security gate).

pub mod ntfy;

pub use ntfy::{NtfyClient, NtfyConfig, NtfyMessage, NtfyPriority};
