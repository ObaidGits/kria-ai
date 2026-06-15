//! GUI Cognition turn budget + runtime-guard flag (Task 1.1 — NFR budgets).
//!
//! [`TurnBudget`] is the single configurable source of truth for the
//! non-functional execution budgets that bound a GUI Cognition turn
//! (Requirement 19). It is a pure data type: this subtask defines the budget,
//! its defaults, and its configurability. Wiring the budget into the workflow
//! loop (cancel / watchdog / abort) is Task 1.2 / 1.3 and consumes this type.
//!
//! Requirement 19 (NFR budgets) defaults encoded here:
//! - 19.1 Single-primitive turn completes/stops within a configurable budget
//!   (default ≤ 8 s) → [`DEFAULT_SINGLE_PRIMITIVE_BUDGET_MS`].
//! - 19.2 Combo turn bounded by `max_steps` (default ≤ 12) and a turn watchdog
//!   (default ≤ 90 s) → [`DEFAULT_MAX_STEPS`], [`DEFAULT_TURN_WATCHDOG_MS`].
//! - 19.3 Per-step target resolution + verification each have a bounded timeout
//!   → [`DEFAULT_STEP_RESOLVE_MS`], [`DEFAULT_STEP_VERIFY_MS`].
//! - 19.4 Re-observe count per turn capped (default ≤ max_steps + 4)
//!   → [`DEFAULT_MAX_REOBSERVE`] / [`TurnBudget::effective_max_reobserve`].
//! - 19.5 Budgets configurable and asserted in tests → serde (`#[serde(default)]`
//!   per field) + [`TurnBudget::from_env`].
//!
//! The design data model specifies
//! `TurnBudget: { max_steps, turn_watchdog_ms, step_resolve_ms, step_verify_ms,
//! max_reobserve }`. We additionally carry `single_primitive_budget_ms` because
//! Requirement 19.1 mandates a configurable single-primitive budget (default
//! ≤ 8 s) that none of the design's five fields directly expresses.
//!
//! Feature flag: enforcement of this budget is gated behind
//! `gui_cog_runtime_guards` (default OFF), surfaced here as
//! [`GuiRuntimeGuardConfig`]. While the flag is OFF, existing Step 1–12 behavior
//! is preserved; the budget type itself may exist and be inspected regardless.

use std::collections::VecDeque;
use std::time::Instant;

use serde::{Deserialize, Serialize};

/// Default `max_steps` for a combo turn (Requirement 19.2, default ≤ 12).
pub const DEFAULT_MAX_STEPS: u32 = 12;
/// Default turn watchdog in milliseconds (Requirement 19.2, default ≤ 90 s).
pub const DEFAULT_TURN_WATCHDOG_MS: u64 = 90_000;
/// Default per-step target-resolution timeout in milliseconds (Requirement 19.3).
pub const DEFAULT_STEP_RESOLVE_MS: u64 = 4_000;
/// Default per-step verification timeout in milliseconds (Requirement 19.3).
pub const DEFAULT_STEP_VERIFY_MS: u64 = 4_000;
/// Default single-primitive turn budget in milliseconds (Requirement 19.1, ≤ 8 s).
pub const DEFAULT_SINGLE_PRIMITIVE_BUDGET_MS: u64 = 8_000;
/// Default re-observe cap per turn (Requirement 19.4, default ≤ max_steps + 4).
pub const DEFAULT_MAX_REOBSERVE: u32 = DEFAULT_MAX_STEPS + 4;
/// Default consecutive verification-failure cap before a safe abort
/// (Requirement 21.4, "repeated verification failure ... SHALL abort rather
/// than loop"). Kept small so the loop never burns its budget re-trying a
/// verification that keeps failing.
pub const DEFAULT_MAX_VERIFICATION_FAILURES: u32 = 2;
/// Default flapping threshold (Requirement 21.4, screen "flapping"): if the same
/// observed screen state recurs this many times within the recent window without
/// progress, the loop aborts instead of oscillating.
pub const DEFAULT_FLAPPING_THRESHOLD: u32 = 3;

/// Slack added to `max_steps` to derive the re-observe cap invariant (19.4).
pub const REOBSERVE_SLACK_OVER_MAX_STEPS: u32 = 4;

/// Environment variable that enables the `gui_cog_runtime_guards` flag.
///
/// Truthy (`1`/`true`/`yes`/`on`) turns budget enforcement ON. Default (unset
/// or any other value) keeps it OFF so existing behavior is preserved.
pub const RUNTIME_GUARDS_ENV_FLAG: &str = "KRIA_GUI_COG_RUNTIME_GUARDS";
/// Environment variable that enables the `gui_cog_reobserve` flag (Task 3).
///
/// Truthy (`1`/`true`/`yes`/`on`) turns the explicit per-step re-observe hook
/// ON. Default (unset or any other value) keeps it OFF: existing re-observe
/// behavior is preserved and only the additive hook instrumentation is gated.
/// The Wave 3 gate (Task 3.6) flips the live/desktop path to default ON.
pub const REOBSERVE_ENV_FLAG: &str = "KRIA_GUI_COG_REOBSERVE";
/// Override env var for [`TurnBudget::max_steps`].
pub const ENV_MAX_STEPS: &str = "KRIA_GUI_COG_MAX_STEPS";
/// Override env var for [`TurnBudget::turn_watchdog_ms`].
pub const ENV_TURN_WATCHDOG_MS: &str = "KRIA_GUI_COG_TURN_WATCHDOG_MS";
/// Override env var for [`TurnBudget::step_resolve_ms`].
pub const ENV_STEP_RESOLVE_MS: &str = "KRIA_GUI_COG_STEP_RESOLVE_MS";
/// Override env var for [`TurnBudget::step_verify_ms`].
pub const ENV_STEP_VERIFY_MS: &str = "KRIA_GUI_COG_STEP_VERIFY_MS";
/// Override env var for [`TurnBudget::single_primitive_budget_ms`].
pub const ENV_SINGLE_PRIMITIVE_BUDGET_MS: &str = "KRIA_GUI_COG_SINGLE_PRIMITIVE_BUDGET_MS";
/// Override env var for [`TurnBudget::max_reobserve`].
pub const ENV_MAX_REOBSERVE: &str = "KRIA_GUI_COG_MAX_REOBSERVE";
/// Override env var for [`TurnBudget::max_verification_failures`].
pub const ENV_MAX_VERIFICATION_FAILURES: &str = "KRIA_GUI_COG_MAX_VERIFICATION_FAILURES";
/// Override env var for [`TurnBudget::flapping_threshold`].
pub const ENV_FLAPPING_THRESHOLD: &str = "KRIA_GUI_COG_FLAPPING_THRESHOLD";

/// Stable `cause` tags emitted on [`run_aborted_event`] for runaway-control
/// aborts (Task 1.3). They are part of the event contract, so keep them stable.
///
/// [`run_aborted_event`]: crate::agent::gui_cognition::workflow_runtime::run_aborted_event
pub mod abort_cause {
    /// Step count exceeded `max_steps` (Requirement 19.2 / 21.3).
    pub const BUDGET_MAX_STEPS: &str = "budget_max_steps";
    /// Turn watchdog (`turn_watchdog_ms`) elapsed (Requirement 19.2 / 21.3).
    pub const BUDGET_WATCHDOG: &str = "budget_watchdog";
    /// Re-observe count exceeded the effective cap (Requirement 19.4 / 21.3).
    pub const BUDGET_MAX_REOBSERVE: &str = "budget_max_reobserve";
    /// Screen state oscillated/repeated without progress (Requirement 21.4).
    pub const FLAPPING: &str = "flapping";
    /// Consecutive verification failures hit the cap (Requirement 21.4).
    pub const REPEATED_VERIFICATION_FAILURE: &str = "repeated_verification_failure";
}

/// Errors raised when validating a [`TurnBudget`] configuration.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum TurnBudgetError {
    /// A budget field that must be positive was configured as zero.
    #[error("turn budget field `{field}` must be greater than zero")]
    NonPositive {
        /// The offending field name.
        field: &'static str,
    },
}

/// Configurable non-functional budgets bounding a single GUI Cognition turn.
///
/// All time fields are milliseconds. Construct defaults with [`TurnBudget::default`],
/// override individual fields via the builder-style setters, deserialize from a
/// config file (serde, with per-field defaults so partial configs are valid), or
/// derive from the process environment with [`TurnBudget::from_env`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct TurnBudget {
    /// Maximum number of steps a combo turn may execute (Requirement 19.2).
    pub max_steps: u32,
    /// Turn watchdog ceiling in milliseconds for a combo turn (Requirement 19.2).
    pub turn_watchdog_ms: u64,
    /// Bounded per-step target-resolution timeout in milliseconds (Requirement 19.3).
    pub step_resolve_ms: u64,
    /// Bounded per-step verification timeout in milliseconds (Requirement 19.3).
    pub step_verify_ms: u64,
    /// Single-primitive turn budget in milliseconds (Requirement 19.1, ≤ 8 s default).
    pub single_primitive_budget_ms: u64,
    /// Maximum re-observations allowed per turn (Requirement 19.4, ≤ max_steps + 4).
    pub max_reobserve: u32,
    /// Maximum consecutive verification failures before a safe abort
    /// (Requirement 21.4: repeated verification failure aborts rather than loops).
    pub max_verification_failures: u32,
    /// How many times the same screen state may recur within the recent window
    /// before the turn is treated as "flapping" and aborted (Requirement 21.4).
    pub flapping_threshold: u32,
}

impl Default for TurnBudget {
    fn default() -> Self {
        Self {
            max_steps: DEFAULT_MAX_STEPS,
            turn_watchdog_ms: DEFAULT_TURN_WATCHDOG_MS,
            step_resolve_ms: DEFAULT_STEP_RESOLVE_MS,
            step_verify_ms: DEFAULT_STEP_VERIFY_MS,
            single_primitive_budget_ms: DEFAULT_SINGLE_PRIMITIVE_BUDGET_MS,
            max_reobserve: DEFAULT_MAX_REOBSERVE,
            max_verification_failures: DEFAULT_MAX_VERIFICATION_FAILURES,
            flapping_threshold: DEFAULT_FLAPPING_THRESHOLD,
        }
    }
}

impl TurnBudget {
    /// Builder-style override for [`Self::max_steps`].
    pub fn with_max_steps(mut self, max_steps: u32) -> Self {
        self.max_steps = max_steps;
        self
    }

    /// Builder-style override for [`Self::turn_watchdog_ms`].
    pub fn with_turn_watchdog_ms(mut self, turn_watchdog_ms: u64) -> Self {
        self.turn_watchdog_ms = turn_watchdog_ms;
        self
    }

    /// Builder-style override for [`Self::step_resolve_ms`].
    pub fn with_step_resolve_ms(mut self, step_resolve_ms: u64) -> Self {
        self.step_resolve_ms = step_resolve_ms;
        self
    }

    /// Builder-style override for [`Self::step_verify_ms`].
    pub fn with_step_verify_ms(mut self, step_verify_ms: u64) -> Self {
        self.step_verify_ms = step_verify_ms;
        self
    }

    /// Builder-style override for [`Self::single_primitive_budget_ms`].
    pub fn with_single_primitive_budget_ms(mut self, single_primitive_budget_ms: u64) -> Self {
        self.single_primitive_budget_ms = single_primitive_budget_ms;
        self
    }

    /// Builder-style override for [`Self::max_reobserve`].
    pub fn with_max_reobserve(mut self, max_reobserve: u32) -> Self {
        self.max_reobserve = max_reobserve;
        self
    }

    /// Builder-style override for [`Self::max_verification_failures`].
    pub fn with_max_verification_failures(mut self, max_verification_failures: u32) -> Self {
        self.max_verification_failures = max_verification_failures;
        self
    }

    /// Builder-style override for [`Self::flapping_threshold`].
    pub fn with_flapping_threshold(mut self, flapping_threshold: u32) -> Self {
        self.flapping_threshold = flapping_threshold;
        self
    }

    /// The re-observe cap invariant ceiling for the configured `max_steps`
    /// (Requirement 19.4: default ≤ max_steps + 4).
    pub fn reobserve_ceiling(&self) -> u32 {
        self.max_steps.saturating_add(REOBSERVE_SLACK_OVER_MAX_STEPS)
    }

    /// Effective re-observe cap, never exceeding the 19.4 ceiling even if a
    /// larger `max_reobserve` was configured.
    pub fn effective_max_reobserve(&self) -> u32 {
        self.max_reobserve.min(self.reobserve_ceiling())
    }

    /// Validate that every positive-only field is greater than zero.
    ///
    /// A zero budget would mean "no time/steps allowed", which is never a valid
    /// runtime guard. Returns [`TurnBudgetError::NonPositive`] for the first
    /// offending field.
    pub fn validate(&self) -> Result<(), TurnBudgetError> {
        let checks: [(&'static str, u64); 8] = [
            ("max_steps", self.max_steps as u64),
            ("turn_watchdog_ms", self.turn_watchdog_ms),
            ("step_resolve_ms", self.step_resolve_ms),
            ("step_verify_ms", self.step_verify_ms),
            ("single_primitive_budget_ms", self.single_primitive_budget_ms),
            ("max_reobserve", self.max_reobserve as u64),
            ("max_verification_failures", self.max_verification_failures as u64),
            ("flapping_threshold", self.flapping_threshold as u64),
        ];
        for (field, value) in checks {
            if value == 0 {
                return Err(TurnBudgetError::NonPositive { field });
            }
        }
        Ok(())
    }

    /// Derive a budget from the process environment, falling back to defaults
    /// for any unset or unparsable override (Requirement 19.5 configurability).
    pub fn from_env() -> Self {
        Self::from_env_lookup(|key| std::env::var(key).ok())
    }

    /// Testable core of [`from_env`] with an injectable lookup.
    pub fn from_env_lookup<F>(lookup: F) -> Self
    where
        F: Fn(&str) -> Option<String>,
    {
        let mut budget = TurnBudget::default();
        if let Some(value) = parse_env(&lookup, ENV_MAX_STEPS) {
            budget.max_steps = value;
        }
        if let Some(value) = parse_env(&lookup, ENV_TURN_WATCHDOG_MS) {
            budget.turn_watchdog_ms = value;
        }
        if let Some(value) = parse_env(&lookup, ENV_STEP_RESOLVE_MS) {
            budget.step_resolve_ms = value;
        }
        if let Some(value) = parse_env(&lookup, ENV_STEP_VERIFY_MS) {
            budget.step_verify_ms = value;
        }
        if let Some(value) = parse_env(&lookup, ENV_SINGLE_PRIMITIVE_BUDGET_MS) {
            budget.single_primitive_budget_ms = value;
        }
        if let Some(value) = parse_env(&lookup, ENV_MAX_REOBSERVE) {
            budget.max_reobserve = value;
        }
        if let Some(value) = parse_env(&lookup, ENV_MAX_VERIFICATION_FAILURES) {
            budget.max_verification_failures = value;
        }
        if let Some(value) = parse_env(&lookup, ENV_FLAPPING_THRESHOLD) {
            budget.flapping_threshold = value;
        }
        budget
    }

    /// Sanitized JSON summary for events/telemetry (no secrets, only budgets).
    pub fn summary_json(&self) -> serde_json::Value {
        serde_json::json!({
            "max_steps": self.max_steps,
            "turn_watchdog_ms": self.turn_watchdog_ms,
            "step_resolve_ms": self.step_resolve_ms,
            "step_verify_ms": self.step_verify_ms,
            "single_primitive_budget_ms": self.single_primitive_budget_ms,
            "max_reobserve": self.max_reobserve,
            "effective_max_reobserve": self.effective_max_reobserve(),
            "max_verification_failures": self.max_verification_failures,
            "flapping_threshold": self.flapping_threshold,
        })
    }
}

/// The `gui_cog_runtime_guards` feature-flag bundle (default OFF).
///
/// Carries the on/off flag plus the [`TurnBudget`] that Task 1.2 / 1.3 enforce.
/// When `enabled` is false (the default), the workflow loop ignores the budget
/// and preserves existing Step 1–12 behavior.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct GuiRuntimeGuardConfig {
    /// Whether budget enforcement (cancel/watchdog/abort) is active.
    pub enabled: bool,
    /// The configurable turn budget consumed when `enabled` is true.
    pub budget: TurnBudget,
}

impl Default for GuiRuntimeGuardConfig {
    fn default() -> Self {
        Self {
            // Requirement 21 / Task 1: flag default OFF — existing behavior preserved.
            enabled: false,
            budget: TurnBudget::default(),
        }
    }
}

impl GuiRuntimeGuardConfig {
    /// Construct an explicitly-enabled guard config with the given budget.
    pub fn enabled(budget: TurnBudget) -> Self {
        Self {
            enabled: true,
            budget,
        }
    }

    /// Derive the guard config from the process environment. The flag is OFF
    /// unless `KRIA_GUI_COG_RUNTIME_GUARDS` is truthy; the budget is derived
    /// from its own overrides regardless (so it can be inspected when OFF).
    pub fn from_env() -> Self {
        Self::from_env_lookup(|key| std::env::var(key).ok())
    }

    /// Testable core of [`from_env`] with an injectable lookup.
    pub fn from_env_lookup<F>(lookup: F) -> Self
    where
        F: Fn(&str) -> Option<String>,
    {
        Self {
            enabled: is_truthy(lookup(RUNTIME_GUARDS_ENV_FLAG).as_deref()),
            budget: TurnBudget::from_env_lookup(&lookup),
        }
    }

    /// Derive the guard config from the process environment with the flag
    /// **default ON** (Task 1.6 gate flip). The live/desktop `execute_live`
    /// path uses this so the runaway-control guards (cancel/watchdog/abort/
    /// preconditions, Requirements 19/21/25) are active by default once the
    /// Task 1 gate has passed. Enforcement stays ON unless
    /// `KRIA_GUI_COG_RUNTIME_GUARDS` is explicitly falsy
    /// (`0`/`false`/`no`/`off`/empty) — that is the documented rollback to
    /// restore the previous Step 1–12 behavior without a code change. The
    /// budget is derived from its own overrides regardless.
    pub fn from_env_default_on() -> Self {
        Self::from_env_lookup_default_on(|key| std::env::var(key).ok())
    }

    /// Testable core of [`from_env_default_on`] with an injectable lookup.
    pub fn from_env_lookup_default_on<F>(lookup: F) -> Self
    where
        F: Fn(&str) -> Option<String>,
    {
        // Default ON: enforcement is active unless the operator explicitly
        // opts out via a falsy env value (the rollback switch).
        Self {
            enabled: !is_falsy(lookup(RUNTIME_GUARDS_ENV_FLAG).as_deref()),
            budget: TurnBudget::from_env_lookup(&lookup),
        }
    }

    /// Whether budget enforcement should run for this turn.
    pub fn is_enforced(&self) -> bool {
        self.enabled
    }
}

/// The `gui_cog_reobserve` feature-flag bundle (default OFF) — Task 3.1.
///
/// Per-step re-observe (capturing a FRESH [`GuiContext`] between steps so a
/// combo acts on the *current* screen, Requirement 2) is the foundation this
/// flag gates. While OFF (the default) the runtime preserves its existing
/// re-observe behavior and only the additive re-observe-hook instrumentation is
/// suppressed; while ON the runtime emits the explicit re-observe hook that
/// Tasks 3.2–3.4 build on (next-target resolution against the fresh context,
/// bounded readiness wait, present/absent distinction).
///
/// Re-observe is ALWAYS bounded by the Task 1 runaway caps regardless of this
/// flag: every re-observe goes through [`GuiTurnBudgetTracker::note_reobserve`]
/// and the `max_reobserve` budget enforced at the loop's pre-action checkpoint
/// (Requirement 19.4 / 21.3), so it can never run unbounded.
///
/// [`GuiContext`]: crate::agent::gui_cognition::context::GuiContext
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct GuiReobserveConfig {
    /// Whether the explicit per-step re-observe hook is active.
    pub enabled: bool,
}

impl Default for GuiReobserveConfig {
    fn default() -> Self {
        // Task 3: flag default OFF until the Wave 3 gate (Task 3.6) flips it.
        Self { enabled: false }
    }
}

impl GuiReobserveConfig {
    /// Construct an explicitly-enabled re-observe config.
    pub fn enabled() -> Self {
        Self { enabled: true }
    }

    /// Construct an explicitly-disabled re-observe config.
    pub fn disabled() -> Self {
        Self { enabled: false }
    }

    /// Whether the explicit per-step re-observe hook should run.
    pub fn is_enabled(&self) -> bool {
        self.enabled
    }

    /// Derive the config from the process environment. The flag is OFF unless
    /// [`REOBSERVE_ENV_FLAG`] is truthy.
    pub fn from_env() -> Self {
        Self::from_env_lookup(|key| std::env::var(key).ok())
    }

    /// Testable core of [`from_env`](Self::from_env) with an injectable lookup.
    pub fn from_env_lookup<F>(lookup: F) -> Self
    where
        F: Fn(&str) -> Option<String>,
    {
        Self {
            enabled: is_truthy(lookup(REOBSERVE_ENV_FLAG).as_deref()),
        }
    }

    /// Derive the config from the process environment with the flag defaulting
    /// **ON** (Wave 3 gate flip, Task 3.6). The explicit per-step re-observe
    /// hook is active unless [`REOBSERVE_ENV_FLAG`] is explicitly falsy
    /// (`0`/`false`/`no`/`off`/empty), which is the documented rollback switch.
    /// An absent env value keeps the flag ON.
    pub fn from_env_default_on() -> Self {
        Self::from_env_lookup_default_on(|key| std::env::var(key).ok())
    }

    /// Testable core of [`from_env_default_on`](Self::from_env_default_on) with
    /// an injectable lookup.
    pub fn from_env_lookup_default_on<F>(lookup: F) -> Self
    where
        F: Fn(&str) -> Option<String>,
    {
        // Default ON: enabled unless the operator explicitly opts out via a
        // falsy env value (the rollback switch). Absent (None) is NOT falsy.
        Self {
            enabled: !is_falsy(lookup(REOBSERVE_ENV_FLAG).as_deref()),
        }
    }
}

/// A runaway-control abort decision produced by [`GuiTurnBudgetTracker`].
///
/// Carries a stable `cause` tag (one of [`abort_cause`]) and a sanitized,
/// human-readable `reason` suitable for the `WorkflowRunAborted` event and the
/// run's `blocked_reason`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BudgetAbort {
    /// Stable cause tag (see [`abort_cause`]).
    pub cause: &'static str,
    /// Sanitized, human-readable reason.
    pub reason: String,
}

impl BudgetAbort {
    fn new(cause: &'static str, reason: impl Into<String>) -> Self {
        Self {
            cause,
            reason: reason.into(),
        }
    }
}

/// Per-turn accounting for the loop-level runaway-control caps (Task 1.3).
///
/// The runtime constructs one tracker per turn (capturing a monotonic
/// [`Instant`] at turn start) and, at the SAME pre-action checkpoint that
/// Task 1.2 added, calls [`evaluate`](Self::evaluate). Between iterations it
/// records progress signals: [`note_step`](Self::note_step),
/// [`note_reobserve`](Self::note_reobserve),
/// [`note_screen_hash`](Self::note_screen_hash) and
/// [`note_verification`](Self::note_verification).
///
/// When `gui_cog_runtime_guards` is OFF the tracker is inert: every `evaluate`
/// returns `None` and existing Step 1–12 behavior is preserved.
#[derive(Debug)]
pub struct GuiTurnBudgetTracker {
    budget: TurnBudget,
    enabled: bool,
    started: Instant,
    steps_executed: u32,
    reobserve_count: u32,
    consecutive_verification_failures: u32,
    screen_history: VecDeque<String>,
}

/// Recent-screen window kept for flapping detection. Bounded so the tracker
/// never grows unbounded over a long turn.
const FLAPPING_WINDOW: usize = 12;

impl GuiTurnBudgetTracker {
    /// Construct a tracker for the current turn, capturing the turn-start clock.
    pub fn new(guards: &GuiRuntimeGuardConfig) -> Self {
        Self {
            budget: guards.budget,
            enabled: guards.is_enforced(),
            started: Instant::now(),
            steps_executed: 0,
            reobserve_count: 0,
            consecutive_verification_failures: 0,
            screen_history: VecDeque::new(),
        }
    }

    /// Whether budget enforcement is active for this turn.
    pub fn is_enforced(&self) -> bool {
        self.enabled
    }

    /// Milliseconds elapsed since turn start (monotonic).
    pub fn elapsed_ms(&self) -> u64 {
        self.started.elapsed().as_millis().min(u128::from(u64::MAX)) as u64
    }

    /// Re-observations recorded so far this turn (Task 3.1: surfaced so the
    /// re-observe hook can report the cap binding). Counts toward the
    /// `max_reobserve` budget enforced at the loop's pre-action checkpoint.
    pub fn reobserve_count(&self) -> u32 {
        self.reobserve_count
    }

    /// The effective re-observe cap for this turn (Requirement 19.4 ceiling),
    /// surfaced for the re-observe hook event.
    pub fn effective_max_reobserve(&self) -> u32 {
        self.budget.effective_max_reobserve()
    }

    /// Record that a plan step is about to be processed (counts toward
    /// `max_steps`).
    pub fn note_step(&mut self) {
        self.steps_executed = self.steps_executed.saturating_add(1);
    }

    /// Record a re-observation (counts toward the re-observe cap).
    pub fn note_reobserve(&mut self) {
        self.reobserve_count = self.reobserve_count.saturating_add(1);
    }

    /// Record the screen hash of a fresh observation for flapping detection.
    pub fn note_screen_hash(&mut self, hash: Option<&str>) {
        let Some(hash) = hash else { return };
        if hash.trim().is_empty() {
            return;
        }
        if self.screen_history.len() >= FLAPPING_WINDOW {
            self.screen_history.pop_front();
        }
        self.screen_history.push_back(hash.to_string());
    }

    /// Record a verification outcome. A pass resets the consecutive-failure
    /// counter; a failure increments it (Requirement 21.4).
    pub fn note_verification(&mut self, passed: bool) {
        if passed {
            self.consecutive_verification_failures = 0;
        } else {
            self.consecutive_verification_failures =
                self.consecutive_verification_failures.saturating_add(1);
        }
    }

    /// Number of times the most-recent screen hash recurs within the window.
    fn latest_screen_repeat_count(&self) -> u32 {
        let Some(latest) = self.screen_history.back() else {
            return 0;
        };
        self.screen_history
            .iter()
            .filter(|hash| *hash == latest)
            .count() as u32
    }

    /// Evaluate every loop-level cap using the monotonic clock. Returns the
    /// first breached cap (if any), or `None` to proceed.
    pub fn evaluate(&self) -> Option<BudgetAbort> {
        self.evaluate_at(self.elapsed_ms())
    }

    /// Testable core of [`evaluate`](Self::evaluate) with an injected elapsed
    /// time so the watchdog path is deterministic in unit tests.
    pub fn evaluate_at(&self, elapsed_ms: u64) -> Option<BudgetAbort> {
        if !self.enabled {
            return None;
        }
        // Hard time/step/loop caps first (Requirement 19.2/19.4, 21.3).
        if elapsed_ms >= self.budget.turn_watchdog_ms {
            return Some(BudgetAbort::new(
                abort_cause::BUDGET_WATCHDOG,
                format!(
                    "turn watchdog elapsed ({elapsed_ms} ms >= {} ms budget)",
                    self.budget.turn_watchdog_ms
                ),
            ));
        }
        if self.steps_executed >= self.budget.max_steps {
            return Some(BudgetAbort::new(
                abort_cause::BUDGET_MAX_STEPS,
                format!(
                    "step budget reached ({} of max {} steps)",
                    self.steps_executed, self.budget.max_steps
                ),
            ));
        }
        if self.reobserve_count >= self.budget.effective_max_reobserve() {
            return Some(BudgetAbort::new(
                abort_cause::BUDGET_MAX_REOBSERVE,
                format!(
                    "re-observe budget reached ({} of max {})",
                    self.reobserve_count,
                    self.budget.effective_max_reobserve()
                ),
            ));
        }
        // Then progress-based caps (Requirement 21.4): abort rather than loop.
        if self.consecutive_verification_failures >= self.budget.max_verification_failures {
            return Some(BudgetAbort::new(
                abort_cause::REPEATED_VERIFICATION_FAILURE,
                format!(
                    "verification failed {} times in a row (max {})",
                    self.consecutive_verification_failures, self.budget.max_verification_failures
                ),
            ));
        }
        if self.latest_screen_repeat_count() >= self.budget.flapping_threshold {
            return Some(BudgetAbort::new(
                abort_cause::FLAPPING,
                format!(
                    "screen state repeated {} times without progress (flapping threshold {})",
                    self.latest_screen_repeat_count(),
                    self.budget.flapping_threshold
                ),
            ));
        }
        None
    }
}

fn parse_env<F, T>(lookup: &F, key: &str) -> Option<T>
where
    F: Fn(&str) -> Option<String>,
    T: std::str::FromStr,
{
    lookup(key)
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .and_then(|value| value.parse::<T>().ok())
}

fn is_truthy(value: Option<&str>) -> bool {
    matches!(
        value.map(|v| v.trim().to_ascii_lowercase()).as_deref(),
        Some("1") | Some("true") | Some("yes") | Some("on")
    )
}

/// Whether an env value is an explicit opt-OUT. Used by the default-ON path
/// ([`GuiRuntimeGuardConfig::from_env_lookup_default_on`]) as the documented
/// rollback switch: an empty or `0`/`false`/`no`/`off` value disables the
/// runtime guards. An absent value (None) is NOT falsy — default stays ON.
fn is_falsy(value: Option<&str>) -> bool {
    matches!(
        value.map(|v| v.trim().to_ascii_lowercase()).as_deref(),
        Some("0") | Some("false") | Some("no") | Some("off") | Some("")
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn lookup_from(pairs: &[(&str, &str)]) -> impl Fn(&str) -> Option<String> {
        let map: HashMap<String, String> = pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();
        move |key: &str| map.get(key).cloned()
    }

    #[test]
    fn defaults_match_nfr_requirement_19() {
        let budget = TurnBudget::default();
        // 19.2 combo bounds
        assert_eq!(budget.max_steps, 12);
        assert_eq!(budget.turn_watchdog_ms, 90_000);
        // 19.3 per-step bounded timeouts
        assert_eq!(budget.step_resolve_ms, 4_000);
        assert_eq!(budget.step_verify_ms, 4_000);
        // 19.1 single-primitive budget (≤ 8 s)
        assert_eq!(budget.single_primitive_budget_ms, 8_000);
        assert!(budget.single_primitive_budget_ms <= 8_000);
        // 19.4 re-observe cap ≤ max_steps + 4
        assert_eq!(budget.max_reobserve, 16);
        assert_eq!(budget.max_reobserve, budget.max_steps + 4);
        // 21.4 runaway-control defaults
        assert_eq!(budget.max_verification_failures, 2);
        assert_eq!(budget.flapping_threshold, 3);
    }

    #[test]
    fn default_budget_is_valid() {
        assert_eq!(TurnBudget::default().validate(), Ok(()));
    }

    #[test]
    fn builders_apply_custom_configuration() {
        let budget = TurnBudget::default()
            .with_max_steps(6)
            .with_turn_watchdog_ms(30_000)
            .with_step_resolve_ms(1_500)
            .with_step_verify_ms(2_500)
            .with_single_primitive_budget_ms(5_000)
            .with_max_reobserve(9);
        assert_eq!(budget.max_steps, 6);
        assert_eq!(budget.turn_watchdog_ms, 30_000);
        assert_eq!(budget.step_resolve_ms, 1_500);
        assert_eq!(budget.step_verify_ms, 2_500);
        assert_eq!(budget.single_primitive_budget_ms, 5_000);
        assert_eq!(budget.max_reobserve, 9);
        assert_eq!(budget.validate(), Ok(()));
    }

    #[test]
    fn effective_max_reobserve_is_capped_at_ceiling() {
        // Configured above the 19.4 ceiling → clamped to max_steps + 4.
        let budget = TurnBudget::default().with_max_steps(8).with_max_reobserve(100);
        assert_eq!(budget.reobserve_ceiling(), 12);
        assert_eq!(budget.effective_max_reobserve(), 12);

        // Configured below the ceiling → preserved.
        let budget = TurnBudget::default().with_max_steps(8).with_max_reobserve(5);
        assert_eq!(budget.effective_max_reobserve(), 5);
    }

    #[test]
    fn validate_rejects_zero_fields() {
        assert_eq!(
            TurnBudget::default().with_max_steps(0).validate(),
            Err(TurnBudgetError::NonPositive { field: "max_steps" })
        );
        assert_eq!(
            TurnBudget::default().with_turn_watchdog_ms(0).validate(),
            Err(TurnBudgetError::NonPositive {
                field: "turn_watchdog_ms"
            })
        );
        assert_eq!(
            TurnBudget::default().with_max_reobserve(0).validate(),
            Err(TurnBudgetError::NonPositive {
                field: "max_reobserve"
            })
        );
    }

    #[test]
    fn serde_roundtrip_preserves_custom_budget() {
        let budget = TurnBudget::default()
            .with_max_steps(7)
            .with_turn_watchdog_ms(45_000);
        let encoded = serde_json::to_string(&budget).expect("serialize");
        let decoded: TurnBudget = serde_json::from_str(&encoded).expect("deserialize");
        assert_eq!(decoded, budget);
    }

    #[test]
    fn serde_partial_config_fills_field_defaults() {
        // Only one field provided; the rest must fall back to NFR defaults.
        let decoded: TurnBudget =
            serde_json::from_str(r#"{ "max_steps": 4 }"#).expect("deserialize partial");
        assert_eq!(decoded.max_steps, 4);
        assert_eq!(decoded.turn_watchdog_ms, DEFAULT_TURN_WATCHDOG_MS);
        assert_eq!(decoded.step_resolve_ms, DEFAULT_STEP_RESOLVE_MS);
        assert_eq!(decoded.step_verify_ms, DEFAULT_STEP_VERIFY_MS);
        assert_eq!(
            decoded.single_primitive_budget_ms,
            DEFAULT_SINGLE_PRIMITIVE_BUDGET_MS
        );
        assert_eq!(decoded.max_reobserve, DEFAULT_MAX_REOBSERVE);
    }

    #[test]
    fn from_env_overrides_only_provided_fields() {
        let budget = TurnBudget::from_env_lookup(lookup_from(&[
            (ENV_MAX_STEPS, "20"),
            (ENV_TURN_WATCHDOG_MS, "120000"),
        ]));
        assert_eq!(budget.max_steps, 20);
        assert_eq!(budget.turn_watchdog_ms, 120_000);
        // Unset fields keep defaults.
        assert_eq!(budget.step_resolve_ms, DEFAULT_STEP_RESOLVE_MS);
        assert_eq!(budget.max_reobserve, DEFAULT_MAX_REOBSERVE);
    }

    #[test]
    fn from_env_ignores_unparsable_and_empty_overrides() {
        let budget = TurnBudget::from_env_lookup(lookup_from(&[
            (ENV_MAX_STEPS, "not-a-number"),
            (ENV_TURN_WATCHDOG_MS, "   "),
        ]));
        assert_eq!(budget.max_steps, DEFAULT_MAX_STEPS);
        assert_eq!(budget.turn_watchdog_ms, DEFAULT_TURN_WATCHDOG_MS);
    }

    #[test]
    fn runtime_guard_flag_defaults_off() {
        let guards = GuiRuntimeGuardConfig::default();
        assert!(!guards.enabled);
        assert!(!guards.is_enforced());
        assert_eq!(guards.budget, TurnBudget::default());
    }

    #[test]
    fn runtime_guard_flag_off_unless_truthy_env() {
        let guards = GuiRuntimeGuardConfig::from_env_lookup(lookup_from(&[]));
        assert!(!guards.is_enforced());

        for raw in ["0", "false", "no", "off", ""] {
            let guards = GuiRuntimeGuardConfig::from_env_lookup(lookup_from(&[(
                RUNTIME_GUARDS_ENV_FLAG,
                raw,
            )]));
            assert!(!guards.is_enforced(), "flag {raw:?} must keep guards OFF");
        }
    }

    #[test]
    fn runtime_guard_flag_on_when_truthy_env() {
        for raw in ["1", "true", "YES", "On"] {
            let guards = GuiRuntimeGuardConfig::from_env_lookup(lookup_from(&[(
                RUNTIME_GUARDS_ENV_FLAG,
                raw,
            )]));
            assert!(guards.is_enforced(), "flag {raw:?} must enable guards");
        }
    }

    #[test]
    fn runtime_guard_default_on_when_env_absent_or_truthy() {
        // Task 1.6 gate flip: the live/desktop path defaults ON.
        let guards = GuiRuntimeGuardConfig::from_env_lookup_default_on(lookup_from(&[]));
        assert!(
            guards.is_enforced(),
            "default-on path must enable guards when env is unset"
        );

        for raw in ["1", "true", "YES", "On", "anything-else"] {
            let guards = GuiRuntimeGuardConfig::from_env_lookup_default_on(lookup_from(&[(
                RUNTIME_GUARDS_ENV_FLAG,
                raw,
            )]));
            assert!(
                guards.is_enforced(),
                "default-on path must keep guards ON for {raw:?}"
            );
        }
    }

    #[test]
    fn runtime_guard_default_on_rolls_back_when_env_explicitly_falsy() {
        // Documented rollback: KRIA_GUI_COG_RUNTIME_GUARDS=0/false/no/off/"".
        for raw in ["0", "false", "no", "off", "", " OFF "] {
            let guards = GuiRuntimeGuardConfig::from_env_lookup_default_on(lookup_from(&[(
                RUNTIME_GUARDS_ENV_FLAG,
                raw,
            )]));
            assert!(
                !guards.is_enforced(),
                "explicit falsy {raw:?} must roll back (disable) the guards"
            );
        }
    }

    #[test]
    fn runtime_guard_default_on_threads_budget_overrides() {
        let guards = GuiRuntimeGuardConfig::from_env_lookup_default_on(lookup_from(&[(
            ENV_MAX_STEPS,
            "3",
        )]));
        assert!(guards.is_enforced());
        assert_eq!(guards.budget.max_steps, 3);
    }

    #[test]
    fn runtime_guard_threads_budget_overrides_even_when_off() {
        // Budget overrides are honored for inspection regardless of flag state.
        let guards = GuiRuntimeGuardConfig::from_env_lookup(lookup_from(&[(ENV_MAX_STEPS, "3")]));
        assert!(!guards.is_enforced());
        assert_eq!(guards.budget.max_steps, 3);
    }

    #[test]
    fn enabled_constructor_sets_flag_and_budget() {
        let budget = TurnBudget::default().with_max_steps(5);
        let guards = GuiRuntimeGuardConfig::enabled(budget);
        assert!(guards.is_enforced());
        assert_eq!(guards.budget.max_steps, 5);
    }

    #[test]
    fn summary_json_exposes_all_budgets_without_secrets() {
        let summary = TurnBudget::default().summary_json();
        assert_eq!(summary["max_steps"], 12);
        assert_eq!(summary["turn_watchdog_ms"], 90_000);
        assert_eq!(summary["step_resolve_ms"], 4_000);
        assert_eq!(summary["step_verify_ms"], 4_000);
        assert_eq!(summary["single_primitive_budget_ms"], 8_000);
        assert_eq!(summary["max_reobserve"], 16);
        assert_eq!(summary["effective_max_reobserve"], 16);
        assert_eq!(summary["max_verification_failures"], 2);
        assert_eq!(summary["flapping_threshold"], 3);
    }

    // ── Task 1.3: GuiTurnBudgetTracker runaway-control caps ──────────────────

    fn tracker_on(budget: TurnBudget) -> GuiTurnBudgetTracker {
        GuiTurnBudgetTracker::new(&GuiRuntimeGuardConfig::enabled(budget))
    }

    #[test]
    fn tracker_defaults_to_proceed() {
        let tracker = tracker_on(TurnBudget::default());
        assert!(tracker.is_enforced());
        assert_eq!(tracker.evaluate_at(0), None);
    }

    #[test]
    fn tracker_disabled_never_aborts() {
        // Flag OFF: even a wildly breached state must proceed (behavior preserved).
        let mut tracker = GuiTurnBudgetTracker::new(&GuiRuntimeGuardConfig::default());
        assert!(!tracker.is_enforced());
        for _ in 0..100 {
            tracker.note_step();
            tracker.note_reobserve();
            tracker.note_verification(false);
            tracker.note_screen_hash(Some("stuck"));
        }
        assert_eq!(tracker.evaluate_at(u64::MAX), None);
    }

    #[test]
    fn tracker_aborts_on_max_steps() {
        let mut tracker = tracker_on(TurnBudget::default().with_max_steps(2));
        tracker.note_step();
        assert_eq!(tracker.evaluate_at(0), None, "one step is within budget");
        tracker.note_step();
        let abort = tracker.evaluate_at(0).expect("max_steps abort");
        assert_eq!(abort.cause, abort_cause::BUDGET_MAX_STEPS);
        assert!(abort.reason.contains("step budget"), "{}", abort.reason);
    }

    #[test]
    fn tracker_aborts_on_watchdog() {
        let tracker = tracker_on(TurnBudget::default().with_turn_watchdog_ms(1_000));
        assert_eq!(tracker.evaluate_at(999), None);
        let abort = tracker.evaluate_at(1_000).expect("watchdog abort");
        assert_eq!(abort.cause, abort_cause::BUDGET_WATCHDOG);
        assert!(abort.reason.contains("watchdog"), "{}", abort.reason);
    }

    #[test]
    fn tracker_aborts_on_max_reobserve() {
        // effective cap = min(max_reobserve, max_steps + 4) = 2.
        let mut tracker =
            tracker_on(TurnBudget::default().with_max_steps(8).with_max_reobserve(2));
        tracker.note_reobserve();
        assert_eq!(tracker.evaluate_at(0), None);
        tracker.note_reobserve();
        let abort = tracker.evaluate_at(0).expect("max_reobserve abort");
        assert_eq!(abort.cause, abort_cause::BUDGET_MAX_REOBSERVE);
        assert!(abort.reason.contains("re-observe"), "{}", abort.reason);
    }

    #[test]
    fn tracker_aborts_on_repeated_verification_failure() {
        let mut tracker = tracker_on(TurnBudget::default().with_max_verification_failures(2));
        tracker.note_verification(false);
        assert_eq!(tracker.evaluate_at(0), None, "one failure is tolerated");
        tracker.note_verification(false);
        let abort = tracker.evaluate_at(0).expect("repeated verification abort");
        assert_eq!(abort.cause, abort_cause::REPEATED_VERIFICATION_FAILURE);
    }

    #[test]
    fn tracker_verification_pass_resets_the_streak() {
        let mut tracker = tracker_on(TurnBudget::default().with_max_verification_failures(2));
        tracker.note_verification(false);
        tracker.note_verification(true); // reset
        tracker.note_verification(false);
        assert_eq!(tracker.evaluate_at(0), None, "streak was reset by the pass");
    }

    #[test]
    fn tracker_aborts_on_flapping() {
        // Same screen hash recurring `flapping_threshold` times → flapping.
        let mut tracker = tracker_on(TurnBudget::default().with_flapping_threshold(3));
        tracker.note_screen_hash(Some("screen-A"));
        tracker.note_screen_hash(Some("screen-A"));
        assert_eq!(tracker.evaluate_at(0), None, "two repeats is not yet flapping");
        tracker.note_screen_hash(Some("screen-A"));
        let abort = tracker.evaluate_at(0).expect("flapping abort");
        assert_eq!(abort.cause, abort_cause::FLAPPING);
        assert!(abort.reason.contains("flapping"), "{}", abort.reason);
    }

    #[test]
    fn tracker_distinct_screens_do_not_flap() {
        let mut tracker = tracker_on(TurnBudget::default().with_flapping_threshold(3));
        for seq in 0..10 {
            tracker.note_screen_hash(Some(&format!("screen-{seq}")));
            assert_eq!(tracker.evaluate_at(0), None, "progress is not flapping");
        }
    }

    #[test]
    fn tracker_ignores_empty_and_missing_screen_hashes() {
        let mut tracker = tracker_on(TurnBudget::default().with_flapping_threshold(2));
        tracker.note_screen_hash(None);
        tracker.note_screen_hash(Some("  "));
        tracker.note_screen_hash(None);
        assert_eq!(tracker.evaluate_at(0), None);
    }

    #[test]
    fn tracker_watchdog_has_priority_over_step_cap() {
        // Both watchdog and step cap breached; watchdog is reported first.
        let mut tracker = tracker_on(
            TurnBudget::default()
                .with_max_steps(1)
                .with_turn_watchdog_ms(1_000),
        );
        tracker.note_step();
        let abort = tracker.evaluate_at(2_000).expect("abort");
        assert_eq!(abort.cause, abort_cause::BUDGET_WATCHDOG);
    }

    // ── Task 3.1: GuiReobserveConfig flag + tracker accessors ────────────────

    #[test]
    fn reobserve_flag_defaults_off() {
        assert!(!GuiReobserveConfig::default().is_enabled());
        assert!(GuiReobserveConfig::enabled().is_enabled());
        assert!(!GuiReobserveConfig::disabled().is_enabled());
    }

    #[test]
    fn reobserve_flag_off_unless_truthy_env() {
        // Unset env → OFF.
        assert!(!GuiReobserveConfig::from_env_lookup(lookup_from(&[])).is_enabled());
        // Non-truthy values stay OFF.
        for raw in ["0", "false", "no", "off", "", "maybe"] {
            let cfg = GuiReobserveConfig::from_env_lookup(lookup_from(&[(REOBSERVE_ENV_FLAG, raw)]));
            assert!(!cfg.is_enabled(), "flag {raw:?} must keep re-observe OFF");
        }
    }

    #[test]
    fn reobserve_flag_on_when_truthy_env() {
        for raw in ["1", "true", "YES", "On", " on "] {
            let cfg = GuiReobserveConfig::from_env_lookup(lookup_from(&[(REOBSERVE_ENV_FLAG, raw)]));
            assert!(cfg.is_enabled(), "flag {raw:?} must enable re-observe");
        }
    }

    #[test]
    fn reobserve_default_on_when_env_absent_or_truthy() {
        // Wave 3 gate flip (Task 3.6): the live/desktop default-on path.
        assert!(
            GuiReobserveConfig::from_env_lookup_default_on(lookup_from(&[])).is_enabled(),
            "default-on path must enable re-observe when env is unset"
        );
        for raw in ["1", "true", "YES", "On", "anything-else"] {
            let cfg = GuiReobserveConfig::from_env_lookup_default_on(lookup_from(&[(
                REOBSERVE_ENV_FLAG,
                raw,
            )]));
            assert!(cfg.is_enabled(), "default-on path must keep ON for {raw:?}");
        }
    }

    #[test]
    fn reobserve_default_on_rolls_back_when_env_explicitly_falsy() {
        for raw in ["0", "false", "no", "off", "", " OFF "] {
            let cfg = GuiReobserveConfig::from_env_lookup_default_on(lookup_from(&[(
                REOBSERVE_ENV_FLAG,
                raw,
            )]));
            assert!(
                !cfg.is_enabled(),
                "explicit falsy {raw:?} must roll back (disable) re-observe"
            );
        }
    }

    #[test]
    fn tracker_exposes_reobserve_count_and_cap() {
        // Re-observe accounting is surfaced so the Task 3.1 hook can report the
        // cap binding; the effective cap honors the 19.4 ceiling.
        let mut tracker =
            tracker_on(TurnBudget::default().with_max_steps(8).with_max_reobserve(100));
        assert_eq!(tracker.reobserve_count(), 0);
        assert_eq!(tracker.effective_max_reobserve(), 12); // min(100, 8 + 4)
        tracker.note_reobserve();
        tracker.note_reobserve();
        assert_eq!(tracker.reobserve_count(), 2);
    }
}
