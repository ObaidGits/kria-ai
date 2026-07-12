//! Prompt-driven settings scope type.
//!
//! Historically this module hosted a second settings decider (`PromptAnalyzer` +
//! `evaluate`/`PatchOutcome`). That decider was SUPERSEDED by the unified
//! `config::nl` pipeline + `SettingsHandler` (settings-nl-control Wave 5 F15) and
//! removed so there is exactly ONE decider. Only the small [`Scope`] type remains
//! here (it is a stable public vocabulary type used by `config::nl`, the
//! `config_patch` tool, and `RequestOverride`).

/// Scope of a requested change.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Scope {
    /// Turn-scoped, auto-reverts, no persistence.
    Temp,
    /// Persisted (subject to risk gating + approval).
    Permanent,
}
