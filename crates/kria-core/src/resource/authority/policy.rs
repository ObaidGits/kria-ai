//! Runtime Resource Policy Engine (HRA Task 68+70 / redesign G2, G4, G7).
//!
//! The deterministic brain that decides whether the runtime should change a model's residency.
//! It owns the DECISION; the watchdog/executor only performs the I/O it is handed and re-validates
//! preconditions immediately before acting (G2). The governing law is encoded structurally:
//!
//!   *Never restart for performance. A restart is permitted only for (a) correctness/safety, or
//!    (b) an explicit user workflow.*
//!
//! Performance promotions (`Optimize`) are therefore gated behind a strict AND of conditions (G4)
//! and only emitted in the `Maintenance` runtime mode (DeepIdle). The default output in
//! `Interactive` mode is always `Stay`. This module is pure: it maps an immutable `PolicyInputs`
//! snapshot to a `Decision` and is fully unit-testable.

use serde::{Deserialize, Serialize};

use super::activity::ActivityState;
use super::benefit::{Benefit, BenefitEval};
use super::simulator::{Estimate, RiskLevel};

/// Runtime mode (G7). Derived from activity + telemetry health + recovery/emergency posture.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RuntimeMode {
    /// User is engaged. Hold residency locked; NO restart-for-performance.
    Interactive,
    /// Deep idle. The only mode where a single safe promotion may occur.
    Maintenance,
    /// Restoring last-good after a crash/restart. Restore, do not optimize.
    Recovery,
    /// OOM / driver fault. Shrink to safe immediately; upsize forbidden.
    Emergency,
    /// No UI focus. Background/cloud work at low priority; no foreground preempt.
    Background,
    /// Idle but not deep-idle / still focused. Hold; background-class allowed.
    Idle,
    /// Local unavailable/unhealthy → prefer cloud routing.
    Cloud,
}

/// Telemetry confidence — Unknown telemetry must never drive an optimization (C2/C5 + G4).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Confidence {
    Measured,
    Unknown,
}

/// Health posture handed in from the watchdog/telemetry hub.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HealthPosture {
    Healthy,
    /// Local GPU runtime is degraded/failed → cloud preferred.
    LocalUnhealthy,
    /// Correctness-threatening VRAM pressure sustained → must shrink.
    Pressure,
    /// Hard fault (OOM / driver reset) — emergency.
    Faulted,
    /// Recovering last-good config after a restart.
    Recovering,
}

/// Where the model currently sits relative to the Resident Lock state machine (G3). The policy
/// only treats `PreLockResident` and `CpuOrCloudWantingPromotion` as *eligible* for a promotion;
/// once `Locked`, performance restarts are structurally impossible.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LockPosture {
    /// Settled on GPU and locked — steady state for ~all of a session. No perf restart.
    Locked,
    /// Loaded on GPU but not yet locked (the post-startup stabilize window).
    PreLockResident,
    /// Running on CPU or cloud and would benefit from a one-time GPU promotion.
    CpuOrCloudWantingPromotion,
    /// Mid-transition (loading/recovering) — never start a new optimization.
    Transitioning,
}

/// Immutable snapshot the policy decides on.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PolicyInputs {
    pub activity: ActivityState,
    pub confidence: Confidence,
    pub health: HealthPosture,
    pub lock: LockPosture,
    /// Whether the per-host cooldown since the last GPU failure/restart has elapsed.
    pub cooldown_elapsed: bool,
    /// Forecast says free VRAM is sustainably sufficient (low volatility, not trending down).
    pub forecast_sustainable: bool,
    /// Simulator estimate for the proposed promotion (None if no promotion is on the table).
    pub sim: Option<Estimate>,
    /// Benefit evaluation for the proposed promotion (None if none proposed).
    pub benefit: Option<BenefitEval>,
}

/// The policy output. `Optimize`/`Migrate` are Restart-class; everything else is non-disruptive.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Action {
    /// Do nothing — hold the current (locked) residency. The overwhelming default.
    Stay,
    /// Perform a single safe GPU promotion (Restart-class). Only in Maintenance, all gates passed.
    Optimize,
    /// Move residency across devices (Restart-class) — e.g. multi-GPU rebalance. Same gates.
    Migrate,
    /// Conditions not met now but might be later — re-evaluate, do not act.
    Defer,
    /// Restore last-good after a crash/restart (correctness, not performance).
    Recover,
    /// Route to cloud (local unhealthy or privacy/headroom).
    Cloud,
    /// Shrink to a safe size immediately (correctness/safety; may break the lock).
    Reject,
}

/// Rationale code for decision-grade logging (G11).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PolicyReason {
    HoldLocked,
    EmergencyShrink,
    RecoverLastGood,
    CloudPreferred,
    NotEligibleActivity,
    NotEligibleMode,
    NotEligibleConfidence,
    NotEligibleLock,
    NotEligibleCooldown,
    NotEligibleForecast,
    NotEligibleSimulator,
    NotEligibleBenefit,
    PromotionApproved,
}

impl PolicyReason {
    pub fn human(&self) -> &'static str {
        match self {
            Self::HoldLocked => "Model is locked in place; no change needed.",
            Self::EmergencyShrink => "Reducing GPU use to stay stable.",
            Self::RecoverLastGood => "Recovering the last known-good configuration.",
            Self::CloudPreferred => "Using cloud — local runtime is unavailable.",
            Self::NotEligibleActivity => "Holding — you are active; never restart mid-work.",
            Self::NotEligibleMode => "Holding — not in a maintenance (deep-idle) window.",
            Self::NotEligibleConfidence => "Holding — GPU memory readings are not trustworthy yet.",
            Self::NotEligibleLock => "Holding — model is already locked at a good size.",
            Self::NotEligibleCooldown => {
                "Holding — waiting out the cooldown after a recent change."
            }
            Self::NotEligibleForecast => "Holding — free memory is not reliably sufficient.",
            Self::NotEligibleSimulator => "Holding — a resize is predicted to be unsafe.",
            Self::NotEligibleBenefit => "Holding — a resize would not be worth the interruption.",
            Self::PromotionApproved => "Promoting to GPU — safe and worthwhile while idle.",
        }
    }
}

/// Full decision with the data needed for the journal (G11).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Decision {
    pub action: Action,
    pub reason: PolicyReason,
    /// Expected speedup if this is a promotion (1.0 otherwise).
    pub expected_speedup: f32,
    /// Estimated cost (ms) of the action (0 for Stay).
    pub expected_cost_ms: u32,
    /// Risk of the action.
    pub risk: RiskLevel,
}

impl Decision {
    fn stay(reason: PolicyReason) -> Self {
        Self {
            action: Action::Stay,
            reason,
            expected_speedup: 1.0,
            expected_cost_ms: 0,
            risk: RiskLevel::Low,
        }
    }

    /// True when this decision requires a process restart (kill+respawn).
    pub fn is_restart_class(&self) -> bool {
        matches!(self.action, Action::Optimize | Action::Migrate)
    }
}

/// Decision-grade log record (redesign G11). Every policy decision is journaled with the full
/// context needed to explain it post-hoc: who asked, why, the before/after intent, the predicted
/// benefit/cost/risk, and the outcome. Low-cardinality fields only (codes, not free text) so it is
/// safe to emit as structured tracing + persist to the decision journal.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PolicyLog {
    /// Correlation id (turn/session) tying this to the request and events.
    pub correlation_id: String,
    /// Consumer the decision was made for.
    pub who: super::types::ConsumerId,
    pub mode: RuntimeMode,
    pub action: Action,
    pub reason: PolicyReason,
    pub expected_speedup: f32,
    pub expected_cost_ms: u32,
    pub risk: RiskLevel,
    /// Monotonic ms when the decision was made.
    pub when_ms: u64,
}

impl PolicyLog {
    pub fn from_decision(
        correlation_id: impl Into<String>,
        who: super::types::ConsumerId,
        mode: RuntimeMode,
        d: &Decision,
        when_ms: u64,
    ) -> Self {
        Self {
            correlation_id: correlation_id.into(),
            who,
            mode,
            action: d.action,
            reason: d.reason,
            expected_speedup: d.expected_speedup,
            expected_cost_ms: d.expected_cost_ms,
            risk: d.risk,
            when_ms,
        }
    }

    /// Emit as a structured tracing event (G11). Restart-class decisions log at info; holds at debug
    /// so the normal "Stay" steady state is quiet.
    pub fn emit(&self) {
        if matches!(
            self.action,
            Action::Optimize | Action::Migrate | Action::Reject | Action::Recover
        ) {
            tracing::info!(
                correlation_id = %self.correlation_id,
                who = ?self.who,
                mode = ?self.mode,
                action = ?self.action,
                reason = ?self.reason,
                expected_speedup = self.expected_speedup,
                expected_cost_ms = self.expected_cost_ms,
                risk = ?self.risk,
                "policy decision"
            );
        } else {
            tracing::debug!(
                correlation_id = %self.correlation_id,
                action = ?self.action,
                reason = ?self.reason,
                "policy hold"
            );
        }
    }
}

/// Derive the runtime mode (G7) from activity + health. Health (correctness) dominates activity.
pub fn derive_mode(
    activity: ActivityState,
    health: HealthPosture,
    foreground_focus: bool,
) -> RuntimeMode {
    match health {
        HealthPosture::Faulted => RuntimeMode::Emergency,
        HealthPosture::Recovering => RuntimeMode::Recovery,
        HealthPosture::Pressure => RuntimeMode::Emergency, // sustained pressure = correctness path
        HealthPosture::LocalUnhealthy => RuntimeMode::Cloud,
        HealthPosture::Healthy => match activity {
            ActivityState::Active => RuntimeMode::Interactive,
            ActivityState::Idle => {
                if foreground_focus {
                    RuntimeMode::Idle
                } else {
                    RuntimeMode::Background
                }
            }
            ActivityState::DeepIdle => RuntimeMode::Maintenance,
        },
    }
}

/// The core policy decision (G2 + G4). Pure mapping from inputs → decision.
///
/// Resolution order encodes the governing law's precedence:
///   Emergency (correctness) > Recovery > Cloud(health) > Optimization-eligibility > Stay.
pub fn decide(inputs: &PolicyInputs, mode: RuntimeMode) -> Decision {
    // 1. Correctness/safety first — these may break the lock.
    match mode {
        RuntimeMode::Emergency => {
            return Decision {
                action: Action::Reject,
                reason: PolicyReason::EmergencyShrink,
                expected_speedup: 1.0,
                expected_cost_ms: 0,
                risk: RiskLevel::High,
            };
        }
        RuntimeMode::Recovery => {
            return Decision {
                action: Action::Recover,
                reason: PolicyReason::RecoverLastGood,
                expected_speedup: 1.0,
                expected_cost_ms: 0,
                risk: RiskLevel::Med,
            };
        }
        RuntimeMode::Cloud => {
            return Decision {
                action: Action::Cloud,
                reason: PolicyReason::CloudPreferred,
                expected_speedup: 1.0,
                expected_cost_ms: 0,
                risk: RiskLevel::Low,
            };
        }
        _ => {}
    }

    // 2. Performance optimization is ONLY eligible in Maintenance mode. Every other mode holds.
    if mode != RuntimeMode::Maintenance {
        return Decision::stay(PolicyReason::NotEligibleMode);
    }

    // 3. State-driven eligibility (G4) — ALL must hold, else Stay/Defer.
    if !inputs.activity.allows_perf_restart() {
        return Decision::stay(PolicyReason::NotEligibleActivity);
    }
    if inputs.confidence != Confidence::Measured {
        return Decision::stay(PolicyReason::NotEligibleConfidence);
    }
    match inputs.lock {
        LockPosture::PreLockResident | LockPosture::CpuOrCloudWantingPromotion => {}
        LockPosture::Locked => return Decision::stay(PolicyReason::NotEligibleLock),
        LockPosture::Transitioning => return Decision::stay(PolicyReason::NotEligibleLock),
    }
    if !inputs.cooldown_elapsed {
        return Decision::stay(PolicyReason::NotEligibleCooldown);
    }
    if !inputs.forecast_sustainable {
        return Decision::stay(PolicyReason::NotEligibleForecast);
    }
    let sim = match inputs.sim {
        Some(s) => s,
        None => return Decision::stay(PolicyReason::NotEligibleSimulator),
    };
    if sim.breaches_hard_limit || sim.risk == RiskLevel::High {
        return Decision::stay(PolicyReason::NotEligibleSimulator);
    }
    let benefit = match inputs.benefit {
        Some(b) => b,
        None => return Decision::stay(PolicyReason::NotEligibleBenefit),
    };
    if benefit.benefit != Benefit::WorthIt {
        return Decision::stay(PolicyReason::NotEligibleBenefit);
    }

    // 4. All gates passed → approve a single promotion.
    Decision {
        action: Action::Optimize,
        reason: PolicyReason::PromotionApproved,
        expected_speedup: benefit.expected_speedup,
        expected_cost_ms: sim.est_latency_ms,
        risk: sim.risk,
    }
}

// ── G9: Image generation admission policy ───────────────────────────────────────────────────────
//
// Image generation is the ONE legitimate restart workflow (governing law (b): explicit user
// action). This pure helper decides HOW to satisfy an image request without ever silently
// disrupting an active chat: co-resident if it fits, simulator-gated Tier-B eviction only when the
// user is not actively mid-turn, cloud when local cannot serve safely, else an explicit reject.
// Restoration after Tier-B is deterministic (reuse the locked LLM config) — handled by the caller.

use super::simulator::{simulate, SimAction, SimDeviceState};

/// Outcome of an image-generation admission decision.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ImageAdmission {
    /// Image backend fits alongside the resident LLM — no restart. UX: "Preparing image…".
    CoResident,
    /// Must evict the LLM to free the GPU (Tier-B restart), permitted because the user is idle and
    /// the simulator predicts it is safe. UX: "Freeing GPU for image…" → restore after.
    TierBEvict,
    /// Route the image to cloud (local cannot serve safely / user active / privacy allows).
    CloudFallback,
    /// No viable path — reject with an explicit reason.
    Reject,
}

/// Decide how to admit an image request.
///
/// * `required_vram_mb` — VRAM the image backend needs resident.
/// * `llm_vram_mb`      — VRAM the LLM currently holds (recoverable by eviction).
/// * `state`            — live device slice for the simulator.
/// * `activity`         — user activity (Tier-B eviction is forbidden while Active).
/// * `cloud_ok`         — whether cloud routing is permitted (privacy + config).
pub fn decide_image_admission(
    required_vram_mb: u64,
    llm_vram_mb: u64,
    state: &SimDeviceState,
    activity: ActivityState,
    cloud_ok: bool,
) -> ImageAdmission {
    // 1. Already fits co-resident? (No restart — best outcome.)
    if state.free_vram_mb >= required_vram_mb {
        return ImageAdmission::CoResident;
    }

    // 2. Doesn't fit. If the user is actively engaged, never evict mid-work — prefer cloud.
    if matches!(activity, ActivityState::Active) {
        return if cloud_ok {
            ImageAdmission::CloudFallback
        } else {
            ImageAdmission::Reject
        };
    }

    // 3. User is idle: simulate evicting the LLM to RAM and check the image then fits safely.
    let est = simulate(
        &SimAction::EvictToRam {
            model_vram_mb: llm_vram_mb,
        },
        state,
    );
    let freed = (state.free_vram_mb as i64 + est.d_vram_mb).max(0) as u64;
    let evict_is_safe = est.risk != RiskLevel::High && !est.breaches_hard_limit;
    if freed >= required_vram_mb && evict_is_safe {
        return ImageAdmission::TierBEvict;
    }

    // 4. Eviction won't free enough or is unsafe → cloud if allowed, else reject.
    if cloud_ok {
        ImageAdmission::CloudFallback
    } else {
        ImageAdmission::Reject
    }
}

#[cfg(test)]
mod tests {
    use super::super::benefit::BenefitReason;
    use super::super::simulator::Disruption;
    use super::*;

    fn good_sim() -> Estimate {
        Estimate {
            d_vram_mb: -2000,
            d_ram_mb: 0,
            est_latency_ms: 2500,
            disruption: Disruption::Interactive,
            risk: RiskLevel::Low,
            projected_free_vram_mb: 3000,
            breaches_hard_limit: false,
        }
    }

    fn worth_it() -> BenefitEval {
        BenefitEval {
            benefit: Benefit::WorthIt,
            reason: BenefitReason::WorthIt,
            expected_speedup: 2.5,
        }
    }

    fn eligible_inputs() -> PolicyInputs {
        PolicyInputs {
            activity: ActivityState::DeepIdle,
            confidence: Confidence::Measured,
            health: HealthPosture::Healthy,
            lock: LockPosture::CpuOrCloudWantingPromotion,
            cooldown_elapsed: true,
            forecast_sustainable: true,
            sim: Some(good_sim()),
            benefit: Some(worth_it()),
        }
    }

    #[test]
    fn interactive_mode_always_stays() {
        let inp = eligible_inputs();
        let d = decide(&inp, RuntimeMode::Interactive);
        assert_eq!(d.action, Action::Stay);
        assert_eq!(d.reason, PolicyReason::NotEligibleMode);
    }

    #[test]
    fn fully_eligible_maintenance_promotes() {
        let inp = eligible_inputs();
        let d = decide(&inp, RuntimeMode::Maintenance);
        assert_eq!(d.action, Action::Optimize);
        assert!(d.is_restart_class());
        assert!(d.expected_speedup > 1.0);
    }

    #[test]
    fn locked_model_never_optimizes() {
        let mut inp = eligible_inputs();
        inp.lock = LockPosture::Locked;
        assert_eq!(
            decide(&inp, RuntimeMode::Maintenance).reason,
            PolicyReason::NotEligibleLock
        );
    }

    #[test]
    fn unknown_confidence_blocks() {
        let mut inp = eligible_inputs();
        inp.confidence = Confidence::Unknown;
        assert_eq!(
            decide(&inp, RuntimeMode::Maintenance).reason,
            PolicyReason::NotEligibleConfidence
        );
    }

    #[test]
    fn cooldown_or_forecast_blocks() {
        let mut inp = eligible_inputs();
        inp.cooldown_elapsed = false;
        assert_eq!(decide(&inp, RuntimeMode::Maintenance).action, Action::Stay);
        let mut inp2 = eligible_inputs();
        inp2.forecast_sustainable = false;
        assert_eq!(
            decide(&inp2, RuntimeMode::Maintenance).reason,
            PolicyReason::NotEligibleForecast
        );
    }

    #[test]
    fn high_risk_sim_blocks() {
        let mut inp = eligible_inputs();
        let mut s = good_sim();
        s.risk = RiskLevel::High;
        inp.sim = Some(s);
        assert_eq!(
            decide(&inp, RuntimeMode::Maintenance).reason,
            PolicyReason::NotEligibleSimulator
        );
    }

    #[test]
    fn not_worth_it_benefit_blocks() {
        let mut inp = eligible_inputs();
        inp.benefit = Some(BenefitEval {
            benefit: Benefit::NotWorthIt,
            reason: BenefitReason::InsufficientSpeedup,
            expected_speedup: 1.05,
        });
        assert_eq!(
            decide(&inp, RuntimeMode::Maintenance).reason,
            PolicyReason::NotEligibleBenefit
        );
    }

    #[test]
    fn emergency_rejects_to_shrink() {
        let inp = eligible_inputs();
        let d = decide(&inp, RuntimeMode::Emergency);
        assert_eq!(d.action, Action::Reject);
        assert_eq!(d.reason, PolicyReason::EmergencyShrink);
    }

    #[test]
    fn recovery_recovers() {
        let inp = eligible_inputs();
        assert_eq!(decide(&inp, RuntimeMode::Recovery).action, Action::Recover);
    }

    #[test]
    fn cloud_mode_routes_cloud() {
        let inp = eligible_inputs();
        assert_eq!(decide(&inp, RuntimeMode::Cloud).action, Action::Cloud);
    }

    #[test]
    fn mode_derivation_health_dominates_activity() {
        assert_eq!(
            derive_mode(ActivityState::DeepIdle, HealthPosture::Faulted, false),
            RuntimeMode::Emergency
        );
        assert_eq!(
            derive_mode(ActivityState::Active, HealthPosture::Healthy, true),
            RuntimeMode::Interactive
        );
        assert_eq!(
            derive_mode(ActivityState::DeepIdle, HealthPosture::Healthy, false),
            RuntimeMode::Maintenance
        );
        assert_eq!(
            derive_mode(ActivityState::Idle, HealthPosture::Healthy, true),
            RuntimeMode::Idle
        );
        assert_eq!(
            derive_mode(ActivityState::Idle, HealthPosture::Healthy, false),
            RuntimeMode::Background
        );
    }

    #[test]
    fn policy_log_captures_decision() {
        let inp = eligible_inputs();
        let d = decide(&inp, RuntimeMode::Maintenance);
        let log = PolicyLog::from_decision(
            "turn-42",
            super::super::types::ConsumerId::Llm,
            RuntimeMode::Maintenance,
            &d,
            1234,
        );
        assert_eq!(log.action, Action::Optimize);
        assert_eq!(log.reason, PolicyReason::PromotionApproved);
        assert_eq!(log.correlation_id, "turn-42");
        assert!(log.expected_speedup > 1.0);
        log.emit(); // must not panic
                    // serde round-trip (journal persistence)
        let json = serde_json::to_string(&log).unwrap();
        let back: PolicyLog = serde_json::from_str(&json).unwrap();
        assert_eq!(log, back);
    }

    // ── G9: image admission tests ────────────────────────────────────────────

    fn img_state(
        free_vram: u64,
        total: u64,
        free_ram: u64,
    ) -> super::super::simulator::SimDeviceState {
        use super::super::budget::{BandPolicy, Budget};
        super::super::simulator::SimDeviceState {
            free_vram_mb: free_vram,
            total_vram_mb: total,
            free_ram_mb: free_ram,
            budget: Budget::derive(total, 512, BandPolicy::default()),
        }
    }

    #[test]
    fn image_fits_co_resident_without_restart() {
        let s = img_state(5000, 12288, 16000);
        let d = decide_image_admission(4000, 4000, &s, ActivityState::Active, true);
        assert_eq!(d, ImageAdmission::CoResident);
    }

    #[test]
    fn image_active_user_prefers_cloud_over_eviction() {
        // Doesn't fit (free 2000 < need 4500), user active → never evict mid-work.
        let s = img_state(2000, 12288, 16000);
        let d = decide_image_admission(4500, 4000, &s, ActivityState::Active, true);
        assert_eq!(d, ImageAdmission::CloudFallback);
    }

    #[test]
    fn image_active_no_cloud_rejects() {
        let s = img_state(2000, 12288, 16000);
        let d = decide_image_admission(4500, 4000, &s, ActivityState::Active, false);
        assert_eq!(d, ImageAdmission::Reject);
    }

    #[test]
    fn image_idle_safe_eviction_does_tier_b() {
        // free 2000, evicting 4000 of LLM frees to ~6000 ≥ need 4500, RAM ample → Tier-B.
        let s = img_state(2000, 12288, 16000);
        let d = decide_image_admission(4500, 4000, &s, ActivityState::DeepIdle, true);
        assert_eq!(d, ImageAdmission::TierBEvict);
    }

    #[test]
    fn image_idle_but_eviction_insufficient_falls_back_cloud() {
        // Even after evicting the small LLM (1000) there isn't enough for a huge need.
        let s = img_state(1000, 12288, 16000);
        let d = decide_image_admission(9000, 1000, &s, ActivityState::Idle, true);
        assert_eq!(d, ImageAdmission::CloudFallback);
    }
}
