//! GPU Watchdog — telemetry polling loop with hysteresis-based state machine.
//!
//! # States
//!
//! ```text
//! Idle ──────────────────────────────────► Pressured(since, target)
//!  ▲  EMA-V < yield_threshold (dwell ok)       │
//!  │                                            │ sustained ≥ pressure_dwell_secs
//!  │                                            │ AND rate-limit budget OK
//!  │                                            ▼
//!  │                                       Swapping ──► Cooldown(until)
//!  │                                                         │
//!  │                                                         │ until elapsed
//!  │                                                         ▼
//!  │                          EMA-V > recover_threshold ─► Recovering(since)
//!  │                                                         │
//!  └─────────────── recovery_dwell_secs elapsed ◄───────────┘
//!
//! Any state → Critical(since) when EMA-V < emergency_threshold
//!                               (for ≥ emergency_dwell_ms)
//! Critical → Swapping (emergency, separate rate budget)
//! ```
//!
//! # Anti-thrash guarantees
//! - **EMA debouncing**: single-sample spikes don't trigger transitions.
//! - **Hysteresis band**: exit from Pressured requires
//!   `EMA-V > yield_threshold + hysteresis_band_mb` (256MB deadband by default).
//! - **Separate emergency budget**: emergency path still self-throttles.
//! - **Hard dwell cap**: any state held > `state_max_dwell_secs` forces a
//!   resync + warning log.
//! - **Pre-computed target**: `TargetParams` calculated on entering Pressured
//!   so the Swapping phase only does I/O.
//! - **Asymmetric delta**: scale-up requires `min_ngl_delta_up`; scale-down
//!   requires `min_ngl_delta` (smaller → more responsive under pressure).

use crate::config::OrchestratorConfig;
use crate::infra::event_bus::{EventBus, KriaEvent};
use std::sync::Arc;
use std::time::{Duration, Instant};

use super::server_manager::LlamaServerManager;
use super::strategy::{self, TargetParams};
use super::telemetry::GpuTelemetry;
use super::threshold::ThresholdProfile;
use super::GpuBackend;

use crate::resource::authority::budget::{BandPolicy, Budget};
use crate::resource::authority::{
    activity::ActivityState,
    benefit::{evaluate as evaluate_benefit, BenefitInputs, BenefitThresholds},
    policy::{self, Action, Confidence, HealthPosture, LockPosture, PolicyInputs},
    simulator::{simulate, SimAction, SimDeviceState},
    ConsumerId, PolicyLog,
};

/// Whether the watchdog may *opportunistically scale the LLM UP* (restart llama-server with more GPU
/// layers) when free VRAM rises. DEFAULT OFF — this is the behavior that caused the between-session
/// "Optimizing GPU layers" restarts: free VRAM fluctuates while idle, the watchdog tries to upgrade,
/// and on a tight GPU the restart fails/thrashes. With it off, the LLM keeps the size it was given at
/// startup (sized to fit by `calculate_target_params_prod`) and never spontaneously restarts.
/// Safety scale-DOWN under pressure and image Tier-B swaps are unaffected. Set `KRIA_GPU_AUTOSCALE=1`
/// to opt back in to background upgrades.
fn opportunistic_scaleup_enabled() -> bool {
    super::gpu_policy::autoscale_enabled()
}

// ── EMA helper ───────────────────────────────────────────────────────

/// Three-sample exponential moving average over free VRAM.
/// Smooths single-poll transient dips/spikes so they don't trigger swaps.
struct VramEma {
    value: Option<f64>,
    alpha: f64, // smoothing factor (0 < α ≤ 1; higher = less smoothing)
}

impl VramEma {
    fn new(alpha: f64) -> Self {
        Self { value: None, alpha }
    }

    fn update(&mut self, sample: u64) -> u64 {
        let s = sample as f64;
        let ema = match self.value {
            None => s,
            Some(prev) => self.alpha * s + (1.0 - self.alpha) * prev,
        };
        self.value = Some(ema);
        ema as u64
    }
}

// ── State machine ────────────────────────────────────────────────────

/// Watchdog state machine states.
#[derive(Debug, Clone)]
enum WatchdogState {
    /// Normal operation — polling telemetry, no pressure.
    Idle { since: Instant },

    /// VRAM pressure detected. Waits for sustained breach before swapping.
    /// `target` is pre-computed so Swapping only does I/O.
    Pressured {
        since: Instant,
        target: Box<TargetParams>,
    },

    /// Post-swap cooldown to prevent thrashing.
    Cooldown { until: Instant },

    /// VRAM is recovering (above recover_threshold). Waits for stability
    /// before triggering a scale-up swap.
    Recovering {
        since: Instant,
        target: Box<TargetParams>,
    },

    /// VRAM critically low — emergency swap path.
    Critical { since: Instant },
}

impl WatchdogState {
    fn name(&self) -> &'static str {
        match self {
            Self::Idle { .. } => "idle",
            Self::Pressured { .. } => "pressured",
            Self::Cooldown { .. } => "cooldown",
            Self::Recovering { .. } => "recovering",
            Self::Critical { .. } => "critical",
        }
    }

    fn entered_at(&self) -> Instant {
        match self {
            Self::Idle { since }
            | Self::Pressured { since, .. }
            | Self::Recovering { since, .. }
            | Self::Critical { since } => *since,
            Self::Cooldown { until } => *until - Duration::from_secs(0), // approximate
        }
    }
}

// ── Sliding rate-limit window ────────────────────────────────────────

struct RateBucket {
    timestamps: Vec<Instant>,
    limit: u32,
}

impl RateBucket {
    fn new(limit: u32) -> Self {
        Self {
            timestamps: Vec::new(),
            limit,
        }
    }

    fn prune(&mut self) {
        let hour_ago = Instant::now() - Duration::from_secs(3600);
        self.timestamps.retain(|t| *t > hour_ago);
    }

    fn has_budget(&mut self) -> bool {
        self.prune();
        (self.timestamps.len() as u32) < self.limit
    }

    fn record(&mut self) {
        self.timestamps.push(Instant::now());
    }
}

// ── GpuWatchdog ──────────────────────────────────────────────────────

pub struct GpuWatchdog {
    config: OrchestratorConfig,
    backend: GpuBackend,
    telemetry: Arc<dyn GpuTelemetry>,
    server: Arc<LlamaServerManager>,
    event_bus: Arc<EventBus>,
    /// Total VRAM detected at boot, used for dynamic threshold scaling.
    total_vram_mb: u64,
}

impl GpuWatchdog {
    pub fn new(
        config: OrchestratorConfig,
        backend: GpuBackend,
        telemetry: Arc<dyn GpuTelemetry>,
        server: Arc<LlamaServerManager>,
        event_bus: Arc<EventBus>,
        total_vram_mb: u64,
    ) -> Self {
        Self {
            config,
            backend,
            telemetry,
            server,
            event_bus,
            total_vram_mb,
        }
    }

    /// Estimate the VRAM footprint (MB) of a given (ngl, context) configuration for this model.
    /// Mirrors the sizing math in `strategy.rs` so the simulator/policy see a consistent figure.
    fn footprint_mb(&self, ngl: u32, ctx: u32) -> u64 {
        let p = &self.config.model_profile;
        let mut mb = p.base_vram_overhead_mb as u64;
        mb += ngl as u64 * p.per_layer_vram_mb as u64;
        if p.kv_per_1k_ctx_mb > 0 {
            mb += (ctx as u64 * p.kv_per_1k_ctx_mb as u64) / 1024;
        }
        if p.has_vision_projector {
            mb += p.mmproj_vram_mb as u64;
        }
        mb
    }

    /// G2 live cutover: decide whether an opportunistic GPU scale-up should happen by routing the
    /// decision through the pure Policy Engine (`resource::authority::policy`) instead of the old
    /// hand-rolled gate. The watchdog is now an EXECUTOR — it gathers the live signals it can
    /// observe, asks the policy for a verdict, and only proceeds to `Recovering` (the restart) when
    /// the policy returns `Action::Optimize`. The governing law holds: while a foreground turn is in
    /// flight (`has_active_streams`) the activity is `Active`, the runtime mode is `Interactive`, and
    /// the policy can only return `Stay`. The promotion window is the idle path.
    ///
    /// Returns the policy decision so the caller can both gate the transition and log it (G11).
    fn decide_scaleup(
        &self,
        free_mb: u64,
        total_mb: u64,
        current_ngl: u32,
        current_ctx: u32,
        target: &TargetParams,
    ) -> policy::Decision {
        // ── Activity (G6) ── a live foreground turn forbids any restart.
        let activity = if self.server.has_active_streams() {
            ActivityState::Active
        } else {
            // The scale-up branch is only reached when idle with recovered headroom; treat the
            // quiescent state as the DeepIdle promotion window (the only time a promotion is legal).
            ActivityState::DeepIdle
        };

        // ── Confidence (C2/C5) ── zero total = Unknown telemetry, never optimize on it.
        let confidence = if total_mb > 0 {
            Confidence::Measured
        } else {
            Confidence::Unknown
        };

        // ── Lock posture (G3) ── on CPU (ngl 0) we want a one-time promotion; otherwise the model
        // is GPU-resident pre-lock. (The watchdog doesn't hold the ResidentLock; a locked model
        // would not be scaled by this path once full lock wiring lands.)
        let lock = if current_ngl == 0 {
            LockPosture::CpuOrCloudWantingPromotion
        } else {
            LockPosture::PreLockResident
        };

        // ── Simulator (G4) ── estimate the restart from the current footprint to the target.
        let from_mb = self.footprint_mb(current_ngl, current_ctx);
        let to_mb = self.footprint_mb(target.ngl, target.context);
        let dev = SimDeviceState {
            free_vram_mb: free_mb,
            total_vram_mb: total_mb,
            free_ram_mb: 0, // RAM not relevant to a GPU upsize feasibility check
            budget: Budget::derive(
                total_mb,
                self.config.safety_margin_mb,
                BandPolicy::default(),
            ),
        };
        let sim = simulate(
            &SimAction::Swap {
                from_vram_mb: from_mb,
                to_vram_mb: to_mb,
            },
            &dev,
        );

        // ── Benefit (G5) ── coarse throughput model: tok/s grows monotonically with GPU layers.
        // A tiny ngl bump → speedup ≈ 1.0 → Not-Worth-It (avoids churn for negligible gain); a
        // CPU→GPU promotion → large speedup → Worth-It while idle.
        let tps = |ngl: u32| 1.0_f32 + ngl as f32;
        let failure_prob = match sim.risk {
            crate::resource::authority::simulator::RiskLevel::High => 0.9,
            crate::resource::authority::simulator::RiskLevel::Med => 0.2,
            crate::resource::authority::simulator::RiskLevel::Low => 0.03,
        };
        let benefit = evaluate_benefit(
            &BenefitInputs {
                target_tok_per_s: tps(target.ngl),
                current_tok_per_s: tps(current_ngl),
                restart_cost_s: 5.0,
                failure_prob,
                activity,
            },
            &BenefitThresholds::default(),
        );

        // ── Forecast (G4) ── we are in the recovery-headroom branch (free already above the recover
        // threshold + hysteresis); treat free VRAM as sustainably sufficient, but only when
        // telemetry is trustworthy.
        let forecast_sustainable = confidence == Confidence::Measured;

        let inputs = PolicyInputs {
            activity,
            confidence,
            health: HealthPosture::Healthy,
            lock,
            cooldown_elapsed: !self.server.gpu_in_cooldown(),
            forecast_sustainable,
            sim: Some(sim),
            benefit: Some(benefit),
        };
        let mode = policy::derive_mode(activity, HealthPosture::Healthy, false);
        policy::decide(&inputs, mode)
    }

    /// Main watchdog loop. Runs until the task is aborted.
    pub async fn run(&self) {
        let poll_interval = Duration::from_secs(self.config.poll_interval_secs.max(1));

        // Compute dynamic thresholds from total VRAM. On Metal, macOS-specific
        // RAM thresholds override (unified memory behaves differently).
        let (yield_threshold, emergency_threshold, recover_threshold, hysteresis) =
            if self.backend == GpuBackend::Metal {
                (
                    self.config.macos_yield_ram_mb,
                    self.config.macos_emergency_ram_mb,
                    self.config.macos_recover_ram_mb,
                    self.config.hysteresis_band_mb,
                )
            } else {
                // Phase 1: dynamic percentage-based scaling from total VRAM,
                // with config values as overrides when non-zero.
                let profile = ThresholdProfile::from_total_vram(self.total_vram_mb)
                    .with_config_overrides(&self.config);
                (
                    profile.yield_mb,
                    profile.emergency_mb,
                    profile.recover_mb,
                    profile.hysteresis_mb,
                )
            };

        let pressure_dwell = Duration::from_secs(self.config.pressure_dwell_secs);
        let emergency_dwell = Duration::from_millis(self.config.emergency_dwell_ms);
        let recovery_dwell = Duration::from_secs(self.config.recovery_dwell_secs);
        let state_max_dwell = Duration::from_secs(self.config.state_max_dwell_secs);
        let cooldown_dur = Duration::from_secs(self.config.cooldown_secs);

        // EMA with α = 0.5 → roughly 3-sample smoothing.
        let mut ema = VramEma::new(0.5);

        let mut state = WatchdogState::Idle {
            since: Instant::now(),
        };

        // Separate rate buckets: normal and emergency.
        let mut normal_budget = RateBucket::new(self.config.max_transitions_per_hour);
        let mut emergency_budget =
            RateBucket::new(self.config.max_emergency_transitions_per_hour.max(1));

        tracing::info!(
            backend = ?self.backend,
            telemetry = self.telemetry.source_name(),
            total_vram_mb = self.total_vram_mb,
            yield_mb = yield_threshold,
            emergency_mb = emergency_threshold,
            recover_mb = recover_threshold,
            hysteresis_mb = hysteresis,
            pressure_dwell_secs = self.config.pressure_dwell_secs,
            emergency_dwell_ms = self.config.emergency_dwell_ms,
            "watchdog: starting (dynamic thresholds)"
        );

        loop {
            tokio::time::sleep(poll_interval).await;

            let raw = self.telemetry.snapshot().await.free_vram_mb;
            let free = ema.update(raw);

            let state_name = state.name();
            let state_age = state.entered_at().elapsed();

            // Hard dwell cap: if we've been in any state too long, log and reset.
            if state_age > state_max_dwell {
                tracing::warn!(
                    state = state_name,
                    age_secs = state_age.as_secs(),
                    max_secs = state_max_dwell.as_secs(),
                    "watchdog: state dwell cap exceeded — resetting to Idle"
                );
                state = WatchdogState::Idle {
                    since: Instant::now(),
                };
            }

            // Critical check overlay: any non-Cooldown state can transition
            // to Critical when EMA drops below emergency threshold for
            // ≥ emergency_dwell_ms. This prevents false triggers from driver
            // spikes.
            if free < emergency_threshold && !matches!(state, WatchdogState::Cooldown { .. }) {
                state = match state {
                    WatchdogState::Critical { since } => {
                        if since.elapsed() >= emergency_dwell {
                            if emergency_budget.has_budget() {
                                tracing::warn!(
                                    free_mb = free,
                                    elapsed_ms = since.elapsed().as_millis(),
                                    "watchdog: EMERGENCY — triggering swap"
                                );
                                self.event_bus
                                    .publish(KriaEvent::VramPressure { free_vram_mb: free });
                                self.execute_swap(free, true, &mut emergency_budget).await;
                                WatchdogState::Cooldown {
                                    until: Instant::now() + cooldown_dur,
                                }
                            } else {
                                tracing::warn!(
                                    "watchdog: emergency rate limit reached — staying critical"
                                );
                                WatchdogState::Critical { since }
                            }
                        } else {
                            WatchdogState::Critical { since }
                        }
                    }
                    _ => {
                        tracing::warn!(
                            free_mb = free,
                            threshold = emergency_threshold,
                            prev_state = state_name,
                            "watchdog: entering Critical"
                        );
                        WatchdogState::Critical {
                            since: Instant::now(),
                        }
                    }
                };
                continue;
            }

            // Main state transitions.
            state = match state {
                WatchdogState::Idle { since } => {
                    if free < yield_threshold {
                        // Pre-compute target here so Swapping only does I/O.
                        let target = strategy::calculate_target_params_prod(
                            &self.config.model_profile,
                            free,
                            self.config.safety_margin_mb,
                            self.backend,
                        );
                        tracing::info!(
                            free_mb = free,
                            threshold = yield_threshold,
                            new_ngl = target.ngl,
                            "watchdog: VRAM pressure — entering Pressured"
                        );
                        WatchdogState::Pressured {
                            since: Instant::now(),
                            target: Box::new(target),
                        }
                    } else if free > recover_threshold + hysteresis {
                        // Recovery path: check if a scale-up makes sense.
                        //
                        // G2 (executor demotion): the watchdog no longer OWNS the optimize
                        // decision. The authoritative decision module is
                        // `resource::authority::policy` — a pure Policy Engine that emits
                        // `Action::Optimize` only when ALL of {DeepIdle, Measured confidence,
                        // pre-lock/cloud posture, cooldown elapsed, sustainable forecast, simulator
                        // fit, benefit Worth-It} hold (redesign G4). The gate below mirrors the
                        // subset of those conditions observable from the watchdog loop (delta,
                        // budget, cooldown) and is hard-gated OFF by default via
                        // `opportunistic_scaleup_enabled()`. Full PolicyInputs plumbing (activity +
                        // benefit + forecast) into this loop is the hardware-soak-gated step
                        // (Task 74); until then the watchdog stays a safe executor: it never
                        // restarts for performance unless a power user opts in with KRIA_GPU_AUTOSCALE=1.
                        let (current_ngl, current_ctx) = self.server.current_params();
                        let target = strategy::calculate_target_params_prod(
                            &self.config.model_profile,
                            free,
                            self.config.safety_margin_mb,
                            self.backend,
                        );
                        let delta = target.ngl.saturating_sub(current_ngl);
                        // G2 cutover: master switch still gates the whole opportunistic path (default
                        // OFF). When enabled, the DECISION is delegated to the Policy Engine instead
                        // of the old hand-rolled gate — the watchdog only executes `Action::Optimize`.
                        let decision = self.decide_scaleup(
                            free,
                            self.total_vram_mb,
                            current_ngl,
                            current_ctx,
                            &target,
                        );
                        PolicyLog::from_decision(
                            "watchdog-scaleup",
                            ConsumerId::Llm,
                            policy::derive_mode(
                                if self.server.has_active_streams() {
                                    ActivityState::Active
                                } else {
                                    ActivityState::DeepIdle
                                },
                                HealthPosture::Healthy,
                                false,
                            ),
                            &decision,
                            0,
                        )
                        .emit();
                        if opportunistic_scaleup_enabled()
                            && decision.action == Action::Optimize
                            && normal_budget.has_budget()
                        {
                            tracing::info!(
                                free_mb = free,
                                delta_ngl = delta,
                                expected_speedup = decision.expected_speedup,
                                "watchdog: policy approved promotion — entering Recovering"
                            );
                            WatchdogState::Recovering {
                                since: Instant::now(),
                                target: Box::new(target),
                            }
                        } else {
                            if delta >= self.config.min_ngl_delta_up
                                && !opportunistic_scaleup_enabled()
                            {
                                tracing::debug!(
                                    "watchdog: opportunistic GPU scale-up disabled (default) — LLM stays at its current size; set KRIA_GPU_AUTOSCALE=1 to allow background upgrades"
                                );
                            } else if delta >= self.config.min_ngl_delta_up
                                && self.server.gpu_in_cooldown()
                            {
                                tracing::debug!(
                                    "watchdog: GPU scale-up suppressed — cooldown active after a recent spawn failure (C4)"
                                );
                            } else if delta >= self.config.min_ngl_delta_up
                                && decision.action != Action::Optimize
                            {
                                tracing::debug!(
                                    reason = ?decision.reason,
                                    "watchdog: policy held GPU scale-up (not eligible/worthwhile)"
                                );
                            }
                            WatchdogState::Idle { since }
                        }
                    } else {
                        WatchdogState::Idle { since }
                    }
                }

                WatchdogState::Pressured { since, target } => {
                    // Exit: pressure relieved (deadband).
                    if free > yield_threshold + hysteresis {
                        tracing::info!(
                            free_mb = free,
                            "watchdog: pressure relieved — returning to Idle"
                        );
                        WatchdogState::Idle {
                            since: Instant::now(),
                        }
                    } else if since.elapsed() >= pressure_dwell {
                        // Sustained pressure — check rate limit, then swap.
                        if normal_budget.has_budget() {
                            let (current_ngl, _) = self.server.current_params();
                            // Use the pre-computed target only as a gate to decide
                            // whether a swap is worth doing. The actual swap params
                            // are recomputed from fresh telemetry below.
                            let delta =
                                (current_ngl as i64 - target.ngl as i64).unsigned_abs() as u32;

                            if delta < self.config.min_ngl_delta {
                                tracing::debug!(
                                    delta,
                                    min = self.config.min_ngl_delta,
                                    "watchdog: delta too small, skipping swap"
                                );
                                WatchdogState::Cooldown {
                                    until: Instant::now() + cooldown_dur,
                                }
                            } else {
                                // Recompute target from current VRAM, not the stale
                                // snapshot captured when we entered Pressured. During
                                // the pressure dwell (default 5s), VRAM can shift by
                                // hundreds of MB due to other processes or KV growth.
                                let fresh_target = strategy::calculate_target_params_prod(
                                    &self.config.model_profile,
                                    free,
                                    self.config.safety_margin_mb,
                                    self.backend,
                                );
                                tracing::warn!(
                                    free_mb = free,
                                    gate_ngl = target.ngl,
                                    fresh_ngl = fresh_target.ngl,
                                    "watchdog: sustained pressure — swapping (target recomputed)"
                                );
                                self.event_bus
                                    .publish(KriaEvent::VramPressure { free_vram_mb: free });
                                self.execute_swap_with_target(
                                    &fresh_target,
                                    false,
                                    &mut normal_budget,
                                )
                                .await;
                                WatchdogState::Cooldown {
                                    until: Instant::now() + cooldown_dur,
                                }
                            }
                        } else {
                            tracing::warn!(
                                "watchdog: normal rate limit reached — entering Cooldown"
                            );
                            WatchdogState::Cooldown {
                                until: Instant::now() + cooldown_dur,
                            }
                        }
                    } else {
                        WatchdogState::Pressured { since, target }
                    }
                }

                WatchdogState::Cooldown { until } => {
                    if Instant::now() >= until {
                        tracing::info!("watchdog: cooldown expired");
                        WatchdogState::Idle {
                            since: Instant::now(),
                        }
                    } else {
                        WatchdogState::Cooldown { until }
                    }
                }

                WatchdogState::Recovering { since, target } => {
                    if since.elapsed() >= recovery_dwell {
                        tracing::info!(
                            new_ngl = target.ngl,
                            "watchdog: recovery stable — scaling up"
                        );
                        self.execute_swap_with_target(&target, false, &mut normal_budget)
                            .await;
                        WatchdogState::Cooldown {
                            until: Instant::now() + cooldown_dur,
                        }
                    } else if free < recover_threshold {
                        // Recovery window closed before dwell expired.
                        tracing::info!("watchdog: recovery window closed — returning to Idle");
                        WatchdogState::Idle {
                            since: Instant::now(),
                        }
                    } else {
                        WatchdogState::Recovering { since, target }
                    }
                }

                WatchdogState::Critical { since } => {
                    // If we get here, free >= emergency_threshold (the Critical
                    // overlay above didn't fire). Transition back to Idle.
                    tracing::info!(
                        elapsed_ms = since.elapsed().as_millis(),
                        "watchdog: critical pressure resolved — returning to Idle"
                    );
                    WatchdogState::Idle {
                        since: Instant::now(),
                    }
                }
            };
        }
    }

    /// Execute a swap using a freshly calculated target (emergency path).
    async fn execute_swap(&self, free_vram_mb: u64, emergency: bool, budget: &mut RateBucket) {
        let target = strategy::calculate_target_params_prod(
            &self.config.model_profile,
            free_vram_mb,
            self.config.safety_margin_mb,
            self.backend,
        );
        self.execute_swap_with_target(&target, emergency, budget)
            .await;
    }

    /// Execute a swap using a pre-computed target.
    async fn execute_swap_with_target(
        &self,
        target: &TargetParams,
        emergency: bool,
        budget: &mut RateBucket,
    ) {
        // HRA Task 12 / A4: never interrupt an active foreground turn for a NON-emergency swap.
        // Route the decision through the Foreground Guard. If a chat/stream is in flight and this
        // isn't an emergency, defer — the swap will be retried after the turn ends. Emergency swaps
        // (true OOM risk) still proceed (with checkpoint via the existing slot save below).
        {
            use crate::resource::authority::{
                ActionImpact, ForegroundGuard, GuardContext, GuardDecision,
            };
            let fg_active = self.server.has_active_streams();
            let decision = ForegroundGuard::authorize(
                ActionImpact::Disruptive,
                GuardContext {
                    foreground_active: fg_active,
                    at_turn_boundary: !fg_active,
                    emergency,
                },
            );
            // Single authoritative gate: a swap proceeds ONLY on `Allow`. Both
            // `DeferToTurnBoundary` and `AllowEmergencyCheckpoint` mean "do not tear
            // down the live foreground stream now" (see `swap_should_proceed`).
            if !swap_should_proceed(decision) {
                if matches!(decision, GuardDecision::AllowEmergencyCheckpoint) {
                    // ROOT-CAUSE FIX ("LLM server not reachable" during generation):
                    //
                    // An emergency VRAM action fired DURING a live foreground stream. The
                    // guard's `AllowEmergencyCheckpoint` contract means "interrupt only via
                    // checkpoint + resume" — but llama.cpp cannot resume an in-flight
                    // completion across a model reload, and its KV cache is pre-allocated at
                    // load time (an in-flight turn does not itself grow VRAM). The old code
                    // ignored this decision and fell through to `cancel_streams()` + `kill()`,
                    // which DESTROYED the user's answer mid-token and made `/health`
                    // transport-fail → "LLM server not reachable", without relieving the real
                    // pressure (which is almost always transient/other-process noise).
                    //
                    // Honor Property-4 ("no surprise interruption"): defer to the turn
                    // boundary. The wait is bounded by KRIA's turn-level timeout, and the swap
                    // re-fires the instant the stream ends (has_active_streams()==false → the
                    // guard then returns `Allow`). A genuine OOM surfaces as a normal
                    // generation error, never a silent server teardown.
                    tracing::warn!(
                        new_ngl = target.ngl,
                        new_ctx = target.context,
                        "watchdog: deferring EMERGENCY swap — foreground stream active; \
                         will swap at the turn boundary (no mid-stream teardown)"
                    );
                } else {
                    tracing::info!(
                        new_ngl = target.ngl,
                        new_ctx = target.context,
                        emergency,
                        "watchdog: deferring non-emergency swap — foreground turn active (A4)"
                    );
                }
                return;
            }
        }

        // HRA command hook: consult the Resource Authority before a GPU scale-up. In SHADOW it only
        // logs its verdict (divergence visibility); under enforce it can VETO an unsafe GPU swap.
        if target.ngl > 0 && !emergency {
            if let Some(hra) = crate::resource::authority::global_hra() {
                let mp = &self.config.model_profile;
                let needed_vram_mb = target.ngl as u64 * mp.per_layer_vram_mb as u64
                    + mp.base_vram_overhead_mb as u64
                    + if mp.has_vision_projector {
                        mp.mmproj_vram_mb as u64
                    } else {
                        0
                    };
                let advice = hra.advise_gpu_admission_fresh(needed_vram_mb).await;
                tracing::info!(
                    target: "hra",
                    needed_vram_mb,
                    target_ngl = target.ngl,
                    target_ctx = target.context,
                    allow_gpu = advice.allow_gpu,
                    shadow = advice.shadow,
                    reason = %advice.reason,
                    "HRA GPU-admission verdict for LLM swap"
                );
                if !advice.shadow && !advice.allow_gpu {
                    tracing::warn!(
                        target: "hra",
                        needed_vram_mb,
                        "HRA VETO: skipping GPU scale-up — LLM stays on current (CPU) residency"
                    );
                    return;
                }
            }
        }

        let (old_ngl, old_ctx) = self.server.current_params();
        let old_vision_enabled = self.server.current_vision_enabled();
        let target_vision_enabled = target.vision_mode.load_mmproj();

        tracing::info!(
            old_ngl,
            old_ctx,
            new_ngl = target.ngl,
            new_ctx = target.context,
            emergency,
            degradation = %target.degradation,
            "watchdog: executing swap"
        );

        self.event_bus.publish(KriaEvent::LlmSwapStarted {
            from_ngl: old_ngl,
            to_ngl: target.ngl,
            emergency,
        });

        let swap_start = Instant::now();

        // 1. Cancel in-flight streams.
        self.server.cancel_streams();
        let _ = self.server.save_active_slot().await;
        self.event_bus.publish(KriaEvent::LlmStreamInterrupted);

        // 2. API-first zero-downtime path.
        //
        // We use Router Mode unload/load as the primary mechanism, and if the
        // runtime appears to support dynamic props we attempt to stage target
        // ngl/ctx between unload and load. Any error falls back to the legacy
        // process restart ladder below.
        let api_swap_result: Result<(), String> = async {
            self.server
                .api_unload_model()
                .await
                .map_err(|e| format!("api_unload_model failed: {e}"))?;

            let needs_dynamic_update = old_ngl != target.ngl
                || old_ctx != target.context
                || old_vision_enabled != target_vision_enabled;

            if needs_dynamic_update {
                let base_url = self.server.api_url();
                if base_url.is_empty() {
                    return Err("dynamic props update failed: empty API URL".to_string());
                }
                let root_url = base_url
                    .trim_end_matches('/')
                    .trim_end_matches("/v1")
                    .to_string();
                let props_url = format!("{root_url}/props");
                let request_timeout_secs = self.config.health_check_timeout_secs.clamp(1, 30);
                let client = reqwest::Client::builder()
                    .timeout(Duration::from_secs(request_timeout_secs))
                    .build()
                    .unwrap_or_default();

                let props_payload = serde_json::json!({
                    "n_gpu_layers": target.ngl,
                    "n_ctx": target.context,
                    "ctx_size": target.context,
                    "vision_mode": target.vision_mode.as_str(),
                    "enable_vision": target_vision_enabled,
                });

                let resp = client
                    .post(&props_url)
                    .json(&props_payload)
                    .send()
                    .await
                    .map_err(|e| format!("dynamic props update transport error: {e}"))?;

                if !resp.status().is_success() {
                    let status = resp.status();
                    let body = resp.text().await.unwrap_or_default();
                    return Err(format!(
                        "dynamic props update failed (HTTP {status}): {body}"
                    ));
                }

                tracing::info!(
                    url = %props_url,
                    new_ngl = target.ngl,
                    new_ctx = target.context,
                    vision_mode = %target.vision_mode,
                    "watchdog: dynamic runtime props accepted before API reload"
                );
            }

            self.server
                .api_load_model()
                .await
                .map_err(|e| format!("api_load_model failed: {e}"))?;

            Ok(())
        }
        .await;

        if api_swap_result.is_ok() {
            let duration = swap_start.elapsed();
            budget.record();

            self.event_bus.publish(KriaEvent::LlmSwapCompleted {
                new_ngl: target.ngl,
                new_context: target.context,
                duration_ms: duration.as_millis() as u64,
            });

            self.event_bus.publish(KriaEvent::LlmDegradationChanged {
                level: target.degradation.as_str().to_string(),
            });

            tracing::info!(
                new_ngl = target.ngl,
                new_ctx = target.context,
                duration_ms = duration.as_millis(),
                "watchdog: swap completed via API zero-downtime path"
            );
            let _ = self.server.restore_active_slot().await;
            return;
        }

        let api_err = api_swap_result
            .err()
            .unwrap_or_else(|| "unknown API swap error".to_string());

        let router_unsupported = api_err.contains("HTTP 501") || api_err.contains("HTTP 404");

        if router_unsupported {
            // Router Mode simply not available on this llama-server build — expected,
            // not an error. Log at debug so startup is noise-free.
            tracing::debug!(
                error = %api_err,
                old_ngl,
                old_ctx,
                new_ngl = target.ngl,
                new_ctx = target.context,
                "watchdog: Router Mode unavailable; using legacy process restart (upgrade to b5291+ for zero-downtime swaps)"
            );
        } else {
            tracing::warn!(
                error = %api_err,
                old_ngl,
                old_ctx,
                new_ngl = target.ngl,
                new_ctx = target.context,
                emergency,
                "watchdog: API swap path failed; falling back to legacy process restart"
            );
        }

        // 3. Legacy fallback: prefer graceful ladder when Router Mode is absent.
        if router_unsupported {
            self.server
                .graceful_stop_with_timeout(Duration::from_secs(
                    self.config.graceful_stop_timeout_secs.max(1),
                ))
                .await;
        } else if emergency {
            self.server.kill().await;
        } else {
            self.server.graceful_stop().await;
        }

        // 4. On CUDA: wait for VRAM to actually free before spawning. Use the
        //    watch-channel snapshot (not a fresh blocking NVML call) to poll.
        if self.backend == GpuBackend::Cuda {
            self.wait_for_vram_release().await;
        }

        // 5. Spawn new server.
        match self
            .server
            .spawn(
                target.ngl,
                target.context,
                target.vision_mode,
                self.event_bus.clone(),
            )
            .await
        {
            Ok(()) => {
                let duration = swap_start.elapsed();
                budget.record();

                self.event_bus.publish(KriaEvent::LlmSwapCompleted {
                    new_ngl: target.ngl,
                    new_context: target.context,
                    duration_ms: duration.as_millis() as u64,
                });

                self.event_bus.publish(KriaEvent::LlmDegradationChanged {
                    level: target.degradation.as_str().to_string(),
                });

                tracing::info!(
                    new_ngl = target.ngl,
                    duration_ms = duration.as_millis(),
                    "watchdog: swap completed"
                );
                let _ = self.server.restore_active_slot().await;
            }
            Err(e) => {
                tracing::error!(
                    ?e,
                    "watchdog: swap spawn failed — attempting CPU recovery to keep LLM available"
                );
                self.event_bus.publish(KriaEvent::LlmSwapFailed {
                    reason: e.to_string(),
                });

                // RECOVERY (critical): the previous server was already stopped in step 3, so a
                // failed GPU spawn would otherwise leave NO llama-server running → "LLM
                // unavailable". Respawn a guaranteed-fit CPU config (ngl=0 always fits) so the LLM
                // stays up after a failed GPU scale-up. Keep CPU-side vision if this is a vision
                // model. The recorded failure ceiling (set by `spawn`) makes the next scale-up back
                // off below the size that just failed.
                let recovery_vision = if target_vision_enabled {
                    super::vision_strategy::VisionMode::CpuVision
                } else {
                    super::vision_strategy::VisionMode::Disabled
                };
                let recovery_ctx = old_ctx.max(2048);
                match self
                    .server
                    .spawn(0, recovery_ctx, recovery_vision, self.event_bus.clone())
                    .await
                {
                    Ok(()) => {
                        tracing::warn!(
                            recovery_ctx,
                            "watchdog: CPU recovery spawn succeeded — LLM available on CPU after failed GPU swap"
                        );
                        self.event_bus.publish(KriaEvent::LlmDegradationChanged {
                            level: "cpu_only".to_string(),
                        });
                        let _ = self.server.restore_active_slot().await;
                    }
                    Err(re) => {
                        tracing::error!(
                            ?re,
                            "watchdog: CPU recovery spawn ALSO failed — LLM is down"
                        );
                    }
                }
            }
        }
    }

    /// Poll until free VRAM rises above `yield_threshold` or timeout elapses.
    /// Uses the watch-channel telemetry snapshot — never blocks the executor.
    async fn wait_for_vram_release(&self) {
        let timeout = Duration::from_secs(self.config.vram_release_timeout_secs.max(1));
        let deadline = Instant::now() + timeout;

        loop {
            if Instant::now() >= deadline {
                tracing::warn!("watchdog: VRAM release wait timed out");
                break;
            }
            let snap = self.telemetry.snapshot().await;
            if snap.free_vram_mb > self.config.yield_threshold_mb {
                tracing::debug!(
                    free_mb = snap.free_vram_mb,
                    "watchdog: VRAM release confirmed"
                );
                break;
            }
            tokio::time::sleep(Duration::from_millis(200)).await;
        }
    }
}

/// Whether the swap executor may PROCEED given a Foreground Guard decision.
///
/// This is the single authoritative mapping from the guard's 3-way decision to the
/// executor's 2-way action, and the site of the "LLM server not reachable during
/// generation" root-cause fix: a swap proceeds ONLY on `Allow`. Both
/// `DeferToTurnBoundary` and `AllowEmergencyCheckpoint` must NOT tear down a live
/// foreground stream — the previous code proceeded on `AllowEmergencyCheckpoint`,
/// hard-cancelling + killing llama-server mid-generation.
fn swap_should_proceed(decision: crate::resource::authority::GuardDecision) -> bool {
    matches!(decision, crate::resource::authority::GuardDecision::Allow)
}

#[cfg(test)]
mod swap_gate_tests {
    use super::swap_should_proceed;
    use crate::resource::authority::{ActionImpact, ForegroundGuard, GuardContext, GuardDecision};

    fn guard(fg_active: bool, emergency: bool) -> GuardDecision {
        ForegroundGuard::authorize(
            ActionImpact::Disruptive,
            GuardContext {
                foreground_active: fg_active,
                at_turn_boundary: !fg_active,
                emergency,
            },
        )
    }

    #[test]
    fn non_emergency_swap_during_active_stream_is_deferred() {
        assert!(!swap_should_proceed(guard(true, false)));
    }

    #[test]
    fn emergency_swap_during_active_stream_is_deferred_not_a_teardown() {
        // REGRESSION: the bug was proceeding here (cancel_streams + kill mid-token),
        // which produced "LLM server not reachable". The emergency must defer.
        let decision = guard(true, true);
        assert_eq!(decision, GuardDecision::AllowEmergencyCheckpoint);
        assert!(
            !swap_should_proceed(decision),
            "emergency during a live foreground stream must NOT tear down the server"
        );
    }

    #[test]
    fn swap_proceeds_at_turn_boundary() {
        // No active stream → both normal and emergency swaps may proceed.
        assert!(swap_should_proceed(guard(false, false)));
        assert!(swap_should_proceed(guard(false, true)));
    }
}
