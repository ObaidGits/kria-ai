//! Phase 2 — Collaborative Autonomy Engine.
//!
//! # Core Mission
//!
//! KRIA dynamically decides HOW to proceed with a requested action, rather
//! than either blindly executing or constantly asking for confirmation. The
//! engine models the user as a collaborator: always informed, never surprised,
//! never annoyed by unnecessary interruptions.
//!
//! # Design Philosophy
//!
//! ```text
//! "A good coworker doesn't ask permission for obvious things.
//!  They also don't silently do dangerous things.
//!  They tell you what they're doing when it matters."
//! ```
//!
//! # Decision Taxonomy
//!
//! | Situation                    | Decision                        |
//! |------------------------------|---------------------------------|
//! | Low-risk, high-confidence    | ProceedSilently                 |
//! | Informational / novel        | ProceedWithNotice               |
//! | Ambiguous target/scope       | Clarify                         |
//! | Destructive operation        | Confirm (HITL gate)             |
//! | Security-sensitive           | Confirm + explicit approval     |
//! | Repeated known workflow      | ProceedSilently (learned pref)  |
//! | Uncertainty > threshold      | Clarify                         |
//! | Unrecoverable if wrong       | Escalate to HITL                |
//! | Transient failure            | Retry (bounded)                 |
//! | Human interrupted            | Pause                           |
//!
//! # Bounded Learning
//!
//! The engine learns workflow preferences (e.g., "always proceed silently for
//! `cargo test`") and persists them to PSDG WorldModelStore. Learning is
//! explicit (from user feedback) and bounded (max 100 preferences).
//!
//! NO uncontrolled online learning. NO autonomous self-modification.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use tracing::{debug, info};

use crate::agent::psdg::PsdgHandle;
use crate::agent::turn_gate::{HazardHint, Operation};
use crate::agent::world_model::FactSource;
use crate::safety::RiskLevel;

// ─── Autonomy Decision ────────────────────────────────────────────────────────

/// The decision the `CollaborativeAutonomyEngine` produces.
///
/// The caller MUST act on this decision. There is no "ignore decision" path.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum AutonomyDecision {
    /// Proceed silently — routine or low-risk operation.
    ///
    /// KRIA does not announce this in the response unless it's important.
    ProceedSilently,

    /// Proceed, but surface a brief notice to the user.
    ///
    /// Used for novel operations, first-time workflows, or operations the
    /// user might not have expected (e.g., KRIA installing a dependency).
    ProceedWithNotice { summary: String },

    /// Ask a clarifying question before proceeding.
    ///
    /// Raised when the target, scope, or intent is ambiguous. The caller
    /// must send this to the user and re-plan after the response.
    Clarify {
        question: String,
        options: Vec<String>,
        can_skip: bool,
    },

    /// Require explicit confirmation before proceeding (HITL gate).
    ///
    /// Raised for destructive, security-sensitive, or irreversible operations.
    Confirm {
        question: String,
        risk_level: RiskLevel,
        consequence_summary: String,
    },

    /// Escalate to human — KRIA cannot safely proceed or retry.
    ///
    /// Raised when: confidence too low, repeated failures, conflicting
    /// instructions, or the operation is outside KRIA's scope.
    Escalate { reason: String, guidance: String },

    /// Pause the workflow — a recoverable external interruption occurred.
    ///
    /// The caller should save state via `WorkflowContinuationRuntime` and
    /// inform the user.
    Pause { reason: String, resume_hint: String },

    /// Retry the last action (bounded).
    ///
    /// The caller tracks attempt count; `CollaborativeAutonomyEngine` only
    /// recommends the decision, never loops itself.
    Retry {
        attempt: u8,
        max_attempts: u8,
        delay_ms: u64,
        reason: String,
    },
}

impl AutonomyDecision {
    /// Returns `true` if the decision requires user interaction before proceeding.
    pub fn requires_user_interaction(&self) -> bool {
        matches!(
            self,
            AutonomyDecision::Clarify { .. }
                | AutonomyDecision::Confirm { .. }
                | AutonomyDecision::Escalate { .. }
        )
    }

    /// Returns a short human-readable label for logging.
    pub fn label(&self) -> &'static str {
        match self {
            AutonomyDecision::ProceedSilently => "proceed_silent",
            AutonomyDecision::ProceedWithNotice { .. } => "proceed_notice",
            AutonomyDecision::Clarify { .. } => "clarify",
            AutonomyDecision::Confirm { .. } => "confirm",
            AutonomyDecision::Escalate { .. } => "escalate",
            AutonomyDecision::Pause { .. } => "pause",
            AutonomyDecision::Retry { .. } => "retry",
        }
    }
}

// ─── Autonomy Context ─────────────────────────────────────────────────────────

/// Context provided to the autonomy engine to make its decision.
#[derive(Debug, Clone)]
pub struct AutonomyContext {
    /// The operation being requested.
    pub operation: Operation,
    /// Risk level from the safety policy.
    pub hazard: HazardHint,
    /// Confidence score from TurnGate (0.0–1.0).
    pub intent_confidence: f32,
    /// Confidence from the execution verifier (0.0–1.0), if available.
    pub verify_confidence: f32,
    /// Whether this is the first time this workflow/tool has been invoked.
    pub is_novel: bool,
    /// Whether a previous attempt of this action failed.
    pub is_retry: bool,
    /// Current retry count (0 = first attempt).
    pub retry_count: u8,
    /// Whether the workflow is currently paused due to external interruption.
    pub interrupted: bool,
    /// Whether there are unresolved ambiguities in the task spec.
    pub has_ambiguities: bool,
    /// Whether the action is irreversible (e.g., send email, delete file).
    pub is_irreversible: bool,
    /// Brief description of what is about to happen (for Confirm/Notice messages).
    pub action_summary: String,
    /// Tool/action name for preference lookup.
    pub tool_name: Option<String>,
    /// Whether PSDG has recent evidence supporting this action.
    pub psdg_backed: bool,
}

impl AutonomyContext {
    /// Create a basic context with defaults.
    pub fn new(
        operation: Operation,
        hazard: HazardHint,
        confidence: f32,
        summary: impl Into<String>,
    ) -> Self {
        Self {
            operation,
            hazard,
            intent_confidence: confidence,
            verify_confidence: 0.0,
            is_novel: false,
            is_retry: false,
            retry_count: 0,
            interrupted: false,
            has_ambiguities: false,
            is_irreversible: false,
            action_summary: summary.into(),
            tool_name: None,
            psdg_backed: false,
        }
    }

    pub fn with_tool(mut self, tool: impl Into<String>) -> Self {
        self.tool_name = Some(tool.into());
        self
    }

    pub fn with_ambiguities(mut self) -> Self {
        self.has_ambiguities = true;
        self
    }

    pub fn as_irreversible(mut self) -> Self {
        self.is_irreversible = true;
        self
    }

    pub fn as_retry(mut self, count: u8) -> Self {
        self.is_retry = true;
        self.retry_count = count;
        self
    }

    pub fn as_novel(mut self) -> Self {
        self.is_novel = true;
        self
    }
}

// ─── Workflow Preference ──────────────────────────────────────────────────────

/// A learned workflow preference from user feedback.
///
/// Persisted to PSDG WorldModelStore under `workflow_preferences.{tool_key}`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkflowPreference {
    /// Tool or workflow key (e.g., "cargo_test", "git_commit").
    pub key: String,
    /// Preferred autonomy level for this workflow.
    pub preferred_decision: PreferredAutonomyLevel,
    /// How many times the user confirmed this preference.
    pub reinforcement_count: u32,
    /// Epoch seconds when last updated.
    pub updated_at: u64,
}

/// The user's preferred autonomy level for a specific workflow.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum PreferredAutonomyLevel {
    /// Always proceed silently.
    AlwaysProceed,
    /// Always ask before proceeding.
    AlwaysAsk,
    /// Use KRIA's default decision (no override).
    UseDefault,
}

// ─── User Feedback ────────────────────────────────────────────────────────────

/// Explicit feedback from the user about a recent autonomy decision.
#[derive(Debug, Clone)]
pub struct UserFeedback {
    /// The tool/workflow this feedback applies to.
    pub workflow_key: String,
    /// What the user preferred.
    pub preferred: PreferredAutonomyLevel,
    /// Optional free-text note from the user.
    pub note: Option<String>,
}

// ─── Autonomy Decision Policy ─────────────────────────────────────────────────

/// Policy rules for autonomy decisions.
///
/// Defaults are conservative: prefer visible confirmation for novel/risky ops,
/// allow silent execution for known-safe routine ops.
#[derive(Debug, Clone)]
pub struct AutonomyDecisionPolicy {
    /// Minimum intent confidence to proceed silently (default: 0.75).
    pub confidence_threshold_silent: f32,
    /// Minimum confidence to proceed with notice (default: 0.5).
    pub confidence_threshold_notice: f32,
    /// Maximum auto-retry attempts before escalating (default: 2).
    pub max_auto_retries: u8,
    /// Retry delay in milliseconds (exponential: delay * 2^attempt).
    pub base_retry_delay_ms: u64,
    /// Whether to require confirmation for all destructive operations.
    pub require_confirm_destructive: bool,
    /// Whether to require confirmation for security-sensitive operations.
    pub require_confirm_security: bool,
    /// Whether to require notice for novel operations.
    pub notice_for_novel: bool,
    /// Learned workflow preferences.
    pub(crate) preferences: HashMap<String, WorkflowPreference>,
    /// Maximum number of stored preferences.
    max_preferences: usize,
}

impl Default for AutonomyDecisionPolicy {
    fn default() -> Self {
        Self {
            confidence_threshold_silent: 0.75,
            confidence_threshold_notice: 0.50,
            max_auto_retries: 2,
            base_retry_delay_ms: 500,
            require_confirm_destructive: true,
            require_confirm_security: true,
            notice_for_novel: true,
            preferences: HashMap::new(),
            max_preferences: 100,
        }
    }
}

impl AutonomyDecisionPolicy {
    /// Look up a learned preference for a tool/workflow key.
    pub fn preference_for(&self, key: &str) -> Option<&WorkflowPreference> {
        self.preferences.get(key)
    }

    /// Update a learned preference (bounded: max 100 entries).
    pub fn set_preference(&mut self, pref: WorkflowPreference) {
        if self.preferences.len() >= self.max_preferences
            && !self.preferences.contains_key(&pref.key)
        {
            // Evict oldest preference (by updated_at)
            if let Some(oldest_key) = self
                .preferences
                .values()
                .min_by_key(|p| p.updated_at)
                .map(|p| p.key.clone())
            {
                self.preferences.remove(&oldest_key);
            }
        }
        self.preferences.insert(pref.key.clone(), pref);
    }
}

// ─── Collaborative Autonomy Engine ────────────────────────────────────────────

/// Decides how KRIA should proceed with a requested action.
///
/// The engine is stateless per-decision: it takes `AutonomyContext` and
/// produces `AutonomyDecision`. State (preferences) lives in `policy`.
pub struct CollaborativeAutonomyEngine {
    policy: AutonomyDecisionPolicy,
    psdg: Option<PsdgHandle>,
}

impl CollaborativeAutonomyEngine {
    /// Create a new engine with default policy and optional PSDG backing.
    pub fn new(psdg: Option<PsdgHandle>) -> Self {
        let mut engine = Self {
            policy: AutonomyDecisionPolicy::default(),
            psdg: psdg.clone(),
        };
        // Load persisted preferences from PSDG on startup.
        if let Some(ref h) = psdg {
            engine.load_preferences(h);
        }
        engine
    }

    /// Make an autonomy decision for a given context.
    ///
    /// Decision priority (highest → lowest):
    /// 1. User interruption → Pause
    /// 2. Learned preference (AlwaysProceed / AlwaysAsk)
    /// 3. Destructive / security gate → Confirm
    /// 4. Ambiguity → Clarify
    /// 5. Retry budget → Retry
    /// 6. Confidence gate → Proceed/Clarify/Escalate
    /// 7. Novel operation notice
    /// 8. Default: ProceedSilently
    pub fn decide(&self, ctx: &AutonomyContext) -> AutonomyDecision {
        // 1. External interruption overrides everything.
        if ctx.interrupted {
            return AutonomyDecision::Pause {
                reason: "Workflow interrupted by external event".to_string(),
                resume_hint: format!(
                    "Resume {} after resolving the interruption",
                    ctx.action_summary
                ),
            };
        }

        // 2. Check learned preferences.
        if let Some(tool_key) = &ctx.tool_name {
            if let Some(pref) = self.policy.preference_for(tool_key) {
                match pref.preferred_decision {
                    PreferredAutonomyLevel::AlwaysProceed => {
                        debug!(
                            target: "collaborative_autonomy",
                            key = %tool_key,
                            "Learned preference: AlwaysProceed"
                        );
                        return AutonomyDecision::ProceedSilently;
                    }
                    PreferredAutonomyLevel::AlwaysAsk => {
                        return AutonomyDecision::Confirm {
                            question: format!("Proceed with: {}?", ctx.action_summary),
                            risk_level: hazard_to_risk(ctx.hazard),
                            consequence_summary: "User set this workflow to always ask.".into(),
                        };
                    }
                    PreferredAutonomyLevel::UseDefault => {}
                }
            }
        }

        // 3. Destructive / security-sensitive gate.
        let is_destructive =
            ctx.is_irreversible || matches!(ctx.hazard, HazardHint::Red | HazardHint::Black);
        let is_security = matches!(ctx.operation, Operation::ConfigureSystem)
            && matches!(
                ctx.hazard,
                HazardHint::Yellow | HazardHint::Red | HazardHint::Black
            );

        if is_destructive && self.policy.require_confirm_destructive {
            return AutonomyDecision::Confirm {
                question: format!(
                    "This action is irreversible: {}. Proceed?",
                    ctx.action_summary
                ),
                risk_level: hazard_to_risk(ctx.hazard),
                consequence_summary: "This operation cannot be undone.".into(),
            };
        }

        if is_security && self.policy.require_confirm_security {
            return AutonomyDecision::Confirm {
                question: format!(
                    "Security-sensitive operation: {}. Confirm?",
                    ctx.action_summary
                ),
                risk_level: RiskLevel::Yellow,
                consequence_summary: "This modifies system security configuration.".into(),
            };
        }

        // 4. Ambiguity → Clarify.
        if ctx.has_ambiguities {
            return AutonomyDecision::Clarify {
                question: format!(
                    "I need more information to complete: {}.",
                    ctx.action_summary
                ),
                options: vec![
                    "Provide more details".into(),
                    "Use the default/common option".into(),
                    "Cancel this action".into(),
                ],
                can_skip: false,
            };
        }

        // 5. Retry budget.
        if ctx.is_retry {
            if ctx.retry_count < self.policy.max_auto_retries {
                let delay = self.policy.base_retry_delay_ms * (1 << ctx.retry_count as u64);
                return AutonomyDecision::Retry {
                    attempt: ctx.retry_count + 1,
                    max_attempts: self.policy.max_auto_retries,
                    delay_ms: delay,
                    reason: format!(
                        "Retrying after transient failure (attempt {}/{})",
                        ctx.retry_count + 1,
                        self.policy.max_auto_retries
                    ),
                };
            } else {
                return AutonomyDecision::Escalate {
                    reason: format!(
                        "Exceeded retry limit ({}) for: {}",
                        self.policy.max_auto_retries, ctx.action_summary
                    ),
                    guidance: "Please check the error and retry manually, or cancel.".into(),
                };
            }
        }

        // 6. Confidence gate.
        let effective_confidence = if ctx.verify_confidence > 0.0 {
            (ctx.intent_confidence + ctx.verify_confidence) / 2.0
        } else {
            ctx.intent_confidence
        };

        if effective_confidence < self.policy.confidence_threshold_notice {
            // Below notice threshold — clarify rather than guess.
            return AutonomyDecision::Clarify {
                question: format!(
                    "I'm not fully confident about: {}. Did you mean this?",
                    ctx.action_summary
                ),
                options: vec![
                    "Yes, proceed".into(),
                    "No, let me re-state".into(),
                    "Cancel".into(),
                ],
                can_skip: false,
            };
        }

        // 7. Novel operation → ProceedWithNotice (if policy enabled).
        if ctx.is_novel && self.policy.notice_for_novel {
            return AutonomyDecision::ProceedWithNotice {
                summary: format!("Running for the first time: {}", ctx.action_summary),
            };
        }

        // 8. High-confidence routine op → ProceedSilently.
        if effective_confidence >= self.policy.confidence_threshold_silent {
            return AutonomyDecision::ProceedSilently;
        }

        // Between thresholds → ProceedWithNotice.
        AutonomyDecision::ProceedWithNotice {
            summary: format!("Proceeding with: {}", ctx.action_summary),
        }
    }

    /// Assimilate user feedback to update workflow preferences.
    ///
    /// Preferences are bounded (max 100). Updates are persisted to PSDG.
    pub fn learn_from_feedback(&mut self, feedback: UserFeedback) {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);

        let pref = if let Some(existing) = self.policy.preferences.get_mut(&feedback.workflow_key) {
            existing.preferred_decision = feedback.preferred.clone();
            existing.reinforcement_count += 1;
            existing.updated_at = now;
            existing.clone()
        } else {
            WorkflowPreference {
                key: feedback.workflow_key.clone(),
                preferred_decision: feedback.preferred,
                reinforcement_count: 1,
                updated_at: now,
            }
        };

        info!(
            target: "collaborative_autonomy",
            key = %feedback.workflow_key,
            decision = ?pref.preferred_decision,
            reinforcement = pref.reinforcement_count,
            "Workflow preference updated"
        );

        self.policy.set_preference(pref.clone());

        // Persist to PSDG
        if let Some(ref h) = self.psdg {
            self.persist_preference(h, &pref);
        }
    }

    /// Persist a single preference to PSDG WorldModelStore.
    fn persist_preference(&self, psdg: &PsdgHandle, pref: &WorkflowPreference) {
        let store = psdg.store_arc();
        let key = format!("pref_{}", pref.key);
        let value = format!("{:?}", pref.preferred_decision);
        let confidence = (0.5 + 0.05 * pref.reinforcement_count as f64).min(0.99);
        tokio::task::spawn_blocking(move || {
            let _ = store.upsert(
                "workflow_preferences",
                &key,
                &value,
                confidence,
                FactSource::Inferred,
                "user_feedback",
            );
        });
    }

    /// Load preferences from PSDG WorldModelStore on startup.
    fn load_preferences(&mut self, psdg: &PsdgHandle) {
        let facts = psdg
            .store()
            .query_subject("workflow_preferences")
            .unwrap_or_default();
        #[allow(unused_variables)]
        for fact in facts {
            if fact.confidence >= 0.5 {
                let key = fact
                    .predicate
                    .strip_prefix("pref_")
                    .unwrap_or(&fact.predicate)
                    .to_string();
                let preferred = match fact.object.as_str() {
                    "AlwaysProceed" => PreferredAutonomyLevel::AlwaysProceed,
                    "AlwaysAsk" => PreferredAutonomyLevel::AlwaysAsk,
                    _ => PreferredAutonomyLevel::UseDefault,
                };
                let now = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_secs())
                    .unwrap_or(0);
                self.policy.set_preference(WorkflowPreference {
                    key: key.clone(),
                    preferred_decision: preferred,
                    reinforcement_count: 1,
                    updated_at: fact.last_verified.timestamp() as u64,
                });
                debug!(
                    target: "collaborative_autonomy",
                    key = %key,
                    "Loaded workflow preference from PSDG"
                );
            }
        }
    }

    /// Export the current decision policy (for testing / telemetry).
    pub fn policy(&self) -> &AutonomyDecisionPolicy {
        &self.policy
    }
}

/// Convert `HazardHint` to `RiskLevel` for the safety gate.
fn hazard_to_risk(hazard: HazardHint) -> RiskLevel {
    match hazard {
        HazardHint::Green => RiskLevel::Green,
        HazardHint::Yellow => RiskLevel::Yellow,
        HazardHint::Red => RiskLevel::Red,
        HazardHint::Black | HazardHint::Unknown => RiskLevel::Red,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn engine() -> CollaborativeAutonomyEngine {
        CollaborativeAutonomyEngine::new(None)
    }

    fn ctx(op: Operation, hazard: HazardHint, conf: f32, summary: &str) -> AutonomyContext {
        AutonomyContext::new(op, hazard, conf, summary)
    }

    // ── Decision taxonomy tests ────────────────────────────────────────────

    #[test]
    fn high_confidence_low_risk_proceeds_silently() {
        let eng = engine();
        let ctx = ctx(Operation::Automate, HazardHint::Green, 0.95, "open firefox");
        assert_eq!(eng.decide(&ctx), AutonomyDecision::ProceedSilently);
    }

    #[test]
    fn interrupted_workflow_pauses() {
        let eng = engine();
        let mut ctx = ctx(Operation::Automate, HazardHint::Green, 0.9, "deploy");
        ctx.interrupted = true;
        assert!(matches!(eng.decide(&ctx), AutonomyDecision::Pause { .. }));
    }

    #[test]
    fn irreversible_op_requires_confirmation() {
        let eng = engine();
        let ctx = ctx(
            Operation::Send,
            HazardHint::Yellow,
            0.95,
            "send email to team",
        )
        .as_irreversible();
        assert!(matches!(eng.decide(&ctx), AutonomyDecision::Confirm { .. }));
    }

    #[test]
    fn ambiguous_spec_requests_clarification() {
        let eng = engine();
        let ctx = ctx(
            Operation::Automate,
            HazardHint::Green,
            0.9,
            "open something",
        )
        .with_ambiguities();
        assert!(matches!(eng.decide(&ctx), AutonomyDecision::Clarify { .. }));
    }

    #[test]
    fn retry_within_budget_retries() {
        let eng = engine();
        let ctx = ctx(
            Operation::ExecuteShell,
            HazardHint::Green,
            0.9,
            "cargo build",
        )
        .as_retry(1);
        let decision = eng.decide(&ctx);
        assert!(matches!(
            decision,
            AutonomyDecision::Retry { attempt: 2, .. }
        ));
    }

    #[test]
    fn retry_exceeding_budget_escalates() {
        let eng = engine();
        let ctx = ctx(
            Operation::ExecuteShell,
            HazardHint::Green,
            0.9,
            "cargo build",
        )
        .as_retry(2);
        assert!(matches!(
            eng.decide(&ctx),
            AutonomyDecision::Escalate { .. }
        ));
    }

    #[test]
    fn low_confidence_clarifies() {
        let eng = engine();
        let ctx = ctx(Operation::Automate, HazardHint::Green, 0.3, "do the thing");
        assert!(matches!(eng.decide(&ctx), AutonomyDecision::Clarify { .. }));
    }

    #[test]
    fn novel_op_proceeds_with_notice() {
        let eng = engine();
        let ctx = ctx(
            Operation::Automate,
            HazardHint::Green,
            0.9,
            "install package",
        )
        .as_novel();
        assert!(matches!(
            eng.decide(&ctx),
            AutonomyDecision::ProceedWithNotice { .. }
        ));
    }

    #[test]
    fn medium_confidence_proceeds_with_notice() {
        let eng = engine();
        let ctx = ctx(Operation::Automate, HazardHint::Green, 0.60, "open gedit");
        assert!(matches!(
            eng.decide(&ctx),
            AutonomyDecision::ProceedWithNotice { .. }
        ));
    }

    #[test]
    fn red_hazard_triggers_confirm() {
        let eng = engine();
        let ctx =
            ctx(Operation::Delete, HazardHint::Red, 0.95, "delete all logs").as_irreversible();
        assert!(matches!(
            eng.decide(&ctx),
            AutonomyDecision::Confirm {
                risk_level: RiskLevel::Red,
                ..
            }
        ));
    }

    // ── Learned preference tests ───────────────────────────────────────────

    #[test]
    fn always_proceed_preference_overrides_novel() {
        let mut eng = engine();
        eng.learn_from_feedback(UserFeedback {
            workflow_key: "cargo_test".into(),
            preferred: PreferredAutonomyLevel::AlwaysProceed,
            note: None,
        });
        let ctx = ctx(
            Operation::ExecuteShell,
            HazardHint::Green,
            0.9,
            "cargo test",
        )
        .with_tool("cargo_test")
        .as_novel();
        assert_eq!(eng.decide(&ctx), AutonomyDecision::ProceedSilently);
    }

    #[test]
    fn always_ask_preference_triggers_confirm() {
        let mut eng = engine();
        eng.learn_from_feedback(UserFeedback {
            workflow_key: "git_push".into(),
            preferred: PreferredAutonomyLevel::AlwaysAsk,
            note: None,
        });
        let ctx =
            ctx(Operation::ExecuteShell, HazardHint::Green, 0.9, "git push").with_tool("git_push");
        assert!(matches!(eng.decide(&ctx), AutonomyDecision::Confirm { .. }));
    }

    #[test]
    fn preference_count_bounded_at_100() {
        let mut eng = engine();
        for i in 0..110 {
            eng.learn_from_feedback(UserFeedback {
                workflow_key: format!("tool_{}", i),
                preferred: PreferredAutonomyLevel::AlwaysProceed,
                note: None,
            });
        }
        assert!(
            eng.policy().preferences.len() <= 100,
            "Preferences must be bounded at 100"
        );
    }

    // ── Decision interface ─────────────────────────────────────────────────

    #[test]
    fn proceed_silent_does_not_require_user_interaction() {
        assert!(!AutonomyDecision::ProceedSilently.requires_user_interaction());
    }

    #[test]
    fn confirm_requires_user_interaction() {
        assert!(AutonomyDecision::Confirm {
            question: "?".into(),
            risk_level: RiskLevel::Yellow,
            consequence_summary: "x".into(),
        }
        .requires_user_interaction());
    }

    #[test]
    fn escalate_requires_user_interaction() {
        assert!(AutonomyDecision::Escalate {
            reason: "out of retries".into(),
            guidance: "manual intervention needed".into(),
        }
        .requires_user_interaction());
    }
}
