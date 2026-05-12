//! RFC v2 (P2): Operational environment grounding.
//!
//! Returns a **closed set of operational facts** about the current desktop
//! state. Strictly bounded: ≤32 facts per turn, ≤10 s TTL (per RFC 008 §1.5),
//! no graph, no embeddings, no arbitrary key-value memory.
//!
//! See `docs/GUI_INTELLIGENCE_REVIEW.md` §4.3.

use crate::agent::intent_compiler::GuiTaskSpec;
use std::path::PathBuf;
use std::time::Instant;

#[derive(Debug, Clone)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub w: u32,
    pub h: u32,
}

#[derive(Debug, Clone)]
pub struct WindowFact {
    pub title: String,
    pub class: String,
    pub pid: u32,
    pub monitor_id: u32,
}

#[derive(Debug, Clone)]
pub struct ProcessFact {
    pub binary: String,
    pub pid: u32,
    pub cpu_share: f32,
}

#[derive(Debug, Clone)]
pub struct FileFact {
    pub path: PathBuf,
    pub exists: bool,
    pub size: Option<u64>,
}

#[derive(Debug, Clone)]
pub struct TerminalFact {
    pub binary: Option<String>,
    pub focused: bool,
}

#[derive(Debug, Clone)]
pub struct MonitorFact {
    pub id: u32,
    pub geometry: Rect,
    pub scale: f32,
    pub primary: bool,
}

/// The closed-enum fact bundle returned by the grounder.
///
/// Hard caps prevent symbolic state explosion. Code review must reject any
/// addition that introduces unbounded collections or generic key-value stores.
#[derive(Debug, Clone)]
pub struct OperationalFacts {
    pub focused_window: Option<WindowFact>,
    /// Capped at 8 entries.
    pub foreground_processes: Vec<ProcessFact>,
    pub workspace_root: Option<PathBuf>,
    /// Capped at 16 entries, only files named in `GuiTaskSpec.targets`.
    pub file_facts: Vec<FileFact>,
    pub terminal: Option<TerminalFact>,
    pub monitors: Vec<MonitorFact>,
    pub captured_at: Instant,
}

impl OperationalFacts {
    /// Returns true if the facts are still within the RFC 008 §1.5 TTL.
    pub fn is_fresh(&self) -> bool {
        const TTL_SECS: u64 = 10;
        self.captured_at.elapsed().as_secs() < TTL_SECS
    }
}

/// Grounder contract.
#[async_trait::async_trait]
pub trait EnvironmentGrounder: Send + Sync {
    async fn ground(&self, spec: &GuiTaskSpec) -> OperationalFacts;
}

/// Placeholder no-op grounder. Returns empty facts. Replaced in P2.
pub struct NoopEnvironmentGrounder;

#[async_trait::async_trait]
impl EnvironmentGrounder for NoopEnvironmentGrounder {
    async fn ground(&self, _spec: &GuiTaskSpec) -> OperationalFacts {
        OperationalFacts {
            focused_window: None,
            foreground_processes: Vec::new(),
            workspace_root: None,
            file_facts: Vec::new(),
            terminal: None,
            monitors: Vec::new(),
            captured_at: Instant::now(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::intent_compiler::{NoopIntentCompiler, IntentCompiler};
    use crate::agent::turn_gate::{
        ComputeClass, HazardHint, IntentEnvelope, IntentSource, Modality, Operation,
    };

    #[tokio::test]
    async fn noop_grounder_returns_fresh_empty_facts() {
        let intent = IntentEnvelope::new(
            Modality::Text,
            Operation::Automate,
            HazardHint::Green,
            ComputeClass::ToolOnly,
            0.9,
            IntentSource::FastEmbedSemanticRouter,
        );
        let spec = NoopIntentCompiler.compile("x", &intent).unwrap();
        let facts = NoopEnvironmentGrounder.ground(&spec).await;
        assert!(facts.is_fresh());
        assert!(facts.foreground_processes.is_empty());
    }
}
