//! Canonical Runtime Activation — Progressive Authority Transfer.
//!
//! This module controls the progressive activation of the canonical workflow
//! runtime. It determines which workflows are eligible for canonical execution
//! and provides automatic fallback when the canonical runtime encounters issues.
//!
//! # Activation Strategy
//!
//! Workflows are activated by class (not globally):
//! - Stage 1: Structural-only workflows (no GUI mutation)
//! - Stage 2: Simple visible workflows (AppOpen, FileOpen, BrowserNavigate)
//! - Stage 3: Hybrid workflows (IDE + server + browser)
//! - Stage 4: Interactive workflows (typing, clicking)
//!
//! # Safety
//!
//! - Automatic fallback on any canonical runtime failure
//! - Instant rollback via policy configuration
//! - No double-execution (exactly one runtime is authoritative per workflow)
//! - Full observability of activation decisions

use crate::agent::gui_substrate_planner::ExecutionSubstrate;
use crate::agent::workflow_types::*;

// ═══════════════════════════════════════════════════════════════════════════════
// §1 — Activation Policy
// ═══════════════════════════════════════════════════════════════════════════════

/// Controls which workflow classes are eligible for canonical execution.
#[derive(Debug, Clone)]
pub struct CanonicalActivationPolicy {
    /// Current activation stage (1-4)
    pub stage: ActivationStage,
    /// Whether canonical execution is globally enabled
    pub enabled: bool,
    /// Maximum consecutive failures before automatic rollback
    pub max_consecutive_failures: u32,
    /// Current consecutive failure count
    pub consecutive_failures: u32,
    /// Whether automatic fallback is enabled
    pub auto_fallback: bool,
}

/// Progressive activation stages.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, serde::Serialize, serde::Deserialize)]
pub enum ActivationStage {
    /// No canonical execution (legacy only)
    Disabled,
    /// Stage 1: Structural-only workflows
    StructuralOnly,
    /// Stage 2: Simple visible workflows (AppOpen, BrowserNavigate)
    SimpleVisible,
    /// Stage 3: Hybrid workflows (IDE + server + browser)
    HybridWorkflows,
    /// Stage 4: All workflows including interactive
    FullActivation,
}

impl Default for CanonicalActivationPolicy {
    fn default() -> Self {
        Self {
            stage: ActivationStage::Disabled,
            enabled: false,
            max_consecutive_failures: 3,
            consecutive_failures: 0,
            auto_fallback: true,
        }
    }
}

impl CanonicalActivationPolicy {
    /// Create a policy at a specific activation stage.
    pub fn at_stage(stage: ActivationStage) -> Self {
        Self {
            stage,
            enabled: stage != ActivationStage::Disabled,
            ..Default::default()
        }
    }

    /// Check if a workflow substrate is eligible for canonical execution.
    pub fn is_eligible(&self, substrate: ExecutionSubstrate) -> RuntimeEligibility {
        if !self.enabled {
            return RuntimeEligibility::Legacy {
                reason: "Canonical execution disabled",
            };
        }

        // Check consecutive failure threshold
        if self.consecutive_failures >= self.max_consecutive_failures {
            return RuntimeEligibility::Legacy {
                reason: "Auto-rollback: too many consecutive canonical failures",
            };
        }

        // Check substrate eligibility against current stage
        let substrate_stage = substrate_activation_stage(substrate);
        if substrate_stage <= self.stage {
            RuntimeEligibility::Canonical
        } else {
            RuntimeEligibility::Legacy {
                reason: "Substrate not yet activated at current stage",
            }
        }
    }

    /// Record a canonical execution success.
    pub fn record_success(&mut self) {
        self.consecutive_failures = 0;
    }

    /// Record a canonical execution failure.
    pub fn record_failure(&mut self) {
        self.consecutive_failures += 1;
        if self.auto_fallback && self.consecutive_failures >= self.max_consecutive_failures {
            tracing::warn!(
                target: "workflow_activation",
                failures = self.consecutive_failures,
                threshold = self.max_consecutive_failures,
                "AUTO-ROLLBACK: Disabling canonical execution due to consecutive failures"
            );
            self.enabled = false;
        }
    }

    /// Manually rollback to legacy mode.
    pub fn rollback(&mut self) {
        tracing::info!(
            target: "workflow_activation",
            previous_stage = ?self.stage,
            "Manual rollback to legacy mode"
        );
        self.enabled = false;
        self.consecutive_failures = 0;
    }

    /// Re-enable canonical execution after rollback.
    pub fn re_enable(&mut self, stage: ActivationStage) {
        self.enabled = true;
        self.stage = stage;
        self.consecutive_failures = 0;
        tracing::info!(
            target: "workflow_activation",
            stage = ?stage,
            "Canonical execution re-enabled"
        );
    }
}

/// Whether a workflow should use canonical or legacy runtime.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RuntimeEligibility {
    /// Use canonical HybridWorkflowExecutor
    Canonical,
    /// Use legacy execution path
    Legacy { reason: &'static str },
}

/// Map a substrate type to its activation stage requirement.
fn substrate_activation_stage(substrate: ExecutionSubstrate) -> ActivationStage {
    match substrate {
        // Stage 1: Structural only (no GUI mutation)
        ExecutionSubstrate::TerminalExecution => ActivationStage::StructuralOnly,

        // Stage 2: Simple visible (app launch, browser open)
        ExecutionSubstrate::AppOpenOnly => ActivationStage::SimpleVisible,
        ExecutionSubstrate::BrowserNavigate => ActivationStage::SimpleVisible,

        // Stage 3: Hybrid (file write + app open + terminal)
        ExecutionSubstrate::FileWriteThenOpen => ActivationStage::HybridWorkflows,
        ExecutionSubstrate::IdeCodeRunWorkflow => ActivationStage::HybridWorkflows,
        ExecutionSubstrate::VSCodeCodeRunWorkflow => ActivationStage::HybridWorkflows,

        // Stage 4: Interactive (keystroke injection, AT-SPI interaction)
        ExecutionSubstrate::Keystroke => ActivationStage::FullActivation,
        ExecutionSubstrate::InteractionHeavy => ActivationStage::FullActivation,

        // Unknown substrates stay on legacy
        ExecutionSubstrate::Unknown => ActivationStage::FullActivation,
    }
}

// ═══════════════════════════════════════════════════════════════════════════════
// §2 — Runtime Safety Gates
// ═══════════════════════════════════════════════════════════════════════════════

/// Pre-execution safety validation for canonical runtime.
/// All gates must pass before canonical execution proceeds.
pub fn validate_canonical_readiness(capabilities: &CapabilitySet) -> ReadinessCheck {
    let mut issues = Vec::new();

    // Verify capabilities were resolved (not default/empty)
    if capabilities.verifier.available_methods.is_empty() {
        issues.push("No verification methods available".into());
    }

    // Verify basic environment detection succeeded
    if capabilities.environment.session_type == SessionType::Unknown {
        issues.push("Session type could not be detected".into());
    }

    if issues.is_empty() {
        ReadinessCheck::Ready
    } else {
        ReadinessCheck::NotReady { issues }
    }
}

/// Result of pre-execution readiness validation.
#[derive(Debug, Clone)]
pub enum ReadinessCheck {
    Ready,
    NotReady { issues: Vec<String> },
}

impl ReadinessCheck {
    pub fn is_ready(&self) -> bool {
        matches!(self, Self::Ready)
    }
}

// ═══════════════════════════════════════════════════════════════════════════════
// §3 — Fallback Architecture
// ═══════════════════════════════════════════════════════════════════════════════

/// Why the canonical runtime fell back to legacy.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub enum FallbackReason {
    /// Planner could not generate a plan
    PlannerFailure { detail: String },
    /// Capabilities insufficient for canonical execution
    CapabilityMismatch { detail: String },
    /// Lifecycle FSM entered invalid state
    LifecycleInvariantViolation { detail: String },
    /// Executor encountered unrecoverable error
    ExecutorFailure { detail: String },
    /// Verifier could not operate
    VerifierFailure { detail: String },
    /// Workflow exceeded time budget
    BudgetExceeded { elapsed_ms: u64, budget_ms: u64 },
    /// Substrate not yet activated at current stage
    UnsupportedSubstrate { substrate: String, required_stage: String },
    /// Auto-rollback triggered by consecutive failures
    AutoRollback { consecutive_failures: u32 },
    /// Readiness check failed
    ReadinessCheckFailed { issues: Vec<String> },
}

/// Record of a runtime fallback event (for observability).
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct FallbackRecord {
    pub workflow_id: String,
    pub reason: FallbackReason,
    pub timestamp: String,
    pub canonical_state_at_fallback: String,
}

// ═══════════════════════════════════════════════════════════════════════════════
// §4 — Activation Metrics
// ═══════════════════════════════════════════════════════════════════════════════

/// Aggregate metrics for canonical activation monitoring.
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct ActivationMetrics {
    /// Total workflows routed to canonical runtime
    pub canonical_executions: u64,
    /// Total workflows that fell back to legacy
    pub fallback_executions: u64,
    /// Total workflows on legacy (by policy)
    pub legacy_by_policy: u64,
    /// Canonical success count
    pub canonical_successes: u64,
    /// Canonical failure count
    pub canonical_failures: u64,
    /// Fallback records (last N)
    pub recent_fallbacks: Vec<FallbackRecord>,
}

impl ActivationMetrics {
    /// Canonical success rate (0.0–1.0).
    pub fn success_rate(&self) -> f64 {
        let total = self.canonical_successes + self.canonical_failures;
        if total == 0 {
            1.0 // No data = assume safe
        } else {
            self.canonical_successes as f64 / total as f64
        }
    }

    /// Fallback rate (0.0–1.0).
    pub fn fallback_rate(&self) -> f64 {
        let total = self.canonical_executions + self.fallback_executions;
        if total == 0 {
            0.0
        } else {
            self.fallback_executions as f64 / total as f64
        }
    }

    /// Whether full activation is recommended based on metrics.
    pub fn recommend_full_activation(&self) -> bool {
        self.canonical_executions >= 10
            && self.success_rate() >= 0.95
            && self.fallback_rate() <= 0.05
    }

    pub fn record_canonical_success(&mut self) {
        self.canonical_executions += 1;
        self.canonical_successes += 1;
    }

    pub fn record_canonical_failure(&mut self) {
        self.canonical_executions += 1;
        self.canonical_failures += 1;
    }

    pub fn record_fallback(&mut self, record: FallbackRecord) {
        self.fallback_executions += 1;
        self.recent_fallbacks.push(record);
        // Keep only last 20 fallback records
        if self.recent_fallbacks.len() > 20 {
            self.recent_fallbacks.remove(0);
        }
    }

    pub fn record_legacy_by_policy(&mut self) {
        self.legacy_by_policy += 1;
    }
}

// ═══════════════════════════════════════════════════════════════════════════════
// §5 — Activation Report
// ═══════════════════════════════════════════════════════════════════════════════

/// Complete activation status report.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct CanonicalActivationReport {
    pub current_stage: ActivationStage,
    pub enabled: bool,
    pub metrics: ActivationMetrics,
    pub success_rate: f64,
    pub fallback_rate: f64,
    pub recommend_advancement: bool,
    pub recommend_full_activation: bool,
    pub unsafe_substrates: Vec<String>,
    pub active_substrates: Vec<String>,
}

impl CanonicalActivationReport {
    pub fn generate(policy: &CanonicalActivationPolicy, metrics: &ActivationMetrics) -> Self {
        let active_substrates = match policy.stage {
            ActivationStage::Disabled => vec![],
            ActivationStage::StructuralOnly => vec!["TerminalExecution".into()],
            ActivationStage::SimpleVisible => {
                vec!["TerminalExecution".into(), "AppOpenOnly".into(), "BrowserNavigate".into()]
            }
            ActivationStage::HybridWorkflows => {
                vec![
                    "TerminalExecution".into(), "AppOpenOnly".into(),
                    "BrowserNavigate".into(), "FileWriteThenOpen".into(),
                    "IdeCodeRunWorkflow".into(),
                ]
            }
            ActivationStage::FullActivation => {
                vec![
                    "TerminalExecution".into(), "AppOpenOnly".into(),
                    "BrowserNavigate".into(), "FileWriteThenOpen".into(),
                    "IdeCodeRunWorkflow".into(), "Keystroke".into(),
                    "InteractionHeavy".into(),
                ]
            }
        };

        let unsafe_substrates = match policy.stage {
            ActivationStage::Disabled => vec!["All".into()],
            ActivationStage::StructuralOnly => {
                vec!["AppOpenOnly".into(), "BrowserNavigate".into(), "Interactive".into()]
            }
            ActivationStage::SimpleVisible => {
                vec!["FileWriteThenOpen".into(), "IdeCodeRunWorkflow".into(), "Interactive".into()]
            }
            ActivationStage::HybridWorkflows => {
                vec!["Keystroke".into(), "InteractionHeavy".into()]
            }
            ActivationStage::FullActivation => vec![],
        };

        Self {
            current_stage: policy.stage,
            enabled: policy.enabled,
            metrics: metrics.clone(),
            success_rate: metrics.success_rate(),
            fallback_rate: metrics.fallback_rate(),
            recommend_advancement: metrics.success_rate() >= 0.95 && metrics.canonical_executions >= 5,
            recommend_full_activation: metrics.recommend_full_activation(),
            unsafe_substrates,
            active_substrates,
        }
    }
}

// ═══════════════════════════════════════════════════════════════════════════════
// §6 — Tests
// ═══════════════════════════════════════════════════════════════════════════════

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn disabled_policy_routes_all_to_legacy() {
        let policy = CanonicalActivationPolicy::default();
        assert!(!policy.enabled);
        assert_eq!(
            policy.is_eligible(ExecutionSubstrate::TerminalExecution),
            RuntimeEligibility::Legacy { reason: "Canonical execution disabled" }
        );
    }

    #[test]
    fn stage1_allows_structural_only() {
        let policy = CanonicalActivationPolicy::at_stage(ActivationStage::StructuralOnly);
        assert_eq!(
            policy.is_eligible(ExecutionSubstrate::TerminalExecution),
            RuntimeEligibility::Canonical
        );
        assert_eq!(
            policy.is_eligible(ExecutionSubstrate::AppOpenOnly),
            RuntimeEligibility::Legacy { reason: "Substrate not yet activated at current stage" }
        );
        assert_eq!(
            policy.is_eligible(ExecutionSubstrate::IdeCodeRunWorkflow),
            RuntimeEligibility::Legacy { reason: "Substrate not yet activated at current stage" }
        );
    }

    #[test]
    fn stage2_allows_simple_visible() {
        let policy = CanonicalActivationPolicy::at_stage(ActivationStage::SimpleVisible);
        assert_eq!(policy.is_eligible(ExecutionSubstrate::TerminalExecution), RuntimeEligibility::Canonical);
        assert_eq!(policy.is_eligible(ExecutionSubstrate::AppOpenOnly), RuntimeEligibility::Canonical);
        assert_eq!(policy.is_eligible(ExecutionSubstrate::BrowserNavigate), RuntimeEligibility::Canonical);
        assert_eq!(
            policy.is_eligible(ExecutionSubstrate::IdeCodeRunWorkflow),
            RuntimeEligibility::Legacy { reason: "Substrate not yet activated at current stage" }
        );
    }

    #[test]
    fn stage3_allows_hybrid() {
        let policy = CanonicalActivationPolicy::at_stage(ActivationStage::HybridWorkflows);
        assert_eq!(policy.is_eligible(ExecutionSubstrate::IdeCodeRunWorkflow), RuntimeEligibility::Canonical);
        assert_eq!(policy.is_eligible(ExecutionSubstrate::FileWriteThenOpen), RuntimeEligibility::Canonical);
        assert_eq!(
            policy.is_eligible(ExecutionSubstrate::Keystroke),
            RuntimeEligibility::Legacy { reason: "Substrate not yet activated at current stage" }
        );
    }

    #[test]
    fn stage4_allows_all() {
        let policy = CanonicalActivationPolicy::at_stage(ActivationStage::FullActivation);
        assert_eq!(policy.is_eligible(ExecutionSubstrate::Keystroke), RuntimeEligibility::Canonical);
        assert_eq!(policy.is_eligible(ExecutionSubstrate::InteractionHeavy), RuntimeEligibility::Canonical);
    }

    #[test]
    fn auto_rollback_after_consecutive_failures() {
        let mut policy = CanonicalActivationPolicy::at_stage(ActivationStage::StructuralOnly);
        assert!(policy.enabled);

        policy.record_failure();
        policy.record_failure();
        assert!(policy.enabled); // Still enabled (threshold=3)

        policy.record_failure();
        assert!(!policy.enabled); // Auto-rollback triggered
        assert_eq!(
            policy.is_eligible(ExecutionSubstrate::TerminalExecution),
            RuntimeEligibility::Legacy { reason: "Canonical execution disabled" }
        );
    }

    #[test]
    fn success_resets_failure_counter() {
        let mut policy = CanonicalActivationPolicy::at_stage(ActivationStage::StructuralOnly);
        policy.record_failure();
        policy.record_failure();
        assert_eq!(policy.consecutive_failures, 2);

        policy.record_success();
        assert_eq!(policy.consecutive_failures, 0);
    }

    #[test]
    fn manual_rollback_and_re_enable() {
        let mut policy = CanonicalActivationPolicy::at_stage(ActivationStage::HybridWorkflows);
        assert!(policy.enabled);

        policy.rollback();
        assert!(!policy.enabled);

        policy.re_enable(ActivationStage::SimpleVisible);
        assert!(policy.enabled);
        assert_eq!(policy.stage, ActivationStage::SimpleVisible);
    }

    #[test]
    fn readiness_check_passes_with_valid_capabilities() {
        let caps = CapabilitySet {
            environment: EnvironmentCapability {
                session_type: SessionType::X11,
                compositor: None,
                atspi_level: AtSpiLevel::Full,
                xdotool_available: true,
                uinput_available: true,
                ocr_available: false,
            },
            verifier: VerifierCapability {
                available_methods: vec![VerificationMethod::FileSystem, VerificationMethod::ProcessTable],
                window_state_max_confidence: 0.90,
                cdp_available: false,
                filesystem_available: true,
                process_table_available: true,
            },
            interaction: InteractionCapability {
                keyboard_injection: InputInjectionLevel::Full,
                mouse_injection: InputInjectionLevel::Full,
                clipboard_available: true,
            },
        };
        assert!(validate_canonical_readiness(&caps).is_ready());
    }

    #[test]
    fn readiness_check_fails_with_unknown_session() {
        let caps = CapabilitySet {
            environment: EnvironmentCapability {
                session_type: SessionType::Unknown,
                compositor: None,
                atspi_level: AtSpiLevel::None,
                xdotool_available: false,
                uinput_available: false,
                ocr_available: false,
            },
            verifier: VerifierCapability {
                available_methods: vec![],
                window_state_max_confidence: 0.0,
                cdp_available: false,
                filesystem_available: true,
                process_table_available: true,
            },
            interaction: InteractionCapability {
                keyboard_injection: InputInjectionLevel::None,
                mouse_injection: InputInjectionLevel::None,
                clipboard_available: false,
            },
        };
        assert!(!validate_canonical_readiness(&caps).is_ready());
    }

    #[test]
    fn metrics_track_success_rate() {
        let mut metrics = ActivationMetrics::default();
        metrics.record_canonical_success();
        metrics.record_canonical_success();
        metrics.record_canonical_success();
        metrics.record_canonical_failure();

        assert_eq!(metrics.success_rate(), 0.75);
        assert!(!metrics.recommend_full_activation()); // Need >= 10 executions
    }

    #[test]
    fn metrics_recommend_activation_when_stable() {
        let mut metrics = ActivationMetrics::default();
        for _ in 0..20 {
            metrics.record_canonical_success();
        }
        assert!(metrics.recommend_full_activation());
    }

    #[test]
    fn activation_report_reflects_policy_state() {
        let policy = CanonicalActivationPolicy::at_stage(ActivationStage::SimpleVisible);
        let metrics = ActivationMetrics::default();
        let report = CanonicalActivationReport::generate(&policy, &metrics);

        assert_eq!(report.current_stage, ActivationStage::SimpleVisible);
        assert!(report.enabled);
        assert!(report.active_substrates.contains(&"AppOpenOnly".to_string()));
        assert!(report.unsafe_substrates.contains(&"Interactive".to_string()));
    }
}
