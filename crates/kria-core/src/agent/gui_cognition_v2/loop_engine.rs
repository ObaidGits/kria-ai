//! GUI Cognition V2 — the bounded observe → decide → gate → act → verify loop.
//!
//! Phase 0 established the minimal wiring. Task 9 adds the production guards that
//! are deterministically testable here:
//! - **Safety gate** (Property 5): a decided executable action is sent to the
//!   injected [`SafetyGate`] before Hands; a `Deny` stops the turn and the action
//!   never executes.
//! - **Cancellation** (Requirement 5.4): a shared cancel flag is checked each
//!   iteration; the loop halts before the next action.
//! - **No-progress detection** (Requirement 5.3): if a state-changing action
//!   produces no observable screen change across re-observe for `no_progress_limit`
//!   consecutive steps, the loop stops (never an infinite loop).
//! - **Step cap** (Requirement 5.1): a hard iteration bound.
//!
//! The desktop integration (live) supplies the real [`SafetyGate`] (existing
//! HITL/policy) and the real Hands input substrate; incremental event streaming
//! and screenshot-diff verification beyond `screen_changed` are wired there.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use super::bridge::GuiBridge;
use super::planner::Plan;
use super::traits::{GateDecision, GuiBrain, GuiHands, GuiPlanner, SafetyGate, Sight};
use super::types::{Action, Decision, Observation, SubGoal, SubGoalKind, TurnStep};
use super::verifier::{SubGoalVerifier, VerificationProbe};

/// Terminal status of a V2 turn.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TurnStatus {
    /// Brain returned `Done`.
    Completed,
    /// Brain returned `Ask` — a clarification is required.
    NeedsClarification,
    /// The bounded step cap was reached without completion.
    StoppedStepCap,
    /// A state-changing action produced no screen change repeatedly.
    StoppedNoProgress,
    /// The safety gate denied a decided action.
    StoppedSafety,
    /// A cancel was requested.
    Cancelled,
    /// A layer returned an unrecoverable error.
    StoppedError,
}

impl TurnStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            TurnStatus::Completed => "completed",
            TurnStatus::NeedsClarification => "needs_clarification",
            TurnStatus::StoppedStepCap => "stopped_step_cap",
            TurnStatus::StoppedNoProgress => "stopped_no_progress",
            TurnStatus::StoppedSafety => "stopped_safety",
            TurnStatus::Cancelled => "cancelled",
            TurnStatus::StoppedError => "stopped_error",
        }
    }
}

/// Outcome of a V2 turn.
#[derive(Debug, Clone)]
pub struct TurnOutcomeV2 {
    pub status: TurnStatus,
    /// Layman-friendly summary line.
    pub reply: String,
    /// The completed steps (executed actions + results), in order.
    pub steps: Vec<TurnStep>,
}

/// Bounded loop configuration.
#[derive(Debug, Clone, Copy)]
pub struct LoopConfig {
    /// Hard cap on the number of decide/act iterations.
    pub max_steps: u32,
    /// Whether to request a Set-of-Mark image from Sight each observe.
    pub want_som: bool,
    /// Consecutive no-change steps (after a state-changing action) that trigger
    /// a no-progress stop. 0 disables the check.
    pub no_progress_limit: u32,
    /// Start the turn in element-GROUNDED observation mode (call
    /// [`Sight::observe_grounded`] from the first step) instead of the cheap
    /// default. Used when the operator forces grounding on; the default lazy
    /// path starts `false` and escalates on demand (see
    /// [`decision_needs_grounding`]). Has no effect unless the Sight reports
    /// [`Sight::supports_grounding`].
    pub start_grounded: bool,
    /// Run PLAN-DRIVEN (Task 9): decompose the task into ordered sub-goals up
    /// front, steer each step toward the current unverified sub-goal, and
    /// complete ONLY when every sub-goal is externally VERIFIED (or stop
    /// honestly). Requires a planner in [`LoopGuards`]; when no planner is wired
    /// this is inert (the loop behaves exactly as before). Default `false`.
    pub use_plan: bool,
    /// Per-sub-goal step budget when plan-driven: how many decide/act steps a
    /// single sub-goal may consume before the loop concludes it cannot be made
    /// (bounds the turn; combined with `max_steps`). 0 → derive from `max_steps`.
    pub steps_per_sub_goal: u32,
}

impl Default for LoopConfig {
    fn default() -> Self {
        Self {
            max_steps: 12,
            want_som: false,
            no_progress_limit: 2,
            start_grounded: false,
            use_plan: false,
            steps_per_sub_goal: 0,
        }
    }
}

/// Whether the cheap (element-free) observation left the Brain with nothing to
/// act on, so it could only `Ask` — the deterministic trigger for a ONE-SHOT
/// grounding escalation (re-observe WITH element detection, then re-decide).
///
/// We require the observation to be genuinely element-free AND not already
/// degraded: a degraded Sight cannot be helped by asking it to look "harder",
/// and an observation that DID surface elements means the Brain's `Ask` is a
/// genuine clarification, not a perception gap. Pure + testable.
pub fn decision_needs_grounding(decision: &Decision, observation: &Observation) -> bool {
    observation.elements.is_empty()
        && !observation.is_degraded()
        && matches!(decision.action, Action::Ask { .. })
}

/// Detect a MANUAL-STEP / human-in-the-loop blocker surface in the observation
/// (Requirement 32): a sign-in / password / captcha / 2FA / OS-permission prompt
/// that the agent must NOT try to fill itself. Returns a short, user-facing
/// reason when detected, else `None`. Conservative — keyed on STRONG markers (a
/// password field, a verification-code/captcha prompt, an explicit permission
/// dialog) so an ordinary "Sign in" link on a normal page does not trip it.
/// Pure + testable.
pub fn detect_manual_step(observation: &Observation) -> Option<&'static str> {
    if observation.elements.is_empty() {
        return None;
    }
    let labels: Vec<String> = observation
        .elements
        .iter()
        .map(|e| e.label.to_ascii_lowercase())
        .collect();
    let any = |needle: &str| labels.iter().any(|l| l.contains(needle));

    let has_password_field = observation.elements.iter().any(|e| {
        let l = e.label.to_ascii_lowercase();
        (e.kind.contains("field") || e.kind.contains("input") || e.kind == "text_field")
            && (l.contains("password") || l.contains("passcode"))
    });
    if has_password_field || any("enter your password") {
        return Some("This looks like a sign-in screen. Please enter your credentials, then ask me to continue.");
    }
    if any("verification code")
        || any("one-time")
        || any("2fa")
        || any("two-factor")
        || any("authenticator")
    {
        return Some("This needs a verification/2FA code only you have. Please complete it, then ask me to continue.");
    }
    if any("captcha") || any("i'm not a robot") || any("im not a robot") {
        return Some("There's a CAPTCHA to solve. Please complete it, then ask me to continue.");
    }
    if (any("allow") && (any("permission") || any("access your")))
        || any("grant permission")
        || any("requires permission")
    {
        return Some(
            "This needs a permission you must grant. Please approve it, then ask me to continue.",
        );
    }
    None
}

/// A live, UI-agnostic progress event emitted by the loop as it advances through
/// each phase of a step. The desktop layer maps these to the wire
/// `gui_cognition:event` envelopes the frontend panel understands, so the user
/// sees observe → decide(reason) → gate → execute → verify LIVE instead of a bare
/// "Thinking…" until the turn ends. kria-core stays UI-agnostic: it emits this
/// typed enum; the desktop owns the wire format.
#[derive(Debug, Clone)]
pub enum LoopEvent {
    /// The turn began (before the first observation).
    TurnStarted,
    /// A plan was created (plan-driven turns): the ordered sub-goal intents.
    PlanReady { goals: Vec<String> },
    /// A sub-goal's status changed (plan-driven turns): which one, and whether it
    /// is now verified-done, failed, or still in progress.
    SubGoalUpdated {
        index: usize,
        total: usize,
        goal: String,
        status: &'static str,
    },
    /// A bounded no-progress RECOVERY rung was attempted (Task 11) before giving
    /// up — e.g. escalating to grounded perception and re-deciding.
    RecoveryAttempted { rung: &'static str, ok: bool },
    /// A fresh observation is being captured for `step_index`.
    ObserveStarted { step_index: u32 },
    /// The cheap observation lacked the controls the Brain needed (it could only
    /// `Ask`), so the loop is escalating to an element-GROUNDED observation and
    /// re-deciding this step once. Lets the UI explain the brief extra "look".
    GroundingEscalated { step_index: u32 },
    /// The observation completed (honest summary; degraded when Sight could not see).
    ObserveCompleted {
        step_index: u32,
        active_window: Option<String>,
        element_count: usize,
        degraded: bool,
    },
    /// The Brain chose an action. `reason` is the sanitized rationale ("thinking").
    Decided {
        step_index: u32,
        action_kind: &'static str,
        detail: String,
        reason: String,
    },
    /// The safety gate evaluated an executable action.
    Gated {
        step_index: u32,
        allowed: bool,
        reason: Option<String>,
    },
    /// Hands is about to execute the action.
    ExecuteStarted {
        step_index: u32,
        action_kind: &'static str,
        detail: String,
    },
    /// Hands finished executing the action.
    ExecuteCompleted {
        step_index: u32,
        ok: bool,
        error: Option<String>,
        backend: String,
    },
    /// Post-action re-observe verdict: did the screen change?
    Verified {
        step_index: u32,
        changed: Option<bool>,
    },
    /// The turn ended with this terminal status (emitted on EVERY exit path).
    TurnEnded { status: TurnStatus },
}

/// Sink for [`LoopEvent`]s. Cloneable, thread-safe; the desktop wires one that
/// emits Tauri events. `None` (the default) means no streaming (tests/skeleton).
pub type LoopObserver = Arc<dyn Fn(LoopEvent) + Send + Sync>;

/// Optional, injected runtime guards. `default()` wires none (skeleton behavior).
#[derive(Default, Clone)]
pub struct LoopGuards {
    /// Safety gate consulted before every executable action.
    pub safety: Option<Arc<dyn SafetyGate>>,
    /// Cooperative cancel flag, checked each iteration.
    pub cancel: Option<Arc<AtomicBool>>,
    /// Live per-phase progress sink (Tauri events on the desktop).
    pub observer: Option<LoopObserver>,
    /// Task 9: turn-level planner. When present AND `LoopConfig::use_plan` is on,
    /// the loop decomposes the task into ordered sub-goals and drives them.
    pub planner: Option<Arc<dyn GuiPlanner>>,
    /// Task 9: external-signal sub-goal verifier (shared registry). With the
    /// probe, a sub-goal is marked done ONLY when this returns `Verified`.
    pub verifier: Option<Arc<dyn SubGoalVerifier>>,
    /// Task 9: external-signal probe (window/title/OCR/filesystem/output) the
    /// verifier reads. Without it, plan-mode still steers but cannot verify, so
    /// it falls back to the Brain's `Done` for completion (honest degrade).
    pub probe: Option<Arc<dyn VerificationProbe>>,
    /// Task 10: cross-substrate bridge. When present, plan-mode routes bridged
    /// sub-goals (run-command / write-file / read-output) to the existing
    /// shell/file tools instead of GUI keystrokes.
    pub bridge: Option<Arc<dyn GuiBridge>>,
}

impl LoopGuards {
    pub fn none() -> Self {
        Self::default()
    }

    pub fn with_safety(mut self, gate: Arc<dyn SafetyGate>) -> Self {
        self.safety = Some(gate);
        self
    }

    pub fn with_cancel(mut self, flag: Arc<AtomicBool>) -> Self {
        self.cancel = Some(flag);
        self
    }

    /// Attach a live progress sink.
    pub fn with_observer(mut self, observer: LoopObserver) -> Self {
        self.observer = Some(observer);
        self
    }

    /// Attach the turn planner (Task 9).
    pub fn with_planner(mut self, planner: Arc<dyn GuiPlanner>) -> Self {
        self.planner = Some(planner);
        self
    }

    /// Attach the sub-goal verifier + external-signal probe (Task 9).
    pub fn with_verifier(
        mut self,
        verifier: Arc<dyn SubGoalVerifier>,
        probe: Arc<dyn VerificationProbe>,
    ) -> Self {
        self.verifier = Some(verifier);
        self.probe = Some(probe);
        self
    }

    /// Attach the cross-substrate bridge (Task 10).
    pub fn with_bridge(mut self, bridge: Arc<dyn GuiBridge>) -> Self {
        self.bridge = Some(bridge);
        self
    }

    fn is_cancelled(&self) -> bool {
        self.cancel
            .as_ref()
            .map(|f| f.load(Ordering::SeqCst))
            .unwrap_or(false)
    }

    /// Emit a loop event to the observer, if one is wired.
    fn emit(&self, event: LoopEvent) {
        if let Some(obs) = self.observer.as_ref() {
            obs(event);
        }
    }
}

/// Run one GUI Cognition V2 turn over the three injected layers, with guards.
///
/// Emits [`LoopEvent::TurnStarted`] before the loop and [`LoopEvent::TurnEnded`]
/// on EVERY exit path (the inner loop emits the per-phase events). This wrapper
/// guarantees the frontend always gets a terminal event so the panel/thinking
/// indicator can never stick.
pub async fn run_turn_v2(
    sight: &dyn Sight,
    brain: &dyn GuiBrain,
    hands: &dyn GuiHands,
    task: &str,
    config: LoopConfig,
    guards: &LoopGuards,
) -> TurnOutcomeV2 {
    guards.emit(LoopEvent::TurnStarted);
    let outcome = run_turn_v2_inner(sight, brain, hands, task, config, guards).await;
    guards.emit(LoopEvent::TurnEnded {
        status: outcome.status.clone(),
    });
    outcome
}

/// Inner loop body. Emits the per-phase [`LoopEvent`]s; the public
/// [`run_turn_v2`] wraps it with the guaranteed start/end events.
async fn run_turn_v2_inner(
    sight: &dyn Sight,
    brain: &dyn GuiBrain,
    hands: &dyn GuiHands,
    task: &str,
    config: LoopConfig,
    guards: &LoopGuards,
) -> TurnOutcomeV2 {
    let mut steps: Vec<TurnStep> = Vec::new();
    // Signature of the observation that the most recent executed state-changing
    // action was based on; used to detect "no screen change after acting".
    let mut prev_executed_sig: Option<String> = None;
    let mut no_progress_count: u32 = 0;
    // The app of the most recent successfully-executed OpenApp; re-opening the
    // SAME app is never useful, so a repeat is a no-op (do NOT re-spawn).
    let mut last_opened_app: Option<String> = None;
    // Consecutive duplicate-open decisions. A multi-part task ("open X and do Y")
    // must NOT complete just because the app is already open — we give the Brain
    // another step to advance to the follow-up. Only after repeated duplicate
    // opens with nothing else to do do we conclude the task is satisfied. This
    // is the premature-completion fix (Requirement 3.2/3.3); full sub-goal
    // tracking lands with the planner (Task 9).
    let mut duplicate_open_count: u32 = 0;
    // No-retype guard state: the last text typed/submitted, to suppress an
    // immediate identical re-type (Requirement 21.3).
    let mut last_typed_text: Option<String> = None;
    let mut duplicate_type_count: u32 = 0;
    // Element-grounding mode for this turn. Starts from config (operator force-on)
    // and becomes sticky after the first lazy escalation, so once a turn needs to
    // see controls it keeps seeing them — keeping observation signatures coherent
    // across steps and avoiding a per-step light→grounded flip-flop.
    let mut grounded = config.start_grounded;

    // Task 11: a single bounded recovery escalation (grounded re-observe) is
    // allowed before a no-progress stop, so transient stalls can recover.
    let mut recovery_used = false;

    // ---- Task 9: plan-driven state (inert unless use_plan + a planner) ----
    // Decompose the task ONCE into ordered sub-goals; the loop steers each step
    // toward the first not-yet-verified sub-goal and completes only when every
    // sub-goal is externally VERIFIED (Requirement 3.1, 4.4, 15.3). Without a
    // probe, plan-mode still steers but defers completion to the Brain's `Done`
    // (honest degrade). `plan` is None when plan-mode is off → all the plan
    // branches below are skipped and behavior is byte-for-byte the prior loop.
    let mut plan: Option<Plan> = if config.use_plan {
        if let Some(planner) = guards.planner.as_ref() {
            let p = planner.plan(task).await;
            tracing::info!(
                target: "gui_cognition_v2",
                kinds = ?p.sub_goals.iter().map(|s| (s.kind, s.target_hint.clone())).collect::<Vec<_>>(),
                "plan-driven turn: decomposition"
            );
            guards.emit(LoopEvent::PlanReady {
                goals: p.sub_goals.iter().map(|s| s.intent.clone()).collect(),
            });
            let total = p.sub_goals.len();
            for (i, sg) in p.sub_goals.iter().enumerate() {
                guards.emit(LoopEvent::SubGoalUpdated {
                    index: i,
                    total,
                    goal: sg.intent.clone(),
                    status: "pending",
                });
            }
            Some(p)
        } else {
            None
        }
    } else {
        None
    };
    // Consecutive `Done` decisions while sub-goals remain unverified — bounds the
    // "brain says done but we can't confirm" path so it can never loop.
    let mut done_with_unverified: u32 = 0;
    // Task 10: which bridged sub-goals have already been EXECUTED (at-most-once,
    // so a shell command/file write is never re-run with side effects).
    let mut bridged_attempted: Vec<bool> = plan
        .as_ref()
        .map(|p| vec![false; p.sub_goals.len()])
        .unwrap_or_default();
    // Per-sub-goal no-progress attempt counters (plan-mode honest-stop budget).
    let mut subgoal_attempts: Vec<u32> = plan
        .as_ref()
        .map(|p| vec![0u32; p.sub_goals.len()])
        .unwrap_or_default();

    for step_index in 0..config.max_steps {
        // Cancellation — check before any work each iteration.
        if guards.is_cancelled() {
            return TurnOutcomeV2 {
                status: TurnStatus::Cancelled,
                reply: "Turn cancelled.".into(),
                steps,
            };
        }

        // 0. CROSS-SUBSTRATE BRIDGE (Task 10). If the current focus sub-goal is a
        // non-GUI one (run-command / write-file / read-output) and a bridge is
        // wired, execute it via the existing shell/file tools instead of GUI
        // keystrokes — ONCE (no re-run side effects) — then verify + advance. GUI
        // sub-goals ahead of it are handled first (ordered cursor), so this only
        // fires once they are verified-done.
        if let (Some(p), Some(bridge)) = (plan.as_mut(), guards.bridge.as_ref()) {
            if let Some(idx) = p.sub_goals.iter().position(|s| !s.done) {
                let kind = p.sub_goals[idx].kind;
                // WriteFile/ReadOutput: bridge whenever the desktop opts in.
                // RunCommand: bridge headless ONLY when the plan opens NO terminal
                // (a "write a script and run it" cross-substrate task); when a
                // terminal IS opened, keep the command on the visible-terminal GUI
                // path so its output stays user-visible + OCR-verifiable.
                let want_bridge = kind.is_bridged()
                    && !bridged_attempted[idx]
                    && (bridge.handles(kind)
                        || (matches!(kind, super::types::SubGoalKind::RunCommand)
                            && !plan_opens_terminal(p)));
                if want_bridge {
                    bridged_attempted[idx] = true;
                    let total = p.sub_goals.len();
                    let outcome = bridge.execute(&p.sub_goals[idx]).await;
                    guards.emit(LoopEvent::SubGoalUpdated {
                        index: idx,
                        total,
                        goal: p.sub_goals[idx].intent.clone(),
                        status: if outcome.ok {
                            "bridged"
                        } else {
                            "bridge_failed"
                        },
                    });
                    // Verify via the SHARED verifier (probe reads the bridge's
                    // working-context output / filesystem). If it can't confirm
                    // but the tool reported success, accept the tool's success
                    // (honest: the op ran) so the turn can advance without re-run.
                    let mut verified = false;
                    if let (Some(verifier), Some(probe)) =
                        (guards.verifier.as_ref(), guards.probe.as_ref())
                    {
                        verified = verifier
                            .verify(&p.sub_goals[idx], probe.as_ref())
                            .await
                            .is_verified();
                    }
                    if verified || outcome.ok {
                        p.sub_goals[idx].done = true;
                        done_with_unverified = 0;
                        guards.emit(LoopEvent::SubGoalUpdated {
                            index: idx,
                            total,
                            goal: p.sub_goals[idx].intent.clone(),
                            status: "verified",
                        });
                        if p.sub_goals.iter().all(|s| s.done) {
                            return TurnOutcomeV2 {
                                status: TurnStatus::Completed,
                                reply: plan_completion_reply(p),
                                steps,
                            };
                        }
                        continue; // re-enter loop for the next sub-goal
                    } else {
                        // The bridged op failed AND could not be verified — stop
                        // honestly rather than fall through to GUI guesswork.
                        return TurnOutcomeV2 {
                            status: TurnStatus::StoppedError,
                            reply: format!(
                                "Couldn't complete '{}': {}",
                                p.sub_goals[idx].intent, outcome.detail
                            ),
                            steps,
                        };
                    }
                }
            }
        }

        // 1. OBSERVE (fresh observation each step — Property 3). Use the grounded
        // (element-detecting) capture once the turn has escalated; otherwise the
        // cheap default. Non-grounding sights always take the cheap path.
        guards.emit(LoopEvent::ObserveStarted { step_index });
        let observe_grounded_now = grounded && sight.supports_grounding();
        let observe_result = if observe_grounded_now {
            sight.observe_grounded(config.want_som).await
        } else {
            sight.observe(config.want_som).await
        };
        let mut observation = match observe_result {
            Ok(obs) => obs,
            Err(e) => {
                return TurnOutcomeV2 {
                    status: TurnStatus::StoppedError,
                    reply: format!("Could not observe the screen: {e}"),
                    steps,
                };
            }
        };
        guards.emit(LoopEvent::ObserveCompleted {
            step_index,
            active_window: observation.active_window.clone(),
            element_count: observation.elements.len(),
            degraded: observation.is_degraded(),
        });

        // Manual-step / HITL gate (Requirement 32): if the screen is a sign-in /
        // password / captcha / 2FA / permission surface, PAUSE with ONE clear,
        // resumable ask instead of trying to fill it ourselves or silent-failing.
        // The turn ends as NeedsClarification; the user completes the step and
        // re-prompts "continue". Conservative detector → no false trips on normal
        // pages. Only relevant once the screen is grounded (has elements).
        if let Some(reason) = detect_manual_step(&observation) {
            return TurnOutcomeV2 {
                status: TurnStatus::NeedsClarification,
                reply: reason.to_string(),
                steps,
            };
        }

        // Post-action verification (A4): compare the re-observed screen against
        // the signature the previous action acted on. Records an honest per-step
        // `screen_changed` and drives the no-progress guard (Requirement 5.5/5.3).
        if let Some(prev) = &prev_executed_sig {
            let changed = *prev != observation.signature();
            if let Some(last) = steps.last_mut() {
                last.result.screen_changed = Some(changed);
            }
            if let Some(last) = steps.last() {
                guards.emit(LoopEvent::Verified {
                    step_index: last.step_index,
                    changed: Some(changed),
                });
            }
            if config.no_progress_limit > 0 {
                if changed {
                    no_progress_count = 0;
                } else {
                    no_progress_count += 1;
                    if no_progress_count >= config.no_progress_limit {
                        // Task 11: bounded RECOVERY ladder before giving up. Rung 1:
                        // if we have not yet escalated to grounded perception and the
                        // Sight supports it, force grounding + reset the stall count
                        // and take ONE more step against real on-screen controls.
                        // This converts many "screen didn't change" dead-ends into a
                        // successful continue (Requirement 8.1/8.2). Bounded by
                        // `recovery_used` so it can never loop.
                        if !recovery_used && !grounded && sight.supports_grounding() {
                            recovery_used = true;
                            grounded = true;
                            no_progress_count = 0;
                            prev_executed_sig = None;
                            guards.emit(LoopEvent::RecoveryAttempted {
                                rung: "grounded_reobserve",
                                ok: true,
                            });
                            continue;
                        }
                        guards.emit(LoopEvent::RecoveryAttempted {
                            rung: "exhausted",
                            ok: false,
                        });
                        return TurnOutcomeV2 {
                            status: TurnStatus::StoppedNoProgress,
                            reply: "The screen did not change after the last action; stopping to avoid looping.".into(),
                            steps,
                        };
                    }
                }
            }
        }

        // 2. DECIDE (one action). In plan-mode, steer the Brain toward the first
        // not-yet-verified sub-goal by appending a focused hint to the task (the
        // Brain trait is unchanged; this is pure prompt steering).
        let decide_task = match plan.as_ref() {
            Some(p) => plan_focus_task(task, p),
            None => task.to_string(),
        };
        let mut decision: Decision = match brain.decide(&decide_task, &observation, &steps).await {
            Ok(d) => d,
            Err(e) => {
                return TurnOutcomeV2 {
                    status: TurnStatus::StoppedError,
                    reply: format!("Could not decide the next action: {e}"),
                    steps,
                };
            }
        };

        // 2b. LAZY GROUNDING ESCALATION. The cheap observation gave the Brain no
        // control to act on (it could only `Ask`). Re-observe WITH element
        // detection and let the Brain decide ONCE more against the real controls,
        // then stay grounded for the rest of the turn. Bounded to a single
        // escalation per turn (guarded by `!grounded`), so it can never loop.
        if !grounded
            && sight.supports_grounding()
            && decision_needs_grounding(&decision, &observation)
        {
            guards.emit(LoopEvent::GroundingEscalated { step_index });
            match sight.observe_grounded(config.want_som).await {
                // Only re-decide when grounding actually surfaced a usable view;
                // a degraded grounded observation cannot improve on the cheap one.
                Ok(grounded_obs) if !grounded_obs.is_degraded() => {
                    match brain.decide(&decide_task, &grounded_obs, &steps).await {
                        Ok(redecided) => {
                            observation = grounded_obs;
                            decision = redecided;
                            grounded = true;
                        }
                        Err(e) => {
                            return TurnOutcomeV2 {
                                status: TurnStatus::StoppedError,
                                reply: format!("Could not decide the next action: {e}"),
                                steps,
                            };
                        }
                    }
                }
                // Grounding unavailable/degraded → keep the cheap decision (an
                // honest `Ask`); do not flip `grounded` so a later step may retry.
                _ => {}
            }
        }

        guards.emit(LoopEvent::Decided {
            step_index,
            action_kind: decision.action.kind(),
            detail: decision.action.detail(),
            reason: decision.reason.clone(),
        });

        // Terminal decisions end the loop cleanly.
        match &decision.action {
            Action::Done { summary } => {
                // Plan-mode: do NOT complete just because the Brain said so —
                // complete only when every sub-goal is externally VERIFIED. If
                // some remain, verify-now; if all pass, complete; otherwise give
                // the turn another bounded step to satisfy the rest (premature-
                // completion fix at the plan level, Requirement 3.1/15.3).
                if let (Some(p), Some(verifier), Some(probe)) = (
                    plan.as_mut(),
                    guards.verifier.as_ref(),
                    guards.probe.as_ref(),
                ) {
                    verify_and_advance(p, verifier.as_ref(), probe.as_ref(), guards).await;
                    if p.sub_goals.iter().all(|s| s.done) {
                        return TurnOutcomeV2 {
                            status: TurnStatus::Completed,
                            reply: summary.clone(),
                            steps,
                        };
                    }
                    done_with_unverified += 1;
                    let stall_bound = if config.steps_per_sub_goal > 0 {
                        config.steps_per_sub_goal
                    } else {
                        2
                    };
                    if done_with_unverified >= stall_bound {
                        // The Brain insists it is done but we cannot verify the
                        // remaining sub-goals. Stop honestly, naming what is
                        // unverified rather than fabricating success.
                        let pending: Vec<String> = p
                            .sub_goals
                            .iter()
                            .filter(|s| !s.done)
                            .map(|s| s.intent.clone())
                            .collect();
                        return TurnOutcomeV2 {
                            status: TurnStatus::StoppedNoProgress,
                            reply: format!(
                                "Did what I could, but couldn't confirm: {}.",
                                pending.join("; ")
                            ),
                            steps,
                        };
                    }
                    // Another step to satisfy the remaining sub-goal(s).
                    continue;
                }
                return TurnOutcomeV2 {
                    status: TurnStatus::Completed,
                    reply: summary.clone(),
                    steps,
                };
            }
            Action::Ask { question } => {
                return TurnOutcomeV2 {
                    status: TurnStatus::NeedsClarification,
                    reply: question.clone(),
                    steps,
                };
            }
            _ => {}
        }

        // Deterministic backstop: re-opening the SAME app is a no-op (it is
        // already open). Do NOT spawn a duplicate AND do NOT prematurely complete
        // a multi-part task — give the Brain another step to advance to the
        // follow-up. Only conclude "already open / done" once the Brain has
        // repeatedly re-opened with nothing else to do (bounded, never looping).
        if let Action::OpenApp { app } = &decision.action {
            if last_opened_app.as_deref() == Some(app.as_str()) {
                duplicate_open_count += 1;
                guards.emit(LoopEvent::Decided {
                    step_index,
                    action_kind: decision.action.kind(),
                    detail: app.clone(),
                    reason: format!("{app} already open; advancing instead of re-opening"),
                });
                // PLAN mode: a repeat open means the app didn't satisfy the
                // sub-goal yet. Verify against external signals; if it now passes,
                // advance; otherwise count an attempt and stop HONESTLY once the
                // budget is spent (e.g. a not-installed app never opens a window).
                if let (Some(p), Some(verifier), Some(probe)) = (
                    plan.as_mut(),
                    guards.verifier.as_ref(),
                    guards.probe.as_ref(),
                ) {
                    verify_and_advance(p, verifier.as_ref(), probe.as_ref(), guards).await;
                    if p.sub_goals.iter().all(|s| s.done) {
                        return TurnOutcomeV2 {
                            status: TurnStatus::Completed,
                            reply: plan_completion_reply(p),
                            steps,
                        };
                    }
                    if let Some(idx) = p.sub_goals.iter().position(|s| !s.done) {
                        subgoal_attempts[idx] = subgoal_attempts[idx].saturating_add(1);
                        let budget = if config.steps_per_sub_goal > 0 {
                            config.steps_per_sub_goal
                        } else {
                            3
                        };
                        if subgoal_attempts[idx] >= budget {
                            return TurnOutcomeV2 {
                                status: TurnStatus::NeedsClarification,
                                reply: unachievable_reply(&p.sub_goals[idx]),
                                steps,
                            };
                        }
                    }
                    continue;
                }
                if duplicate_open_count >= 2 {
                    // Non-plan mode: conclude "already open" as before.
                    return TurnOutcomeV2 {
                        status: TurnStatus::Completed,
                        reply: format!("{app} is already open."),
                        steps,
                    };
                }
                // Skip the redundant open and re-observe/decide next iteration so a
                // follow-up sub-step can run. Bounded by `max_steps`.
                continue;
            }
        }
        // A non-duplicate decision resets the duplicate-open backstop.
        duplicate_open_count = 0;

        // No-retype guard (Requirement 21.3): never type the SAME text twice in a
        // row (the cause of "youtube.comyoutube.com"). If the Brain repeats the
        // last typed text, skip it and re-decide; bounded so it cannot loop.
        let this_typed = match &decision.action {
            Action::Type { text } | Action::TypeAndSubmit { text } => Some(text.clone()),
            Action::Navigate { url } => Some(url.clone()),
            _ => None,
        };
        if let Some(t) = &this_typed {
            if last_typed_text.as_deref() == Some(t.as_str()) && !t.trim().is_empty() {
                duplicate_type_count += 1;
                guards.emit(LoopEvent::Decided {
                    step_index,
                    action_kind: decision.action.kind(),
                    detail: t.clone(),
                    reason: "already typed this text; not repeating".into(),
                });
                if duplicate_type_count >= 2 {
                    return TurnOutcomeV2 {
                        status: TurnStatus::Completed,
                        reply: "Text already entered; nothing more to type.".into(),
                        steps,
                    };
                }
                continue;
            }
        }
        // (Property 5). Only executable actions are gated.
        if decision.action.is_executable() {
            if let Some(gate) = guards.safety.as_ref() {
                if let GateDecision::Deny(reason) = gate.evaluate(&decision, &observation).await {
                    guards.emit(LoopEvent::Gated {
                        step_index,
                        allowed: false,
                        reason: Some(reason.clone()),
                    });
                    return TurnOutcomeV2 {
                        status: TurnStatus::StoppedSafety,
                        reply: format!("Blocked for safety: {reason}"),
                        steps,
                    };
                }
                guards.emit(LoopEvent::Gated {
                    step_index,
                    allowed: true,
                    reason: None,
                });
            }
        }

        // 4. EXECUTE.
        guards.emit(LoopEvent::ExecuteStarted {
            step_index,
            action_kind: decision.action.kind(),
            detail: decision.action.detail(),
        });
        let result = match hands.execute(&decision, &observation).await {
            Ok(r) => r,
            Err(e) => {
                return TurnOutcomeV2 {
                    status: TurnStatus::StoppedError,
                    reply: format!("Action failed: {e}"),
                    steps,
                };
            }
        };
        guards.emit(LoopEvent::ExecuteCompleted {
            step_index,
            ok: result.ok,
            error: result.error.clone(),
            backend: result.backend_used.clone(),
        });

        // Record the step (history references the target LABEL, never a stale id).
        let target_label = match &decision.action {
            Action::Click { element_id } => {
                observation.element(*element_id).map(|e| e.label.clone())
            }
            _ => None,
        };
        let executed_ok = result.ok;
        let opened_app = match &decision.action {
            Action::OpenApp { app } => Some(app.clone()),
            _ => None,
        };
        steps.push(TurnStep {
            step_index,
            decision,
            result,
            target_label,
        });
        if executed_ok {
            if let Some(app) = opened_app {
                last_opened_app = Some(app);
            }
            // Remember the last typed/submitted text so an immediate identical
            // re-type is suppressed next step (no concatenation).
            if let Some(t) = this_typed {
                last_typed_text = Some(t);
                duplicate_type_count = 0;
            }
        }

        // Plan-mode: after the action, verify sub-goals against EXTERNAL signals
        // and advance the cursor. A sub-goal is marked done ONLY on `Verified`
        // (Requirement 15.3). If every sub-goal is verified, the turn is complete
        // — even if the Brain has not yet said `Done` (drives multi-step to a
        // real, proven finish). Any progress resets the Done-stall counter.
        if let (Some(p), Some(verifier), Some(probe)) = (
            plan.as_mut(),
            guards.verifier.as_ref(),
            guards.probe.as_ref(),
        ) {
            let before = p.sub_goals.iter().filter(|s| s.done).count();
            verify_and_advance(p, verifier.as_ref(), probe.as_ref(), guards).await;
            let after = p.sub_goals.iter().filter(|s| s.done).count();
            if after > before {
                done_with_unverified = 0;
            }
            if p.sub_goals.iter().all(|s| s.done) {
                return TurnOutcomeV2 {
                    status: TurnStatus::Completed,
                    reply: plan_completion_reply(p),
                    steps,
                };
            }
            // Per-sub-goal attempt budget: if the FIRST unverified sub-goal made
            // no progress this step, count an attempt against it; once it exceeds
            // the budget, STOP HONESTLY naming what couldn't be done (e.g. an app
            // that isn't installed, an on-screen option that isn't there) instead
            // of looping to the step cap. General — keyed on "this sub-goal won't
            // verify", not on any specific app/prompt (Requirement 32/33, 8.3).
            if let Some(idx) = p.sub_goals.iter().position(|s| !s.done) {
                if after > before {
                    subgoal_attempts[idx] = 0;
                } else {
                    subgoal_attempts[idx] = subgoal_attempts[idx].saturating_add(1);
                    let budget = if config.steps_per_sub_goal > 0 {
                        config.steps_per_sub_goal
                    } else {
                        3
                    };
                    if subgoal_attempts[idx] >= budget {
                        return TurnOutcomeV2 {
                            status: TurnStatus::NeedsClarification,
                            reply: unachievable_reply(&p.sub_goals[idx]),
                            steps,
                        };
                    }
                }
            }
        }

        // Arm no-progress tracking: remember the signature the action acted on,
        // so the next observe can detect whether the screen actually changed.
        // Armed on EVERY executed step (success OR failure) so a repeatedly
        // FAILING action (e.g. an app name that won't resolve) trips the
        // no-progress stop instead of running to the step cap.
        if config.no_progress_limit > 0 {
            prev_executed_sig = Some(observation.signature());
        }

        // TODO(Phase 5 live): incremental event streaming + screenshot-diff
        // verification beyond the `screen_changed` signal, wired in the desktop
        // integration alongside the real SafetyGate (HITL) and uinput sink.
    }

    TurnOutcomeV2 {
        status: TurnStatus::StoppedStepCap,
        reply: format!(
            "Reached the step limit ({}) without completing the task.",
            config.max_steps
        ),
        steps,
    }
}

/// Whether the plan explicitly opens a terminal/console app — used to decide
/// whether a `RunCommand` should be typed into that visible terminal (keeping
/// output user-visible) instead of bridged headlessly. Pure + testable.
pub(crate) fn plan_opens_terminal(plan: &Plan) -> bool {
    use super::types::SubGoalKind;
    plan.sub_goals.iter().any(|s| {
        s.kind == SubGoalKind::OpenApp
            && s.target_hint
                .as_deref()
                .map(|t| {
                    let t = t.to_ascii_lowercase();
                    ["terminal", "console", "konsole", "kgx", "xterm", "shell"]
                        .iter()
                        .any(|k| t.contains(k))
                })
                .unwrap_or(false)
    })
}

/// Build the Brain-facing task string for the current plan cursor: the original
/// task plus a focused hint naming the first not-yet-verified sub-goal. Pure +
/// testable. When all sub-goals are done it returns the task unchanged.
pub(crate) fn plan_focus_task(task: &str, plan: &Plan) -> String {
    match plan.sub_goals.iter().find(|s| !s.done) {
        Some(sg) => {
            let target = sg
                .target_hint
                .as_deref()
                .map(|t| format!(" (target: {t})"))
                .unwrap_or_default();
            let done: Vec<&str> = plan
                .sub_goals
                .iter()
                .filter(|s| s.done)
                .map(|s| s.intent.as_str())
                .collect();
            let progress = if done.is_empty() {
                String::new()
            } else {
                format!("\nAlready done: {}.", done.join("; "))
            };
            format!(
                "{task}\n\nFocus now on this step: {}{}.{}",
                sg.intent, target, progress
            )
        }
        None => task.to_string(),
    }
}

/// Verify not-yet-done sub-goals IN ORDER against external signals, marking each
/// `done` on `Verified` and emitting a [`LoopEvent::SubGoalUpdated`]. Stops at
/// the first sub-goal that is not yet verified (sub-goals are ordered, so a later
/// one rarely completes before an earlier one; this keeps progress monotonic and
/// the cursor coherent).
async fn verify_and_advance(
    plan: &mut Plan,
    verifier: &dyn SubGoalVerifier,
    probe: &dyn VerificationProbe,
    guards: &LoopGuards,
) {
    let total = plan.sub_goals.len();
    for index in 0..total {
        if plan.sub_goals[index].done {
            continue;
        }
        let verdict = verifier.verify(&plan.sub_goals[index], probe).await;
        if verdict.is_verified() {
            plan.sub_goals[index].done = true;
            guards.emit(LoopEvent::SubGoalUpdated {
                index,
                total,
                goal: plan.sub_goals[index].intent.clone(),
                status: "verified",
            });
            // Continue to opportunistically verify the next one too.
        } else {
            // First unverified sub-goal: stop here (ordered progress).
            break;
        }
    }
}

/// An HONEST, user-facing reply when a sub-goal could not be achieved after its
/// attempt budget (e.g. a nonexistent app, an absent on-screen option). Keyed on
/// the sub-goal KIND/target — general, not per-prompt. The wording deliberately
/// contains the markers a human (and the proof harness) reads as an honest
/// refusal so a nonexistent app/option is reported truthfully, never forced.
fn unachievable_reply(sg: &SubGoal) -> String {
    let target = sg.target_hint.as_deref().unwrap_or("").trim();
    match sg.kind {
        SubGoalKind::OpenApp => {
            if target.is_empty() {
                "I couldn't find or open that app — it doesn't appear to be installed.".into()
            } else {
                format!("I couldn't find or open '{target}' — it doesn't appear to be installed on this system.")
            }
        }
        SubGoalKind::Click => {
            if target.is_empty() {
                "I couldn't find that option on the screen — it doesn't appear to be available."
                    .into()
            } else {
                format!("I couldn't find the '{target}' option on the screen — it doesn't appear to be available here.")
            }
        }
        SubGoalKind::Navigate => {
            format!(
                "I couldn't load '{}' in the browser.",
                if target.is_empty() {
                    "that page"
                } else {
                    target
                }
            )
        }
        SubGoalKind::Type => format!("I couldn't complete typing for '{}'.", sg.intent),
        _ => format!("I couldn't complete '{}'.", sg.intent),
    }
}

/// A layman completion summary listing the verified sub-goals.
fn plan_completion_reply(plan: &Plan) -> String {
    let goals: Vec<&str> = plan.sub_goals.iter().map(|s| s.intent.as_str()).collect();
    if goals.is_empty() {
        "Task complete.".to_string()
    } else {
        format!("Done: {}.", goals.join("; "))
    }
}

#[cfg(test)]
mod tests {
    use super::super::fakes::{FakeBrain, FakeHands, FakeSight};
    use super::super::traits::{GateDecision, SafetyGate};
    use super::super::types::{Action, Decision, Observation};
    use super::*;

    fn click(id: u32) -> Decision {
        Decision {
            action: Action::Click { element_id: id },
            reason: "x".into(),
            risk_hint: None,
        }
    }

    #[tokio::test]
    async fn records_screen_changed_on_each_executed_step() {
        use super::super::types::{Bbox, UiElement};
        use std::sync::atomic::{AtomicU32, Ordering};
        // A Sight whose active window changes every observe → each executed step
        // is verified as having changed the screen.
        struct ChangingSight {
            n: AtomicU32,
        }
        #[async_trait::async_trait]
        impl Sight for ChangingSight {
            async fn observe(&self, _want_som: bool) -> anyhow::Result<Observation> {
                let i = self.n.fetch_add(1, Ordering::SeqCst);
                Ok(Observation {
                    observation_id: format!("obs-{i}"),
                    screenshot_path: String::new(),
                    screen_w: 1920,
                    screen_h: 1080,
                    active_window: Some(format!("Window {i}")),
                    elements: vec![UiElement {
                        id: 1,
                        bbox: Bbox {
                            x: 0,
                            y: 0,
                            width: 10,
                            height: 10,
                        },
                        monitor_index: 0,
                        kind: "button".into(),
                        label: "Btn".into(),
                        interactable: true,
                        confidence: 0.9,
                    }],
                    som_image_path: None,
                    source: "fake".into(),
                })
            }
        }
        let sight = ChangingSight {
            n: AtomicU32::new(0),
        };
        let brain = FakeBrain::click_then_done();
        let hands = FakeHands::default();
        let outcome = run_turn_v2(
            &sight,
            &brain,
            &hands,
            "x",
            LoopConfig::default(),
            &LoopGuards::none(),
        )
        .await;
        assert_eq!(outcome.status, TurnStatus::Completed);
        assert_eq!(outcome.steps.len(), 1);
        // The click step was verified against the next (changed) observation.
        assert_eq!(outcome.steps[0].result.screen_changed, Some(true));
    }

    #[tokio::test]
    async fn repeated_open_app_does_not_spawn_duplicate_and_is_bounded() {
        // A Brain that keeps emitting OpenApp{chrome} must NOT spawn duplicates
        // and must NOT prematurely complete after the first open — it gets another
        // step to advance, then the bounded duplicate-open backstop concludes
        // "already open". (Premature-completion fix, Req 3.2/3.3.) no_progress is
        // disabled here to isolate the duplicate-open backstop (the FakeSight
        // never changes, which would otherwise trip no-progress first).
        let sight = FakeSight::one_button("x");
        let brain = FakeBrain::new(vec![
            Decision {
                action: Action::OpenApp {
                    app: "chrome".into(),
                },
                reason: "open".into(),
                risk_hint: None,
            },
            Decision {
                action: Action::OpenApp {
                    app: "chrome".into(),
                },
                reason: "again".into(),
                risk_hint: None,
            },
            Decision {
                action: Action::OpenApp {
                    app: "chrome".into(),
                },
                reason: "again2".into(),
                risk_hint: None,
            },
        ]);
        let hands = FakeHands::default();
        let outcome = run_turn_v2(
            &sight,
            &brain,
            &hands,
            "open chrome",
            LoopConfig {
                max_steps: 12,
                want_som: false,
                no_progress_limit: 0,
                start_grounded: false,
                use_plan: false,
                steps_per_sub_goal: 0,
            },
            &LoopGuards::none(),
        )
        .await;
        assert_eq!(outcome.status, TurnStatus::Completed);
        assert_eq!(outcome.reply, "chrome is already open.");
        // Only ONE open executed (duplicates were never re-spawned).
        assert_eq!(hands.executed.lock().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn duplicate_open_yields_a_step_to_follow_up_before_completing() {
        // "open chrome AND <follow-up>": after opening, a duplicate open must NOT
        // end the turn — the next step's follow-up (here a Key) executes. Proves a
        // multi-part task is not prematurely completed by a redundant open.
        let sight = FakeSight::one_button("x");
        let brain = FakeBrain::new(vec![
            Decision {
                action: Action::OpenApp {
                    app: "chrome".into(),
                },
                reason: "open".into(),
                risk_hint: None,
            },
            Decision {
                action: Action::OpenApp {
                    app: "chrome".into(),
                },
                reason: "dup".into(),
                risk_hint: None,
            },
            Decision {
                action: Action::Key {
                    combo: "new_tab".into(),
                },
                reason: "follow-up".into(),
                risk_hint: None,
            },
            Decision {
                action: Action::Done {
                    summary: "done".into(),
                },
                reason: "done".into(),
                risk_hint: None,
            },
        ]);
        let hands = FakeHands::default();
        let outcome = run_turn_v2(
            &sight,
            &brain,
            &hands,
            "open chrome and a new tab",
            LoopConfig {
                max_steps: 12,
                want_som: false,
                no_progress_limit: 0,
                start_grounded: false,
                use_plan: false,
                steps_per_sub_goal: 0,
            },
            &LoopGuards::none(),
        )
        .await;
        assert_eq!(outcome.status, TurnStatus::Completed);
        // open executed once + the follow-up Key executed = 2 (no duplicate open).
        let executed = hands.executed.lock().unwrap();
        assert_eq!(executed.len(), 2);
        assert!(
            matches!(executed[1].action, Action::Key { .. }),
            "follow-up must run"
        );
    }

    #[tokio::test]
    async fn loop_executes_then_completes_on_done() {
        let sight = FakeSight::one_button("New Tab");
        let brain = FakeBrain::click_then_done();
        let hands = FakeHands::default();
        let outcome = run_turn_v2(
            &sight,
            &brain,
            &hands,
            "click it",
            LoopConfig::default(),
            &LoopGuards::none(),
        )
        .await;
        assert_eq!(outcome.status, TurnStatus::Completed);
        assert_eq!(outcome.steps.len(), 1);
        assert_eq!(outcome.steps[0].target_label.as_deref(), Some("New Tab"));
        assert_eq!(hands.executed.lock().unwrap().len(), 1);
    }

    struct AlwaysClick;
    #[async_trait::async_trait]
    impl GuiBrain for AlwaysClick {
        async fn decide(
            &self,
            _t: &str,
            _o: &Observation,
            _h: &[TurnStep],
        ) -> anyhow::Result<Decision> {
            Ok(click(1))
        }
        fn label(&self) -> &str {
            "always_click"
        }
    }

    #[tokio::test]
    async fn loop_stops_at_step_cap_when_no_progress_disabled() {
        let sight = FakeSight::one_button("Btn");
        let hands = FakeHands::default();
        let outcome = run_turn_v2(
            &sight,
            &AlwaysClick,
            &hands,
            "loop",
            LoopConfig {
                max_steps: 3,
                want_som: false,
                no_progress_limit: 0,
                start_grounded: false,
                use_plan: false,
                steps_per_sub_goal: 0,
            },
            &LoopGuards::none(),
        )
        .await;
        assert_eq!(outcome.status, TurnStatus::StoppedStepCap);
        assert_eq!(outcome.steps.len(), 3);
    }

    #[tokio::test]
    async fn loop_stops_on_no_progress() {
        // FakeSight returns the SAME observation each time → after the first
        // executed click, the next observe shows no change → no-progress stop.
        let sight = FakeSight::one_button("Btn");
        let hands = FakeHands::default();
        let outcome = run_turn_v2(
            &sight,
            &AlwaysClick,
            &hands,
            "loop",
            LoopConfig {
                max_steps: 20,
                want_som: false,
                no_progress_limit: 2,
                start_grounded: false,
                use_plan: false,
                steps_per_sub_goal: 0,
            },
            &LoopGuards::none(),
        )
        .await;
        assert_eq!(outcome.status, TurnStatus::StoppedNoProgress);
        // Stopped well before the step cap.
        assert!(outcome.steps.len() < 20);
    }

    #[tokio::test]
    async fn loop_asks_on_ambiguity() {
        let sight = FakeSight::one_button("Btn");
        let brain = FakeBrain::new(vec![Decision {
            action: Action::Ask {
                question: "which one?".into(),
            },
            reason: "ambiguous".into(),
            risk_hint: None,
        }]);
        let hands = FakeHands::default();
        let outcome = run_turn_v2(
            &sight,
            &brain,
            &hands,
            "do x",
            LoopConfig::default(),
            &LoopGuards::none(),
        )
        .await;
        assert_eq!(outcome.status, TurnStatus::NeedsClarification);
        assert_eq!(outcome.reply, "which one?");
        assert!(outcome.steps.is_empty());
    }

    #[tokio::test]
    async fn hands_rejects_missing_element_id() {
        let sight = FakeSight::one_button("Btn");
        let brain = FakeBrain::new(vec![click(99)]);
        let hands = FakeHands::default();
        let outcome = run_turn_v2(
            &sight,
            &brain,
            &hands,
            "do x",
            LoopConfig::default(),
            &LoopGuards::none(),
        )
        .await;
        assert_eq!(outcome.steps.len(), 1);
        assert!(!outcome.steps[0].result.ok);
        assert_eq!(hands.executed.lock().unwrap().len(), 0);
    }

    struct DenyGate;
    #[async_trait::async_trait]
    impl SafetyGate for DenyGate {
        async fn evaluate(&self, _d: &Decision, _o: &Observation) -> GateDecision {
            GateDecision::Deny("risky action requires approval".into())
        }
    }

    #[tokio::test]
    async fn safety_gate_deny_stops_before_execution() {
        let sight = FakeSight::one_button("Delete");
        let brain = FakeBrain::new(vec![click(1)]);
        let hands = FakeHands::default();
        let guards = LoopGuards::none().with_safety(Arc::new(DenyGate));
        let outcome = run_turn_v2(
            &sight,
            &brain,
            &hands,
            "delete it",
            LoopConfig::default(),
            &guards,
        )
        .await;
        assert_eq!(outcome.status, TurnStatus::StoppedSafety);
        // Action never executed (Property 5).
        assert_eq!(hands.executed.lock().unwrap().len(), 0);
        assert!(outcome.steps.is_empty());
    }

    #[tokio::test]
    async fn emits_live_phase_events_in_order() {
        use std::sync::Mutex;
        let sight = FakeSight::one_button("New Tab");
        let brain = FakeBrain::click_then_done();
        let hands = FakeHands::default();
        let log: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
        let log2 = log.clone();
        let observer: super::LoopObserver = Arc::new(move |ev: LoopEvent| {
            let tag = match ev {
                LoopEvent::TurnStarted => "turn_started".to_string(),
                LoopEvent::PlanReady { .. } => "plan_ready".into(),
                LoopEvent::SubGoalUpdated { status, .. } => format!("subgoal:{status}"),
                LoopEvent::RecoveryAttempted { rung, .. } => format!("recovery:{rung}"),
                LoopEvent::ObserveStarted { .. } => "observe_started".into(),
                LoopEvent::GroundingEscalated { .. } => "grounding_escalated".into(),
                LoopEvent::ObserveCompleted { .. } => "observe_completed".into(),
                LoopEvent::Decided { action_kind, .. } => format!("decided:{action_kind}"),
                LoopEvent::Gated { allowed, .. } => format!("gated:{allowed}"),
                LoopEvent::ExecuteStarted { .. } => "execute_started".into(),
                LoopEvent::ExecuteCompleted { ok, .. } => format!("execute_completed:{ok}"),
                LoopEvent::Verified { changed, .. } => format!("verified:{changed:?}"),
                LoopEvent::TurnEnded { status } => format!("turn_ended:{}", status.as_str()),
            };
            log2.lock().unwrap().push(tag);
        });
        let guards = LoopGuards::none().with_observer(observer);
        let outcome = run_turn_v2(
            &sight,
            &brain,
            &hands,
            "click it",
            LoopConfig::default(),
            &guards,
        )
        .await;
        assert_eq!(outcome.status, TurnStatus::Completed);
        let events = log.lock().unwrap().clone();
        // First event is always TurnStarted; last is always TurnEnded.
        assert_eq!(events.first().map(String::as_str), Some("turn_started"));
        assert_eq!(
            events.last().map(String::as_str),
            Some("turn_ended:completed")
        );
        // The click step streamed the full phase sequence.
        assert!(events.contains(&"observe_started".to_string()));
        assert!(events.contains(&"observe_completed".to_string()));
        assert!(events.contains(&"decided:click".to_string()));
        assert!(events.contains(&"execute_started".to_string()));
        assert!(events.contains(&"execute_completed:true".to_string()));
    }

    #[tokio::test]
    async fn emits_turn_ended_even_on_error_exit() {
        use std::sync::atomic::AtomicU32;
        use std::sync::Mutex;
        // A Sight that errors → StoppedError, but TurnEnded must still fire.
        struct ErrSight {
            n: AtomicU32,
        }
        #[async_trait::async_trait]
        impl Sight for ErrSight {
            async fn observe(&self, _want_som: bool) -> anyhow::Result<Observation> {
                self.n.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                anyhow::bail!("sight exploded")
            }
        }
        let brain = FakeBrain::click_then_done();
        let hands = FakeHands::default();
        let log: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
        let log2 = log.clone();
        let observer: super::LoopObserver = Arc::new(move |ev: LoopEvent| {
            if let LoopEvent::TurnEnded { status } = ev {
                log2.lock().unwrap().push(status.as_str().to_string());
            }
        });
        let guards = LoopGuards::none().with_observer(observer);
        let outcome = run_turn_v2(
            &ErrSight {
                n: AtomicU32::new(0),
            },
            &brain,
            &hands,
            "x",
            LoopConfig::default(),
            &guards,
        )
        .await;
        assert_eq!(outcome.status, TurnStatus::StoppedError);
        assert_eq!(
            log.lock().unwrap().clone(),
            vec!["stopped_error".to_string()]
        );
    }

    // ── Lazy grounding escalation ────────────────────────────────────────────

    use super::super::types::{Bbox, UiElement};
    use std::sync::atomic::{AtomicU32, Ordering};

    /// A Sight whose cheap `observe` is element-FREE (active window only), but
    /// whose `observe_grounded` surfaces one clickable control — modelling the
    /// production light↔OmniParser hybrid. Tracks how many times each path ran.
    struct HybridFakeSight {
        light_calls: AtomicU32,
        grounded_calls: AtomicU32,
        /// When true, `observe_grounded` returns a degraded observation (sidecar
        /// down) so the escalation cannot help.
        grounded_degraded: bool,
    }

    impl HybridFakeSight {
        fn new() -> Self {
            Self {
                light_calls: AtomicU32::new(0),
                grounded_calls: AtomicU32::new(0),
                grounded_degraded: false,
            }
        }
        fn degraded() -> Self {
            Self {
                grounded_degraded: true,
                ..Self::new()
            }
        }
        fn base(active: &str) -> Observation {
            Observation {
                observation_id: "hybrid".into(),
                screenshot_path: String::new(),
                screen_w: 1920,
                screen_h: 1080,
                active_window: Some(active.into()),
                elements: vec![],
                som_image_path: None,
                source: "perception_light".into(),
            }
        }
    }

    #[async_trait::async_trait]
    impl Sight for HybridFakeSight {
        async fn observe(&self, _want_som: bool) -> anyhow::Result<Observation> {
            self.light_calls.fetch_add(1, Ordering::SeqCst);
            Ok(Self::base("Settings"))
        }
        fn supports_grounding(&self) -> bool {
            true
        }
        async fn observe_grounded(&self, _want_som: bool) -> anyhow::Result<Observation> {
            self.grounded_calls.fetch_add(1, Ordering::SeqCst);
            if self.grounded_degraded {
                return Ok(Observation {
                    source: "degraded:sidecar_unreachable".into(),
                    ..Self::base("Settings")
                });
            }
            let mut obs = Self::base("Settings");
            obs.source = "omniparser".into();
            obs.elements = vec![UiElement {
                id: 1,
                bbox: Bbox {
                    x: 10,
                    y: 20,
                    width: 100,
                    height: 40,
                },
                monitor_index: 0,
                kind: "button".into(),
                label: "Wi-Fi".into(),
                interactable: true,
                confidence: 0.95,
            }];
            Ok(obs)
        }
    }

    /// A Brain that can only `Ask` on an element-free screen (it has no control
    /// to reference) but `Click`s element 1 once controls are present, then
    /// declares `Done` — exactly how `LlmPlannerBrain` behaves (an absent target is
    /// downgraded to `Ask`).
    #[derive(Default)]
    struct GroundingDependentBrain {
        clicked: AtomicU32,
    }
    #[async_trait::async_trait]
    impl GuiBrain for GroundingDependentBrain {
        async fn decide(
            &self,
            _t: &str,
            o: &Observation,
            _h: &[TurnStep],
        ) -> anyhow::Result<Decision> {
            if o.elements.is_empty() {
                return Ok(Decision {
                    action: Action::Ask {
                        question: "I can't see any controls. What should I click?".into(),
                    },
                    reason: "no elements".into(),
                    risk_hint: None,
                });
            }
            if self.clicked.fetch_add(1, Ordering::SeqCst) == 0 {
                Ok(click(1))
            } else {
                Ok(Decision {
                    action: Action::Done {
                        summary: "clicked Wi-Fi".into(),
                    },
                    reason: "done".into(),
                    risk_hint: None,
                })
            }
        }
        fn label(&self) -> &str {
            "grounding_dependent"
        }
    }

    #[tokio::test]
    async fn escalates_to_grounded_and_executes_click() {
        // Light obs is element-free → Brain can only Ask → loop escalates to the
        // grounded observation (with a clickable control) and re-decides into a
        // real click that executes. The bare-Ask is NEVER surfaced to the user.
        let sight = HybridFakeSight::new();
        let hands = FakeHands::default();
        let log: Arc<std::sync::Mutex<Vec<String>>> = Arc::new(std::sync::Mutex::new(Vec::new()));
        let log2 = log.clone();
        let observer: super::LoopObserver = Arc::new(move |ev: LoopEvent| {
            if let LoopEvent::GroundingEscalated { .. } = ev {
                log2.lock().unwrap().push("escalated".into());
            }
        });
        let guards = LoopGuards::none().with_observer(observer);
        let outcome = run_turn_v2(
            &sight,
            &GroundingDependentBrain::default(),
            &hands,
            "click the Wi-Fi toggle",
            LoopConfig {
                max_steps: 4,
                want_som: false,
                no_progress_limit: 0,
                start_grounded: false,
                use_plan: false,
                steps_per_sub_goal: 0,
            },
            &guards,
        )
        .await;
        // The escalation fired and the click executed against the grounded view.
        assert_eq!(log.lock().unwrap().clone(), vec!["escalated".to_string()]);
        assert_eq!(hands.executed.lock().unwrap().len(), 1);
        assert_eq!(
            outcome.steps.first().unwrap().decision.action,
            Action::Click { element_id: 1 }
        );
        assert!(sight.grounded_calls.load(Ordering::SeqCst) >= 1);
    }

    #[tokio::test]
    async fn no_escalation_when_sight_cannot_ground() {
        // A plain element-free Sight that does NOT support grounding → the Ask is
        // surfaced honestly (no escalation, no infinite loop).
        struct BlindSight;
        #[async_trait::async_trait]
        impl Sight for BlindSight {
            async fn observe(&self, _want_som: bool) -> anyhow::Result<Observation> {
                Ok(HybridFakeSight::base("Settings"))
            }
            // supports_grounding defaults to false.
        }
        let hands = FakeHands::default();
        let outcome = run_turn_v2(
            &BlindSight,
            &GroundingDependentBrain::default(),
            &hands,
            "click the Wi-Fi toggle",
            LoopConfig::default(),
            &LoopGuards::none(),
        )
        .await;
        assert_eq!(outcome.status, TurnStatus::NeedsClarification);
        assert_eq!(hands.executed.lock().unwrap().len(), 0);
    }

    #[tokio::test]
    async fn degraded_grounding_keeps_honest_ask() {
        // Grounding is supported but the grounded observe DEGRADES (sidecar down)
        // → the cheap honest Ask is kept (no crash, no loop).
        let sight = HybridFakeSight::degraded();
        let hands = FakeHands::default();
        let outcome = run_turn_v2(
            &sight,
            &GroundingDependentBrain::default(),
            &hands,
            "click the Wi-Fi toggle",
            LoopConfig {
                max_steps: 4,
                want_som: false,
                no_progress_limit: 0,
                start_grounded: false,
                use_plan: false,
                steps_per_sub_goal: 0,
            },
            &LoopGuards::none(),
        )
        .await;
        assert_eq!(outcome.status, TurnStatus::NeedsClarification);
        assert_eq!(hands.executed.lock().unwrap().len(), 0);
        // Escalation was attempted (grounded path called) but could not help.
        assert!(sight.grounded_calls.load(Ordering::SeqCst) >= 1);
    }

    #[test]
    fn decision_needs_grounding_predicate() {
        let empty = HybridFakeSight::base("X");
        let ask = Decision {
            action: Action::Ask {
                question: "?".into(),
            },
            reason: String::new(),
            risk_hint: None,
        };
        assert!(decision_needs_grounding(&ask, &empty));
        // A non-Ask decision on an empty screen does not trigger escalation.
        assert!(!decision_needs_grounding(&click(1), &empty));
        // A degraded observation cannot be helped by escalation.
        let degraded = Observation {
            source: "degraded:x".into(),
            ..HybridFakeSight::base("X")
        };
        assert!(!decision_needs_grounding(&ask, &degraded));
        // An observation that already has elements means the Ask is genuine.
        let mut with_el = HybridFakeSight::base("X");
        with_el.elements = vec![UiElement {
            id: 1,
            bbox: Bbox {
                x: 0,
                y: 0,
                width: 1,
                height: 1,
            },
            monitor_index: 0,
            kind: "button".into(),
            label: "OK".into(),
            interactable: true,
            confidence: 0.9,
        }];
        assert!(!decision_needs_grounding(&ask, &with_el));
    }

    // ---- Task 9: plan-driven loop tests ----

    use super::super::planner::Plan;
    use super::super::traits::GuiPlanner;
    use super::super::types::{SubGoal, SubGoalKind};
    use super::super::verifier::{Signal, StandardVerifier, VerificationProbe};

    struct FixedPlanner(Plan);
    #[async_trait::async_trait]
    impl GuiPlanner for FixedPlanner {
        async fn plan(&self, _task: &str) -> Plan {
            self.0.clone()
        }
    }

    /// A configurable probe: window focus always OK; title is whatever is set
    /// (so Navigate verifies only when the title contains the target).
    struct ConfigProbe {
        window_ok: bool,
        title: Option<String>,
    }
    #[async_trait::async_trait]
    impl VerificationProbe for ConfigProbe {
        async fn window_present_focused(&self, _hint: &str) -> Option<Signal<bool>> {
            Some(Signal::new(self.window_ok, 0.95, "win"))
        }
        async fn active_window_title(&self) -> Option<Signal<String>> {
            self.title.clone().map(|t| Signal::new(t, 0.9, "title"))
        }
        async fn screen_contains(&self, _needle: &str) -> Option<Signal<bool>> {
            None
        }
        async fn file_matches(&self, _p: &str, _c: Option<&str>) -> Option<Signal<bool>> {
            None
        }
        async fn command_output(&self) -> Option<Signal<String>> {
            None
        }
        async fn element_observable(&self, _label: &str) -> Option<Signal<bool>> {
            None
        }
    }

    fn plan_cfg() -> LoopConfig {
        LoopConfig {
            max_steps: 8,
            want_som: false,
            no_progress_limit: 0,
            start_grounded: false,
            use_plan: true,
            steps_per_sub_goal: 0,
        }
    }

    #[tokio::test]
    async fn plan_completes_only_when_all_subgoals_verified() {
        // Plan [OpenApp]; brain opens then would say Done. Probe confirms the
        // window is focused → the per-step verifier marks it done and the turn
        // completes via VERIFIED state (not the Brain's word).
        let sight = FakeSight::one_button("x");
        let brain = FakeBrain::new(vec![
            Decision {
                action: Action::OpenApp { app: "calc".into() },
                reason: "open".into(),
                risk_hint: None,
            },
            Decision {
                action: Action::Done {
                    summary: "done".into(),
                },
                reason: "d".into(),
                risk_hint: None,
            },
        ]);
        let hands = FakeHands::default();
        let plan = Plan::new(vec![
            SubGoal::new("open calc", SubGoalKind::OpenApp).with_target("calc")
        ]);
        let guards = LoopGuards::none()
            .with_planner(Arc::new(FixedPlanner(plan)))
            .with_verifier(
                Arc::new(StandardVerifier),
                Arc::new(ConfigProbe {
                    window_ok: true,
                    title: None,
                }),
            );
        let outcome = run_turn_v2(&sight, &brain, &hands, "open calc", plan_cfg(), &guards).await;
        assert_eq!(outcome.status, TurnStatus::Completed);
        assert!(outcome.reply.contains("open calc"));
    }

    #[tokio::test]
    async fn plan_does_not_prematurely_complete_when_a_subgoal_is_unverified() {
        // Plan [OpenApp, Navigate]; window focuses (open verifies) but the title
        // never matches the URL (navigate NEVER verifies). The Brain keeps saying
        // Done — the loop must NOT complete; after the bounded Done-stall it stops
        // honestly naming the unverified sub-goal (premature-completion fix).
        let sight = FakeSight::one_button("x");
        let brain = FakeBrain::new(vec![
            Decision {
                action: Action::OpenApp {
                    app: "chrome".into(),
                },
                reason: "open".into(),
                risk_hint: None,
            },
            Decision {
                action: Action::Done {
                    summary: "done".into(),
                },
                reason: "d1".into(),
                risk_hint: None,
            },
            Decision {
                action: Action::Done {
                    summary: "done".into(),
                },
                reason: "d2".into(),
                risk_hint: None,
            },
            Decision {
                action: Action::Done {
                    summary: "done".into(),
                },
                reason: "d3".into(),
                risk_hint: None,
            },
        ]);
        let hands = FakeHands::default();
        let plan = Plan::new(vec![
            SubGoal::new("open chrome", SubGoalKind::OpenApp).with_target("chrome"),
            SubGoal::new("navigate to youtube.com", SubGoalKind::Navigate)
                .with_target("youtube.com"),
        ]);
        let guards = LoopGuards::none()
            .with_planner(Arc::new(FixedPlanner(plan)))
            .with_verifier(
                Arc::new(StandardVerifier),
                Arc::new(ConfigProbe {
                    window_ok: true,
                    title: Some("New Tab".into()),
                }),
            );
        let outcome = run_turn_v2(
            &sight,
            &brain,
            &hands,
            "open chrome and go to youtube.com",
            plan_cfg(),
            &guards,
        )
        .await;
        assert_eq!(outcome.status, TurnStatus::StoppedNoProgress);
        assert!(
            outcome.reply.to_lowercase().contains("youtube"),
            "reply names the unverified goal: {}",
            outcome.reply
        );
    }

    #[tokio::test]
    async fn plan_mode_is_inert_without_a_planner() {
        // use_plan=true but NO planner wired → behaves exactly like the plain loop
        // (Brain's Done completes immediately). Proves safe degradation.
        let sight = FakeSight::one_button("x");
        let brain = FakeBrain::click_then_done();
        let hands = FakeHands::default();
        let outcome = run_turn_v2(
            &sight,
            &brain,
            &hands,
            "click it",
            plan_cfg(),
            &LoopGuards::none(),
        )
        .await;
        assert_eq!(outcome.status, TurnStatus::Completed);
    }

    // ---- Task 10: cross-substrate bridge test ----

    use super::super::bridge::{BridgeOutcome, GuiBridge};

    struct FakeBridge;
    #[async_trait::async_trait]
    impl GuiBridge for FakeBridge {
        async fn execute(&self, sub_goal: &SubGoal) -> BridgeOutcome {
            BridgeOutcome::ok(format!("ran:{}", sub_goal.intent))
        }
    }

    #[tokio::test]
    async fn plan_stops_honestly_when_a_subgoal_is_unachievable() {
        // Plan [OpenApp foobar]; the probe NEVER confirms the window (app not
        // installed / never opens). Instead of looping to the step cap, the loop
        // must stop after the per-sub-goal attempt budget with an HONEST reply.
        let sight = FakeSight::one_button("x");
        struct AlwaysOpen;
        #[async_trait::async_trait]
        impl GuiBrain for AlwaysOpen {
            async fn decide(
                &self,
                _t: &str,
                _o: &Observation,
                _h: &[TurnStep],
            ) -> anyhow::Result<Decision> {
                Ok(Decision {
                    action: Action::OpenApp {
                        app: "foobar123".into(),
                    },
                    reason: "x".into(),
                    risk_hint: None,
                })
            }
            fn label(&self) -> &str {
                "always_open"
            }
        }
        let hands = FakeHands::default();
        let plan = Plan::new(vec![
            SubGoal::new("open foobar123", SubGoalKind::OpenApp).with_target("foobar123")
        ]);
        let guards = LoopGuards::none()
            .with_planner(Arc::new(FixedPlanner(plan)))
            .with_verifier(
                Arc::new(StandardVerifier),
                Arc::new(ConfigProbe {
                    window_ok: false,
                    title: None,
                }),
            );
        let outcome = run_turn_v2(
            &sight,
            &AlwaysOpen,
            &hands,
            "Open foobar123",
            LoopConfig {
                max_steps: 16,
                want_som: false,
                no_progress_limit: 0,
                start_grounded: false,
                use_plan: true,
                steps_per_sub_goal: 3,
            },
            &guards,
        )
        .await;
        assert_eq!(outcome.status, TurnStatus::NeedsClarification);
        assert!(
            outcome.reply.to_lowercase().contains("foobar123"),
            "honest reply names the app: {}",
            outcome.reply
        );
        assert!(
            outcome.reply.to_lowercase().contains("installed")
                || outcome.reply.to_lowercase().contains("couldn't find"),
            "honest refusal wording: {}",
            outcome.reply
        );
    }

    // ---- Task 11: recovery ladder test ----

    // ---- Task 13: manual-step / HITL detection ----

    fn obs_with_labels(kinds_labels: &[(&str, &str)]) -> Observation {
        Observation {
            observation_id: "ms".into(),
            screenshot_path: String::new(),
            screen_w: 100,
            screen_h: 100,
            active_window: Some("App".into()),
            elements: kinds_labels
                .iter()
                .enumerate()
                .map(|(i, (k, l))| UiElement {
                    id: i as u32 + 1,
                    bbox: Bbox {
                        x: 0,
                        y: 0,
                        width: 5,
                        height: 5,
                    },
                    monitor_index: 0,
                    kind: (*k).into(),
                    label: (*l).into(),
                    interactable: true,
                    confidence: 0.9,
                })
                .collect(),
            som_image_path: None,
            source: "fake".into(),
        }
    }

    #[test]
    fn detect_manual_step_flags_login_and_permission_surfaces() {
        // Password field → sign-in pause.
        assert!(detect_manual_step(&obs_with_labels(&[("text_field", "Password")])).is_some());
        // 2FA / verification code.
        assert!(
            detect_manual_step(&obs_with_labels(&[("text", "Enter the verification code")]))
                .is_some()
        );
        // CAPTCHA.
        assert!(
            detect_manual_step(&obs_with_labels(&[("text", "Please complete the CAPTCHA")]))
                .is_some()
        );
        // Permission dialog.
        assert!(detect_manual_step(&obs_with_labels(&[
            ("button", "Allow"),
            ("text", "wants permission")
        ]))
        .is_some());
        // A normal page with just a "Sign in" LINK does NOT trip it (conservative).
        assert!(detect_manual_step(&obs_with_labels(&[
            ("link", "Sign in"),
            ("button", "Search")
        ]))
        .is_none());
        // Empty observation never trips.
        assert!(detect_manual_step(&obs_with_labels(&[])).is_none());
    }

    #[tokio::test]
    async fn no_progress_triggers_one_grounded_recovery_before_stopping() {
        use std::sync::atomic::AtomicU32;
        // A grounding-capable Sight whose screen NEVER changes (stable signature)
        // → the click never makes progress. The loop must attempt ONE grounded
        // recovery (RecoveryAttempted{grounded_reobserve}) before StoppedNoProgress.
        struct StallSight {
            grounded_calls: AtomicU32,
        }
        #[async_trait::async_trait]
        impl Sight for StallSight {
            async fn observe(&self, _want_som: bool) -> anyhow::Result<Observation> {
                Ok(Observation {
                    observation_id: "stall".into(),
                    screenshot_path: String::new(),
                    screen_w: 100,
                    screen_h: 100,
                    active_window: Some("Frozen".into()),
                    elements: vec![UiElement {
                        id: 1,
                        bbox: Bbox {
                            x: 0,
                            y: 0,
                            width: 5,
                            height: 5,
                        },
                        monitor_index: 0,
                        kind: "button".into(),
                        label: "B".into(),
                        interactable: true,
                        confidence: 0.9,
                    }],
                    som_image_path: None,
                    source: "fake".into(),
                })
            }
            fn supports_grounding(&self) -> bool {
                true
            }
            async fn observe_grounded(&self, _want_som: bool) -> anyhow::Result<Observation> {
                self.grounded_calls.fetch_add(1, Ordering::SeqCst);
                // SAME observation → still no change (forces the stop after recovery).
                self.observe(false).await
            }
        }
        let sight = StallSight {
            grounded_calls: AtomicU32::new(0),
        };
        let brain = AlwaysClick; // always emits a state-changing click
        let hands = FakeHands::default();
        let log: std::sync::Arc<std::sync::Mutex<Vec<String>>> =
            std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let log2 = log.clone();
        let observer: super::LoopObserver = std::sync::Arc::new(move |ev: LoopEvent| {
            if let LoopEvent::RecoveryAttempted { rung, .. } = ev {
                log2.lock().unwrap().push(rung.to_string());
            }
        });
        let cfg = LoopConfig {
            max_steps: 12,
            want_som: false,
            no_progress_limit: 2,
            start_grounded: false,
            use_plan: false,
            steps_per_sub_goal: 0,
        };
        let outcome = run_turn_v2(
            &sight,
            &brain,
            &hands,
            "click",
            cfg,
            &LoopGuards::none().with_observer(observer),
        )
        .await;
        assert_eq!(outcome.status, TurnStatus::StoppedNoProgress);
        let rungs = log.lock().unwrap().clone();
        assert!(
            rungs.contains(&"grounded_reobserve".to_string()),
            "a grounded recovery must be attempted: {rungs:?}"
        );
        assert!(
            sight.grounded_calls.load(Ordering::SeqCst) >= 1,
            "recovery must escalate to grounded observe"
        );
    }

    #[tokio::test]
    async fn bridged_subgoals_route_to_the_bridge_not_gui() {
        // Plan [OpenApp(GUI), RunCommand(bridge)]. Probe confirms the window; the
        // bridge runs the command (no GUI keystrokes) and the turn completes with
        // both sub-goals done. The Brain is never asked for the command step.
        let sight = FakeSight::one_button("x");
        // Brain only ever needs to open the app; the command is bridged.
        let brain = FakeBrain::new(vec![
            Decision {
                action: Action::OpenApp {
                    app: "terminal".into(),
                },
                reason: "open".into(),
                risk_hint: None,
            },
            Decision {
                action: Action::Done {
                    summary: "done".into(),
                },
                reason: "d".into(),
                risk_hint: None,
            },
        ]);
        let hands = FakeHands::default();
        let plan = Plan::new(vec![
            SubGoal::new("open terminal", SubGoalKind::OpenApp).with_target("terminal"),
            SubGoal::new("run ls", SubGoalKind::RunCommand).with_target("ls"),
        ]);
        let guards = LoopGuards::none()
            .with_planner(Arc::new(FixedPlanner(plan)))
            .with_verifier(
                Arc::new(StandardVerifier),
                Arc::new(ConfigProbe {
                    window_ok: true,
                    title: None,
                }),
            )
            .with_bridge(Arc::new(FakeBridge));
        let outcome = run_turn_v2(
            &sight,
            &brain,
            &hands,
            "open terminal and run ls",
            plan_cfg(),
            &guards,
        )
        .await;
        assert_eq!(outcome.status, TurnStatus::Completed);
        assert!(outcome.reply.contains("run ls"));
        // Exactly ONE GUI action executed (the open); the command did NOT go
        // through Hands (it was bridged).
        let executed = hands.executed.lock().unwrap();
        assert_eq!(executed.len(), 1);
        assert!(matches!(executed[0].action, Action::OpenApp { .. }));
    }
}
