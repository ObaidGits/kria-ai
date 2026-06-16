//! GUI Cognition V2 — the three layer traits.
//!
//! Sight, Brain, and Hands are fully decoupled: each is a trait with no
//! compile-time dependency on the others' concrete implementations. They
//! exchange ONLY the canonical [`Observation`], [`Decision`], and
//! [`ActionResult`] types. Dependencies are injected so tests can substitute
//! fakes (Requirement 1). The Brain trait is the pluggable seam — a future
//! `UiTarsBrain` implements the SAME trait and drops in with no changes to
//! Sight, Hands, or the loop (Requirement 3.6).

use async_trait::async_trait;

use super::types::{ActionResult, Decision, Observation, TurnStep};

/// Perception layer: screenshot → structured [`Observation`].
#[async_trait]
pub trait Sight: Send + Sync {
    /// Capture and parse the current screen. When `want_som` is true the
    /// implementation SHOULD also produce a Set-of-Mark overlay image. On
    /// failure the implementation SHOULD return a degraded (but non-error)
    /// observation rather than crash the turn (Requirement 2.3); `Err` is
    /// reserved for unrecoverable internal faults.
    async fn observe(&self, want_som: bool) -> anyhow::Result<Observation>;
}

/// Cognition layer: (task, observation, history) → one next [`Decision`].
///
/// The pluggable seam. Implementations decide what they consume from the
/// observation (element labels / Set-of-Mark image / raw screenshot) but MUST
/// only reference targets present in the supplied observation (Requirement 3.2).
#[async_trait]
pub trait GuiBrain: Send + Sync {
    async fn decide(
        &self,
        task: &str,
        observation: &Observation,
        history: &[TurnStep],
    ) -> anyhow::Result<Decision>;

    /// Stable label of this brain ("qwen" | "ui_tars" | "fake").
    fn label(&self) -> &str;
}

/// Action layer: execute a [`Decision`] against the supplied [`Observation`].
///
/// Hands resolves a `Click{element_id}` to the element's physical-pixel center
/// using the SUPPLIED observation; it must NOT click a fallback location when
/// the id is absent (Requirement 4.6).
#[async_trait]
pub trait GuiHands: Send + Sync {
    async fn execute(
        &self,
        decision: &Decision,
        observation: &Observation,
    ) -> anyhow::Result<ActionResult>;
}

/// Outcome of the safety gate for one decided action (Requirement 6.1, 6.2).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GateDecision {
    /// Safe to execute now.
    Allow,
    /// Blocked — do not execute; stop the turn with this sanitized reason.
    /// In the desktop integration a risky action routes through HITL; a denial
    /// (or denied/expired approval) surfaces here as `Deny`.
    Deny(String),
}

/// Safety gate seam: decides whether a decided action may execute. The desktop
/// integration backs this with the existing safety policy + HITL; tests inject
/// a fake. The loop NEVER executes an action the gate does not `Allow`
/// (Property 5).
#[async_trait]
pub trait SafetyGate: Send + Sync {
    async fn evaluate(&self, decision: &Decision, observation: &Observation) -> GateDecision;
}
