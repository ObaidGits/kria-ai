//! Natural-language settings control (settings-nl-control spec).
//!
//! This module hosts the SINGLE shared settings pipeline + handler used by both
//! the chat turn and the desktop command surface (no divergent implementations).
//!
//! Wave 1 lands the `SettingsHandler` (the one gated execution path) and the
//! `ApprovalDriver` seam (caller-driven HITL + `apply_approved`). The intent
//! classifier (`SettingsIntentPipeline`), entity index, and conversation context
//! land in Wave 2; turn integration in Wave 3.

pub mod catalog;
pub mod conversation;
pub mod diagnostics;
pub mod entity_index;
pub mod evidence;
pub mod flow;
pub mod handler;
pub mod pipeline;
pub mod value;

pub use flow::{ConfigFlowState, FlowEngine, FlowOutcome, FlowStore, ProviderDraft, Slot};


/// Whether the unified NL settings control pipeline is active.
///
/// **Default: ON** — NL settings control is a fully integrated, first-class
/// feature (no env var required to use it). It is disabled ONLY by explicitly
/// setting `KRIA_NL_SETTINGS` to a falsy value (`0` | `false` | `no` | `off`).
/// The legacy `KRIA_CONFIG_PROMPT_CONTROL` truthy value also enables it (folded
/// in for one release); since the default is now on, it can no longer force-off.
///
/// This is the SINGLE source of truth used by the chat turn gate
/// (`run_settings_stage`), the desktop `config_prompt` command, and the
/// `config_patch` tool — so all entry points agree.
pub fn nl_settings_enabled() -> bool {
    fn parse(v: &str) -> Option<bool> {
        match v.trim().to_ascii_lowercase().as_str() {
            "1" | "true" | "yes" | "on" => Some(true),
            "0" | "false" | "no" | "off" => Some(false),
            _ => None,
        }
    }
    // An explicit, parseable KRIA_NL_SETTINGS always wins (allows opt-out).
    if let Ok(v) = std::env::var("KRIA_NL_SETTINGS") {
        if let Some(b) = parse(&v) {
            return b;
        }
    }
    // Unset (or unparseable) ⇒ default ON.
    true
}

pub use conversation::{ConversationContext, SubjectSignal};
pub use entity_index::{FieldCandidate, SchemaEntityIndex};
pub use evidence::{cosine, EvidenceDeps, EvidenceWeights, MemoryEvidenceSource, TextEmbedder};
pub use handler::{
    ApprovalDecision, ApprovalDriver, InfoQuery, SettingsHandler, SettingsOutcome, SettingsRequest,
    SettingsRequestKind,
};
pub use pipeline::{
    IntentThresholds, SettingsDecision, SettingsIntentPipeline, SettingsIntentTrace,
};
