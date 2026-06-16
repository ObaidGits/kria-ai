pub mod audit_ledger;
pub mod browser;
pub mod cancel;
pub mod checkpoint;
pub mod clipboard;
pub mod context;
pub mod event_stream;
pub mod execution_environment;
pub mod executor;
pub mod goal_contract;
pub mod llm_planner;
pub mod perception;
pub mod planner;
pub mod preconditions;
pub mod recovery;
pub mod resolver;
pub mod safety;
pub mod safety_hitl;
pub mod safety_polish;
pub mod turn_budget;
pub mod validator;
pub mod verifier;
pub mod window_focus;
pub mod workflow_runtime;

use uuid::Uuid;

use self::cancel::{evaluate_pre_action_guard, GuiCancelToken, PreActionGuard};
use self::audit_ledger::{GuiActionLedger, GuiActionLedgerRecord};
use self::browser::GuiBrowserConfig;
use self::clipboard::GuiCrossAppConfig;
use self::context::{GuiContext, GuiContextBuildRequest, GuiContextBuilder};
use self::event_stream::{GuiEventStream, GuiEventStreamSink, GuiStreamUxConfig};
use self::execution_environment::GuiExecutionEnvironment;
use self::executor::{
    abs_click_for_target, build_execution_request_from_proposal, gui_abs_pointer_enabled,
    is_password_or_secure_field, physical_bounds_for_target,
    primitive_tier, validate_execution_preconditions, GuiActionBackendStatus, GuiActionExecution,
    GuiActionExecutor, GuiActionKind, GuiActionRequest, GuiExecutionMode,
    GuiExecutionAuthorizationSource, GuiExecutionPreconditionReport, GuiExecutionResult,
    GuiPayloadVault, GuiPrimitivesConfig, GUI_SECRET_FIELD_PLACEHOLDER,
};
use self::goal_contract::{extract_gui_goal_contract, GuiGoalContract};
pub use self::llm_planner::default_idempotent_for;
pub use self::llm_planner::{GuiAutoPrereqConfig, AUTO_PREREQ_ENV_FLAG};
use self::llm_planner::{
    ensure_step_payloads, ensure_step_verification_strategies, parse_llm_plan, plan_summary_json,
    planner_summary_json, typed_plan_steps, validate_llm_plan, validate_plan_for_resolution,
    apply_auto_prerequisite, AppObservability,
    repair_shortcut_steps, shortcut_repair_enabled, backfill_open_app_hints,
    GuiLlmPlan, GuiLlmPlanner, GuiLlmPlannerRequest, GuiPlanValidationReport,
    GuiPlanValidationStatus, GuiPlannerCapability, GuiPlannerHealthSignal, GuiPlannerHealthTracker,
    GuiPlannerMode, GuiPlannerSelection, GuiSmartPlannerConfig, GuiStepCompletenessConfig,
    GuiStructuredPlannerConfig, PlannerCapabilityNotice,
};
use self::perception::{
    app_process_running, collect_observation_with_freshness, control_sample, stable_hash,
    GuiObservationSnapshot,
    GuiPerceptionProvider, GuiProcessProbe, ObservationFreshness, SysinfoProcessProbe,
};
use self::planner::{
    gui_plan_steps, intent_from_goal_contract, GuiCognitionIntent, GuiCognitionIntentKind,
};
use self::recovery::{
    assess_recovery, recovery_blocked_event, should_attempt_recovery, GuiBlocker,
    GuiRecoveryActionKind, GuiRecoveryInput, GuiRecoveryResult, GuiRecoverySignals,
};
use self::resolver::{
    resolve_button, resolve_plan_targets, resolve_type_text_target, resolve_unique_text_field,
    GuiTargetResolutionSummary, TargetResolution,
};
use self::safety::{safety_for_intent, GuiSafetyStatus};use self::safety_hitl::{
    build_action_proposal, decision_from_fixture, evaluate_safety_gate, now_ms,
    GuiActionProposal, GuiHitlDecision, GuiHitlDecisionFixture, GuiSafetyGateResult,
};
use self::safety_polish::{
    ambiguity_no_guess_event, assess_action_boundary, boundary_check_event, is_verify_and_stop_plan,
    recovery_decision_event, verify_and_stop_event, GuiAmbiguityDecisionPoint, GuiBoundaryInput,
};
use self::validator::validate_intent;
use self::verifier::{
    apply_ordered_evidence, apply_verification_contract, select_verification_strategy_with_flag,
    verification_contract_for_with_flag, verify_evidence_enabled, verify_post_action,
    verify_post_action_detailed, verify_post_action_detailed_with_process,
    GuiPostActionVerificationRequest, GuiPostActionVerificationResult, GuiSafetyPolishConfig,
    GuiSecondaryEvidence, GuiVerificationReport, GuiVerifyLiveConfig,
};
use self::workflow_runtime::{
    run_aborted_event, readiness_wait_event, reobserve_hook_event, step_blocked_event,
    step_completed_event, step_started_event, target_presence_event,
    workflow_step_is_state_changing, workflow_step_kind, workflow_step_requires_target,
    GuiWorkflowRun, GuiWorkflowStepKind, GuiWorkflowStepReceipt,
};use self::checkpoint::{
    build_checkpoint, checkpoint_hash, validate_resume, GuiCheckpointPending,
    GuiResumeObservationSignals, GuiWorkflowResumeRequest,
};
use self::turn_budget::{GuiReobserveConfig, GuiRuntimeGuardConfig, GuiTurnBudgetTracker};
use self::preconditions::GuiPreconditionsReport;
use self::window_focus::{
    no_focus_path_message, select_focus_backends, select_window_focus_backend,
    verify_focus_by_reobserve, window_focus_routing_json, GuiWaylandFocusConfig,
    WindowFocusBackend, WindowFocusVerification, WindowIdentity,
};
#[derive(Debug, Clone)]
pub struct GuiTurnRequest {
    pub session_id: String,
    pub turn_id: String,
    pub workflow_id: String,
    pub message: String,
    pub route_path: String,
    pub llm_tool_loop: bool,
    pub hitl_decision_fixture: Option<GuiHitlDecisionFixture>,
    #[doc = "Task 0.3: where this turn is allowed to physically act. Auto-approval \
             (HITL decision) fixtures are honored ONLY when this is a TestSubstrate \
             (Requirement 20.3). Defaults to RealSession for safety."]
    pub execution_environment: GuiExecutionEnvironment,
    pub execution_mode: GuiExecutionMode,
    #[doc = "Step 10: when true, run the multi-step workflow runtime instead of \
             the single-proposal path. Defaults to false to preserve Step 1-9 behavior."]
    pub workflow_enabled: bool,
    #[doc = "Step 11: when set, resume the workflow from this checkpoint instead \
             of starting fresh. The runtime re-observes and revalidates first."]
    pub resume_checkpoint: Option<self::checkpoint::GuiWorkflowCheckpoint>,
    pub resume_reason: Option<String>,
}

#[derive(Debug, Clone)]
pub struct GuiTurnOutcome {
    pub status: String,
    pub reply: String,
    pub response: serde_json::Value,
    pub events: Vec<serde_json::Value>,
}

/// Outcome of a single bounded planner attempt (Task 2.1). Used internally by
/// [`GuiCognitionRuntime::attempt_llm_plan`] so the caller can decide whether
/// to accept the plan, perform the single repair-retry, or fall back.
enum PlanAttempt {
    /// Parsed and passed strict schema validation (Valid or NeedsClarification).
    Accepted {
        plan: GuiLlmPlan,
        validation: GuiPlanValidationReport,
        model: Option<String>,
    },
    /// Strictly rejected: prose/non-object parse error OR validator-blocked.
    /// `blocked_reasons` carries the full validator reason list (empty for a
    /// parse error).
    Rejected {
        reason: String,
        blocked_reasons: Vec<String>,
    },
    /// The planner backend itself failed (unavailable/timeout/provider error).
    ProviderError {
        status: &'static str,
        reason: String,
    },
}

/// Task 3.3: outcome of the bounded readiness wait
/// ([`GuiCognitionRuntime::await_step_readiness`]). The wait re-observes —
/// strictly bounded by the Task 1 caps — until the expected window/app/page is
/// observable, then resolves; or stops without resolving against an un-ready
/// screen.
enum ReadinessOutcome {
    /// The expected window/app/page is observable; the next target may resolve.
    /// (The `WorkflowReadinessWait` event with the attempt count is emitted by
    /// the wait itself before returning.)
    Ready,
    /// A Task 1 runaway cap (max_reobserve / watchdog / flapping) was breached
    /// while waiting — surfaced as a `WorkflowRunAborted` by the caller.
    Aborted(self::turn_budget::BudgetAbort),
    /// Readiness was not reached within the bound; stop safely with a clear,
    /// sanitized reason and do NOT resolve against the un-ready screen.
    NotReady { reason: String },
}

/// Task 3.4 (Requirement 2.3/2.4, Property 2/8): outcome of the
/// present-after-change vs genuinely-absent classification
/// ([`GuiCognitionRuntime::classify_present_or_absent`]). When the per-step
/// target resolution against the FRESH context fails to resolve a required
/// control target, this distinguishes a target that IS observable on the fresh
/// screen (possibly re-identified with a new control_id after a re-render) —
/// which must NOT produce a false "resolved target is no longer present" stop —
/// from one that is genuinely absent. The decision is driven entirely by REAL
/// observation evidence (descriptor matched against the fresh context), never
/// the action kind, and is strictly bounded by the Task 1 caps (no unbounded
/// poll).
enum PresenceResolution {
    /// The expected target is present on the fresh screen AND was re-resolved
    /// against it → CONTINUE with the re-resolved target (the false
    /// "no longer present" stop is eliminated).
    Resolved(Box<GuiTargetResolutionSummary>),
    /// The expected target is present on the fresh screen but still could not be
    /// uniquely/safely resolved within the bound (low confidence) → stop
    /// safely, but NOT with a false "no longer present" reason.
    PresentUnresolved { reason: String },
    /// The expected target is present on the fresh screen but multiple matches
    /// remain → pause and ask (no-guess), never execute.
    Ambiguous { reason: String },
    /// After a bounded readiness wait the expected target is genuinely not
    /// observable on the fresh screen → stop with a clear, sanitized reason.
    GenuinelyAbsent { reason: String },
    /// A Task 1 runaway cap (max_reobserve / watchdog / flapping) tripped while
    /// classifying — surfaced as a `WorkflowRunAborted` by the caller.
    Aborted(self::turn_budget::BudgetAbort),
}

pub struct GuiCognitionRuntime<'a, P, E>
where
    P: GuiPerceptionProvider,
    E: GuiActionExecutor,
{
    perception: &'a P,
    executor: &'a E,
    llm_planner: Option<&'a dyn GuiLlmPlanner>,
    /// Task 0.9 (Requirement 0.9 Rung B): an optional grammar-capable LOCAL
    /// planner used as the ladder's middle rung when the configured planner is
    /// strictly rejected. Only consulted when the `gui_cog_structured_planner`
    /// flag is ON; `None` (the default) keeps the prior two-rung behavior.
    local_grammar_planner: Option<&'a dyn GuiLlmPlanner>,
    runtime_guards: GuiRuntimeGuardConfig,
    cancel_token: Option<GuiCancelToken>,
    smart_planner: GuiSmartPlannerConfig,
    structured_planner: GuiStructuredPlannerConfig,
    reobserve: GuiReobserveConfig,
    wayland_focus: GuiWaylandFocusConfig,
    /// Task 2.1: the `gui_cog_auto_prereq` flag for this turn. Default OFF —
    /// the produced plan is preserved byte-for-byte. When ON, a bare-primitive
    /// plan gets an inferred `OpenApp`/`SwitchWindow` prerequisite prepended (or
    /// an `AskClarification` when no app is inferable).
    auto_prereq: GuiAutoPrereqConfig,
    step_completeness: GuiStepCompletenessConfig,
    primitives: GuiPrimitivesConfig,
    browser: GuiBrowserConfig,
    crossapp: GuiCrossAppConfig,
    safety_polish: GuiSafetyPolishConfig,
    verify_live: GuiVerifyLiveConfig,
    stream_ux: GuiStreamUxConfig,
    event_sink: Option<GuiEventStreamSink>,
    health_tracker: Option<&'a GuiPlannerHealthTracker>,
    /// Issue #2: mockable process-presence probe for OpenApp verification. When
    /// `None` (the default) the live `sysinfo`-backed [`SysinfoProcessProbe`] is
    /// used, but ONLY for OpenApp under the `gui_cog_verify_live` flag; CI/tests
    /// inject a fixed list so they never depend on real processes.
    process_probe: Option<std::sync::Arc<dyn GuiProcessProbe>>,
}

impl<'a, P, E> GuiCognitionRuntime<'a, P, E>
where
    P: GuiPerceptionProvider,
    E: GuiActionExecutor,
{
    pub fn new(perception: &'a P, executor: &'a E) -> Self {
        Self {
            perception,
            executor,
            llm_planner: None,
            local_grammar_planner: None,
            runtime_guards: GuiRuntimeGuardConfig::default(),
            cancel_token: None,
            smart_planner: GuiSmartPlannerConfig::default(),
            structured_planner: GuiStructuredPlannerConfig::default(),
            reobserve: GuiReobserveConfig::default(),
            wayland_focus: GuiWaylandFocusConfig::default(),
            auto_prereq: GuiAutoPrereqConfig::default(),
            step_completeness: GuiStepCompletenessConfig::default(),
            primitives: GuiPrimitivesConfig::default(),
            browser: GuiBrowserConfig::default(),
            crossapp: GuiCrossAppConfig::default(),
            safety_polish: GuiSafetyPolishConfig::default(),
            verify_live: GuiVerifyLiveConfig::default(),
            stream_ux: GuiStreamUxConfig::default(),
            event_sink: None,
            health_tracker: None,
            process_probe: None,
        }
    }

    pub fn with_llm_planner(mut self, planner: Option<&'a dyn GuiLlmPlanner>) -> Self {
        self.llm_planner = planner;
        self
    }

    /// Task 0.9 (Requirement 0.9 Rung B): wire an optional grammar-capable LOCAL
    /// planner as the Capability Ladder's middle rung. When the
    /// `gui_cog_structured_planner` flag is ON and the configured planner's plan
    /// is STILL strictly rejected after its bounded re-ask, the runtime retries
    /// the plan ONCE through this local grammar planner (which posts a real
    /// grammar/json_schema constraint → ~100% schema-valid) and strictly
    /// validates the result. The desktop wiring passes this only when the
    /// configured planner backend is NOT itself grammar-capable AND the local
    /// backend is a DIFFERENT, grammar-capable backend. When `None` (the
    /// default) the ladder collapses to Rung A → Rung C (deterministic),
    /// preserving prior behavior.
    pub fn with_local_grammar_planner(
        mut self,
        planner: Option<&'a dyn GuiLlmPlanner>,
    ) -> Self {
        self.local_grammar_planner = planner;
        self
    }

    /// Task 2.1: configure the `gui_cog_smart_planner` flag for this turn.
    /// Default is OFF (existing single-attempt behavior preserved). When ON, a
    /// first planner attempt that fails strict schema validation triggers
    /// exactly ONE repair-retry (feeding the validation error back) before the
    /// deterministic fallback (Requirement 1.2).
    pub fn with_smart_planner(mut self, smart_planner: GuiSmartPlannerConfig) -> Self {
        self.smart_planner = smart_planner;
        self
    }

    /// Task 0: configure the `gui_cog_structured_planner` flag for this turn.
    /// Default is OFF (existing planner behavior preserved byte-for-byte). When
    /// ON, the shared multi-backend structured-output adapter is used and the
    /// bounded re-ask budget is raised to AT MOST 2 (feeding the validation
    /// error back), still strictly schema-validating and never lenient-scraping
    /// prose (Requirement 0.4). The planner adapter itself must also be
    /// constructed with the matching structured config so it routes through
    /// [`LlmBackend::chat_structured`].
    pub fn with_structured_planner(
        mut self,
        structured_planner: GuiStructuredPlannerConfig,
    ) -> Self {
        self.structured_planner = structured_planner;
        self
    }

    /// Task 2.1: configure the `gui_cog_auto_prereq` flag for this turn. Default
    /// is OFF (the produced plan is preserved byte-for-byte). When ON, after the
    /// plan is selected the runtime runs the auto-prerequisite pass
    /// ([`apply_auto_prerequisite`]): a BARE PRIMITIVE plan (first executable
    /// step is a primitive with no preceding OpenApp/SwitchWindow) whose target
    /// app is not the active window gets an inferred `OpenApp` (app not present)
    /// or `SwitchWindow` (app visible but not focused) prerequisite PREPENDED so
    /// the resolver's prior-app deferral + per-step re-observe resolve the later
    /// primitives against the fresh app context. When no app can be inferred the
    /// plan is replaced with a single `AskClarification` step (never blindly
    /// executed against the wrong context). The prerequisite is built via the
    /// normal step factory — never `allowed_to_execute`, never auto-approved,
    /// verification strategy never weakened. While OFF, none of this runs.
    pub fn with_auto_prereq(mut self, auto_prereq: GuiAutoPrereqConfig) -> Self {
        self.auto_prereq = auto_prereq;
        self
    }

    /// Task 3.1: configure the `gui_cog_reobserve` flag for this turn. Default
    /// is OFF (existing re-observe behavior preserved; only the additive
    /// re-observe-hook instrumentation is gated). When ON, the runtime emits the
    /// explicit per-step re-observe hook ([`reobserve_hook_event`]) that obtains
    /// a FRESH [`GuiContext`] between steps from the desktop-supplied perception
    /// provider (Requirement 2) — the foundation Tasks 3.2–3.4 build on.
    ///
    /// Regardless of this flag, every re-observe is BOUNDED by the Task 1
    /// runaway caps: it flows through [`GuiTurnBudgetTracker::note_reobserve`]
    /// and the `max_reobserve` budget enforced at the loop's pre-action
    /// checkpoint, so re-observe can never run unbounded (Requirement 19.4 /
    /// 21.3, Property 9).
    pub fn with_reobserve(mut self, reobserve: GuiReobserveConfig) -> Self {
        self.reobserve = reobserve;
        self
    }

    /// Task 4.2: configure the `gui_cog_wayland_focus` flag for this turn.
    /// Default is OFF (existing single-path SwitchWindow behavior preserved
    /// byte-for-byte). When ON, a `SwitchWindow` action is routed through the
    /// Wayland-safe [`window_focus`] abstraction: the ordered backend chain is
    /// selected by session ([`select_focus_backends`]), an
    /// activate-by-window-identity path is preferred over a blind Alt+Tab
    /// fallback ([`select_window_focus_backend`]), and the actual
    /// [`backend_used`](executor::GuiExecutionResult::backend_used) is reported
    /// truthfully in the execution result + events (Requirement 3). The
    /// re-observe verification of "active window == requested" is wired by Task
    /// 4.3; the Wave 3 gate (Task 4.5) flips this flag ON for the live/desktop
    /// path. While OFF, none of this code path runs.
    pub fn with_wayland_focus(mut self, wayland_focus: GuiWaylandFocusConfig) -> Self {
        self.wayland_focus = wayland_focus;
        self
    }

    /// Task 5.1: configure the `gui_cog_step_completeness` flag for this turn.
    /// Default is OFF (the produced plan is preserved byte-for-byte). When ON,
    /// the runtime runs a plan post-processing pass
    /// ([`ensure_step_verification_strategies`]) AFTER the plan is selected
    /// (LLM-assisted OR deterministic) that guarantees every typed step carries
    /// a `verification_strategy` VALID for its step type, filling the
    /// type-correct default for any step whose strategy is
    /// missing/empty/incompatible (Requirement 4.2, Property 3). The pass NEVER
    /// assigns a strategy that is invalid for the step type and NEVER turns an
    /// unverifiable/invalid step into a fake-valid one — a step type with no
    /// supported default is left unchanged for the validator to reject. While
    /// OFF, none of this code path runs.
    pub fn with_step_completeness(mut self, step_completeness: GuiStepCompletenessConfig) -> Self {
        self.step_completeness = step_completeness;
        self
    }

    /// Task 6.1: configure the `gui_cog_primitives` flag for this turn. Default
    /// is OFF (the produced action kinds + events are preserved byte-for-byte —
    /// the prior Step 1–12 executor path). When ON, the runtime resolves an
    /// action type to its correct typed primitive
    /// ([`GuiPrimitivesConfig::resolve_action_kind`]) so visible single actions
    /// (clear/select-all/checkbox/dialog-close/in-app-search) route through the
    /// right executor mapping/backend instead of the legacy `ClickControl`
    /// catch-all, and it annotates control actions with DPI/multi-monitor-aware
    /// physical bounds ([`physical_bounds_for_target`]) computed ONLY from the
    /// observed `monitor_layout` + resolved-target logical bounds (never invented
    /// coordinates). Recognized legacy verbs resolve identically to the flag-OFF
    /// path, so the only behavioral change is the previously-unsupported
    /// primitives. The wave gate (Task 6.5) flips this flag ON for the
    /// live/desktop path. While OFF, none of this code path runs.
    pub fn with_primitives(mut self, primitives: GuiPrimitivesConfig) -> Self {
        self.primitives = primitives;
        self
    }

    /// Task 7.1: configure the `gui_cog_browser` flag for this turn. Default is
    /// OFF (the executor / resolver path is preserved byte-for-byte — the prior
    /// Step 1–12 behavior). When ON, browser **chrome-UI** controls (address/URL
    /// bar, tab strip / individual tabs, back/forward, reload/stop, in-page Find
    /// bar) become targetable: [`browser::resolve_browser_chrome_target`] maps a
    /// browser-chrome target hint to the matching REAL observed a11y control
    /// (data-driven from [`GuiContext::fused_controls`], never invented and never
    /// coordinate-based) so the existing resolver can resolve it by role+label
    /// when the active app is a recognized browser. Non-browser apps are
    /// unaffected, and page-content targeting stays out of scope (Task 7.2). The
    /// wave gate (Task 7.5) flips this flag ON for the live/desktop path. While
    /// OFF, none of this code path runs.
    pub fn with_browser(mut self, browser: GuiBrowserConfig) -> Self {
        self.browser = browser;
        self
    }

    /// Task 8.1: configure the `gui_cog_crossapp` flag for this turn. Default is
    /// OFF (the executor / runtime path is preserved byte-for-byte — the prior
    /// Step 1–12 behavior). When ON, cross-app clipboard combos (Task 8.2) use
    /// the clipboard-safe SAVE → USE → RESTORE helper
    /// ([`clipboard::with_clipboard`] / [`clipboard::ClipboardSession`]) with
    /// serialized, process-wide access so the user's existing clipboard contents
    /// are captured before a cross-app operation and restored afterwards — the
    /// clipboard is never clobbered (Requirement 8). Clipboard contents are
    /// treated as opaque and never logged. The wave gate (Task 8.5) flips this
    /// flag ON for the live/desktop path. While OFF, none of this code path runs.
    pub fn with_crossapp(mut self, crossapp: GuiCrossAppConfig) -> Self {
        self.crossapp = crossapp;
        self
    }

    /// Task 8.1: this turn's `gui_cog_crossapp` flag config. Task 8.2's cross-app
    /// clipboard combo reads this to decide whether to route a copy→switch→paste
    /// sequence through the clipboard-safe SAVE → USE → RESTORE helper. While the
    /// flag is OFF (the default) the cross-app path does not run.
    pub fn crossapp_config(&self) -> GuiCrossAppConfig {
        self.crossapp
    }
    /// Task 9.1: configure the `gui_cog_safety_polish` flag for this turn.
    /// Default is OFF (the post-action verification verdict is preserved
    /// byte-for-byte — the prior Step 1–12 / Task 8 behavior). When ON, the
    /// runtime formalizes the per-action-type verification CONTRACT
    /// ([`verification_contract_for`]): the predicate (the action's
    /// verification strategy), the EVIDENCE source used to check it
    /// (accessibility / observation / active-window probe / backend receipt —
    /// never OCR-only or coordinates), a BOUNDED WAIT (reusing this turn's Task
    /// 1 re-observe/verify caps; never an unbounded poll), and a CONFIDENCE bar.
    /// It then applies that contract ([`apply_verification_contract`]) so a weak
    /// or unreliable-evidence `verified` becomes the honest `inconclusive`
    /// verdict (never a false verified), and surfaces the contract as ADDITIVE
    /// telemetry on the action/verification events. The wave gate (Task 9.7)
    /// flips this flag ON for the live/desktop path. While OFF, none of this
    /// code path runs and the verdict is unchanged.
    pub fn with_safety_polish(mut self, safety_polish: GuiSafetyPolishConfig) -> Self {
        self.safety_polish = safety_polish;
        self
    }

    /// Task 9.1: this turn's `gui_cog_safety_polish` flag config.
    pub fn safety_polish_config(&self) -> GuiSafetyPolishConfig {
        self.safety_polish
    }

    /// Phase 1 (Requirement 1): configure the `gui_cog_verify_live` flag for
    /// this turn. Default is OFF (the OpenApp verification predicate stays
    /// `active_window_match` and the verdict is byte-for-byte the prior
    /// behavior). When ON, an `OpenApp` action verifies `window_visible` (the
    /// app's window PRESENT/visible in the desktop open-window set, alias-
    /// tolerant, evidence `observation`/desktop-state) instead of
    /// `active_window_match`, with a BOUNDED readiness wait (this turn's Task 1
    /// re-observe cap; never an unbounded poll) before concluding the verdict.
    /// `SwitchWindow` is never affected (it stays `active_window_match`; a later
    /// phase fixes window activation). The desktop wires `from_env_default_on()`
    /// since prior waves are ON; flag-OFF restores the exact prior verdict.
    pub fn with_verify_live(mut self, verify_live: GuiVerifyLiveConfig) -> Self {
        self.verify_live = verify_live;
        self
    }

    /// Phase 1: this turn's `gui_cog_verify_live` flag config.
    pub fn verify_live_config(&self) -> GuiVerifyLiveConfig {
        self.verify_live
    }

    /// Issue #2: inject a mockable [`GuiProcessProbe`] for OpenApp
    /// process-launched verification. When unset (the default) the live
    /// `sysinfo`-backed [`SysinfoProcessProbe`] is used. The probe is consulted
    /// ONLY for `OpenApp` under the `gui_cog_verify_live` flag, so tests can
    /// inject a fixed running-process list without touching real processes.
    pub fn with_process_probe(
        mut self,
        process_probe: Option<std::sync::Arc<dyn GuiProcessProbe>>,
    ) -> Self {
        self.process_probe = process_probe;
        self
    }

    /// Issue #2: the lowercased names of currently-running processes, from the
    /// injected probe or the live `sysinfo` default. O(processes); called only
    /// for OpenApp verification/readiness under `gui_cog_verify_live`.
    fn running_process_names(&self) -> Vec<String> {
        match &self.process_probe {
            Some(probe) => probe.running_process_names(),
            None => SysinfoProcessProbe.running_process_names(),
        }
    }

    /// Issue #2: the matched binary name when an OpenApp `app_hint`'s PROCESS is
    /// running, or `None`. Returns `None` (no probe cost) unless
    /// `gui_cog_verify_live` is ON, so flag-OFF behavior is byte-for-byte
    /// unchanged and other action types are never affected.
    fn open_app_process_evidence(&self, app_hint: Option<&str>) -> Option<String> {
        if !self.verify_live.is_enabled() {
            return None;
        }
        let hint = app_hint
            .map(str::trim)
            .filter(|value| !value.is_empty())?;
        app_process_running(hint, &self.running_process_names())
    }
    /// Task 10.1: configure the `gui_cog_stream_ux` flag for this turn. Default
    /// is OFF (the runtime emits its `gui_cognition:event` envelopes only as the
    /// end-of-turn batch in [`GuiTurnOutcome::events`] — byte-for-byte unchanged
    /// behavior). When ON **and** a streaming sink is attached
    /// ([`with_event_sink`](Self::with_event_sink)), the runtime ALSO forwards
    /// each envelope through the sink's mpsc channel the moment it is produced
    /// (observe → plan → per-step execute/verify), so the desktop layer can emit
    /// incremental progress to the frontend via the EXISTING `gui_cognition:event`
    /// Tauri event instead of waiting for the batch (Requirement 16, 24). The
    /// streamed sequence is exactly equal to the final batch. While OFF, no sink
    /// is attached regardless of [`with_event_sink`](Self::with_event_sink) and
    /// none of this code path runs. Streaming is passive, append-only telemetry:
    /// it never feeds back into the planner/executor, reorders/drops events, or
    /// alters the turn's control flow — the runtime stays the authoritative
    /// orchestrator. The wave gate (Task 10.7) flips this flag ON.
    pub fn with_stream_ux(mut self, stream_ux: GuiStreamUxConfig) -> Self {
        self.stream_ux = stream_ux;
        self
    }

    /// Task 10.1: attach the streaming sink for this turn. The sink is honored
    /// ONLY when the `gui_cog_stream_ux` flag is ON
    /// ([`with_stream_ux`](Self::with_stream_ux)); while the flag is OFF the sink
    /// is ignored and the end-of-turn batch behavior is preserved exactly. The
    /// runtime forwards each `gui_cognition:event` envelope to this sink as it is
    /// produced, in the same FIFO order as the returned batch.
    pub fn with_event_sink(mut self, event_sink: Option<GuiEventStreamSink>) -> Self {
        self.event_sink = event_sink;
        self
    }

    /// Task 10.1: this turn's `gui_cog_stream_ux` flag config.
    pub fn stream_ux_config(&self) -> GuiStreamUxConfig {
        self.stream_ux
    }

    /// Task 10.1: the streaming sink to attach for this turn — `Some` only when
    /// the `gui_cog_stream_ux` flag is ON and a sink was supplied. While the flag
    /// is OFF this is always `None`, so the runtime buffers to the end-of-turn
    /// batch exactly as before (byte-for-byte unchanged).
    fn stream_sink_for_turn(&self) -> Option<GuiEventStreamSink> {
        if self.stream_ux.is_enabled() {
            self.event_sink.clone()
        } else {
            None
        }
    }
    /// Task 7.1: resolve a browser **chrome-UI** target hint (e.g. "address
    /// bar", "reload", "new tab", "find") against the observed context, honoring
    /// this turn's `gui_cog_browser` flag. Delegates to
    /// [`browser::resolve_browser_chrome_target`]; returns `None` when the flag
    /// is OFF, the active app is not a recognized browser, the hint is not a
    /// chrome control, or no observed control matches (never invents a target).
    pub fn resolve_browser_chrome_target(
        &self,
        context: &GuiContext,
        hint: &str,
    ) -> Option<self::browser::BrowserChromeMatch> {
        self::browser::resolve_browser_chrome_target(&self.browser, context, hint)
    }

    /// Task 7.2 (Requirements 5, 9, 26): classify a browser target hint into its
    /// v1 scope, honoring this turn's `gui_cog_browser` flag. Browser web-page
    /// CONTENT (links/buttons/fields inside the rendered page) is OUT OF SCOPE
    /// for v1; only the Task 7.1 chrome-UI surface is targetable. Delegates to
    /// [`browser::classify_browser_target_scope`]; returns
    /// [`browser::BrowserTargetScope::NotApplicable`] when the flag is OFF or the
    /// active app is not a recognized browser (existing path unaffected).
    pub fn classify_browser_target_scope(
        &self,
        context: &GuiContext,
        hint: &str,
    ) -> self::browser::BrowserTargetScope {
        self::browser::classify_browser_target_scope(&self.browser, context, hint)
    }

    /// Task 7.2 (Requirements 5, 9, 26): the clear, actionable refusal for a
    /// browser target hint that names out-of-scope web-page CONTENT, honoring
    /// this turn's `gui_cog_browser` flag. Returns `Some(message)` only when the
    /// flag is ON, the active app is a recognized browser, and the hint is page
    /// content (not chrome-UI) — the caller MUST refuse rather than guess or act
    /// on an OCR-only page target (Requirement 9 — KRIA never executes from
    /// OCR/visual-only evidence). Returns `None` for chrome-UI hints, non-browser
    /// apps, or when the flag is OFF, so the existing path is unaffected.
    /// Delegates to [`browser::browser_page_content_refusal`].
    pub fn browser_page_content_refusal(
        &self,
        context: &GuiContext,
        hint: &str,
    ) -> Option<String> {
        self::browser::browser_page_content_refusal(&self.browser, context, hint)
    }

    /// Task 7.3 (Requirements 5, 9, 26): summarize the visible OCR/page content
    /// as UNTRUSTED data, honoring this turn's `gui_cog_browser` flag. Reads only
    /// already-sanitized observed OCR/page text and returns a
    /// [`browser::VisibleContentSummary`] tagged untrusted (with the existing OCR
    /// injection markers preserved). The observed text reaches the summary's data
    /// fields ONLY — it is never returned as a plan step, target hint, or action,
    /// so it cannot influence the planner or executor (injection defense). The
    /// planner request construction already EXCLUDES raw OCR/page text from its
    /// instructions. Returns `None` when the flag is OFF, so the existing
    /// summarize path is byte-for-byte unchanged. Delegates to
    /// [`browser::summarize_visible_content_as_data`].
    pub fn summarize_visible_content_as_data(
        &self,
        context: &GuiContext,
    ) -> Option<self::browser::VisibleContentSummary> {
        self::browser::summarize_visible_content_as_data(&self.browser, context)
    }

    /// Task 2.6 (Requirement 1.5): attach a cross-turn
    /// [`GuiPlannerHealthTracker`] so a *persistent* `llm_rejected_fallback` on a
    /// healthy, grammar-capable planner model escalates into a failing
    /// `persistent_defect` health signal (rather than each turn being scored in
    /// isolation as a one-off `defect_suspected`). The caller owns the tracker
    /// across turns; a recovering (completed/repaired) turn resets the streak.
    ///
    /// The escalation only takes effect when the `gui_cog_smart_planner` flag is
    /// ON. While the flag is OFF the prior single-turn behavior is preserved (the
    /// streak is neither consulted nor advanced), so existing Step 1–12 behavior
    /// and the `gui_cog_runtime_guards` path are unaffected.
    pub fn with_health_tracker(mut self, tracker: Option<&'a GuiPlannerHealthTracker>) -> Self {
        self.health_tracker = tracker;
        self
    }

    /// Task 1.2: configure the `gui_cog_runtime_guards` flag + budget for this
    /// turn. Default is OFF (existing Step 1–12 behavior preserved). When ON,
    /// the workflow loop checks the GlobalSafetyHalt kill-switch and the
    /// cooperative cancel token before each action (Requirement 21).
    pub fn with_runtime_guards(mut self, guards: GuiRuntimeGuardConfig) -> Self {
        self.runtime_guards = guards;
        self
    }

    /// Task 1.2: attach a cooperative [`GuiCancelToken`] for this turn so the
    /// UI/API can halt it mid-flight before the next action (Requirement 21.1).
    pub fn with_cancel_token(mut self, cancel_token: Option<GuiCancelToken>) -> Self {
        self.cancel_token = cancel_token;
        self
    }

    pub async fn run_turn(&self, request: GuiTurnRequest) -> GuiTurnOutcome {
        let mut request = request;
        let mut events = GuiEventStream::with_sink(self.stream_sink_for_turn());
        events.push(serde_json::json!({
            "type": "TurnStarted",
            "mode_id": "gui_cognition",
        }));
        events.push(serde_json::json!({
            "type": "RouteConfirmed",
            "path": request.route_path,
            "llm_tool_loop": request.llm_tool_loop,
        }));

        let observation = self.observe_with_events(&mut events).await;
        let context =
            GuiContextBuilder::new().build(GuiContextBuildRequest::new(observation.clone()));
        events.push(context.context_built_event());
        let action_backend = self.executor.action_backend_status().await;
        events.push(action_backend_event(&action_backend));

        // Task 1.4 (Requirement 25): preconditions health-gate evaluated BEFORE
        // any live action executes. Reuses the existing probes — the GUI action
        // backend status (uinput/action-backend availability, focus backend,
        // DISPLAY/session type) and the perception observation (AT-SPI bus). When
        // the `gui_cog_runtime_guards` flag is ON we report readiness and, if a
        // required precondition is missing in ExecuteLive mode, degrade the turn
        // to observe/plan-only (downgrade to SafetyOnly) with a clear, sanitized
        // reason rather than attempting actions that would fail. While the flag
        // is OFF, existing Step 1–12 behavior is preserved (no degrade, no event);
        // the readiness summary is still surfaced in the response (additive).
        let preconditions = GuiPreconditionsReport::evaluate(&action_backend, &context.observation);
        if self.runtime_guards.is_enforced() {
            events.push(preconditions.checked_event());
            if matches!(request.execution_mode, GuiExecutionMode::ExecuteLive)
                && !preconditions.ready
            {
                events.push(preconditions.degraded_event());
                // Degrade to observe/plan-only: never attempt live actions when a
                // required precondition is missing (Requirement 25.2).
                request.execution_mode = GuiExecutionMode::SafetyOnly;
            }
        }
        let preconditions_summary = preconditions.summary_json();

        let lower_message = request.message.to_lowercase();
        let goal_report = extract_gui_goal_contract(&request.message, Some(&context));
        let mut goal_contract = goal_report.contract;
        let mut intent = intent_from_goal_contract(&request.message, &goal_contract, &lower_message);

        // Task 6.2 (Requirement 5/15): when the `gui_cog_primitives` flag is ON
        // and this turn targets a PASSWORD / secure-entry field, any typed
        // payload destined for that field is treated as secret BEFORE any event
        // is emitted: every echoed summary (goal contract → plan steps →
        // proposal → execution) carries a redacted placeholder, never the raw
        // value, while the value-derived hash stays so the secret flag is forced
        // downstream (the vault rejects the placeholder, so no value is ever
        // typed or read back). Detection reads ONLY sanitized signals — the
        // prompt's own secure-field phrasing, the extracted control hint, and the
        // OBSERVED control roles (e.g. an AT-SPI "password text" entry) — never a
        // raw secret. While the flag is OFF this is a no-op and the
        // contract/intent are byte-for-byte unchanged. Resolution-time role
        // detection in `execute_authorized_proposal` is the defense-in-depth
        // complement.
        if self.primitives.is_enabled()
            && (prompt_targets_secure_field(
                &lower_message,
                goal_contract.target_control_hint.as_deref(),
            ) || context_has_secure_text_field(&context))
        {
            goal_contract.redact_secret_payload();
            if intent
                .typed_text
                .as_deref()
                .is_some_and(|value| !value.trim().is_empty())
            {
                intent.typed_text = Some(GUI_SECRET_FIELD_PLACEHOLDER.to_string());
            }
        }

        let deterministic_steps = gui_plan_steps(&intent, &context.observation);
        let plan_id = Uuid::new_v4().to_string();

        // Task 8.2 (Requirements 6, 7, 8): when the `gui_cog_crossapp` flag is ON,
        // recognize a cross-app clipboard COMBO ("copy X from A and paste into B")
        // from the prompt and thread the SOURCE/TARGET app hints onto the goal
        // contract so the deterministic planner emits the complete typed sequence
        // Copy(source) → SwitchWindow(target) → FocusField(target input) → Paste →
        // VerifyState. Each step re-observes via the Task 3 per-step re-observe so
        // the paste targets the target app's REAL focused field after the window
        // switch. Clipboard semantics: a genuine copy→paste combo uses the
        // clipboard for its REAL purpose, so the copied content legitimately
        // remains the post-combo clipboard (no restore); the SAVE→USE→RESTORE
        // helper from Task 8.1 protects a PRE-EXISTING clipboard only for a
        // transient borrow, whose full integration is Task 8.4. While the flag is
        // OFF this is a no-op and the contract/plan are byte-for-byte unchanged.
        if self.crossapp.is_enabled() {
            goal_contract.enrich_cross_app_clipboard(&request.message);
            // Task 8.3 (Requirements 6, 7, 8): also recognize a NON-DESTRUCTIVE
            // file-manager select flow ("open the file manager and select the
            // newest/first file and tell me its name") and thread the file-manager
            // app hint + order/position selection onto the contract so the
            // deterministic planner emits OpenApp → Observe → FocusField(select) →
            // SummarizeVisibleContent. Selecting + reading the name ONLY (no
            // delete/move/rename); the selection resolves against the OBSERVED
            // file entries via per-step re-observe (Task 3), never an invented
            // filename. The detector returns None for any destructive prompt, so
            // a destructive request keeps its safety-gated path. While the flag is
            // OFF this is a no-op and the contract/plan are byte-for-byte unchanged.
            goal_contract.enrich_file_manager_select(&request.message);
        }

        events.push(goal_contract.event_payload());
        let planner_request = GuiLlmPlannerRequest::from_context(
            &goal_contract,
            &context,
            deterministic_steps.clone(),
        );
        let planner_selection = self
            .select_plan_with_optional_llm(&mut events, &planner_request, &intent, &context)
            .await;
        let mut plan_event = plan_summary_json(&plan_id, &planner_selection);
        if let Some(object) = plan_event.as_object_mut() {
            object.insert("type".into(), serde_json::json!("PlanCreated"));
            // Task 6.3 (Requirements 5, 15): when the `gui_cog_primitives` flag is
            // ON, surface each step's GREEN/YELLOW tier (+ its idempotent flag) as
            // an ADDITIVE telemetry field so a primitive's tier is inspectable in
            // events. Only GREEN/YELLOW primitives are annotated; destructive /
            // approval-gated steps (RED/BLACK band) yield no tier and stay
            // governed by the safety/HITL gate. Flag OFF = no field added →
            // PlanCreated output is byte-for-byte unchanged.
            if self.primitives.is_enabled() {
                let primitive_tiers = typed_plan_steps(&planner_selection.plan)
                    .iter()
                    .filter_map(|step| {
                        primitive_tier(&step.step_type).map(|tier| {
                            serde_json::json!({
                                "step_id": step.step_id,
                                "step_type": step.step_type,
                                "tier": tier.as_str(),
                                "idempotent": step.idempotent,
                            })
                        })
                    })
                    .collect::<Vec<_>>();
                object.insert("primitive_tiers".into(), serde_json::json!(primitive_tiers));
            }
        }
        events.push(plan_event);
        let readiness_validation =
            validate_plan_for_resolution(&planner_selection.plan, &planner_request, &plan_id);
        events.push(readiness_validation.event_payload(&plan_id));

        let mut state = RuntimeState::new(gui_observation_reply(&context.observation));
        state.action_backend = Some(action_backend);
        state.preconditions = Some(preconditions_summary);

        if request.workflow_enabled
            && matches!(
                readiness_validation.status,
                GuiPlanValidationStatus::Valid | GuiPlanValidationStatus::ApprovalRequired
            )
        {
            self.run_workflow(
                &mut events,
                &request,
                &context,
                &goal_contract,
                &planner_selection.plan,
                &readiness_validation,
                &plan_id,
                &mut state,
            )
            .await;

            events.push(serde_json::json!({
                "type": "TurnCompleted",
                "status": state.status,
            }));

            let response = self.response_json(
                &request,
                &context,
                &goal_contract,
                &intent,
                &plan_id,
                &planner_selection,
                &readiness_validation,
                &state,
            );

            return GuiTurnOutcome {
                status: state.status,
                reply: state.reply,
                response,
                events: events.into_events(),
            };
        }

        match readiness_validation.status {
            GuiPlanValidationStatus::Valid => {
                self.handle_target_resolution_only(
                    &mut events,
                    &context,
                    &planner_selection.plan,
                    &readiness_validation,
                    &plan_id,
                    &mut state,
                );
            }
            GuiPlanValidationStatus::ApprovalRequired => {
                self.handle_target_resolution_only(
                    &mut events,
                    &context,
                    &planner_selection.plan,
                    &readiness_validation,
                    &plan_id,
                    &mut state,
                );
            }
            GuiPlanValidationStatus::NeedsClarification
            | GuiPlanValidationStatus::Blocked
            | GuiPlanValidationStatus::Rejected => {
                state.status = "blocked".into();
                let reason = readiness_validation
                    .blocked_reasons
                    .first()
                    .cloned()
                    .or_else(|| planner_selection.plan.clarification_question.clone())
                    .unwrap_or_else(|| "Plan validation blocked target resolution.".into());
                let clarification_question = if matches!(
                    readiness_validation.status,
                    GuiPlanValidationStatus::NeedsClarification
                ) {
                    planner_selection
                        .plan
                        .clarification_question
                        .clone()
                        .or_else(|| Some("Which exact visible target should I use?".into()))
                } else {
                    planner_selection.plan.clarification_question.clone()
                };
                state.blocker = Some(GuiBlocker::new("plan_validation", reason.clone()));
                events.push(serde_json::json!({
                    "type": "PlanBlocked",
                    "reason": reason.clone(),
                    "clarification_question": clarification_question.clone(),
                }));
                // Task 9.4 (Requirements 11, 12, 22): ambiguity → ask, never
                // guess. When the plan is under-specified / needs clarification,
                // KRIA pauses and asks rather than picking a target by guessing.
                // The additive `AmbiguityNoGuess` telemetry makes that decision
                // inspectable (candidate count + reason + no-guess flag). Emitted
                // ONLY when the `gui_cog_safety_polish` flag is ON; while OFF the
                // events are byte-for-byte unchanged.
                if self.safety_polish.is_enabled()
                    && matches!(
                        readiness_validation.status,
                        GuiPlanValidationStatus::NeedsClarification
                    )
                {
                    events.push(ambiguity_no_guess_event(
                        planner_selection.plan.ambiguity_count,
                        &reason,
                        GuiAmbiguityDecisionPoint::PlanValidation,
                        clarification_question.as_deref(),
                    ));
                }
                state.reply =
                    "Plan validation blocked target resolution, so I stopped before execution."
                        .into();
                self.handle_target_resolution_only(
                    &mut events,
                    &context,
                    &planner_selection.plan,
                    &readiness_validation,
                    &plan_id,
                    &mut state,
                );
            }
        }

        self.handle_safety_gate(
            &mut events,
            &request,
            &context,
            &goal_contract,
            &planner_selection.plan,
            &readiness_validation,
            &plan_id,
            &mut state,
        )
        .await;

        events.push(serde_json::json!({
            "type": "TurnCompleted",
            "status": state.status,
        }));

        let response = self.response_json(
            &request,
            &context,
            &goal_contract,
            &intent,
            &plan_id,
            &planner_selection,
            &readiness_validation,
            &state,
        );

        GuiTurnOutcome {
            status: state.status,
            reply: state.reply,
            response,
            events: events.into_events(),
        }
    }

    async fn select_plan_with_optional_llm(
        &self,
        events: &mut GuiEventStream,
        request: &GuiLlmPlannerRequest,
        intent: &GuiCognitionIntent,
        context: &GuiContext,
    ) -> GuiPlannerSelection {
        // Task 2.2 (Requirement 1.2/1.5): truthful, always-on planner
        // capability report. This is additive and never gates live behavior on
        // its own — it surfaces whether the configured planner model can do
        // grammar-constrained JSON, so a non-grammar model's deterministic
        // fallback is understood as the expected path (not a defect).
        let capability = self
            .llm_planner
            .map(|planner| planner.capability())
            .unwrap_or_else(GuiPlannerCapability::absent);
        events.push(capability.checked_event());

        let mut selection = self
            .select_plan_inner(events, request, intent, context)
            .await;

        // ── Task 0.9 (Requirement 0.9): Planner Capability Ladder ──────────────
        // Flag-gated: the entire ladder (and its `ladder_rung`/`capability_notice`
        // telemetry) runs ONLY when `gui_cog_structured_planner` is ON. Flag-OFF
        // leaves `ladder_rung`/`capability_notice` as `None`, so the planner
        // summary stays byte-for-byte unchanged.
        if self.structured_planner.is_enabled() {
            self.apply_capability_ladder(events, &capability, &mut selection, request, intent, context)
                .await;
        }

        // Task 2.2/2.6 (Requirement 1.5): truthful health signal. A
        // capability-validated model that still produces llm_rejected_fallback is
        // a *defect suspicion* on a single turn; a *persistent* run across turns
        // is a confirmed defect. When the `gui_cog_smart_planner` flag is ON and a
        // cross-turn tracker is attached, the real consecutive-rejection streak is
        // fed in so persistence surfaces as a failing `persistent_defect` signal
        // (and a recovering turn resets it). While the flag is OFF we preserve the
        // prior single-turn behavior (count = 1) so existing behavior and the
        // runtime-guards path are unaffected — the occurrence is still reported so
        // it is never silently swallowed.
        let consecutive_rejected_fallbacks = if self.smart_planner.is_enabled()
            || self.structured_planner.is_enabled()
        {
            match self.health_tracker {
                Some(tracker) => tracker.record(&selection.mode, &selection.llm_status).max(1),
                None => 1,
            }
        } else {
            1
        };
        let health = GuiPlannerHealthSignal::evaluate(
            &capability,
            &selection.mode,
            &selection.llm_status,
            consecutive_rejected_fallbacks,
        );
        if health.should_report() {
            events.push(health.event_payload());
        }
        selection.capability = Some(capability);
        selection.health_signal = Some(health);

        // Task 5.1 (Requirement 4.2; Property 3): when the
        // `gui_cog_step_completeness` flag is ON, post-process the produced plan
        // so every typed step carries a `verification_strategy` VALID for its
        // step type. This only fills the type-correct default for a step whose
        // strategy is missing/empty/incompatible; it never assigns an invalid
        // strategy nor relaxes the validator. While OFF, the plan is preserved
        // byte-for-byte and this code path does not run.
        if self.step_completeness.is_enabled() {
            // Task 5.2 (Requirement 4.1): ensure payload-bearing steps carry a
            // sanitized payload sourced from the goal contract; when a payload is
            // genuinely missing, convert the step into an `AskClarification` step
            // rather than emitting an invalid/blocked payload step. Runs before
            // the verification-strategy pass so converted clarification steps and
            // sourced payload steps are both well-formed for the validator.
            let payload_outcome =
                ensure_step_payloads(&mut selection.plan, &request.contract);
            if payload_outcome.changed() {
                events.push(serde_json::json!({
                    "type": "PlanStepPayloadCompletenessApplied",
                    "payloads_sourced": payload_outcome.sourced,
                    "steps_converted_to_clarification": payload_outcome.clarified,
                }));
            }
            let filled = ensure_step_verification_strategies(&mut selection.plan);
            if filled > 0 {
                events.push(serde_json::json!({
                    "type": "PlanStepCompletenessApplied",
                    "verification_strategies_filled": filled,
                }));
            }
        }

        // Task 2.1 (Requirement 2): when the `gui_cog_auto_prereq` flag is ON,
        // run the auto-prerequisite pass on the FINALIZED plan against the
        // INITIAL context, ONCE, before any step resolves/executes. A bare
        // primitive plan whose target app is not already the active window gets
        // an inferred OpenApp/SwitchWindow prerequisite prepended (so the
        // resolver's prior-app deferral fires for the later primitives), or — when
        // no app can be inferred — the plan is replaced with a single
        // AskClarification step. While OFF, this code path does not run and the
        // plan is preserved byte-for-byte.
        if self.auto_prereq.is_enabled() {
            let outcome = apply_auto_prerequisite(
                &mut selection.plan,
                &request.contract,
                |app| app_observability(context, app),
            );
            if outcome.changed() {
                events.push(serde_json::json!({
                    "type": "PlanAutoPrerequisiteApplied",
                    "outcome": outcome.as_str(),
                }));
            }
        }

        selection
    }

    /// Task 0.9 (Requirement 0.9): apply the Planner Capability Ladder, recording
    /// which rung produced the final plan and (Task 0.10) emitting the honest
    /// layman capability notice when no LLM rung could plan reliably. Called only
    /// when the `gui_cog_structured_planner` flag is ON.
    ///
    /// * Rung A — the configured planner (already attempted by the caller). If it
    ///   produced a schema-valid LLM plan, record `configured_llm` and stop.
    /// * Rung B — local grammar fallback. If Rung A's plan was strictly REJECTED
    ///   and a grammar-capable LOCAL planner (distinct from the configured
    ///   backend) is wired, retry the plan ONCE through it (reusing the same
    ///   bounded re-ask) and strictly validate. On success, swap in that plan and
    ///   record `local_grammar_fallback`.
    /// * Rung C — deterministic fallback (Rung A's plan stands). Record
    ///   `deterministic`; emit the capability notice ONLY when Rung A was an LLM
    ///   rejection (NOT the expected "no planner configured"/provider-error path).
    async fn apply_capability_ladder(
        &self,
        events: &mut GuiEventStream,
        configured_capability: &GuiPlannerCapability,
        selection: &mut GuiPlannerSelection,
        request: &GuiLlmPlannerRequest,
        intent: &GuiCognitionIntent,
        context: &GuiContext,
    ) {
        use self::llm_planner::ladder_rung;

        // Rung A produced a schema-valid LLM plan → configured_llm, UNLESS the
        // plan is GOAL-ACTION-DEGENERATE: a strictly-valid plan that does NOT
        // pursue the prompt's explicit concrete app action. For an `open_app` /
        // `switch_window` goal with a NAMED target, a "valid" cloud plan that
        // contains no OpenApp/SwitchWindow step (e.g. an Observe-only plan —
        // observed live for "switch to the calculator window") would never act,
        // so it is rerouted to the reliable deterministic plan (which always
        // emits the correct OpenApp/SwitchWindow step). This is NOT lenient
        // scraping and never weakens an honest clarification: it only fires when
        // the goal is a concrete app action with a resolved target and the LLM
        // plan omits that action entirely.
        if matches!(selection.mode, GuiPlannerMode::LlmAssisted) {
            if llm_plan_pursues_goal_action(&selection.plan, request) {
                selection.ladder_rung = Some(ladder_rung::CONFIGURED_LLM.into());
                return;
            }

            // Multi-step preservation (gui_cog_plan_prereq_merge, default ON):
            // before discarding a schema-valid LLM plan that merely OMITS the
            // leading app action, try to REPAIR it by prepending the inferred
            // OpenApp/SwitchWindow prerequisite — the same auto-prerequisite pass
            // that already runs post-selection. If the repaired plan now pursues
            // the goal action, KEEP it: this preserves the LLM's additional steps
            // (e.g. "create a new tab", "go to Wi-Fi options") instead of
            // collapsing to the open-only deterministic plan. Reuses existing
            // machinery; never fabricates steps; falsy env rolls back to the
            // prior discard-and-substitute behavior byte-for-byte.
            if self.auto_prereq.is_enabled() && plan_prereq_merge_enabled() {
                let outcome = apply_auto_prerequisite(
                    &mut selection.plan,
                    &request.contract,
                    |app| app_observability(context, app),
                );
                if outcome.changed() && llm_plan_pursues_goal_action(&selection.plan, request) {
                    events.push(serde_json::json!({
                        "type": "PlannerLadderRungAttempt",
                        "rung": ladder_rung::CONFIGURED_LLM,
                        "reason": "LLM plan repaired with an app-open prerequisite; multi-step plan preserved",
                        "auto_prereq_outcome": outcome.as_str(),
                        "context_id": request.context_id,
                    }));
                    selection.ladder_rung = Some(ladder_rung::CONFIGURED_LLM.into());
                    return;
                }
            }

            events.push(serde_json::json!({
                "type": "PlannerLadderRungAttempt",
                "rung": ladder_rung::DETERMINISTIC,
                "reason": "configured planner plan did not pursue the requested app action (open/switch); using the deterministic plan",
                "context_id": request.context_id,
            }));
            let mut det = GuiPlannerSelection::deterministic_fallback(
                request,
                intent,
                context,
                true,
                "rejected",
                "configured LLM plan did not pursue the requested open/switch app action; \
                 deterministic plan emits the correct action",
            );
            det.ladder_rung = Some(ladder_rung::DETERMINISTIC.into());
            det.capability = selection.capability.clone();
            *selection = det;
            return;
        }

        // Only a strict REJECTION of the configured LLM's plan is a capability
        // shortfall. An expected deterministic path ("no planner configured") or
        // a provider transport error is NOT — those keep `llm_status` of
        // `unavailable`/`failed` rather than starting with `rejected`.
        let configured_llm_rejected =
            matches!(selection.mode, GuiPlannerMode::DeterministicFallback)
                && selection.llm_status.starts_with("rejected");

        // Rung B: local grammar fallback (only on a configured-LLM rejection,
        // and only via a DIFFERENT, genuinely grammar-capable local backend).
        if configured_llm_rejected {
            if let Some(local) = self.local_grammar_planner {
                let local_capability = local.capability();
                let genuine_grammar = local_capability.posts_grammar_constraint();
                let different_backend =
                    local_capability.model_label != configured_capability.model_label;
                if genuine_grammar && different_backend {
                    events.push(serde_json::json!({
                        "type": "PlannerLadderRungAttempt",
                        "rung": ladder_rung::LOCAL_GRAMMAR_FALLBACK,
                        "reason": "configured planner rejected; retrying via grammar-capable local backend",
                        "context_id": request.context_id,
                    }));
                    let local_selection = self
                        .select_plan_with_planner(events, local, request, intent, context)
                        .await;
                    if matches!(local_selection.mode, GuiPlannerMode::LlmAssisted) {
                        // Rung B succeeded with a strictly-validated plan.
                        let mut local_selection = local_selection;
                        local_selection.ladder_rung =
                            Some(ladder_rung::LOCAL_GRAMMAR_FALLBACK.into());
                        *selection = local_selection;
                        return;
                    }
                }
            }
        }

        // Rung C: deterministic fallback (Rung A's plan stands).
        selection.ladder_rung = Some(ladder_rung::DETERMINISTIC.into());

        // Task 0.10: emit the honest layman capability notice ONLY when the
        // deterministic fallback is used BECAUSE no LLM rung could produce a
        // schema-valid plan — the configured LLM was rejected AND Rung B did not
        // (could not) recover. NOT emitted when the deterministic fallback is the
        // EXPECTED path (no planner configured / provider transport error).
        if configured_llm_rejected {
            let notice = PlannerCapabilityNotice::model_not_capable();
            events.push(notice.event_payload());
            selection.capability_notice = Some(notice);
        }
    }

    async fn select_plan_inner(
        &self,
        events: &mut GuiEventStream,
        request: &GuiLlmPlannerRequest,
        intent: &GuiCognitionIntent,
        context: &GuiContext,
    ) -> GuiPlannerSelection {
        let Some(planner) = self.llm_planner else {
            events.push(serde_json::json!({
                "type": "LlmPlanningFailed",
                "status": "unavailable",
                "reason": "LLM planner backend unavailable; deterministic plan used.",
            }));
            return GuiPlannerSelection::deterministic(request, intent, context);
        };

        self.select_plan_with_planner(events, planner, request, intent, context)
            .await
    }

    /// Run the configured re-ask/strict-validate pipeline for an ARBITRARY
    /// planner. Shared by Rung A (the configured planner, via
    /// [`select_plan_inner`](Self::select_plan_inner)) and Rung B (the local
    /// grammar fallback planner, Task 0.9). The bounded re-ask budget is the same
    /// one the structured flag selects (AT MOST 2), so the ladder never loops
    /// unbounded.
    async fn select_plan_with_planner(
        &self,
        events: &mut GuiEventStream,
        planner: &dyn GuiLlmPlanner,
        request: &GuiLlmPlannerRequest,
        intent: &GuiCognitionIntent,
        context: &GuiContext,
    ) -> GuiPlannerSelection {
        events.push(serde_json::json!({
            "type": "LlmPlanningStarted",
            "planner_mode": "llm_schema",
            "context_id": request.context_id,
            "observation_id": request.observation_id,
        }));

        // FIRST attempt: constrained-JSON decode → strict schema-validate.
        match self.attempt_llm_plan(events, planner, request, "first").await {
            PlanAttempt::Accepted {
                plan,
                validation,
                model,
            } => GuiPlannerSelection {
                mode: GuiPlannerMode::LlmAssisted,
                llm_attempted: true,
                llm_status: "completed".into(),
                llm_failure_reason: None,
                raw_model: model,
                plan,
                validation,
                capability: None,
                health_signal: None,
                ladder_rung: None,
                capability_notice: None,
            },
            PlanAttempt::ProviderError { status, reason } => {
                events.push(serde_json::json!({
                    "type": "LlmPlanningFailed",
                    "status": status,
                    "reason": reason,
                }));
                GuiPlannerSelection::deterministic_fallback(
                    request, intent, context, true, status, reason,
                )
            }
            PlanAttempt::Rejected {
                reason,
                blocked_reasons,
            } => {
                // Task 2.1 (Requirement 1.2) + Task 0.4 (Requirement 0.4):
                // bounded re-ask. The budget is:
                //   * 0  when neither flag is on — prior single-attempt behavior
                //         (immediate deterministic fallback, no retry);
                //   * 1  when only `gui_cog_smart_planner` is on — exactly ONE
                //         repair-retry (prior behavior, byte-for-byte);
                //   * 2  when `gui_cog_structured_planner` is on — AT MOST TWO
                //         re-asks, each feeding the strict validation error back.
                // We NEVER lenient-scrape prose and NEVER loop unbounded.
                let reask_budget = if self.structured_planner.is_enabled() {
                    2
                } else if self.smart_planner.is_enabled() {
                    1
                } else {
                    0
                };

                if reask_budget == 0 {
                    events.push(serde_json::json!({
                        "type": "LlmPlanningFailed",
                        "status": "rejected",
                        "reason": reason,
                    }));
                    let mut fallback = GuiPlannerSelection::deterministic_fallback(
                        request, intent, context, true, "rejected", reason,
                    );
                    fallback.validation.warnings.extend(blocked_reasons);
                    return fallback;
                }

                // Bounded re-ask: feed the strict validation error back to the
                // model so it can self-correct. Capped at `reask_budget` extra
                // calls — there is no uncontrolled loop (KRIA runtime authority).
                let mut current_reason = reason;
                let mut attempts_done = 0usize;
                loop {
                    events.push(serde_json::json!({
                        "type": "LlmPlanRepairRetry",
                        "status": "retrying",
                        "reason": current_reason,
                        "context_id": request.context_id,
                        "observation_id": request.observation_id,
                    }));
                    let repair_request =
                        request.clone().with_repair_feedback(current_reason.clone());
                    attempts_done += 1;
                    match self
                        .attempt_llm_plan(events, planner, &repair_request, "repair")
                        .await
                    {
                        PlanAttempt::Accepted {
                            plan,
                            validation,
                            model,
                        } => {
                            return GuiPlannerSelection {
                                mode: GuiPlannerMode::LlmAssisted,
                                llm_attempted: true,
                                llm_status: "repaired".into(),
                                llm_failure_reason: None,
                                raw_model: model,
                                plan,
                                validation,
                                capability: None,
                                health_signal: None,
                                ladder_rung: None,
                                capability_notice: None,
                            };
                        }
                        PlanAttempt::ProviderError {
                            status,
                            reason: repair_reason,
                        } => {
                            events.push(serde_json::json!({
                                "type": "LlmPlanningFailed",
                                "status": status,
                                "reason": repair_reason,
                                "after_repair_retry": true,
                            }));
                            return GuiPlannerSelection::deterministic_fallback(
                                request,
                                intent,
                                context,
                                true,
                                status,
                                repair_reason,
                            );
                        }
                        PlanAttempt::Rejected {
                            reason: repair_reason,
                            blocked_reasons: repair_blocked,
                        } => {
                            if attempts_done < reask_budget {
                                // Budget remains: re-ask once more, feeding the
                                // latest validation error back.
                                current_reason = repair_reason;
                                continue;
                            }
                            // Re-ask budget exhausted → deterministic fallback.
                            // We never attempt another repair and never
                            // lenient-scrape prose.
                            events.push(serde_json::json!({
                                "type": "LlmPlanningFailed",
                                "status": "rejected",
                                "reason": repair_reason,
                                "after_repair_retry": true,
                            }));
                            let mut fallback = GuiPlannerSelection::deterministic_fallback(
                                request,
                                intent,
                                context,
                                true,
                                "rejected_after_repair",
                                repair_reason,
                            );
                            fallback.validation.warnings.extend(repair_blocked);
                            return fallback;
                        }
                    }
                }
            }
        }
    }

    /// Run a single planner attempt: constrained-JSON decode (already enforced
    /// by the backend via `chat_with_grammar` + the plan schema) → parse a JSON
    /// object (prose is rejected, never lenient-scraped) → strict
    /// schema-validate. Emits `LlmPlanningCompleted` on a successful parse. The
    /// returned [`PlanAttempt`] tells the caller whether the plan was accepted,
    /// strictly rejected (parse error OR validator-blocked), or hit a provider
    /// transport error.
    async fn attempt_llm_plan(
        &self,
        events: &mut GuiEventStream,
        planner: &dyn GuiLlmPlanner,
        request: &GuiLlmPlannerRequest,
        attempt_label: &str,
    ) -> PlanAttempt {
        match planner.plan(request.clone()).await {
            Ok(raw) => match parse_llm_plan(&raw.content) {
                Ok(mut plan) => {
                    // Deterministic shortcut-repair (default ON): convert an
                    // ungroundable standard-action click ("new tab", "save",
                    // "reload", ...) into a PressKey carrying the universal
                    // shortcut, so a valid multi-step LLM plan is KEPT instead of
                    // being rejected → falling back to "open app only". Rollback:
                    // `KRIA_GUI_COG_SHORTCUT_REPAIR=0`.
                    let shortcut_repaired = if shortcut_repair_enabled() {
                        repair_shortcut_steps(&mut plan, &request.contract)
                    } else {
                        0
                    };
                    let app_backfilled = backfill_open_app_hints(&mut plan, &request.contract);
                    let _ = app_backfilled;
                    let validation = validate_llm_plan(&plan, request);
                    events.push(serde_json::json!({
                        "type": "LlmPlanningCompleted",
                        "attempt": attempt_label,
                        "status": validation.status.as_str(),
                        "model": raw.model,
                        "confidence": plan.confidence,
                        "step_count": plan.typed_steps.len().max(plan.steps.len()),
                        "risk_level": plan.risk_level,
                        "shortcut_repaired": shortcut_repaired,
                    }));
                    if matches!(validation.status, GuiPlanValidationStatus::Blocked) {
                        let reason = validation
                            .blocked_reasons
                            .first()
                            .cloned()
                            .unwrap_or_else(|| {
                                "LLM plan rejected by deterministic validator.".into()
                            });
                        PlanAttempt::Rejected {
                            reason,
                            blocked_reasons: validation.blocked_reasons,
                        }
                    } else {
                        PlanAttempt::Accepted {
                            plan,
                            validation,
                            model: raw.model,
                        }
                    }
                }
                Err(error) => PlanAttempt::Rejected {
                    reason: sanitize_event_text(&error),
                    blocked_reasons: Vec::new(),
                },
            },
            Err(error) => {
                let reason = error.safe_reason();
                let status = if reason.contains("unavailable") {
                    "unavailable"
                } else {
                    "failed"
                };
                PlanAttempt::ProviderError { status, reason }
            }
        }
    }

    async fn observe_with_events(
        &self,
        events: &mut GuiEventStream,
    ) -> GuiObservationSnapshot {
        // Default freshness: pre-action / non-verification observe (caches OK).
        self.observe_with_events_fresh(events, ObservationFreshness::Default)
            .await
    }

    /// Task 3 (Issue #9): freshness-aware observe. A POST-ACTION re-observe used
    /// for verification passes [`ObservationFreshness::ForceFresh`] so it is a
    /// true fresh capture — never served a pre-action cached observation/OCR/
    /// screenshot frame. The one coherence rule (see
    /// [`perception::collect_observation_with_freshness`]) is enforced here for
    /// ALL verification re-observes, generalizing the Task-2 browser
    /// navigation-wait guarantee. Gated by `gui_cog_cache_coherence` (default
    /// ON); flag-OFF treats `ForceFresh` exactly like `Default` (byte-for-byte).
    async fn observe_with_events_fresh(
        &self,
        events: &mut GuiEventStream,
        freshness: ObservationFreshness,
    ) -> GuiObservationSnapshot {
        events.push(serde_json::json!({
            "type": "ObservationStarted",
            "cache_policy": self.perception.observation_cache_policy(),
            "sources": [
                "get_active_window",
                "get_desktop_state",
                "get_accessibility_capabilities",
                "accessibility_tree_summary",
                "capture_screenshot",
                "ocr",
                "monitor_layout",
                "cursor_focus",
                "find_ui_elements"
            ],
        }));
        let observation = collect_observation_with_freshness(
            self.perception,
            Uuid::new_v4().to_string(),
            Uuid::new_v4().to_string(),
            freshness,
        )
        .await;
        if !observation.has_useful_signal() {
            events.push(serde_json::json!({
                "type": "ObservationBlocked",
                "reason": "no_useful_perception_source",
                "blockers": {
                    "active_window": observation.capabilities.active_window.blocker,
                    "desktop_state": observation.capabilities.desktop_state.blocker,
                    "accessibility": observation.capabilities.accessibility.blocker,
                    "screenshot": observation.capabilities.screenshot.blocker,
                    "ocr": observation.capabilities.ocr.blocker,
                    "monitor": observation.capabilities.monitor.blocker,
                    "cursor_focus": observation.capabilities.cursor_focus.blocker,
                },
            }));
        }
        let source_blockers = source_blockers_json(&observation);
        events.push(observation_completed_event(&observation, source_blockers));
        observation
    }

    /// Task 3.1: per-step re-observe hook. Obtains a FRESH [`GuiContext`]
    /// between steps from the desktop-supplied perception provider
    /// (`self.perception` — the existing [`GuiPerceptionProvider`]) so a combo
    /// acts on the *current* screen rather than the stale initial observation
    /// (Requirement 2.1).
    ///
    /// Re-observe is ALWAYS bounded by the Task 1 runaway caps: every call
    /// records a re-observe on the budget tracker
    /// ([`note_reobserve`](GuiTurnBudgetTracker::note_reobserve) +
    /// `note_screen_hash`), and the loop's pre-action checkpoint aborts with the
    /// `budget_max_reobserve` cause once `max_reobserve` is hit
    /// (Requirement 19.4 / 21.3, Property 9) — so the hook can never drive an
    /// unbounded observation loop.
    ///
    /// The `gui_cog_reobserve` flag (default OFF) gates ONLY the additive
    /// `WorkflowReobserveHook` event; the underlying re-observe + cap accounting
    /// run regardless, so existing behavior and the Task 1 caps are unchanged.
    /// This is the foundation Task 3.2 (resolve the next target against the
    /// fresh context), Task 3.3 (bounded readiness wait), and Task 3.4
    /// (present/absent distinction) build on.
    async fn reobserve_fresh_context(
        &self,
        events: &mut GuiEventStream,
        budget_tracker: &mut GuiTurnBudgetTracker,
        step_index: usize,
        cause: &'static str,
    ) -> GuiContext {
        if self.reobserve.is_enabled() {
            events.push(reobserve_hook_event(
                cause,
                step_index,
                budget_tracker.reobserve_count(),
                budget_tracker.effective_max_reobserve(),
            ));
        }
        let observation = self
            .observe_with_events_fresh(events, ObservationFreshness::ForceFresh)
            .await;
        let context =
            GuiContextBuilder::new().build(GuiContextBuildRequest::new(observation));
        // Bounded by the Task 1 caps: record the re-observe + screen hash so the
        // loop's pre-action checkpoint enforces `max_reobserve` and flapping.
        budget_tracker.note_reobserve();
        budget_tracker.note_screen_hash(context.observation.screen_hash.as_deref());
        context
    }

    /// Task 3.3 (Requirement 2.5, Property 9): bounded readiness wait before the
    /// next step's target is resolved. When the next step depends on a
    /// window/app/page that may still be loading (the previous step changed GUI
    /// state — OpenApp / BrowserNavigate / type / click / navigate / …), the
    /// runtime re-observes until the expected window/app/page becomes observable,
    /// THEN lets resolution proceed. This prevents resolving the next target
    /// against a half-loaded screen (the Blocker #4 "resolved target is no longer
    /// present" failure).
    ///
    /// The wait is STRICTLY BOUNDED — there is NO unbounded polling loop (KRIA
    /// runtime authority invariant):
    /// - Every re-observe flows through the Task 1 budget tracker
    ///   ([`note_reobserve`](GuiTurnBudgetTracker::note_reobserve)); when the
    ///   `gui_cog_runtime_guards` flag is ON, [`evaluate`](GuiTurnBudgetTracker::evaluate)
    ///   trips the existing `budget_max_reobserve` / `budget_watchdog` /
    ///   `flapping` abort and the wait returns [`ReadinessOutcome::Aborted`].
    /// - Independently of that flag, the loop is hard-capped by the Task 1
    ///   re-observe cap ([`effective_max_reobserve`](GuiTurnBudgetTracker::effective_max_reobserve)),
    ///   so it terminates even if budget enforcement is OFF.
    ///
    /// Gated entirely behind `gui_cog_reobserve`: the caller only invokes this
    /// when the flag is ON, so flag-OFF behavior is byte-for-byte preserved.
    /// The already-fresh `context` (from the Task 3.2 pre-resolution re-observe)
    /// is checked first, so a target that is already ready costs ZERO extra
    /// re-observes — only an additive `WorkflowReadinessWait` event.
    /// Issue #2: per-iteration OpenApp readiness via PROCESS presence. A
    /// just-launched app that has no observable window yet (Wayland focus-
    /// stealing prevention + GNOME Eval disabled) is caught when its process is
    /// running. Returns `false` unless the step is `OpenApp` AND
    /// `gui_cog_verify_live` is ON, so non-OpenApp steps and flag-OFF behavior
    /// are unchanged. Bounded: one O(processes) probe per re-observe iteration,
    /// within the existing `max_reobserve` cap (never an unbounded poll).
    fn open_app_step_process_ready(
        &self,
        step: &self::llm_planner::GuiTypedPlanStep,
        expected_hint: Option<&str>,
    ) -> bool {
        if step.step_type != "OpenApp" {
            return false;
        }
        let hint = expected_hint
            .or(step.target_app_hint.as_deref())
            .or(step.target_window_hint.as_deref());
        self.open_app_process_evidence(hint).is_some()
    }

    async fn await_step_readiness(
        &self,
        events: &mut GuiEventStream,
        budget_tracker: &mut GuiTurnBudgetTracker,
        step_index: usize,
        step: &self::llm_planner::GuiTypedPlanStep,
        context: &mut GuiContext,
    ) -> ReadinessOutcome {
        let expected_hint = step
            .target_window_hint
            .as_deref()
            .or(step.target_app_hint.as_deref())
            .map(str::trim)
            .filter(|hint| !hint.is_empty());

        // The hard local ceiling on additional re-observes for THIS wait — the
        // Task 1 re-observe cap. Holds even when `gui_cog_runtime_guards` is OFF
        // (the tracker is inert then), guaranteeing termination (no unbounded
        // poll). The shared `reobserve_count` also bounds the whole turn.
        let max_reobserve = budget_tracker.effective_max_reobserve();
        let mut attempts: u32 = 0;

        loop {
            if step_ready(context, expected_hint)
                || self.open_app_step_process_ready(step, expected_hint)
            {
                events.push(readiness_wait_event(
                    step_index,
                    "readiness_wait",
                    expected_hint,
                    true,
                    attempts,
                    budget_tracker.reobserve_count(),
                    max_reobserve,
                    None,
                ));
                return ReadinessOutcome::Ready;            }

            // Not ready yet. Enforce the Task 1 runaway caps BEFORE spending
            // another re-observe so the wait can never poll unbounded.
            if let Some(abort) = budget_tracker.evaluate() {
                events.push(readiness_wait_event(
                    step_index,
                    "readiness_wait",
                    expected_hint,
                    false,
                    attempts,
                    budget_tracker.reobserve_count(),
                    max_reobserve,
                    Some(&abort.reason),
                ));
                return ReadinessOutcome::Aborted(abort);
            }

            // Hard local bound (independent of the runtime-guards flag): never
            // exceed the Task 1 re-observe cap of additional waits.
            if attempts >= max_reobserve || budget_tracker.reobserve_count() >= max_reobserve {
                let reason = match expected_hint {
                    Some(hint) => format!(
                        "Expected window/app '{}' did not become ready within the bounded re-observe budget ({} of max {}), so I stopped before resolving against an un-ready screen.",
                        sanitize_event_text(hint),
                        budget_tracker.reobserve_count(),
                        max_reobserve
                    ),
                    None => format!(
                        "The screen did not become observable within the bounded re-observe budget ({} of max {}), so I stopped before resolving against an un-ready screen.",
                        budget_tracker.reobserve_count(),
                        max_reobserve
                    ),
                };
                events.push(readiness_wait_event(
                    step_index,
                    "readiness_wait",
                    expected_hint,
                    false,
                    attempts,
                    budget_tracker.reobserve_count(),
                    max_reobserve,
                    Some(&reason),
                ));
                return ReadinessOutcome::NotReady { reason };
            }

            // Re-observe once more (bounded: counts toward `max_reobserve`).
            *context = self
                .reobserve_fresh_context(events, budget_tracker, step_index, "readiness_wait")
                .await;
            attempts = attempts.saturating_add(1);
        }
    }

    /// Task 3.4 (Requirement 2.3/2.4, Property 2/8): the core Blocker #4 fix.
    /// After the per-step target resolution against the FRESH context (Task 3.2)
    /// fails to resolve a REQUIRED control target, distinguish:
    ///
    /// - **present after change** — the expected target/control IS observable on
    ///   the fresh screen (matched by role + label/descriptor, TOLERANT of a
    ///   changed `control_id` after a re-render). The runtime re-resolves against
    ///   the fresh context and, if it resolves, CONTINUES — eliminating the false
    ///   "resolved target is no longer present" stop. If it is present but still
    ///   not uniquely/safely resolvable it stops WITHOUT claiming absence; if
    ///   multiple matches remain it pauses to ask (no-guess).
    /// - **genuinely absent** — after a bounded readiness wait the expected
    ///   target is truly not observable on the fresh screen → STOP with a clear,
    ///   sanitized reason.
    ///
    /// The decision is driven entirely by REAL observation evidence (the
    /// descriptor matched against the fresh context via
    /// [`window_or_app_observable`](self::perception::GuiObservationSnapshot::window_or_app_observable)
    /// / [`control_descriptor_observable`](self::perception::GuiObservationSnapshot::control_descriptor_observable)),
    /// NEVER the action kind (preserves the Task 2.5 invariant). It is STRICTLY
    /// BOUNDED by the Task 1 caps — every re-observe flows through the budget
    /// tracker and the loop is hard-capped by `effective_max_reobserve`, so there
    /// is no unbounded poll (Property 9). Gated entirely behind `gui_cog_reobserve`
    /// (the caller only invokes this when the flag is ON), so flag-OFF behavior is
    /// byte-for-byte preserved.
    #[allow(clippy::too_many_arguments)]
    async fn classify_present_or_absent(
        &self,
        events: &mut GuiEventStream,
        budget_tracker: &mut GuiTurnBudgetTracker,
        step_index: usize,
        step: &self::llm_planner::GuiTypedPlanStep,
        sub_plan: &self::llm_planner::GuiLlmPlan,
        readiness_validation: &GuiPlanValidationReport,
        step_plan_id: &str,
        context: &mut GuiContext,
        state: &mut RuntimeState,
    ) -> PresenceResolution {
        let expected_hint = presence_expected_hint(step);
        let max_reobserve = budget_tracker.effective_max_reobserve();
        let mut attempts: u32 = 0;
        let mut ever_present = false;

        loop {
            if step_target_observable(context, step) {
                ever_present = true;
                // Re-resolve against the FRESH context. The resolver matches by
                // role + label, never identity, so a target re-rendered with a
                // new control_id resolves here rather than being treated as gone.
                let summary = self.resolve_step_target_for_workflow(
                    events,
                    step,
                    sub_plan,
                    readiness_validation,
                    context,
                    step_plan_id,
                    state,
                );
                if summary.status == "resolved" {
                    events.push(target_presence_event(
                        step_index,
                        expected_hint.as_deref(),
                        "present_after_change",
                        true,
                        attempts,
                        budget_tracker.reobserve_count(),
                        max_reobserve,
                        None,
                    ));
                    return PresenceResolution::Resolved(Box::new(summary));
                }
                let ambiguous = summary.status == "ambiguous"
                    || summary.status == "needs_clarification"
                    || summary.ambiguity_count > 0;
                if ambiguous {
                    let reason = summary
                        .ambiguity_reasons
                        .first()
                        .cloned()
                        .unwrap_or_else(|| {
                            "The expected target is present but multiple matches remain, so I paused to ask instead of guessing.".into()
                        });
                    events.push(target_presence_event(
                        step_index,
                        expected_hint.as_deref(),
                        "present_ambiguous",
                        false,
                        attempts,
                        budget_tracker.reobserve_count(),
                        max_reobserve,
                        Some(&reason),
                    ));
                    return PresenceResolution::Ambiguous { reason };
                }
                // Present but not yet uniquely resolvable (e.g. still settling /
                // low confidence). Fall through to a bounded re-observe + retry.
            }

            // Enforce the Task 1 runaway caps BEFORE spending another re-observe.
            if let Some(abort) = budget_tracker.evaluate() {
                events.push(target_presence_event(
                    step_index,
                    expected_hint.as_deref(),
                    if ever_present {
                        "present_unresolved"
                    } else {
                        "genuinely_absent"
                    },
                    false,
                    attempts,
                    budget_tracker.reobserve_count(),
                    max_reobserve,
                    Some(&abort.reason),
                ));
                return PresenceResolution::Aborted(abort);
            }

            // Hard local bound (independent of the runtime-guards flag): never
            // exceed the Task 1 re-observe cap, guaranteeing termination.
            if attempts >= max_reobserve || budget_tracker.reobserve_count() >= max_reobserve {
                if ever_present {
                    let reason = match expected_hint.as_deref() {
                        Some(hint) => format!(
                            "The expected target '{}' is present on the current screen but could not be uniquely and safely resolved within the bounded re-observe budget ({} of max {}), so I stopped without guessing.",
                            sanitize_event_text(hint),
                            budget_tracker.reobserve_count(),
                            max_reobserve
                        ),
                        None => format!(
                            "The expected target is present on the current screen but could not be uniquely and safely resolved within the bounded re-observe budget ({} of max {}), so I stopped without guessing.",
                            budget_tracker.reobserve_count(),
                            max_reobserve
                        ),
                    };
                    events.push(target_presence_event(
                        step_index,
                        expected_hint.as_deref(),
                        "present_unresolved",
                        false,
                        attempts,
                        budget_tracker.reobserve_count(),
                        max_reobserve,
                        Some(&reason),
                    ));
                    return PresenceResolution::PresentUnresolved { reason };
                }
                let reason = match expected_hint.as_deref() {
                    Some(hint) => format!(
                        "The expected target '{}' is not present on the current screen after a bounded re-observe ({} of max {}), so I stopped safely.",
                        sanitize_event_text(hint),
                        budget_tracker.reobserve_count(),
                        max_reobserve
                    ),
                    None => format!(
                        "The expected target is not present on the current screen after a bounded re-observe ({} of max {}), so I stopped safely.",
                        budget_tracker.reobserve_count(),
                        max_reobserve
                    ),
                };
                events.push(target_presence_event(
                    step_index,
                    expected_hint.as_deref(),
                    "genuinely_absent",
                    false,
                    attempts,
                    budget_tracker.reobserve_count(),
                    max_reobserve,
                    Some(&reason),
                ));
                return PresenceResolution::GenuinelyAbsent { reason };
            }

            // Re-observe once more (bounded: counts toward `max_reobserve`).
            *context = self
                .reobserve_fresh_context(events, budget_tracker, step_index, "presence_recheck")
                .await;
            attempts = attempts.saturating_add(1);
        }
    }

    #[allow(dead_code)]
    async fn handle_intent(
        &self,
        events: &mut GuiEventStream,
        context: &GuiContext,
        intent: &GuiCognitionIntent,
        state: &mut RuntimeState,
    ) {
        let validation = validate_intent(intent, context);

        match intent.kind {
            GuiCognitionIntentKind::Observe => {
                state.reply.push_str(
                    " No click, typing, submit, delete, or external action was executed.",
                );
            }
            GuiCognitionIntentKind::AnalyzePlan
            | GuiCognitionIntentKind::BrowserSearchPlan
            | GuiCognitionIntentKind::FillFormPlan
            | GuiCognitionIntentKind::AmbiguityCheck
            | GuiCognitionIntentKind::FocusRecovery => {
                let plan_text = gui_plan_steps(intent, &context.observation)
                    .iter()
                    .enumerate()
                    .map(|(idx, step)| format!("{}. {}", idx + 1, step))
                    .collect::<Vec<_>>()
                    .join(" ");
                state.reply = format!(
                    "{} Planned safely: {} No GUI action was executed in this planning/validation response.",
                    gui_observation_reply(&context.observation),
                    plan_text
                );
            }
            GuiCognitionIntentKind::TargetAvailabilityCheck => {
                state.status = "blocked".into();
                let reason = "No concrete target was provided for resolution, so GUI Cognition cannot safely choose or act.";
                events.push(serde_json::json!({
                    "type": "PlanBlocked",
                    "reason": "missing_or_ambiguous_target",
                    "clarification_question": "Which exact visible target should I use?",
                    "options": control_sample(&context.observation.buttons, 6),
                }));
                state.blocker = Some(
                    GuiBlocker::new("target_resolution", reason).with_candidate_count(
                        context.observation.buttons.len() + context.observation.text_fields.len(),
                    ),
                );
                state.reply =
                    format!("{reason} I stopped safely and did not execute any GUI action.");
            }
            GuiCognitionIntentKind::RiskApproval => {
                self.emit_approval_required(events, intent, state, "This request can affect external state or sensitive data, so GUI Cognition paused before execution.");
                state.reply = format!(
                    "{} Safety gate result: approval required. Reason: {}. I did not execute the risky action.",
                    gui_observation_reply(&context.observation),
                    state.blocker
                        .as_ref()
                        .map(|blocker| blocker.options.join("; "))
                        .unwrap_or_else(|| "approval required".into())
                );
            }
            GuiCognitionIntentKind::FocusInput | GuiCognitionIntentKind::SafeAction => {
                self.handle_focus_intent(events, context, state).await;
            }
            GuiCognitionIntentKind::TypeText => {
                if !validation.reasons.is_empty() {
                    self.handle_type_validation_block(
                        events,
                        intent,
                        state,
                        &validation.reasons[0],
                    );
                } else {
                    self.handle_type_intent(events, context, intent, state)
                        .await;
                }
            }
            GuiCognitionIntentKind::ClickControl => {
                if intent.control_name.is_none() {
                    self.handle_missing_click_target(events, state);
                } else {
                    self.handle_click_intent(events, context, intent, state)
                        .await;
                }
            }
        }
    }

    fn handle_target_resolution_only(
        &self,
        events: &mut GuiEventStream,
        context: &GuiContext,
        plan: &self::llm_planner::GuiLlmPlan,
        readiness_validation: &GuiPlanValidationReport,
        plan_id: &str,
        state: &mut RuntimeState,
    ) {
        events.push(serde_json::json!({
            "type": "TargetResolutionStarted",
            "plan_id": plan_id,
            "validation_id": readiness_validation.validation_id.as_deref(),
            "mode": "step5_target_resolver",
        }));
        let can_resolve_for_approval = matches!(
            readiness_validation.status,
            GuiPlanValidationStatus::ApprovalRequired
        ) || readiness_validation.readiness_status.as_deref() == Some("approval_required");
        let summary = if readiness_validation.can_proceed_to_target_resolution || can_resolve_for_approval {
            resolve_plan_targets(plan, readiness_validation, context, plan_id)
        } else {
            GuiTargetResolutionSummary::skipped(
                plan,
                readiness_validation,
                context,
                plan_id,
                readiness_validation
                    .blocked_reasons
                    .first()
                    .cloned()
                    .unwrap_or_else(|| {
                        "Plan validation did not allow Step 5 target resolution.".into()
                    }),
            )
        };
        events.push(summary.event_payload());
        state.target_resolution = Some(summary.summary_json());
        if let Some(target) = &summary.resolved_target {
            state.target = Some(serde_json::json!({
                "label": target.label,
                "role": target.role,
                "target_type": target.target_kind,
                "control_id": target.control_id,
                "target_hash": target.target_hash,
                "bounds": target.bounds.clone(),
                "confidence": summary.confidence,
                "can_execute": false,
            }));
        }
        match summary.status.as_str() {
            "resolved" => {
                state.status = "ok".into();
                state.reply = format!(
                    "{} Target resolution completed for Step 5. I did not execute any GUI action.",
                    gui_observation_reply(&context.observation)
                );
            }
            "ambiguous" | "needs_clarification" | "blocked" | "rejected" => {
                state.status = "blocked".into();
                let reason = summary
                    .ambiguity_reasons
                    .first()
                    .cloned()
                    .or_else(|| summary.blockers.first().cloned())
                    .unwrap_or_else(|| {
                        "Target resolution did not find a safe unique target.".into()
                    });
                state.blocker = Some(GuiBlocker::new("target_resolution", reason));
                state.reply =
                    "Target resolution stopped before execution because the target is not safely resolved."
                        .into();
            }
            _ => {
                if state.status != "needs_approval" {
                    state.reply =
                        "Target resolution was skipped after plan validation. I did not execute any GUI action."
                            .into();
                }
            }
        }
    }

    async fn handle_safety_gate(
        &self,
        events: &mut GuiEventStream,
        request: &GuiTurnRequest,
        context: &GuiContext,
        goal_contract: &GuiGoalContract,
        plan: &self::llm_planner::GuiLlmPlan,
        readiness_validation: &GuiPlanValidationReport,
        plan_id: &str,
        state: &mut RuntimeState,
    ) {
        let Some(target_resolution_value) = &state.target_resolution else {
            return;
        };
        let target_resolution: GuiTargetResolutionSummary =
            match serde_json::from_value(target_resolution_value.clone()) {
                Ok(summary) => summary,
                Err(_) => return,
            };
        if target_resolution.status == "skipped"
            && !goal_contract.requires_user_approval
            && !matches!(
                readiness_validation.status,
                GuiPlanValidationStatus::ApprovalRequired
            )
        {
            if state.status == "ok" {
                state.reply =
                    "Plan has no action target for Step 6 safety gating. I did not execute any GUI action."
                        .into();
            }
            return;
        }
        events.push(serde_json::json!({
            "type": "SafetyGateStarted",
            "plan_id": plan_id,
            "resolution_id": target_resolution.resolution_id.clone(),
            "mode": "step6_safety_hitl",
            "can_execute": false,
            "prompt_hash": goal_contract.prompt_hash.clone(),
        }));
        let proposal = build_action_proposal(
            &request.session_id,
            &request.workflow_id,
            goal_contract,
            plan_id,
            plan,
            readiness_validation,
            &target_resolution,
            context,
            now_ms(),
        );
        let safety_gate = evaluate_safety_gate(proposal, &target_resolution);
        events.push(safety_gate.event_payload());
        state.safety_gate = Some(safety_gate.summary_json());

        // Task 9.4 (Requirements 11, 12, 15, 22): boundaries strictly respected.
        // Before authorizing/executing, assess whether the bound proposal stays
        // within the requested capability boundary (no destructive verb beyond a
        // non-destructive request, no out-of-scope app). The additive
        // `BoundaryCheck` telemetry records the decision; on a crossing KRIA
        // REFUSES the action and stops. Emitted/enforced ONLY when the
        // `gui_cog_safety_polish` flag is ON; while OFF this is a no-op and the
        // gate behavior is byte-for-byte unchanged. The check never carries a raw
        // prompt, secret, or coordinates.
        if self.safety_polish.is_enabled() {
            let proposed_app = target_resolution
                .resolved_target
                .as_ref()
                .and_then(|target| target.app_hint.as_deref());
            let boundary = assess_action_boundary(&GuiBoundaryInput {
                requested_action_type: goal_contract.action_type.as_str(),
                requested_risk_level: goal_contract.risk_level.as_str(),
                requested_approval: goal_contract.requires_user_approval,
                requested_app: goal_contract.target_app_hint.as_deref(),
                proposed_action_type: &safety_gate.proposal.action_type,
                proposed_risk_level: &safety_gate.proposal.risk_level,
                proposed_app,
                // The resolver/validator already gate unresolved/ambiguous targets
                // (and ambiguity → ask is handled separately), so the gate-time
                // boundary check focuses on the destructive-beyond-scope and
                // out-of-scope-app crossings; it never re-flags an unobserved
                // target here.
                requires_target: false,
                target_resolved: target_resolution.resolved_target.is_some(),
            });
            events.push(boundary_check_event(&boundary));
            if boundary.must_refuse() {
                let reason = boundary
                    .reason
                    .clone()
                    .unwrap_or_else(|| "Proposed action is outside the requested scope.".into());
                state.status = "blocked".into();
                state.blocker = Some(GuiBlocker::new("boundary", reason.clone()));
                state.reply = format!(
                    "{reason} It is outside the requested capability boundary, so I refused it and did not execute any GUI action."
                );
                return;
            }
        }

        match safety_gate.status.as_str() {
            "safe_no_approval_required" => {
                if request.execution_mode.allows_execution() {
                    self.execute_authorized_proposal(
                        events,
                        context,
                        &safety_gate.proposal,
                        &target_resolution,
                        None,
                        GuiExecutionAuthorizationSource::SafeNoApprovalRequired,
                        request.execution_mode,
                        state,
                    )
                    .await;
                } else {
                    state.status = "ok".into();
                    state.reply = format!(
                        "{} Safety gate completed for Step 6. This low-risk proposal is authorized for Step 7 review only; I did not execute any GUI action.",
                        gui_observation_reply(&context.observation)
                    );
                }
            }
            "approval_required" => {
                events.push(safety_gate.hitl_required_event());
                state.status = "needs_approval".into();
                state.blocker = Some(
                    GuiBlocker::new(
                        "approval_required",
                        safety_gate
                            .approval_reason
                            .clone()
                            .unwrap_or_else(|| "GUI action requires approval".into()),
                    )
                    .with_options(safety_gate.proposal.risk_reasons.clone()),
                );
                state.reply = format!(
                    "{} Safety gate paused because approval required. Approval authorizes only the same fresh bound proposal for Step 7; I did not execute any GUI action.",
                    gui_observation_reply(&context.observation)
                );
                // Task 9.3 (Requirements 10, 11, 12, 13, 14, 15, 22, 23): track
                // the approval lifecycle so the `gui_cog_safety_polish` flag can
                // emit additive `ApprovalLifecycle` telemetry below (paused →
                // decision → executed/blocked/gated, with the decision verdict +
                // hash-match/freshness status + carried decision id). These vars
                // are populated but never alter the verdict path — the invariant
                // enforcement is unchanged from the flag-OFF behavior.
                let mut lifecycle_decision: Option<GuiHitlDecision> = None;
                let mut lifecycle_outcome = "gated_awaiting_human";
                let mut lifecycle_executed = false;
                if let Some(fixture) = &request.hitl_decision_fixture {
                    // Task 0.3 / Requirement 20.3: auto-approval fixtures are
                    // rejected outside the test substrate. An *authorizing*
                    // fixture (one that can approve a RED/BLACK action) must NEVER
                    // be honored on the user's real session — otherwise a test
                    // artifact could drive a destructive action without a human.
                    let decision = decision_from_fixture(&safety_gate.proposal, fixture, now_ms());
                    let would_authorize = decision.can_authorize_step7;
                    lifecycle_decision = Some(decision.clone());
                    if would_authorize && !request.execution_environment.allows_auto_approval() {
                        events.push(serde_json::json!({
                            "type": "HitlFixtureRejected",
                            "reason": "auto_approval_requires_test_substrate",
                            "environment": request.execution_environment.label(),
                            "detail": "Auto-approval fixtures are rejected outside the test \
                                       substrate (Requirement 20.3); no GUI action executed.",
                        }));
                        state.hitl_decision = Some(serde_json::json!({
                            "decision": "fixture_rejected",
                            "reason": "auto_approval_requires_test_substrate",
                            "environment": request.execution_environment.label(),
                        }));
                        // Leave status as needs_approval: the action stays gated.
                        lifecycle_outcome = "fixture_rejected_outside_substrate";
                    } else if matches!(
                        decision.decision.as_str(),
                        "stale_rejected" | "hash_mismatch_rejected" | "expired"
                    ) {
                        events.push(decision.invalidated_event_payload());
                        state.hitl_decision = Some(decision.summary_json());
                        lifecycle_outcome = "invalidated";
                    } else {
                        events.push(decision.event_payload());
                        if decision.can_authorize_step7 {
                            state.status = "approved_for_step7".into();
                            if request.execution_mode.allows_execution() {
                                self.execute_authorized_proposal(
                                    events,
                                    context,
                                    &safety_gate.proposal,
                                    &target_resolution,
                                    Some(&decision),
                                    GuiExecutionAuthorizationSource::HitlApproved,
                                    request.execution_mode,
                                    state,
                                )
                                .await;
                                lifecycle_outcome = "executed_on_fresh_approval";
                                lifecycle_executed = true;
                            } else {
                                lifecycle_outcome = "authorized_step7_not_executed";
                            }
                        } else if decision.decision == "denied" {
                            state.status = "blocked".into();
                            lifecycle_outcome = "blocked_denied";
                        } else {
                            lifecycle_outcome = "not_authorized";
                        }
                        state.hitl_decision = Some(decision.summary_json());
                    }
                }
                // Task 9.3: additive `ApprovalLifecycle` telemetry — emitted ONLY
                // when the `gui_cog_safety_polish` flag is ON. It makes the
                // approval lifecycle inspectable (paused → decision →
                // executed/blocked/gated) with the decision verdict, whether the
                // bound proposal/target hashes matched, whether the decision was
                // fresh (not expired/stale), and the carried decision id. While
                // the flag is OFF this is a no-op: events are byte-for-byte
                // unchanged. This event NEVER carries a secret payload, raw
                // prompt, or coordinates and never alters control flow.
                if self.safety_polish.is_enabled() {
                    events.push(approval_lifecycle_event(
                        &safety_gate,
                        lifecycle_decision.as_ref(),
                        lifecycle_outcome,
                        lifecycle_executed,
                        &state.status,
                        request.execution_environment.label(),
                    ));
                }
            }
            "blocked" | "rejected" | "stale" => {
                state.status = "blocked".into();
                let reason = safety_gate
                    .blockers
                    .first()
                    .cloned()
                    .unwrap_or_else(|| "Safety gate blocked this GUI proposal.".into());
                state.blocker = Some(GuiBlocker::new("safety_gate", reason.clone()));
                state.reply = format!("{reason} I did not execute any GUI action.");
            }
            _ => {}
        }
    }

    async fn execute_authorized_proposal(
        &self,
        events: &mut GuiEventStream,
        context: &GuiContext,
        proposal: &GuiActionProposal,
        target_resolution: &GuiTargetResolutionSummary,
        hitl_decision: Option<&GuiHitlDecision>,
        authorization_source: GuiExecutionAuthorizationSource,
        execution_mode: GuiExecutionMode,
        state: &mut RuntimeState,
    ) {
        let Some(backend) = state.action_backend.clone() else {
            return;
        };
        let pre_observation = context.observation.clone();
        let now = now_ms();
        let mut payload_vault = GuiPayloadVault::default();
        let execution_request = build_execution_request_from_proposal(
            proposal,
            target_resolution,
            authorization_source,
            hitl_decision.map(|decision| decision.decision_id.clone()),
            &mut payload_vault,
            now,
        );
        let precondition = validate_execution_preconditions(
            execution_mode,
            &execution_request,
            proposal,
            target_resolution,
            &backend,
            hitl_decision,
            &payload_vault,
            now,
            self.primitives.is_enabled(),
        );
        if !precondition.can_start_action {
            let reason = precondition
                .blockers
                .first()
                .cloned()
                .unwrap_or_else(|| "Execution precondition blocked action.".into());
            let result = GuiExecutionResult {
                execution_id: execution_request.execution_id.clone(),
                proposal_id: execution_request.proposal_id.clone(),
                proposal_hash: execution_request.proposal_hash.clone(),
                action_type: execution_request.action_type.clone(),
                status: if reason.contains("expired") {
                    "stale_rejected".into()
                } else {
                    "blocked".into()
                },
                started_at_ms: now,
                completed_at_ms: now,
                backend_used: backend.selected_backend.clone(),
                precondition_check: precondition,
                postcondition_check: execution_request.expected_postcondition.clone(),
                verification_result: "blocked_before_action".into(),
                error_code: Some("precondition_blocked".into()),
                safe_error_summary: Some(sanitize_event_text(&reason)),
                can_retry: false,
                recovery_hint: Some("Re-observe, re-resolve the target, and request a fresh authorization.".into()),
                prompt_hash: execution_request.prompt_hash.clone(),
            };
            events.push(result.blocked_event_payload(&backend));
            events.push(result.verification_event_payload());
            state.status = "blocked".into();
            state.execution_blocker = Some(result.summary_json());
            state.execution_result = Some(result.summary_json());
            state.blocker = Some(GuiBlocker::new("execution", reason.clone()));
            state.reply = format!("{reason} I did not execute any GUI action.");
            return;
        }

        let action_kind = self
            .primitives
            .resolve_action_kind(&execution_request.action_type);
        let payload_value = match (
            execution_request.text_payload_handle.as_deref(),
            execution_request.text_payload_hash.as_deref(),
        ) {
            (Some(handle), Some(hash)) => payload_vault
                .get(handle, &execution_request.proposal_id, hash, now)
                .map(str::to_string),
            _ => None,
        };
        let is_secret_payload =
            execution_request.text_payload_hash.is_some() && payload_value.is_none();
        // Task 6.2 (Requirement 5/15): force the secret treatment when the
        // RESOLVED target is a password / secure-entry field (role/label
        // descriptor), even if the payload text itself was not recognized as a
        // credential. This selects the secret-safe verification strategy (which
        // never reads the field text back via `text_present`) and keeps the
        // value out of every event/summary/reply. Gated by the
        // `gui_cog_primitives` flag; OFF = unchanged. This is the
        // defense-in-depth complement to the planning-time redaction above.
        let target_is_secure_field = self.primitives.is_enabled()
            && target_resolution
                .resolved_target
                .as_ref()
                .map(|target| is_password_or_secure_field(&target.role, &target.label, false))
                .unwrap_or(false);
        let target_is_secure_field = target_is_secure_field
            || (self.primitives.is_enabled()
                && proposal
                    .target_role
                    .as_deref()
                    .map(|role| is_password_or_secure_field(role, "", false))
                    .unwrap_or(false));
        let is_secret_payload = is_secret_payload || target_is_secure_field;
        let expected_text = payload_value.clone();
        let target_name = proposal
            .target_label
            .clone()
            .or_else(|| proposal.target_control_id.clone())
            .unwrap_or_default();
        let role = proposal
            .target_role
            .clone()
            .unwrap_or_else(|| role_for_action(&action_kind).into());
        // Task 7 (Issue #4): for a ClickControl with TRUSTED logical bounds,
        // compute the normalized absolute-pointer click target from the observed
        // monitor layout so the desktop executor can land the click on a native
        // Wayland window via the uinput EV_ABS path. Gated by the abs-pointer
        // flag; `None` (no bounds / degraded layout / flag OFF) preserves the
        // prior AT-SPI/role click path byte-for-byte (never an invented coord).
        let abs_click = if matches!(action_kind, GuiActionKind::ClickControl)
            && gui_abs_pointer_enabled()
        {
            target_resolution
                .resolved_target
                .as_ref()
                .and_then(|t| t.bounds.as_ref())
                .and_then(|bounds| abs_click_for_target(&context.monitor_layout, bounds, None))
        } else {
            None
        };
        let action_request = GuiActionRequest {
            kind: action_kind.clone(),
            role,
            target_name: target_name.clone(),
            value: payload_value,
            execution_hint: gui_execution_hint_for(&action_kind, &target_name).into(),
            abs_click,
        };

        // Task 4.2 (Requirement 3): when the `gui_cog_wayland_focus` flag is ON,
        // route a SwitchWindow action through the Wayland-safe window-focus
        // abstraction. The ordered backend chain is selected by session and an
        // activate-by-window-identity path is preferred over a blind Alt+Tab
        // fallback; the chosen backend becomes the truthful `backend_used`. The
        // window identity is built ONLY from sanitized resolved-target data
        // (never the raw prompt). While the flag is OFF this is `None` and the
        // existing single-path behavior is preserved byte-for-byte. The
        // re-observe verification of "active window == requested" is Task 4.3, so
        // `verification` stays `NotAttempted` here.
        let window_focus_route = if self.wayland_focus.is_enabled()
            && action_kind == GuiActionKind::SwitchWindow
        {
            let resolved_target = target_resolution.resolved_target.as_ref();
            let identity = WindowIdentity::new(
                resolved_target.and_then(|target| target.app_hint.as_deref()),
                resolved_target
                    .and_then(|target| target.window_hint.as_deref())
                    .or(proposal.target_label.as_deref()),
            );
            let chain = select_focus_backends(&backend.session_type, &backend);
            let flag_on = self.wayland_focus.is_enabled();
            let selected = select_window_focus_backend(&chain, &identity, |candidate| {
                window_focus_backend_available(candidate, &backend, flag_on)
            });
            Some((identity, chain, selected))
        } else {
            None
        };
        let window_focus_backend_used: Option<WindowFocusBackend> = window_focus_route
            .as_ref()
            .and_then(|(_, _, selected)| selected.as_ref().ok().copied());

        // Task 4.3 (Requirement 3.3): when SwitchWindow is routed through the
        // Wayland-safe abstraction but NO viable focus path exists (no window
        // identity / empty backend chain / every eligible backend unavailable),
        // surface a clear, actionable, sanitized reason and STOP — instead of
        // running the legacy deterministic backend, which would emit the generic
        // "wmctrl required" failure. The error is reported in the execution
        // result AND the ActionFailed event (with the routing `window_focus`
        // object). KRIA stays authoritative: a missing substrate yields a
        // truthful failure, never a fabricated success.
        if let Some((identity, chain, Err(focus_err))) = window_focus_route.as_ref() {
            let started_at_ms = now_ms();
            let reason = no_focus_path_message(focus_err, &backend.session_type);
            let focus_json = window_focus_routing_json(
                identity,
                chain,
                None,
                WindowFocusVerification::NotAttempted,
                Some(focus_err),
            );

            let mut action_started = serde_json::json!({
                "type": "ActionStarted",
                "execution_id": execution_request.execution_id,
                "proposal_id": execution_request.proposal_id,
                "proposal_hash": execution_request.proposal_hash,
                "target_hash": execution_request.target_hash,
                "action_kind": execution_request.action_type,
                "target": target_name,
                "backend_used": backend.selected_backend,
                "authorization_source": execution_request.authorization_source.as_str(),
                "prompt_hash": execution_request.prompt_hash,
            });
            if let Some(obj) = action_started.as_object_mut() {
                obj.insert("window_focus".into(), focus_json.clone());
            }
            events.push(action_started);

            let no_path_result = GuiExecutionResult {
                execution_id: execution_request.execution_id.clone(),
                proposal_id: execution_request.proposal_id.clone(),
                proposal_hash: execution_request.proposal_hash.clone(),
                action_type: execution_request.action_type.clone(),
                status: "failed".into(),
                started_at_ms,
                completed_at_ms: now_ms(),
                // No backend acted — report the truthful "unavailable" tag, never
                // a backend that did not run.
                backend_used: "window_focus_unavailable".into(),
                precondition_check: GuiExecutionPreconditionReport::allowed(
                    started_at_ms,
                    Vec::new(),
                ),
                postcondition_check: execution_request.expected_postcondition.clone(),
                verification_result: "failed".into(),
                error_code: Some("window_focus_unavailable".into()),
                safe_error_summary: Some(sanitize_event_text(&reason)),
                can_retry: false,
                recovery_hint: None,
                prompt_hash: execution_request.prompt_hash.clone(),
            };

            let mut action_failed = serde_json::json!({
                "type": "ActionFailed",
                "execution_id": no_path_result.execution_id,
                "proposal_id": no_path_result.proposal_id,
                "proposal_hash": no_path_result.proposal_hash,
                "target_hash": execution_request.target_hash,
                "action_kind": no_path_result.action_type,
                "status": "failed",
                "backend_used": no_path_result.backend_used,
                "safe_error_summary": no_path_result.safe_error_summary,
                "prompt_hash": no_path_result.prompt_hash,
            });
            if let Some(obj) = action_failed.as_object_mut() {
                obj.insert("window_focus".into(), focus_json.clone());
            }
            events.push(action_failed);
            events.push(no_path_result.verification_event_payload());

            let mut result_summary = no_path_result.summary_json();
            if let Some(obj) = result_summary.as_object_mut() {
                obj.insert("window_focus".into(), focus_json);
            }
            state.status = "blocked".into();
            state.reply = reason;
            state.action = Some(result_summary.clone());
            state.execution_result = Some(result_summary);
            return;
        }

        // ActionStarted carries the routing decision; the verify-by-reobserve
        // verdict is unknown until after execution + re-observe, so it is
        // reported as `not_attempted` here (truthful) and resolved below.
        let window_focus_started_json =
            window_focus_route.as_ref().map(|(identity, chain, selected)| {
                window_focus_routing_json(
                    identity,
                    chain,
                    selected.as_ref().ok().copied(),
                    WindowFocusVerification::NotAttempted,
                    selected.as_ref().err(),
                )
            });

        let mut action_started = serde_json::json!({
            "type": "ActionStarted",
            "execution_id": execution_request.execution_id,
            "proposal_id": execution_request.proposal_id,
            "proposal_hash": execution_request.proposal_hash,
            "target_hash": execution_request.target_hash,
            "action_kind": execution_request.action_type,
            "target": target_name,
            "backend_used": backend.selected_backend,
            "authorization_source": execution_request.authorization_source.as_str(),
            "prompt_hash": execution_request.prompt_hash,
        });
        if let Some(focus_json) = window_focus_started_json.clone() {
            if let Some(obj) = action_started.as_object_mut() {
                obj.insert("window_focus".into(), focus_json);
            }
        }
        // Task 6.1 (Requirement 5): when the `gui_cog_primitives` flag is ON and
        // the resolved target carries trusted logical bounds, annotate the
        // ActionStarted event with the DPI/multi-monitor-aware physical bounds
        // for the target monitor. This is derived ONLY from the observed
        // `monitor_layout` + the resolved-target's logical bounds — never an
        // invented coordinate — and is additive (the flag-OFF path emits the
        // event unchanged). The actual coordinate-driven backend wiring is
        // deepened in Task 6.4.
        if self.primitives.is_enabled() {
            if let Some(target) = target_resolution.resolved_target.as_ref() {
                if let Some(bounds) = target.bounds.as_ref() {
                    if let Some(physical) = physical_bounds_for_target(
                        &context.monitor_layout,
                        bounds,
                        None,
                    ) {
                        if let (Some(obj), Ok(value)) = (
                            action_started.as_object_mut(),
                            serde_json::to_value(&physical),
                        ) {
                            obj.insert("bounds_transform".into(), value);
                        }
                    }
                }
            }
            // Task 6.2 (Requirement 5/15): mark a secret/password-field action so
            // the event stream shows the field is secret WITHOUT ever carrying
            // the value — a redacted placeholder stands in for any payload. This
            // is additive and only present when the flag is ON.
            if is_secret_payload {
                if let Some(obj) = action_started.as_object_mut() {
                    obj.insert("secret_field".into(), serde_json::json!(true));
                    obj.insert(
                        "field_value".into(),
                        serde_json::json!(GUI_SECRET_FIELD_PLACEHOLDER),
                    );
                }
            }
        }
        events.push(action_started);

        let started_at_ms = now_ms();
        let execution = self.executor.execute(action_request).await;
        let completed_at_ms = now_ms();
        // Task 3 (Issue #9): the post-action observe is a verification re-observe
        // — force a FRESH capture so it can NEVER be served the pre-action cached
        // observation/OCR/screenshot frame (the stale-frame-across-an-action-
        // boundary bug). Flag-gated (`gui_cog_cache_coherence`, default ON).
        let mut post_observation = self
            .observe_with_events_fresh(events, ObservationFreshness::ForceFresh)
            .await;

        // Phase 1 (Requirement 1.2/1.3): bounded readiness wait for an OpenApp
        // window to appear. On Wayland a freshly launched app's window maps
        // asynchronously and may not be present in the very first post-action
        // observation. When `gui_cog_verify_live` is ON, re-observe up to this
        // turn's Task 1 re-observe cap (NEVER an unbounded poll) until the
        // launched app's window is present/visible in the desktop window set,
        // then conclude the verdict against that fresh observation. While the
        // flag is OFF this loop never runs and behavior is byte-for-byte
        // unchanged. SwitchWindow is never affected here.
        if self.verify_live.is_enabled()
            && matches!(action_kind, GuiActionKind::OpenApp)
            && execution.success
        {
            let open_app_hint = target_resolution
                .resolved_target
                .as_ref()
                .and_then(|target| {
                    target
                        .window_hint
                        .clone()
                        .or_else(|| target.app_hint.clone())
                })
                .or_else(|| proposal.target_label.clone());
            if let Some(hint) = open_app_hint
                .as_deref()
                .map(str::trim)
                .filter(|value| value.len() >= 2)
            {
                // Hard local bound: never exceed this turn's Task 1 re-observe
                // cap, guaranteeing termination even with no display.
                let max_reobserve = self.runtime_guards.budget.effective_max_reobserve();
                let mut attempts: u32 = 0;
                while attempts < max_reobserve
                    && !post_observation.window_visible_for_app(hint)
                {
                    events.push(readiness_wait_event(
                        0,
                        "open_app_readiness_wait",
                        Some(hint),
                        false,
                        attempts,
                        attempts,
                        max_reobserve,
                        None,
                    ));
                    post_observation = self
                        .observe_with_events_fresh(events, ObservationFreshness::ForceFresh)
                        .await;
                    attempts = attempts.saturating_add(1);
                }
                let ready = post_observation.window_visible_for_app(hint);
                events.push(readiness_wait_event(
                    0,
                    "open_app_readiness_wait",
                    Some(hint),
                    ready,
                    attempts,
                    attempts,
                    max_reobserve,
                    if ready {
                        None
                    } else {
                        Some("app window did not become present within the bounded re-observe budget")
                    },
                ));
            }
        }

        // Task 2 (Issue #3): the browser address-bar action (Ctrl+L + type +
        // Enter, executed atomically) NAVIGATES the browser. The very first
        // post-action observation is typically captured BEFORE the page finishes
        // loading, so the screen looks unchanged. Re-observe (bounded by this
        // turn's Task 1 re-observe cap — never an unbounded poll) until the screen
        // actually changes vs the pre-action frame (navigation rendered) or the
        // active window title updates, then conclude the verdict against that
        // fresh observation. Recognized by the sentinel target label set in
        // `build_action_proposal_for_step`.
        let is_browser_addressbar = matches!(
            action_kind,
            GuiActionKind::TypeText | GuiActionKind::FillField
        ) && proposal.target_label.as_deref()
            == Some(self::llm_planner::BROWSER_ADDRESSBAR_HINT);
        if is_browser_addressbar && execution.success {
            let max_reobserve = self.runtime_guards.budget.effective_max_reobserve();
            let pre_screen = pre_observation.screen_hash.clone();
            let pre_window = pre_observation.active_window_label.clone();
            let mut attempts: u32 = 0;
            while attempts < max_reobserve
                && post_observation.screen_hash == pre_screen
                && post_observation.active_window_label == pre_window
            {
                events.push(readiness_wait_event(
                    0,
                    "browser_navigation_wait",
                    None,
                    false,
                    attempts,
                    attempts,
                    max_reobserve,
                    None,
                ));
                tokio::time::sleep(std::time::Duration::from_millis(400)).await;
                post_observation = self
                    .observe_with_events_fresh(events, ObservationFreshness::ForceFresh)
                    .await;
                attempts = attempts.saturating_add(1);
            }
        }

        // Task 4.3 (Requirement 3.4): verify SwitchWindow by RE-OBSERVING that the
        // active window matches the requested identity. This is a single bounded
        // re-observe (`post_observation` above — no unbounded poll, Property 9);
        // the verdict is truthful (Verified / Failed / Inconclusive) and the
        // Alt+Tab fallback is decided by this re-observe, never trusted blindly
        // (Requirement 3.2). When the fresh active-window probe is unreliable the
        // verdict is `Inconclusive`, never a false `Verified`.
        let window_focus_verification: Option<WindowFocusVerification> = window_focus_route
            .as_ref()
            .and_then(|(identity, _, selected)| {
                selected.as_ref().ok().map(|_backend| {
                    let observed_active_label = if post_observation.active_window_probe_ok {
                        post_observation
                            .active_window
                            .app_name
                            .as_deref()
                            .map(str::trim)
                            .filter(|value| !value.is_empty() && *value != "unknown")
                            .or_else(|| {
                                let label = post_observation.active_window_label.trim();
                                if label.is_empty() || label == "unknown" {
                                    None
                                } else {
                                    Some(label)
                                }
                            })
                    } else {
                        None
                    };
                    verify_focus_by_reobserve(
                        identity,
                        observed_active_label,
                        post_observation.active_window_probe_ok,
                    )
                })
            });

        // Final routing JSON carries the resolved verify-by-reobserve verdict for
        // the ActionCompleted/ActionFailed events + the execution result summary.
        let window_focus_final_json =
            window_focus_route.as_ref().map(|(identity, chain, selected)| {
                window_focus_routing_json(
                    identity,
                    chain,
                    selected.as_ref().ok().copied(),
                    window_focus_verification.unwrap_or(WindowFocusVerification::NotAttempted),
                    selected.as_ref().err(),
                )
            });

        let resolved = target_resolution.resolved_target.as_ref();
        let verification_strategy = if is_browser_addressbar {
            // The address-bar type+navigate cannot be verified by `text_present`
            // (Chrome a11y is off on Wayland, so the typed text is not readable).
            // The honest, observable signal is the screen change from navigation
            // (the bounded wait above captured it).
            self::verifier::GuiVerificationStrategy::ScreenChanged
        } else {
            select_verification_strategy_with_flag(
                &action_kind,
                is_secret_payload,
                self.verify_live.is_enabled(),
            )
        };
        let verification_request = GuiPostActionVerificationRequest {
            verification_id: format!(
                "verification-{}",
                stable_hash(&format!(
                    "{}|{}|{}",
                    execution_request.execution_id, execution_request.proposal_hash, completed_at_ms
                ))
            ),
            execution_id: execution_request.execution_id.clone(),
            proposal_id: execution_request.proposal_id.clone(),
            proposal_hash: execution_request.proposal_hash.clone(),
            action_type: execution_request.action_type.clone(),
            target_hash: execution_request.target_hash.clone(),
            stable_target_identity_hash: execution_request.stable_target_identity_hash.clone(),
            expected_postcondition: execution_request.expected_postcondition.clone(),
            verification_strategy: verification_strategy.as_str().into(),
            pre_action_context_id: pre_observation.context_id.clone(),
            post_action_observation_id: post_observation.observation_id.clone(),
            post_action_context_id: post_observation.context_id.clone(),
            started_at_ms,
            is_secret_payload,
            prompt_hash: execution_request.prompt_hash.clone(),
            target_label: proposal.target_label.clone(),
            target_role: proposal.target_role.clone(),
            target_control_id: proposal.target_control_id.clone(),
            expected_app_hint: resolved.and_then(|target| target.app_hint.clone()),
            expected_window_hint: resolved.and_then(|target| target.window_hint.clone()),
        };
        // Issue #2: OpenApp process-launched evidence. On Wayland/GNOME the
        // launched app may have NO observable window (focus-stealing prevention +
        // GNOME Eval disabled => no usable window list), so confirm "the app
        // opened" by its running process as an ADDITIONAL evidence source. Only
        // for OpenApp; `None` for every other action. Hoisted so the Task-4
        // ordered-evidence step can reuse it as the `Process` fallback signal.
        let open_app_process_evidence = if matches!(action_kind, GuiActionKind::OpenApp) {
            self.open_app_process_evidence(
                verification_request
                    .expected_window_hint
                    .as_deref()
                    .or(verification_request.expected_app_hint.as_deref())
                    .or(verification_request.target_label.as_deref()),
            )
        } else {
            None
        };
        let verification = verify_post_action_detailed_with_process(
            &verification_request,
            &pre_observation,
            &post_observation,
            execution.success,
            if is_secret_payload {
                None
            } else {
                expected_text.as_deref()
            },
            completed_at_ms,
            open_app_process_evidence.as_deref(),
        );

        // Task 4 (Issue #10): ordered-evidence honesty — the FULL ordered model.
        // When `gui_cog_verify_evidence` is ON, the core verdict's PRIMARY evidence
        // source is supplemented by the predicate's ordered FALLBACK sources: a
        // verdict whose primary was unavailable/unreliable (a11y off / no
        // screenshot / unreliable active-window probe) is upgraded to `verified`
        // when a reliable SECONDARY source positively confirms a real effect (a
        // screen-hash change, an active-window change, or a running process), and
        // is otherwise the honest `inconclusive` — never a false `verification_failed`
        // and never a false `verified`. This generalizes the Task-2 browser
        // `screen_changed` override into a per-predicate evidence chain (the same
        // a11y-off → screen-change fallback now applies to ALL predicates, not just
        // the browser address bar). It runs BEFORE the safety-polish contract so an
        // honest secondary-confirmed `verified` is not re-downgraded. Flag-OFF =
        // byte-for-byte no-op.
        let screen_changed_signal = matches!(
            (
                pre_observation.screen_hash.as_deref(),
                post_observation.screen_hash.as_deref(),
            ),
            (Some(before), Some(after)) if before != after
        );
        let active_window_changed_signal = post_observation.active_window_probe_ok
            && (pre_observation.active_window.label != post_observation.active_window.label
                || pre_observation.active_window.app_name
                    != post_observation.active_window.app_name);
        let verification = apply_ordered_evidence(
            &verification,
            &GuiSecondaryEvidence {
                screen_changed: screen_changed_signal,
                active_window_changed: active_window_changed_signal,
                process_running: open_app_process_evidence.is_some(),
                accessibility_ok: post_observation.accessibility_ok,
                screenshot_available: post_observation.screenshot_available,
                active_window_probe_ok: post_observation.active_window_probe_ok,
            },
            verify_evidence_enabled(),
        );

        // Task 9.1 (Requirements 10, 13, 15, 22, 23): when the
        // `gui_cog_safety_polish` flag is ON, formalize the per-action-type
        // verification CONTRACT (predicate + evidence source + bounded wait +
        // confidence) and apply its low-confidence / unreliable-evidence rule so
        // a weak `verified` becomes the honest `inconclusive` verdict (never a
        // false verified). The bounded wait reuses this turn's Task 1 caps (per-
        // step verify budget + effective re-observe cap) so verification never
        // polls unbounded. While the flag is OFF this is a no-op and the verdict
        // is byte-for-byte unchanged.
        let (verification, verification_contract) = if self.safety_polish.is_enabled() {
            let budget = &self.runtime_guards.budget;
            let contract = verification_contract_for_with_flag(
                &action_kind,
                is_secret_payload,
                budget.step_verify_ms,
                budget.effective_max_reobserve(),
                self.verify_live.is_enabled(),
            );
            let adjusted = apply_verification_contract(
                &verification,
                &contract,
                post_observation.active_window_probe_ok,
            );
            (adjusted, Some(contract))
        } else {
            (verification, None)
        };

        // Task 4 (Issue #10): the ordered-evidence honesty step ran above, before
        // the safety-polish contract, so an honest secondary-confirmed `verified`
        // is preserved and a primary-unavailable verdict is the honest
        // `inconclusive` (never a false `verification_failed`).

        let result = GuiExecutionResult {
            execution_id: execution_request.execution_id.clone(),
            proposal_id: execution_request.proposal_id.clone(),
            proposal_hash: execution_request.proposal_hash.clone(),
            action_type: execution_request.action_type.clone(),
            status: if execution.success {
                "completed".into()
            } else {
                "failed".into()
            },
            started_at_ms,
            completed_at_ms,
            // Task 4.2: report the actual window-focus backend that performed the
            // activation when SwitchWindow was routed through the Wayland-safe
            // abstraction and the backend succeeded; otherwise the deterministic
            // executor tool tag (unchanged behavior while the flag is OFF).
            backend_used: match (&window_focus_backend_used, execution.success) {
                (Some(backend_used), true) => backend_used.as_str().to_string(),
                _ => execution.tool.clone(),
            },
            precondition_check: GuiExecutionPreconditionReport::allowed(started_at_ms, Vec::new()),
            postcondition_check: execution_request.expected_postcondition.clone(),
            verification_result: verification.status.clone(),
            error_code: execution.error.as_ref().map(|_| "backend_failed".into()),
            safe_error_summary: execution
                .error
                .as_ref()
                .map(|value| sanitize_event_text(value)),
            can_retry: verification.can_retry,
            recovery_hint: verification.recovery_hint.clone(),
            prompt_hash: execution_request.prompt_hash.clone(),
        };
        if execution.success {
            let mut action_completed = serde_json::json!({
                "type": "ActionCompleted",
                "execution_id": result.execution_id,
                "proposal_id": result.proposal_id,
                "proposal_hash": result.proposal_hash,
                "target_hash": execution_request.target_hash,
                "action_kind": result.action_type,
                "status": "completed",
                "backend_used": result.backend_used,
                "result_summary": "Deterministic GUI action backend reported success.",
                "prompt_hash": result.prompt_hash,
            });
            if let Some(focus_json) = window_focus_final_json.clone() {
                if let Some(obj) = action_completed.as_object_mut() {
                    obj.insert("window_focus".into(), focus_json);
                }
            }
            if let Some(contract) = verification_contract.as_ref() {
                if let Some(obj) = action_completed.as_object_mut() {
                    obj.insert("verification_contract".into(), contract.summary_json());
                }
            }
            events.push(action_completed);
            // Backend success is not final: post-action verification decides the
            // turn outcome. ActionCompleted means backend success only.
            if verification.is_verified() {
                state.status = "completed".into();
                state.reply = format!(
                    "Step 7 executed {} through deterministic backend {} and Step 8 verified the expected result ({}).",
                    result.action_type, result.backend_used, verification.verification_strategy
                );
            } else {
                state.status = verification.status.clone();
                let detail = verification
                    .safe_error_summary
                    .clone()
                    .unwrap_or_else(|| "Post-action state was not confirmed.".into());
                state.reply = format!(
                    "Step 7 executed {} through deterministic backend {}, but Step 8 post-action verification did not pass: {}",
                    result.action_type, result.backend_used, detail
                );
            }
        } else {
            let mut action_failed = serde_json::json!({
                "type": "ActionFailed",
                "execution_id": result.execution_id,
                "proposal_id": result.proposal_id,
                "proposal_hash": result.proposal_hash,
                "target_hash": execution_request.target_hash,
                "action_kind": result.action_type,
                "status": "failed",
                "backend_used": result.backend_used,
                "safe_error_summary": result.safe_error_summary,
                "prompt_hash": result.prompt_hash,
            });
            if let Some(focus_json) = window_focus_final_json.clone() {
                if let Some(obj) = action_failed.as_object_mut() {
                    obj.insert("window_focus".into(), focus_json);
                }
            }
            events.push(action_failed);
            state.status = "blocked".into();
            state.reply = result
                .safe_error_summary
                .clone()
                .unwrap_or_else(|| "Deterministic GUI action failed.".into());
        }
        let mut verification_event = verification.event_payload();
        if let Some(contract) = verification_contract.as_ref() {
            if let Some(obj) = verification_event.as_object_mut() {
                obj.insert("verification_contract".into(), contract.summary_json());
            }
        }
        events.push(verification_event);
        // Task 9.2 (Requirements 10, 11, 12, 13, 14, 15, 22, 23): when the
        // `gui_cog_safety_polish` flag is ON, record this EXECUTED action in the
        // append-only, sanitized audit ledger and emit a ledger event so it
        // surfaces in the turn's event stream. The entry carries only a
        // sanitized target descriptor (label/role), the execution_id /
        // proposal_hash, the authorization source (+ HITL decision id), the
        // verification verdict, timestamps, and the prompt_hash — NEVER a secret
        // payload (passwords/clipboard), coordinates, or the raw prompt. While
        // the flag is OFF this is a no-op: the ledger stays empty and no ledger
        // event is emitted (events unchanged). Only actions whose backend
        // actually ran reach this point — precondition/no-focus-path blocks
        // return earlier and are never ledgered as executed.
        if self.safety_polish.is_enabled() {
            let resolved = target_resolution.resolved_target.as_ref();
            let ledger_label = resolved
                .map(|target| target.label.clone())
                .or_else(|| proposal.target_label.clone());
            let ledger_role = resolved
                .map(|target| target.role.clone())
                .or_else(|| proposal.target_role.clone());
            let record = GuiActionLedgerRecord {
                action_type: execution_request.action_type.clone(),
                target_label: ledger_label,
                target_role: ledger_role,
                execution_id: execution_request.execution_id.clone(),
                proposal_hash: execution_request.proposal_hash.clone(),
                authorization_source: execution_request.authorization_source.as_str().to_string(),
                hitl_decision_id: hitl_decision.map(|decision| decision.decision_id.clone()),
                verification_verdict: verification.status.clone(),
                is_secret_payload,
                started_at_ms,
                completed_at_ms,
                prompt_hash: execution_request.prompt_hash.clone(),
            };
            let ledger_event = {
                let entry = state.action_ledger.append(record).clone();
                state.action_ledger.entry_recorded_event(&entry)
            };
            events.push(ledger_event);
        }
        // Task 4.3: surface the verify-by-reobserve verdict in the execution
        // result summary too (not only the events) when SwitchWindow was routed
        // through the Wayland-safe abstraction. While the flag is OFF this is a
        // no-op and the summary is byte-for-byte unchanged.
        let mut result_summary = result.summary_json();
        if let Some(focus_json) = window_focus_final_json.clone() {
            if let Some(obj) = result_summary.as_object_mut() {
                obj.insert("window_focus".into(), focus_json);
            }
        }
        state.action = Some(result_summary.clone());
        state.execution_result = Some(result_summary);
        state.verification_result = Some(verification.clone());

        // Step 9 / Task 6 (Issue #13): smart bounded recovery runs only when
        // verification did not confirm the expected state (verified actions never
        // recover). The recovery POLICY (`assess_recovery`) is already
        // bounded+idempotent-only: transient failures (load-not-ready, stale,
        // inconclusive, focus-lost, wrong-window) get a single bounded
        // re-observe / re-focus / switch-back / idempotent-retry capped by
        // `RECOVERY_MAX_RETRY_COUNT`; risky / non-idempotent / denied / moved /
        // ambiguous cases always STOP (never auto-retried). The
        // `gui_cog_smart_recovery` flag (default ON) is a kill-switch: when OFF,
        // recovery is skipped entirely and the turn stops on the unverified step
        // (the pre-recovery behavior), so the bounded-retry path can be rolled
        // back without a rebuild.
        if smart_recovery_enabled() && should_attempt_recovery(&verification.status) {
            self.run_recovery_loop(
                events,
                proposal,
                target_resolution,
                hitl_decision,
                &verification,
                &backend,
                &post_observation,
                execution.success,
                now_ms(),
                state,
            )
            .await;
        }
    }

    #[allow(clippy::too_many_arguments)]
    async fn run_recovery_loop(
        &self,
        events: &mut GuiEventStream,
        proposal: &GuiActionProposal,
        target_resolution: &GuiTargetResolutionSummary,
        hitl_decision: Option<&GuiHitlDecision>,
        verification: &GuiPostActionVerificationResult,
        backend: &GuiActionBackendStatus,
        post_action_observation: &GuiObservationSnapshot,
        backend_success: bool,
        now: i64,
        state: &mut RuntimeState,
    ) {
        let action_kind = GuiActionKind::from_action_type(&proposal.action_type);
        let safety_polish_enabled = self.safety_polish.is_enabled();
        let hitl_denied = hitl_decision
            .map(|decision| decision.decision == "denied")
            .unwrap_or(false);
        let hitl_stale = hitl_decision
            .map(|decision| decision.decision != "denied" && !decision.can_authorize_step7)
            .unwrap_or(false);

        // Bounded re-resolve count for control actions: how many post-action
        // controls still match the original target label/role.
        let reresolve_candidate_count = match action_kind {
            GuiActionKind::OpenApp | GuiActionKind::SwitchWindow => 0,
            _ => proposal
                .target_label
                .as_deref()
                .map(|label| {
                    post_action_observation
                        .all_controls()
                        .iter()
                        .filter(|control| {
                            control.name.eq_ignore_ascii_case(label)
                                && proposal
                                    .target_role
                                    .as_deref()
                                    .map(|role| control.role.eq_ignore_ascii_case(role))
                                    .unwrap_or(true)
                        })
                        .count()
                })
                .unwrap_or(0),
        };

        let recovery_id = format!(
            "recovery-{}",
            stable_hash(&format!(
                "{}|{}|{}",
                verification.execution_id, verification.verification_id, now
            ))
        );
        let signals = GuiRecoverySignals {
            backend_success,
            verification_status: verification.status.clone(),
            verification_strategy: verification.verification_strategy.clone(),
            matched_expected_state: verification.matched_expected_state,
            target_still_present: verification.target_still_present,
            target_identity_matches: verification.target_identity_matches,
            modal_present: !post_action_observation.dialogs.is_empty(),
            active_window_known: post_action_observation.active_window_probe_ok
                && post_action_observation.active_window.confidence > 0.0,
            reresolve_candidate_count,
            context_stale: false,
            // Task 9.5 (Requirements 10, 14): a load failure is an open/switch
            // navigation whose expected window/page never became observable.
            // Classified ONLY when the `gui_cog_safety_polish` flag is ON; while
            // OFF this stays false and the recovery routing is unchanged.
            load_failed: safety_polish_enabled
                && matches!(
                    action_kind,
                    GuiActionKind::OpenApp | GuiActionKind::SwitchWindow
                )
                && !verification.matched_expected_state
                && !(post_action_observation.active_window_probe_ok
                    && post_action_observation.active_window.confidence > 0.0),
        };
        let input = GuiRecoveryInput {
            recovery_id: recovery_id.clone(),
            execution_id: verification.execution_id.clone(),
            verification_id: verification.verification_id.clone(),
            proposal_id: proposal.proposal_id.clone(),
            proposal_hash: proposal.proposal_hash.clone(),
            target_hash: proposal.target_hash.clone(),
            action_type: proposal.action_type.clone(),
            risk_level: proposal.risk_level.clone(),
            requires_user_approval: proposal.requires_user_approval,
            hitl_denied,
            hitl_stale,
            retry_count: 0,
            prompt_hash: proposal.prompt_hash.clone(),
            signals,
            safety_polish: safety_polish_enabled,
        };

        let assessment = assess_recovery(&input);
        events.push(assessment.event_payload());
        state.recovery_assessment = Some(assessment.summary_json());

        // Task 9.5 (Requirements 10, 11, 12, 13, 14, 15, 22, 23): additive
        // `RecoveryDecision` telemetry that makes the recovery decision
        // inspectable (recovery kind, idempotent-gated, bounded single retry,
        // unexpected-dialog-stop, load-failure-explain). Emitted ONLY when the
        // `gui_cog_safety_polish` flag is ON; while OFF the event stream is
        // byte-for-byte unchanged. Purely observational — never alters routing.
        if safety_polish_enabled {
            events.push(recovery_decision_event(
                &assessment.recovery_action_kind,
                &assessment.failure_kind,
                &assessment.status,
                default_idempotent_for(action_kind.as_str()),
                assessment.can_execute_recovery,
                assessment.retry_count,
                assessment.max_retry_count,
            ));
        }

        let action_kind_recovery = match assessment.recovery_action_kind.as_str() {
            "ReObserve" => GuiRecoveryActionKind::ReObserve,
            "RefocusSameTarget" => GuiRecoveryActionKind::RefocusSameTarget,
            "SwitchBackToWindow" => GuiRecoveryActionKind::SwitchBackToWindow,
            "RetryIdempotentAction" => GuiRecoveryActionKind::RetryIdempotentAction,
            "ReResolveTarget" => GuiRecoveryActionKind::ReResolveTarget,
            "AskClarification" => GuiRecoveryActionKind::AskClarification,
            _ => GuiRecoveryActionKind::Stop,
        };

        if !assessment.can_execute_recovery || !action_kind_recovery.is_executable_recovery() {
            // No safe recovery action: emit RecoveryBlocked without starting one.
            // The turn status keeps the verification verdict (verification_failed
            // / inconclusive / blocked) unless recovery needs the user.
            events.push(recovery_blocked_event(&assessment));
            match assessment.status.as_str() {
                "needs_clarification" => state.status = "needs_clarification".into(),
                "needs_approval" => state.status = "needs_approval".into(),
                _ => {}
            }
            state.reply = assessment.safe_explanation.clone();
            if let Some(blocker_reason) = assessment.blockers.first() {
                state.blocker = Some(GuiBlocker::new("recovery", blocker_reason.clone()));
            }
            return;
        }

        // Safe, bounded recovery action.
        let started_at_ms = now_ms();
        let mut result = GuiRecoveryResult {
            recovery_id: recovery_id.clone(),
            execution_id: verification.execution_id.clone(),
            status: "recovered".into(),
            recovery_action_kind: action_kind_recovery.as_str().into(),
            started_at_ms,
            completed_at_ms: started_at_ms,
            backend_used: backend.selected_backend.clone(),
            post_recovery_observation_id: None,
            post_recovery_context_id: None,
            verification_result: "recovered".into(),
            safe_error_summary: None,
            next_recommended_state: "retry_original_action".into(),
            can_retry_original_action: true,
            can_continue_workflow: false,
            prompt_hash: proposal.prompt_hash.clone(),
        };
        events.push(result.started_event_payload());

        if matches!(action_kind_recovery, GuiRecoveryActionKind::ReObserve) {
            // Re-observe only: always safe, never touches the input backend.
            // Task 3 (Issue #9): a recovery re-observe assesses post-action state,
            // so force a fresh capture (no stale pre-action cache frame).
            let post_recovery = self
                .observe_with_events_fresh(events, ObservationFreshness::ForceFresh)
                .await;
            result.completed_at_ms = now_ms();
            result.backend_used = "observation".into();
            result.post_recovery_observation_id = Some(post_recovery.observation_id.clone());
            result.post_recovery_context_id = Some(post_recovery.context_id.clone());
            result.verification_result = "reobserved".into();
            result.status = "recovered".into();
            result.next_recommended_state = "replan".into();
            events.push(result.completed_event_payload());
            state.recovery_result = Some(result.summary_json());
            state.status = "recovered".into();
            state.reply = assessment.safe_explanation.clone();
            return;
        }

        // Input-backend recovery: re-run one idempotent action on the same target.
        let recovery_action_request = self.build_recovery_action_request(
            &action_kind_recovery,
            &action_kind,
            proposal,
            target_resolution,
        );
        let recovery_execution = self.executor.execute(recovery_action_request).await;
        // Task 3 (Issue #9): post-recovery-action observe must be fresh.
        let post_recovery = self
            .observe_with_events_fresh(events, ObservationFreshness::ForceFresh)
            .await;
        result.completed_at_ms = now_ms();
        result.backend_used = recovery_execution.tool.clone();
        result.post_recovery_observation_id = Some(post_recovery.observation_id.clone());
        result.post_recovery_context_id = Some(post_recovery.context_id.clone());

        let recovery_strategy = select_verification_strategy_with_flag(
            &action_kind,
            false,
            self.verify_live.is_enabled(),
        );
        let recovery_verification_request = GuiPostActionVerificationRequest {
            verification_id: format!("recovery-verify-{}", stable_hash(&recovery_id)),
            execution_id: verification.execution_id.clone(),
            proposal_id: proposal.proposal_id.clone(),
            proposal_hash: proposal.proposal_hash.clone(),
            action_type: proposal.action_type.clone(),
            target_hash: proposal.target_hash.clone(),
            stable_target_identity_hash: None,
            expected_postcondition: proposal.expected_postcondition.clone(),
            verification_strategy: recovery_strategy.as_str().into(),
            pre_action_context_id: post_action_observation.context_id.clone(),
            post_action_observation_id: post_recovery.observation_id.clone(),
            post_action_context_id: post_recovery.context_id.clone(),
            started_at_ms,
            is_secret_payload: false,
            prompt_hash: proposal.prompt_hash.clone(),
            target_label: proposal.target_label.clone(),
            target_role: proposal.target_role.clone(),
            target_control_id: proposal.target_control_id.clone(),
            expected_app_hint: target_resolution
                .resolved_target
                .as_ref()
                .and_then(|target| target.app_hint.clone()),
            expected_window_hint: target_resolution
                .resolved_target
                .as_ref()
                .and_then(|target| target.window_hint.clone()),
        };
        let recovery_verification = verify_post_action_detailed(
            &recovery_verification_request,
            post_action_observation,
            &post_recovery,
            recovery_execution.success,
            None,
            now_ms(),
        );
        result.verification_result = recovery_verification.status.clone();

        if recovery_execution.success && recovery_verification.is_verified() {
            result.status = "recovered".into();
            result.next_recommended_state = "retry_original_action".into();
            result.can_retry_original_action = true;
            events.push(result.completed_event_payload());
            state.recovery_result = Some(result.summary_json());
            state.status = "recovered".into();
            state.reply = format!(
                "KRIA recovered safely via {} and restored the expected state.",
                result.recovery_action_kind
            );
        } else {
            result.status = "blocked".into();
            result.next_recommended_state = "stop".into();
            result.can_retry_original_action = false;
            result.safe_error_summary = Some(
                "Bounded recovery did not restore the expected state; stopping safely.".into(),
            );
            events.push(result.completed_event_payload());
            state.recovery_result = Some(result.summary_json());
            state.status = "blocked".into();
            state.reply =
                "KRIA attempted one safe recovery but could not confirm the expected state, so it stopped."
                    .into();
            state.blocker = Some(GuiBlocker::new(
                "recovery",
                "bounded recovery did not restore the expected state",
            ));
        }
    }

    fn build_recovery_action_request(
        &self,
        recovery_kind: &GuiRecoveryActionKind,
        original_kind: &GuiActionKind,
        proposal: &GuiActionProposal,
        target_resolution: &GuiTargetResolutionSummary,
    ) -> GuiActionRequest {
        let kind = match recovery_kind {
            GuiRecoveryActionKind::RefocusSameTarget => GuiActionKind::FocusField,
            GuiRecoveryActionKind::SwitchBackToWindow => GuiActionKind::SwitchWindow,
            _ => original_kind.clone(),
        };
        // Task 2.5 (Property 1 / Requirement 1.4): the recovery target name is
        // threaded ONLY from real goal-contract / resolved-target data — never
        // from the action kind. The prior `unwrap_or_else(|| action_type)` tail
        // leaked the action verb (e.g. "focus_field") into the executor as a
        // target name, masking genuinely-missing target data. When no real
        // target descriptor exists we fall back to an empty target (matching the
        // primary execution path) so the executor reports an unresolved target
        // instead of acting on a fabricated action-kind target.
        let resolved = target_resolution.resolved_target.as_ref();
        let target_name = match recovery_kind {
            GuiRecoveryActionKind::SwitchBackToWindow => resolved
                .and_then(|target| target.window_hint.clone())
                .or_else(|| resolved.and_then(|target| target.app_hint.clone()))
                .or_else(|| proposal.target_label.clone())
                .or_else(|| proposal.target_control_id.clone())
                .unwrap_or_default(),
            _ => proposal
                .target_label
                .clone()
                .or_else(|| proposal.target_control_id.clone())
                .or_else(|| resolved.map(|target| target.label.clone()))
                .or_else(|| resolved.map(|target| target.control_id.clone()))
                .unwrap_or_default(),
        };
        let role = proposal
            .target_role
            .clone()
            .unwrap_or_else(|| role_for_action(&kind).into());
        GuiActionRequest {
            kind: kind.clone(),
            role,
            target_name,
            value: None,
            execution_hint: execution_hint_for_action(&kind).into(),
            abs_click: None,
        }
    }

    fn emit_approval_required(
        &self,
        events: &mut GuiEventStream,
        intent: &GuiCognitionIntent,
        state: &mut RuntimeState,
        reason: &str,
    ) {
        state.status = "needs_approval".into();
        let reasons = if intent.risk_reasons.is_empty() {
            vec!["user requested approval before action".to_string()]
        } else {
            intent.risk_reasons.clone()
        };
        let safety = safety_for_intent(intent);
        events.push(serde_json::json!({
            "type": "SafetyGateCompleted",
            "status": safety.status.as_event_status(),
            "risk_level": safety.risk_level,
            "reasons": safety.reasons,
        }));
        events.push(serde_json::json!({
            "type": "HitlRequired",
            "risk_level": intent.risk_level,
            "reason": reason,
        }));
        state.blocker = Some(GuiBlocker::new("approval_required", reason).with_options(reasons));
    }

    #[allow(dead_code)]
    async fn handle_focus_intent(
        &self,
        events: &mut GuiEventStream,
        context: &GuiContext,
        state: &mut RuntimeState,
    ) {
        events.push(serde_json::json!({
            "type": "TargetResolutionStarted",
            "action_kind": GuiActionKind::FocusField.as_str(),
            "role": "text",
        }));
        match resolve_unique_text_field(context) {
            TargetResolution::Resolved(target) => {
                let request = GuiActionRequest {
                    kind: GuiActionKind::FocusField,
                    role: "text".into(),
                    target_name: target.name.clone(),
                    value: None,
                    execution_hint: "click_ui_element".into(),
                    abs_click: None,
                };
                state.target = Some(serde_json::json!({
                    "role": target.role,
                    "name": target.name,
                    "confidence": target.confidence,
                    "evidence": target.evidence,
                }));
                self.execute_and_verify(events, request, state, 0.72, |success, target, error| {
                    if success {
                        format!("Focused the visible text field '{}' and re-observed the GUI. Verification: focused action completed with post-action observation.", target)
                    } else {
                        format!("I found the text field '{}', but focus execution failed: {}. I stopped safely.", target, error.unwrap_or_else(|| "unknown error".into()))
                    }
                }).await;
            }
            TargetResolution::Missing {
                reason,
                candidate_count,
            }
            | TargetResolution::Ambiguous {
                reason,
                candidate_count,
            } => {
                self.emit_target_block(events, state, &reason, candidate_count, None);
                state.reply = format!("{reason} I did not focus or type anything.");
            }
        }
    }

    #[allow(dead_code)]
    fn handle_type_validation_block(
        &self,
        events: &mut GuiEventStream,
        intent: &GuiCognitionIntent,
        state: &mut RuntimeState,
        reason: &str,
    ) {
        if intent.requires_approval || reason.contains("terminal") {
            let risk_reason = if intent.requires_approval {
                "typing request appears sensitive or risky"
            } else {
                reason
            };
            state.status = "needs_approval".into();
            events.push(serde_json::json!({
                "type": "SafetyGateCompleted",
                "status": "RequiresApproval",
                "risk_level": "high",
                "reasons": [risk_reason],
            }));
            state.blocker = Some(GuiBlocker::new("safety", risk_reason));
            state.reply = format!("{risk_reason}. I did not type anything.");
        } else {
            state.status = "needs_clarification".into();
            state.blocker = Some(GuiBlocker::new("missing_text", reason));
            events.push(serde_json::json!({
                "type": "PlanBlocked",
                "reason": "missing_text",
                "clarification_question": "What exact text should I type?",
            }));
            state.reply =
                "Please provide the exact text to type in quotes. I did not type anything.".into();
        }
    }

    #[allow(dead_code)]
    async fn handle_type_intent(
        &self,
        events: &mut GuiEventStream,
        context: &GuiContext,
        intent: &GuiCognitionIntent,
        state: &mut RuntimeState,
    ) {
        events.push(serde_json::json!({
            "type": "TargetResolutionStarted",
            "action_kind": GuiActionKind::FillField.as_str(),
            "role": "text",
        }));

        match resolve_type_text_target(context) {
            TargetResolution::Resolved(target) => {
                let execution_hint = if target.confidence >= 0.8 {
                    "fill_form_field"
                } else {
                    "atspi_type_into_focused"
                };
                let request = GuiActionRequest {
                    kind: GuiActionKind::FillField,
                    role: "text".into(),
                    target_name: target.name.clone(),
                    value: intent.typed_text.clone(),
                    execution_hint: execution_hint.into(),
                    abs_click: None,
                };
                state.target = Some(serde_json::json!({
                    "role": target.role,
                    "name": target.name,
                    "confidence": target.confidence,
                    "evidence": target.evidence,
                }));
                self.execute_and_verify(events, request, state, 0.72, |success, _target, error| {
                    if success {
                        "Typed the requested text into the resolved visible text field and re-observed the GUI. Verification completed with post-action observation.".into()
                    } else {
                        format!("Typing failed during deterministic AT-SPI execution: {}. I stopped safely.", error.unwrap_or_else(|| "unknown error".into()))
                    }
                }).await;
            }
            TargetResolution::Missing {
                reason,
                candidate_count,
            }
            | TargetResolution::Ambiguous {
                reason,
                candidate_count,
            } => {
                self.emit_target_block(events, state, &reason, candidate_count, None);
                state.reply = format!("{reason} I did not type anything.");
            }
        }
    }

    #[allow(dead_code)]
    async fn handle_click_intent(
        &self,
        events: &mut GuiEventStream,
        context: &GuiContext,
        intent: &GuiCognitionIntent,
        state: &mut RuntimeState,
    ) {
        let control_name = intent.control_name.clone().unwrap_or_default();
        events.push(serde_json::json!({
            "type": "TargetResolutionStarted",
            "action_kind": GuiActionKind::ClickControl.as_str(),
            "role": "push button",
            "query": control_name,
        }));

        if intent.requires_approval {
            self.emit_approval_required(
                events,
                intent,
                state,
                "Click action is risky and requires explicit approval.",
            );
            state.reply = "This click may submit/send/delete/pay or otherwise affect external state. I paused and did not click anything.".into();
            return;
        }

        match resolve_button(context, &control_name) {
            TargetResolution::Resolved(target) => {
                let request = GuiActionRequest {
                    kind: GuiActionKind::ClickControl,
                    role: "push button".into(),
                    target_name: target.name.clone(),
                    value: None,
                    execution_hint: "click_ui_element".into(),
                    abs_click: None,
                };
                state.target = Some(serde_json::json!({
                    "role": target.role,
                    "name": target.name,
                    "confidence": target.confidence,
                    "evidence": target.evidence,
                }));
                self.execute_and_verify(events, request, state, 0.68, |success, target, error| {
                    if success {
                        format!("Clicked the resolved safe button '{}' and re-observed the GUI. Verification completed with post-action observation.", target)
                    } else {
                        format!("The target '{}' was resolved, but click execution failed: {}. I stopped safely.", target, error.unwrap_or_else(|| "unknown error".into()))
                    }
                }).await;
            }
            TargetResolution::Missing {
                reason,
                candidate_count,
            }
            | TargetResolution::Ambiguous {
                reason,
                candidate_count,
            } => {
                self.emit_target_block(
                    events,
                    state,
                    &reason,
                    candidate_count,
                    Some(control_name.clone()),
                );
                state.reply = format!(
                    "{reason} Target query: '{}'. I did not click anything.",
                    control_name
                );
            }
        }
    }

    #[allow(dead_code)]
    fn handle_missing_click_target(
        &self,
        events: &mut GuiEventStream,
        state: &mut RuntimeState,
    ) {
        state.status = "needs_clarification".into();
        state.blocker = Some(GuiBlocker::new(
            "missing_target",
            "No button/control name was provided.",
        ));
        events.push(serde_json::json!({
            "type": "PlanBlocked",
            "reason": "missing_target",
            "clarification_question": "Which button or control should I click?",
        }));
        state.reply =
            "I need the button/control name before clicking. I did not click anything.".into();
    }

    #[allow(dead_code)]
    fn emit_target_block(
        &self,
        events: &mut GuiEventStream,
        state: &mut RuntimeState,
        reason: &str,
        candidate_count: usize,
        target_name: Option<String>,
    ) {
        state.status = "needs_clarification".into();
        events.push(serde_json::json!({
            "type": "TargetResolutionBlocked",
            "reason": reason,
            "candidate_count": candidate_count,
        }));
        let mut blocker =
            GuiBlocker::new("target_resolution", reason).with_candidate_count(candidate_count);
        if let Some(target_name) = target_name {
            blocker = blocker.with_target_name(target_name);
        }
        state.blocker = Some(blocker);
    }

    #[allow(dead_code)]
    async fn execute_and_verify<F>(
        &self,
        events: &mut GuiEventStream,
        request: GuiActionRequest,
        state: &mut RuntimeState,
        success_confidence: f64,
        reply_builder: F,
    ) where
        F: FnOnce(bool, String, Option<String>) -> String,
    {
        let safety = GuiSafetyStatus::Allowed;
        let target_type = match &request.kind {
            GuiActionKind::FocusField | GuiActionKind::FillField | GuiActionKind::TypeText => {
                "text_field"
            }
            GuiActionKind::ClickControl => "button",
            GuiActionKind::OpenApp => "application",
            GuiActionKind::SwitchWindow => "window",
            GuiActionKind::PressKey | GuiActionKind::Hotkey => "focused_context",
            GuiActionKind::Scroll => "scrollable",
            GuiActionKind::Copy | GuiActionKind::Paste => "focused_context",
            // Task 6.1 typed primitives.
            GuiActionKind::ClearField | GuiActionKind::SelectAll => "text_field",
            GuiActionKind::SetCheckbox => "checkbox",
            GuiActionKind::CloseDialog => "dialog",
            GuiActionKind::InAppSearch => "search_field",
        };
        events.push(serde_json::json!({
            "type": "TargetResolved",
            "target_type": target_type,
            "label": request.target_name,
            "confidence": state.target.as_ref().and_then(|target| target.get("confidence")).cloned().unwrap_or(serde_json::json!(0.86)),
        }));
        events.push(serde_json::json!({
            "type": "SafetyGateCompleted",
            "status": safety.as_event_status(),
            "risk_level": "low",
        }));

        if let Some(backend) = &state.action_backend {
            if !backend.supports_action(&request.kind) {
                let reason = backend.primary_blocker(&request.kind);
                let action_kind = request.kind.as_str();
                state.status = "blocked".into();
                state.execution_blocker = Some(serde_json::json!({
                    "kind": "action_backend",
                    "reason": reason,
                    "action_kind": action_kind,
                    "selected_backend": backend.selected_backend.clone(),
                    "session_type": backend.session_type.clone(),
                    "blockers": backend.blockers.clone(),
                    "global_halt_engaged": backend.global_halt_engaged,
                    "halt_kind": backend.halt_kind.clone(),
                    "halt_reason": backend.halt_reason.clone(),
                    "release_conditions": backend.release_conditions.clone(),
                    "can_observe": backend.can_observe,
                    "can_plan": backend.can_plan,
                }));
                state.blocker = Some(
                    GuiBlocker::new("action_backend", reason.clone()).with_options(
                        if backend.blockers.is_empty() {
                            vec![format!("selected backend: {}", backend.selected_backend)]
                        } else {
                            backend.blockers.clone()
                        },
                    ),
                );
                events.push(serde_json::json!({
                    "type": "ExecutionBlocked",
                    "reason": reason,
                    "action_kind": action_kind,
                    "selected_backend": backend.selected_backend.clone(),
                    "session_type": backend.session_type.clone(),
                    "global_halt_engaged": backend.global_halt_engaged,
                    "halt_kind": backend.halt_kind.clone(),
                    "halt_reason": backend.halt_reason.clone(),
                    "release_conditions": backend.release_conditions.clone(),
                    "blockers": backend.blockers.clone(),
                }));
                events.push(serde_json::json!({
                    "type": "VerificationStarted",
                    "verification": "execution_blocker",
                }));
                events.push(serde_json::json!({
                    "type": "VerificationCompleted",
                    "status": "blocked",
                    "confidence": 1.0,
                    "summary": "Action was not executed because the GUI action backend is blocked or unavailable.",
                }));
                events.push(serde_json::json!({
                    "type": "RecoveryEvaluationStarted",
                    "reason": "action_backend_blocked",
                    "idempotency": "safe_retry_after_capability_change",
                }));
                events.push(serde_json::json!({
                    "type": "RecoveryProposed",
                    "reason": reason,
                    "options": [
                        "Resolve the GUI action backend blocker, then retry.",
                        "Re-observe the screen without executing an action.",
                        "Ask the user for a different safe target."
                    ],
                }));
                state.verification = Some(GuiVerificationReport {
                    status: "blocked".into(),
                    confidence: 1.0,
                    after_observation_id: String::new(),
                });
                state.reply = reply_builder(false, request.target_name, Some(reason));
                return;
            }
        }

        events.push(serde_json::json!({
            "type": "ActionStarted",
            "action_kind": request.kind.as_str(),
            "target": request.target_name,
        }));

        let target_name = request.target_name.clone();
        let action_kind = request.kind.as_str();
        let execution = self.executor.execute(request).await;
        events.push(serde_json::json!({
            "type": "ActionCompleted",
            "action_kind": action_kind,
            "status": if execution.success { "completed" } else { "failed" },
        }));
        // Task 3 (Issue #9): verification re-observe must be a fresh capture.
        let post_observation = self
            .observe_with_events_fresh(events, ObservationFreshness::ForceFresh)
            .await;
        events.push(serde_json::json!({
            "type": "VerificationStarted",
            "verification": "post_action_observation",
        }));
        let verification = verify_post_action(&execution, &post_observation, success_confidence);
        events.push(serde_json::json!({
            "type": "VerificationCompleted",
            "status": verification.status,
            "confidence": verification.confidence,
        }));

        state.action = Some(action_summary(&execution));
        state.verification = Some(verification);
        if !execution.success {
            state.status = "blocked".into();
            let recovery_reason = execution
                .error
                .clone()
                .unwrap_or_else(|| "GUI action execution failed".into());
            events.push(serde_json::json!({
                "type": "RecoveryEvaluationStarted",
                "reason": recovery_reason,
                "idempotency": "safe_retry_only_after_reobserve",
            }));
            events.push(serde_json::json!({
                "type": "RecoveryProposed",
                "reason": recovery_reason,
                "options": [
                    "Re-observe the screen and resolve the target again.",
                    "Retry once only if the target remains unique and the action is safe.",
                    "Ask the user for clarification if the target changed."
                ],
            }));
        }
        state.reply = reply_builder(execution.success, target_name, execution.error);
    }

    #[allow(clippy::too_many_arguments)]
    async fn run_workflow(
        &self,
        events: &mut GuiEventStream,
        request: &GuiTurnRequest,
        context: &GuiContext,
        goal_contract: &GuiGoalContract,
        plan: &self::llm_planner::GuiLlmPlan,
        readiness_validation: &GuiPlanValidationReport,
        plan_id: &str,
        state: &mut RuntimeState,
    ) {
        let steps = typed_plan_steps(plan);
        let mut run = GuiWorkflowRun::new(
            &request.session_id,
            &request.workflow_id,
            &request.turn_id,
            &goal_contract.contract_id,
            plan_id,
            &context.context_id,
            &steps,
            &plan.risk_level,
            plan.requires_user_approval,
            request.execution_mode.as_str(),
            &goal_contract.prompt_hash,
        );
        let mut current_context = context.clone();
        // Task 3.2 (Requirement 2.1/2.2, Property 2): track whether the most
        // recently completed step changed GUI state. After a state-changing step
        // the next step's target MUST resolve against a FRESH observation, never
        // the stale pre-action screen. Seeded false: the very first step has no
        // predecessor and resolves against the initial observation.
        let mut previous_step_changed_state = false;
        let allows_execution = request.execution_mode.allows_execution();
        let mut resume_start_index = 0usize;
        let mut resumed_completed_indices: Vec<usize> = Vec::new();
        // Task 1.3 (Requirement 19.2/19.4, 21.3/21.4, Property 9): per-turn
        // runaway-control accounting. The clock starts now; the caps are checked
        // at the same pre-action checkpoint as the Task 1.2 cancel/halt guard.
        // Inert while `gui_cog_runtime_guards` is OFF (default).
        let mut budget_tracker = GuiTurnBudgetTracker::new(&self.runtime_guards);
        // Seed flapping history with the initial observation.
        budget_tracker.note_screen_hash(current_context.observation.screen_hash.as_deref());

        // Step 11: resume from a checkpoint. Re-observe (already done before this
        // call) and revalidate before allowing any continuation. Fail closed.
        if let Some(cp) = request.resume_checkpoint.clone() {
            let now = now_ms();
            let resume_request = GuiWorkflowResumeRequest {
                resume_id: format!("resume-{}", stable_hash(&format!("{}|{}", cp.checkpoint_id, now))),
                checkpoint_id: cp.checkpoint_id.clone(),
                workflow_run_id: cp.workflow_run_id.clone(),
                session_id: cp.session_id.clone(),
                requested_at_ms: now,
                current_observation_id: current_context.observation_id.clone(),
                current_context_id: current_context.context_id.clone(),
                current_screen_hash_prefix: current_context
                    .observation
                    .screen_hash
                    .as_ref()
                    .map(|hash| hash.chars().take(16).collect()),
                reason: request
                    .resume_reason
                    .clone()
                    .unwrap_or_else(|| "user_resume".into()),
                prompt_hash: cp.prompt_hash.clone(),
            };
            events.push(serde_json::json!({
                "type": "WorkflowResumeRequested",
                "resume_id": resume_request.resume_id,
                "checkpoint_id": cp.checkpoint_id,
                "workflow_run_id": cp.workflow_run_id,
                "reason": resume_request.reason,
                "prompt_hash": cp.prompt_hash,
            }));
            events.push(serde_json::json!({
                "type": "WorkflowCheckpointLoaded",
                "checkpoint_id": cp.checkpoint_id,
                "checkpoint_hash_prefix": cp.checkpoint_hash.chars().take(12).collect::<String>(),
                "workflow_run_id": cp.workflow_run_id,
                "current_step_index": cp.current_step_index,
                "completed_step_count": cp.completed_step_receipts.len(),
                "can_execute": false,
                "prompt_hash": cp.prompt_hash,
            }));
            let screen_prefix: Option<String> = current_context
                .observation
                .screen_hash
                .as_ref()
                .map(|hash| hash.chars().take(16).collect());
            let screen_changed = match (
                cp.last_screen_hash_prefix.as_deref(),
                screen_prefix.as_deref(),
            ) {
                (Some(before), Some(after)) => before != after,
                _ => false,
            };
            let signals = GuiResumeObservationSignals {
                current_screen_hash_prefix: screen_prefix,
                current_active_window_hash: None,
                // Fail closed: a changed screen means the bound target identity
                // can no longer be trusted without re-resolution.
                pending_target_still_present: !screen_changed,
                pending_target_identity_matches: !screen_changed,
            };
            let recomputed = checkpoint_hash(&cp);
            let resume_result = validate_resume(&cp, &resume_request, &signals, &recomputed, None, now);

            let proceed = matches!(
                resume_result.status.as_str(),
                "resumed" | "needs_approval" | "needs_reobserve"
            );
            if proceed {
                events.push(resume_result.validated_event_payload());
                // Seed completed receipts so completed steps are not replayed.
                run.completed_step_receipts = cp.completed_step_receipts.clone();
                for receipt in &cp.completed_step_receipts {
                    if let Some(slot) = run.step_states.get_mut(receipt.step_index) {
                        slot.status = "completed".into();
                        slot.can_continue = true;
                    }
                    resumed_completed_indices.push(receipt.step_index);
                }
                resume_start_index = cp.current_step_index;
            } else {
                events.push(resume_result.rejected_event_payload());
                run.status = "blocked".into();
                run.blocked_reason = Some(resume_result.safe_explanation.clone());
                run.completed_step_receipts = cp.completed_step_receipts.clone();
                events.push(run.run_terminal_event());
                state.status = "blocked".into();
                state.reply = resume_result.safe_explanation.clone();
                state.workflow_run = Some(run.summary_json());
                if let Some(reason) = resume_result
                    .blockers
                    .first()
                    .or_else(|| resume_result.invalidated_approvals.first())
                    .or_else(|| resume_result.duplicate_action_guards.first())
                {
                    state.blocker = Some(GuiBlocker::new("resume", reason.clone()));
                }
                return;
            }
        }

        events.push(run.run_started_event());

        // Task 1 (Issue #5): GOAL-LEVEL approval gate. When the GOAL is
        // approval-required (risk high/critical, destructive verb, or explicit
        // "after approval"), pause for HITL BEFORE running ANY step — even a
        // benign Observe / prerequisite — so the outcome is DETERMINISTIC
        // (independent of plan shape / desktop state) and NO state changes before
        // approval. Honors the same HITL fixture; on the real session (no
        // fixture) it pauses (CORRECTLY_GATED). Flag-gated by
        // `gui_cog_gate_determinism`; flag-OFF keeps prior per-step-only gating.
        // Cancellation / global-halt takes PRECEDENCE over the goal-level
        // approval gate: if the user has already stopped the turn (or a global
        // halt is engaged) we abort immediately with the `cancelled`/halt cause —
        // we never pause for approval on a turn the user already cancelled.
        // Gated behind `gui_cog_runtime_guards` exactly like the per-step guard
        // (flag OFF ⇒ `evaluate_pre_action_guard` returns `Proceed`, so behavior
        // is byte-for-byte unchanged).
        {
            let guard =
                evaluate_pre_action_guard(&self.runtime_guards, self.cancel_token.as_ref());
            if let PreActionGuard::Halted { reason } | PreActionGuard::Cancelled { reason } =
                &guard
            {
                run.status = "blocked".into();
                run.blocked_reason = Some(reason.clone());
                events.push(run_aborted_event(&run, guard.cause(), reason, 0));
                events.push(run.run_terminal_event());
                state.status = "blocked".into();
                state.reply = format!("Workflow stopped safely: {reason}");
                state.workflow_run = Some(run.summary_json());
                return;
            }
        }

        if gate_determinism_enabled()
            && allows_execution
            && request.resume_checkpoint.is_none()
            && goal_requires_approval(goal_contract, plan)
        {
            let now = now_ms();
            let approval_resolution = GuiTargetResolutionSummary::skipped(
                plan,
                readiness_validation,
                &current_context,
                plan_id,
                "goal-level approval gate (Issue #5)",
            );
            let proposal = build_action_proposal(
                &request.session_id,
                &request.workflow_id,
                goal_contract,
                plan_id,
                plan,
                readiness_validation,
                &approval_resolution,
                &current_context,
                now,
            );
            let safety_gate = evaluate_safety_gate(proposal, &approval_resolution);
            // Proceed (skip the up-front pause) ONLY when an auto-approval fixture
            // is present AND allowed in this environment (test substrate); the
            // per-step gate then re-confirms. On the real session there is no
            // fixture, so the goal-level gate pauses and nothing executes.
            let approved_up_front = request.hitl_decision_fixture.as_ref().is_some_and(|fixture| {
                let decision = decision_from_fixture(&safety_gate.proposal, fixture, now);
                decision.can_authorize_step7 && request.execution_environment.allows_auto_approval()
            });
            if !approved_up_front && safety_gate.status == "approval_required" {
                events.push(safety_gate.event_payload());
                events.push(safety_gate.hitl_required_event());
                run.status = "paused".into();
                run.blocked_reason = Some(
                    safety_gate
                        .approval_reason
                        .clone()
                        .unwrap_or_else(|| "This task requires your approval before any action.".into()),
                );
                events.push(run.run_terminal_event());
                state.status = "needs_approval".into();
                state.safety_gate = Some(safety_gate.summary_json());
                state.blocker = Some(
                    GuiBlocker::new(
                        "approval_required",
                        safety_gate
                            .approval_reason
                            .clone()
                            .unwrap_or_else(|| "GUI action requires approval".into()),
                    )
                    .with_options(safety_gate.proposal.risk_reasons.clone()),
                );
                state.reply = format!(
                    "{} This task needs your approval before I take any action, so I paused and did not execute anything.",
                    gui_observation_reply(&current_context.observation)
                );
                state.workflow_run = Some(run.summary_json());
                return;
            }
        }

        for index in 0..steps.len() {
            if index < resume_start_index || resumed_completed_indices.contains(&index) {
                // Already completed before the checkpoint; never replayed.
                continue;
            }

            // Task 1.2 (Requirement 21.1/21.2, Property 9): cooperative
            // pre-action guard. Checked BEFORE the next action so cancellation /
            // GlobalSafetyHalt halts before anything else executes. Gated behind
            // `gui_cog_runtime_guards` (default OFF preserves existing behavior).
            let guard =
                evaluate_pre_action_guard(&self.runtime_guards, self.cancel_token.as_ref());
            if let PreActionGuard::Halted { reason } | PreActionGuard::Cancelled { reason } =
                &guard
            {
                run.current_step_index = index;
                let mut aborted_state = run.step_states[index].clone();
                aborted_state.status = "aborted".into();
                aborted_state.completed_at_ms = now_ms();
                aborted_state.blockers.push(reason.clone());
                run.step_states[index] = aborted_state;
                run.status = "blocked".into();
                run.blocked_reason = Some(reason.clone());
                events.push(run_aborted_event(&run, guard.cause(), reason, index));
                break;
            }

            // Task 1.3 (Requirement 19.2/19.4, 21.3/21.4, Property 9): the same
            // pre-action checkpoint also enforces the runaway-control caps —
            // step/loop/time budgets, repeated verification failure, and screen
            // flapping. A breach aborts safely with a distinct stable `cause`
            // tag and a sanitized reason. Gated behind `gui_cog_runtime_guards`.
            if let Some(abort) = budget_tracker.evaluate() {
                run.current_step_index = index;
                let mut aborted_state = run.step_states[index].clone();
                aborted_state.status = "aborted".into();
                aborted_state.completed_at_ms = now_ms();
                aborted_state.blockers.push(abort.reason.clone());
                run.step_states[index] = aborted_state;
                run.status = "blocked".into();
                run.blocked_reason = Some(abort.reason.clone());
                events.push(run_aborted_event(&run, abort.cause, &abort.reason, index));
                break;
            }
            // This step now counts toward the step budget (Requirement 19.2).
            budget_tracker.note_step();

            let step = steps[index].clone();
            run.current_step_index = index;
            let mut step_state = run.step_states[index].clone();
            step_state.status = "started".into();
            step_state.started_at_ms = now_ms();
            events.push(step_started_event(&run, &step_state));

            match workflow_step_kind(&step.step_type) {
                GuiWorkflowStepKind::Observe
                | GuiWorkflowStepKind::Summarize
                | GuiWorkflowStepKind::WaitOrVerify => {
                    // Task 3.1: re-observe / verify-by-observation only (no
                    // executor call) via the bounded per-step re-observe hook.
                    current_context = self
                        .reobserve_fresh_context(
                            events,
                            &mut budget_tracker,
                            index,
                            "observe_step",
                        )
                        .await;
                    run.current_context_id = current_context.context_id.clone();
                    let observable = current_context.observation.has_useful_signal();
                    step_state.completed_at_ms = now_ms();
                    if observable {
                        step_state.status = "completed".into();
                        step_state.can_continue = true;
                        let receipt = self.workflow_receipt(
                            &run,
                            &step_state,
                            "completed",
                            None,
                            None,
                            "Re-observed/verified GUI state without executing an action.",
                            &goal_contract.prompt_hash,
                        );
                        run.completed_step_receipts.push(receipt.clone());
                        run.step_states[index] = step_state.clone();
                        events.push(step_completed_event(&run, &step_state, &receipt));
                        self.save_workflow_checkpoint(
                            events,
                            &mut run,
                            steps.get(index + 1).map(|next| next.step_id.clone()),
                            index + 1,
                            &current_context,
                            state,
                        );
                        // Task 3.2: an observe/verify-only step does not change
                        // GUI state, so the next step still resolves against this
                        // same fresh observation.
                        previous_step_changed_state = false;
                    } else {
                        step_state.status = "blocked".into();
                        step_state.blockers.push("no useful perception signal".into());
                        run.step_states[index] = step_state.clone();
                        run.status = "blocked".into();
                        run.blocked_reason = Some("no useful perception signal".into());
                        events.push(step_blocked_event(&run, &step_state));
                        break;
                    }
                }
                GuiWorkflowStepKind::AskClarification => {
                    step_state.status = "blocked".into();
                    step_state
                        .blockers
                        .push("clarification required before continuing".into());
                    run.step_states[index] = step_state.clone();
                    run.status = "paused".into();
                    run.blocked_reason = Some("clarification required".into());
                    events.push(step_blocked_event(&run, &step_state));
                    break;
                }
                GuiWorkflowStepKind::RequireApproval => {
                    step_state.status = "awaiting_approval".into();
                    step_state
                        .blockers
                        .push("explicit approval required before continuing".into());
                    run.step_states[index] = step_state.clone();
                    run.status = "paused".into();
                    run.blocked_reason = Some("approval required".into());
                    events.push(step_blocked_event(&run, &step_state));
                    break;
                }
                GuiWorkflowStepKind::Executable => {
                    // Task 3.2 (Requirement 2.1/2.2, Property 2): re-observe
                    // before resolving this step's target so the resolution runs
                    // against the FRESH context — not the stale initial
                    // observation. The fresh `GuiContext` from the bounded
                    // Task 3.1 hook is threaded straight into `current_context`,
                    // which is exactly what `resolve_step_target_for_workflow` /
                    // `resolve_plan_targets` resolve against below.
                    //
                    // `gui_cog_reobserve` (default OFF) gates ONLY the explicit
                    // instrumentation: the re-observe + fresh-context threading is
                    // preserved for every step after the first regardless of the
                    // flag, so flag-OFF behavior is byte-for-byte identical. When
                    // ON, the hook `cause` distinguishes a re-observe that follows
                    // a genuine state-changing step (the Requirement 2.1 trigger)
                    // from a plain step boundary. Re-observe stays bounded by the
                    // Task 1 caps via the hook's budget-tracker accounting.
                    if index > 0 {
                        let cause = if previous_step_changed_state {
                            "post_state_change_resolution"
                        } else {
                            "pre_step_resolution"
                        };
                        current_context = self
                            .reobserve_fresh_context(
                                events,
                                &mut budget_tracker,
                                index,
                                cause,
                            )
                            .await;

                        // Task 3.3 (Requirement 2.5, Property 9): after a
                        // state-changing step the new window/app/page may still
                        // be loading. Perform a BOUNDED readiness wait — re-observe
                        // until the expected window/app/page is observable, THEN
                        // resolve. Strictly bounded by the Task 1 caps (no
                        // unbounded poll). Gated behind `gui_cog_reobserve`, so
                        // flag-OFF behavior is byte-for-byte unchanged.
                        if self.reobserve.is_enabled() && previous_step_changed_state {
                            match self
                                .await_step_readiness(
                                    events,
                                    &mut budget_tracker,
                                    index,
                                    &step,
                                    &mut current_context,
                                )
                                .await
                            {
                                ReadinessOutcome::Ready => {}
                                ReadinessOutcome::Aborted(abort) => {
                                    // A Task 1 runaway cap tripped while waiting:
                                    // surface the existing safe-abort path.
                                    run.current_step_index = index;
                                    let mut aborted_state = run.step_states[index].clone();
                                    aborted_state.status = "aborted".into();
                                    aborted_state.completed_at_ms = now_ms();
                                    aborted_state.blockers.push(abort.reason.clone());
                                    run.step_states[index] = aborted_state;
                                    run.status = "blocked".into();
                                    run.blocked_reason = Some(abort.reason.clone());
                                    events.push(run_aborted_event(
                                        &run,
                                        abort.cause,
                                        &abort.reason,
                                        index,
                                    ));
                                    break;
                                }
                                ReadinessOutcome::NotReady { reason } => {
                                    // Readiness not reached within the bound: stop
                                    // safely; do NOT resolve against an un-ready
                                    // screen (Requirement 2.4/2.5).
                                    step_state.status = "blocked".into();
                                    step_state.blockers.push(reason.clone());
                                    run.step_states[index] = step_state.clone();
                                    run.status = "blocked".into();
                                    run.blocked_reason = Some(reason);
                                    events.push(step_blocked_event(&run, &step_state));
                                    break;
                                }
                            }
                        }
                    }
                    run.current_context_id = current_context.context_id.clone();

                    let step_plan_id = format!("{plan_id}-s{index}");
                    let sub_plan = single_step_plan(plan, &step);
                    let summary = self.resolve_step_target_for_workflow(
                        events,
                        &step,
                        &sub_plan,
                        readiness_validation,
                        &current_context,
                        &step_plan_id,
                        state,
                    );
                    step_state.target_resolution_id = Some(summary.resolution_id.clone());

                    if workflow_step_requires_target(&step.step_type)
                        && summary.status != "resolved"
                    {
                        let ambiguous = summary.status == "ambiguous"
                            || summary.status == "needs_clarification"
                            || summary.ambiguity_count > 0;

                        // Task 3.4 (Requirement 2.3/2.4, Property 2/8): the core
                        // Blocker #4 fix. When `gui_cog_reobserve` is ON and the
                        // resolution failure is NOT ambiguity, distinguish a
                        // target that is "present after change" (observable on the
                        // fresh screen, possibly re-identified) from one that is
                        // "genuinely absent" — eliminating the false "resolved
                        // target is no longer present" stop. Ambiguity stays a
                        // no-guess pause. While the flag is OFF the exact prior
                        // block-and-stop behavior is preserved byte-for-byte.
                        if self.reobserve.is_enabled() && !ambiguous {
                            match self
                                .classify_present_or_absent(
                                    events,
                                    &mut budget_tracker,
                                    index,
                                    &step,
                                    &sub_plan,
                                    readiness_validation,
                                    &step_plan_id,
                                    &mut current_context,
                                    state,
                                )
                                .await
                            {
                                PresenceResolution::Resolved(resolved_summary) => {
                                    // Present after change AND re-resolved against
                                    // the fresh context → CONTINUE. Fall through to
                                    // the safety gate / execution path below.
                                    step_state.target_resolution_id =
                                        Some(resolved_summary.resolution_id.clone());
                                    run.current_context_id =
                                        current_context.context_id.clone();
                                }
                                PresenceResolution::Aborted(abort) => {
                                    run.current_step_index = index;
                                    let mut aborted_state = run.step_states[index].clone();
                                    aborted_state.status = "aborted".into();
                                    aborted_state.completed_at_ms = now_ms();
                                    aborted_state.blockers.push(abort.reason.clone());
                                    run.step_states[index] = aborted_state;
                                    run.status = "blocked".into();
                                    run.blocked_reason = Some(abort.reason.clone());
                                    events.push(run_aborted_event(
                                        &run,
                                        abort.cause,
                                        &abort.reason,
                                        index,
                                    ));
                                    break;
                                }
                                PresenceResolution::Ambiguous { reason } => {
                                    // Present but multiple matches → pause + ask.
                                    step_state.status = "blocked".into();
                                    step_state.blockers.push(reason.clone());
                                    run.step_states[index] = step_state.clone();
                                    run.status = "paused".into();
                                    // Task 9.4 (Requirements 11, 22): ambiguity →
                                    // ask, never guess. Emit the additive
                                    // `AmbiguityNoGuess` telemetry (flag-ON only)
                                    // so the no-guess pause is inspectable.
                                    if self.safety_polish.is_enabled() {
                                        events.push(ambiguity_no_guess_event(
                                            summary.ambiguity_count.max(2),
                                            &reason,
                                            GuiAmbiguityDecisionPoint::PerStepReobserve,
                                            None,
                                        ));
                                    }
                                    run.blocked_reason = Some(reason);
                                    events.push(step_blocked_event(&run, &step_state));
                                    break;
                                }
                                PresenceResolution::PresentUnresolved { reason }
                                | PresenceResolution::GenuinelyAbsent { reason } => {
                                    // Either present-but-unresolvable (no false
                                    // "no longer present") or genuinely absent →
                                    // stop safely with the classified reason.
                                    step_state.status = "blocked".into();
                                    step_state.blockers.push(reason.clone());
                                    run.step_states[index] = step_state.clone();
                                    run.status = "blocked".into();
                                    run.blocked_reason = Some(reason);
                                    events.push(step_blocked_event(&run, &step_state));
                                    break;
                                }
                            }
                        } else {
                            step_state.status = "blocked".into();
                            let reason = summary
                                .ambiguity_reasons
                                .first()
                                .cloned()
                                .or_else(|| summary.blockers.first().cloned())
                                .unwrap_or_else(|| "target not safely resolved".into());
                            step_state.blockers.push(reason.clone());
                            run.step_states[index] = step_state.clone();
                            run.status = if ambiguous { "paused" } else { "blocked" }.into();
                            // Task 9.4 (Requirements 11, 22): when the stop is due
                            // to ambiguity (multiple matches / needs clarification)
                            // KRIA pauses and asks — never guesses. Emit the
                            // additive `AmbiguityNoGuess` telemetry (flag-ON only).
                            if self.safety_polish.is_enabled() && ambiguous {
                                events.push(ambiguity_no_guess_event(
                                    summary.ambiguity_count.max(2),
                                    &reason,
                                    GuiAmbiguityDecisionPoint::PerStepReobserve,
                                    None,
                                ));
                            }
                            run.blocked_reason = Some(reason);
                            events.push(step_blocked_event(&run, &step_state));
                            break;
                        }
                    }

                    // safety_only: create state + gate but never start an action.
                    if !allows_execution {
                        self.reset_step_execution_state(state);
                        self.handle_safety_gate(
                            events,
                            request,
                            &current_context,
                            goal_contract,
                            &sub_plan,
                            readiness_validation,
                            &step_plan_id,
                            state,
                        )
                        .await;
                        step_state.status = "blocked".into();
                        step_state
                            .warnings
                            .push("execution_mode is safety_only; no action started".into());
                        run.step_states[index] = step_state.clone();
                        run.status = "paused".into();
                        run.blocked_reason =
                            Some("execution_mode is safety_only".into());
                        events.push(step_blocked_event(&run, &step_state));
                        break;
                    }

                    self.reset_step_execution_state(state);
                    self.handle_safety_gate(
                        events,
                        request,
                        &current_context,
                        goal_contract,
                        &sub_plan,
                        readiness_validation,
                        &step_plan_id,
                        state,
                    )
                    .await;

                    // Pull safe IDs from the per-step state for the step record.
                    step_state.proposal_id = json_str(&state.safety_gate, "proposal_id");
                    step_state.proposal_hash = json_str(&state.safety_gate, "proposal_hash");
                    step_state.hitl_decision_id = json_str(&state.hitl_decision, "decision_id");
                    step_state.execution_id = json_str(&state.execution_result, "execution_id");
                    step_state.verification_id = state
                        .verification_result
                        .as_ref()
                        .map(|verification| verification.verification_id.clone());
                    step_state.recovery_id = json_str(&state.recovery_assessment, "recovery_id");
                    step_state.completed_at_ms = now_ms();

                    let verification_status = state
                        .verification_result
                        .as_ref()
                        .map(|verification| verification.status.clone());
                    let recovery_status = json_str(&state.recovery_result, "status");

                    let step_succeeded = matches!(state.status.as_str(), "completed" | "recovered");
                    let needs_approval = matches!(
                        state.status.as_str(),
                        "needs_approval" | "approved_for_step7"
                    ) && state.execution_result.is_none();

                    // Task 1.3 (Requirement 21.4): track consecutive verification
                    // failures so a repeatedly-failing verification aborts rather
                    // than loops. Only counts turns where a verification actually
                    // ran; a pass resets the streak. (Inert while the flag is OFF.)
                    if verification_status.is_some() && !needs_approval {
                        budget_tracker.note_verification(step_succeeded);
                    }

                    if step_succeeded {
                        step_state.status = "completed".into();
                        step_state.can_continue = true;
                        let receipt = self.workflow_receipt(
                            &run,
                            &step_state,
                            "completed",
                            verification_status.clone(),
                            recovery_status.clone(),
                            "Step executed and verified (or safely recovered) before continuing.",
                            &goal_contract.prompt_hash,
                        );
                        run.completed_step_receipts.push(receipt.clone());
                        if recovery_status.is_some() {
                            run.recovery_summary = recovery_status.clone();
                        }
                        run.step_states[index] = step_state.clone();
                        events.push(step_completed_event(&run, &step_state, &receipt));
                        self.save_workflow_checkpoint(
                            events,
                            &mut run,
                            steps.get(index + 1).map(|next| next.step_id.clone()),
                            index + 1,
                            &current_context,
                            state,
                        );
                        // Task 3.2 (Requirement 2.1): record whether this
                        // executed step changed GUI state, so the NEXT step
                        // re-observes and resolves against the fresh screen.
                        previous_step_changed_state =
                            workflow_step_is_state_changing(&step.step_type);
                    } else if needs_approval {
                        step_state.status = "awaiting_approval".into();
                        run.step_states[index] = step_state.clone();
                        run.status = "paused".into();
                        run.blocked_reason = Some("step requires HITL approval".into());
                        events.push(step_blocked_event(&run, &step_state));
                        break;
                    } else {
                        step_state.status = "blocked".into();
                        let reason = state
                            .blocker
                            .as_ref()
                            .map(|blocker| blocker.reason.clone())
                            .unwrap_or_else(|| "step did not complete safely".into());
                        step_state.blockers.push(reason.clone());
                        run.step_states[index] = step_state.clone();
                        run.status = "blocked".into();
                        run.blocked_reason = Some(reason);
                        if recovery_status.is_some() {
                            run.recovery_summary = recovery_status;
                        }
                        events.push(step_blocked_event(&run, &step_state));
                        break;
                    }
                }
            }
        }

        if run.status == "running" {
            run.status = "completed".into();
            run.current_step_index = run.step_count.saturating_sub(1);
        }
        // Step 11: save a checkpoint reflecting the final completed/paused/blocked
        // state (covers pause-for-HITL and block-before-next-step cases).
        let final_index = run.current_step_index;
        let final_step_id = steps.get(final_index).map(|step| step.step_id.clone());
        self.save_workflow_checkpoint(
            events,
            &mut run,
            final_step_id,
            final_index,
            &current_context,
            state,
        );
        events.push(run.run_terminal_event());

        // Task 9.4 (Requirement 13): verify-and-stop terminates after
        // verification. When the plan is a verify-and-stop plan (one or more
        // non-state-changing observe/verify steps terminating in a VerifyState
        // and containing NO executable step), the turn must observe → verify →
        // then STOP with no further action. Emit the additive
        // `VerifyAndStopTerminated` telemetry asserting that ZERO state-changing
        // actions executed during the turn, so the "stop after verification"
        // contract is inspectable. Emitted ONLY when the `gui_cog_safety_polish`
        // flag is ON; while OFF the turn is byte-for-byte unchanged.
        if self.safety_polish.is_enabled() {
            let step_types: Vec<String> =
                steps.iter().map(|step| step.step_type.clone()).collect();
            if is_verify_and_stop_plan(&step_types) {
                let state_changing_executed = run
                    .completed_step_receipts
                    .iter()
                    .filter(|receipt| {
                        steps
                            .get(receipt.step_index)
                            .map(|step| workflow_step_is_state_changing(&step.step_type))
                            .unwrap_or(false)
                    })
                    .count();
                let terminal_step_type = step_types
                    .last()
                    .cloned()
                    .unwrap_or_else(|| "VerifyState".into());
                events.push(verify_and_stop_event(
                    state_changing_executed,
                    &terminal_step_type,
                    &run.status,
                ));
            }
        }

        // Reflect the workflow outcome in the turn status/reply.
        state.status = match run.status.as_str() {
            "completed" => "completed".into(),
            "paused" => "paused".into(),
            _ => "blocked".into(),
        };
        state.reply = match run.status.as_str() {
            "completed" => format!(
                "Workflow completed {} verified step(s) safely, one bound action at a time.",
                run.completed_step_receipts.len()
            ),
            "paused" => run
                .blocked_reason
                .clone()
                .map(|reason| format!("Workflow paused safely: {reason}"))
                .unwrap_or_else(|| "Workflow paused safely.".into()),
            _ => {
                // Task 5 (Issue #12): map the bounded-guard stop reason to the
                // UPSTREAM root cause (target-not-found / field-not-resolvable /
                // app-not-focused / needs-clarification / load-not-ready) and
                // surface THAT, instead of an opaque "screen state repeated N
                // times". The bounded guard itself is unchanged. Flag-OFF keeps
                // the raw reason byte-for-byte.
                let raw = run.blocked_reason.clone();
                let classified = if clear_failure_enabled() {
                    raw.as_deref().and_then(|reason| {
                        let failing = run
                            .step_states
                            .iter()
                            .rev()
                            .find(|s| matches!(s.status.as_str(), "blocked" | "aborted"));
                        classify_gui_stop_root_cause(
                            failing.map(|s| s.step_type.as_str()),
                            failing.map(|s| s.blockers.as_slice()).unwrap_or(&[]),
                            reason,
                        )
                    })
                } else {
                    None
                };
                match classified {
                    Some(root) => {
                        state.blocker = Some(GuiBlocker::new(root.kind, root.message.clone()));
                        format!("Workflow stopped safely: {}", root.message)
                    }
                    None => raw
                        .map(|reason| format!("Workflow stopped safely: {reason}"))
                        .unwrap_or_else(|| "Workflow stopped safely.".into()),
                }
            }
        };
        state.workflow_run = Some(run.summary_json());
    }

    #[allow(clippy::too_many_arguments)]
    fn save_workflow_checkpoint(
        &self,
        events: &mut GuiEventStream,
        run: &mut GuiWorkflowRun,
        pending_step_id: Option<String>,
        pending_index: usize,
        context: &GuiContext,
        state: &mut RuntimeState,
    ) {
        run.current_step_index = pending_index.min(run.step_count.saturating_sub(1));
        let pending = GuiCheckpointPending {
            pending_step_id,
            pending_proposal_id: json_str(&state.safety_gate, "proposal_id"),
            pending_proposal_hash: json_str(&state.safety_gate, "proposal_hash"),
            pending_target_hash: json_str(&state.target, "target_hash"),
            pending_stable_target_identity_hash: None,
            pending_hitl_request_id: json_str(&state.safety_gate, "request_id"),
            approved_decision_id: json_str(&state.hitl_decision, "decision_id"),
            approved_decision_hash: json_str(&state.hitl_decision, "proposal_hash"),
        };
        let screen_prefix: Option<String> = context
            .observation
            .screen_hash
            .as_ref()
            .map(|hash| hash.chars().take(16).collect());
        let checkpoint = build_checkpoint(
            run,
            &pending,
            &context.observation_id,
            &context.context_id,
            screen_prefix,
            None,
            now_ms(),
            WORKFLOW_CHECKPOINT_TTL_MS,
        );
        events.push(checkpoint.saved_event_payload());
        state.workflow_checkpoint = Some(checkpoint.summary_json());
    }

    fn reset_step_execution_state(&self, state: &mut RuntimeState) {
        state.safety_gate = None;
        state.hitl_decision = None;
        state.action = None;
        state.execution_result = None;
        state.execution_blocker = None;
        state.verification_result = None;
        state.recovery_assessment = None;
        state.recovery_result = None;
        state.blocker = None;
    }

    #[allow(clippy::too_many_arguments)]
    fn workflow_receipt(
        &self,
        run: &GuiWorkflowRun,
        step_state: &workflow_runtime::GuiWorkflowStepState,
        status: &str,
        verification_status: Option<String>,
        recovery_status: Option<String>,
        safe_summary: &str,
        prompt_hash: &str,
    ) -> GuiWorkflowStepReceipt {
        let action_type = step_state.step_type.clone();
        let risk_level = run.risk_level.clone();
        let side_effect_kind =
            workflow_runtime::side_effect_kind_for(&action_type, &risk_level).to_string();
        let receipt_hash = workflow_runtime::compute_receipt_hash(
            &run.workflow_run_id,
            &step_state.step_id,
            step_state.step_index,
            step_state.proposal_hash.as_deref(),
            step_state.execution_id.as_deref(),
            verification_status.as_deref(),
        );
        GuiWorkflowStepReceipt {
            receipt_id: run.receipt_id(step_state.step_index),
            workflow_run_id: run.workflow_run_id.clone(),
            step_id: step_state.step_id.clone(),
            step_index: step_state.step_index,
            step_type: step_state.step_type.clone(),
            status: status.into(),
            proposal_id: step_state.proposal_id.clone(),
            action_type: Some(action_type),
            risk_level: Some(risk_level),
            side_effect_kind,
            target_hash: step_state.proposal_hash.clone(),
            proposal_hash: step_state.proposal_hash.clone(),
            execution_id: step_state.execution_id.clone(),
            verification_id: step_state.verification_id.clone(),
            verification_status,
            recovery_id: step_state.recovery_id.clone(),
            recovery_status,
            started_at_ms: step_state.started_at_ms,
            completed_at_ms: step_state.completed_at_ms,
            safe_summary: sanitize_event_text(safe_summary),
            receipt_hash,
            prompt_hash: prompt_hash.chars().take(96).collect(),
        }
    }

    fn resolve_step_target_for_workflow(
        &self,
        events: &mut GuiEventStream,
        step: &self::llm_planner::GuiTypedPlanStep,
        sub_plan: &self::llm_planner::GuiLlmPlan,
        readiness_validation: &GuiPlanValidationReport,
        context: &GuiContext,
        step_plan_id: &str,
        state: &mut RuntimeState,
    ) -> GuiTargetResolutionSummary {
        events.push(serde_json::json!({
            "type": "TargetResolutionStarted",
            "plan_id": step_plan_id,
            "validation_id": readiness_validation.validation_id.as_deref(),
            "mode": "step10_workflow_step",
        }));
        let summary = if workflow_step_requires_target(&step.step_type) {
            resolve_plan_targets(sub_plan, readiness_validation, context, step_plan_id)
        } else {
            // App/window/key steps need no control target; synthesize a resolved
            // summary so the safety gate can proceed without a control.
            GuiTargetResolutionSummary {
                resolution_id: format!("resolution-{step_plan_id}"),
                plan_id: step_plan_id.to_string(),
                validation_id: readiness_validation.validation_id.clone(),
                goal_contract_id: sub_plan
                    .goal_contract_id
                    .clone()
                    .or(readiness_validation.goal_contract_id.clone()),
                context_id: context.context_id.clone(),
                observation_id: context.observation_id.clone(),
                status: "resolved".into(),
                results: Vec::new(),
                resolved_target: None,
                can_proceed_to_safety_gate: true,
                can_execute: false,
                blocker_count: 0,
                blockers: Vec::new(),
                ambiguity_count: 0,
                ambiguity_reasons: Vec::new(),
                confidence: step.confidence,
                prompt_hash: sub_plan.prompt_hash.clone(),
            }
        };
        events.push(summary.event_payload());
        state.target_resolution = Some(summary.summary_json());
        if let Some(target) = &summary.resolved_target {
            state.target = Some(serde_json::json!({
                "label": target.label,
                "role": target.role,
                "target_type": target.target_kind,
                "control_id": target.control_id,
                "target_hash": target.target_hash,
                "bounds": target.bounds.clone(),
                "confidence": summary.confidence,
                "can_execute": false,
            }));
        }
        summary
    }

    fn response_json(
        &self,
        request: &GuiTurnRequest,
        context: &GuiContext,
        goal_contract: &GuiGoalContract,
        intent: &GuiCognitionIntent,
        plan_id: &str,
        planner_selection: &GuiPlannerSelection,
        plan_validation: &GuiPlanValidationReport,
        state: &RuntimeState,
    ) -> serde_json::Value {
        let observation = &context.observation;
        let perception = perception_summary_json(observation);
        let plan = plan_summary_json(plan_id, planner_selection);
        let mut response = serde_json::json!({
            "status": state.status,
            "reply": state.reply,
            "gui_cognition": {
                "mode_id": "gui_cognition",
                "workflow_id": request.workflow_id,
                "turn_id": request.turn_id,
                "observation_id": observation.observation_id,
                "context_id": context.context_id,
                "path": request.route_path,
                "llm_tool_loop": request.llm_tool_loop,
                "execution_environment": request.execution_environment.summary_json(),
                "intent": intent.kind.as_str(),
                "risk_level": intent.risk_level,
                "requires_approval": intent.requires_approval,
                "risk_reasons": intent.risk_reasons,
                "perception": perception,
                "context": context.context_summary(),
                "goal_contract": goal_contract.response_summary(),
                "planner": planner_summary_json(planner_selection),
                "plan": plan,
                "plan_validation": plan_validation.summary_json(plan_id),
                "target_resolution": state.target_resolution.clone().unwrap_or(serde_json::Value::Null),
                "target": state.target.clone().unwrap_or(serde_json::Value::Null),
                "safety_gate": state.safety_gate.clone().unwrap_or(serde_json::Value::Null),
                "hitl_decision": state.hitl_decision.clone().unwrap_or(serde_json::Value::Null),
                "action": state.action.clone().unwrap_or(serde_json::Value::Null),
                "execution": state.execution_result.clone().unwrap_or(serde_json::Value::Null),
                "action_backend": state.action_backend.as_ref().map(action_backend_summary).unwrap_or(serde_json::Value::Null),
                "preconditions": state.preconditions.clone().unwrap_or(serde_json::Value::Null),
                "execution_blocker": state.execution_blocker.clone().unwrap_or(serde_json::Value::Null),
                "verification": state
                    .verification_result
                    .as_ref()
                    .map(GuiPostActionVerificationResult::summary_json)
                    .or_else(|| state.verification.as_ref().map(verification_summary))
                    .unwrap_or(serde_json::Value::Null),
                "recovery_assessment": state.recovery_assessment.clone().unwrap_or(serde_json::Value::Null),
                "recovery": state.recovery_result.clone().unwrap_or(serde_json::Value::Null),
                "workflow_run": state.workflow_run.clone().unwrap_or(serde_json::Value::Null),
                "workflow_checkpoint": state.workflow_checkpoint.clone().unwrap_or(serde_json::Value::Null),
                "blocker": state.blocker.as_ref().map(blocker_summary).unwrap_or(serde_json::Value::Null),
            }
        });
        // Task 9.2: surface the append-only sanitized audit ledger in the turn
        // response (inspectable read API) only when the `gui_cog_safety_polish`
        // flag is ON. While OFF the response is byte-for-byte unchanged.
        if self.safety_polish.is_enabled() {
            if let Some(gui) = response
                .get_mut("gui_cognition")
                .and_then(|value| value.as_object_mut())
            {
                gui.insert("ledger".into(), state.action_ledger.summary_json());
            }
        }
        response
    }
}

const WORKFLOW_CHECKPOINT_TTL_MS: i64 = 10 * 60 * 1000;

fn single_step_plan(
    plan: &self::llm_planner::GuiLlmPlan,
    step: &self::llm_planner::GuiTypedPlanStep,
) -> self::llm_planner::GuiLlmPlan {
    let mut sub_plan = plan.clone();
    sub_plan.typed_steps = vec![step.clone()];
    sub_plan.steps = Vec::new();
    sub_plan
}

/// Task 3.3: readiness predicate for the bounded readiness wait. The fresh
/// observation is "ready" for the next step when:
/// - it carries some useful perception signal at all
///   ([`has_useful_signal`](self::perception::GuiObservationSnapshot::has_useful_signal)),
///   AND
/// - if the step names a concrete window/app (`expected_hint`), that
///   window/app is currently observable
///   ([`window_or_app_observable`](self::perception::GuiObservationSnapshot::window_or_app_observable)).
///
/// With no concrete hint, a useful signal alone is enough (the page/screen has
/// settled). This is the minimal readiness check Task 3.3 needs; the richer
/// present-vs-genuinely-absent distinction is Task 3.4.
fn step_ready(context: &GuiContext, expected_hint: Option<&str>) -> bool {
    let observation = &context.observation;
    if !observation.has_useful_signal() {
        return false;
    }
    match expected_hint {
        Some(hint) => observation.window_or_app_observable(hint),
        None => true,
    }
}

/// Task 3.4: the expected role family for a required-target step, used by the
/// tolerant presence predicate to look at the right kind of control on the
/// fresh screen. This maps the step's SEMANTIC target (a FocusField targets a
/// text/entry control; a ClickControl targets a button/checkbox/link/…) — it is
/// NOT the action kind being used as the target name (the Task 2.5 invariant):
/// presence is still decided by matching real observed controls' role + label.
/// An empty list matches any role.
fn workflow_step_role_groups(step_type: &str) -> &'static [&'static str] {
    match step_type {
        "FocusField" | "TypeText" => &["text", "entry", "edit", "combo", "search"],
        "ClickControl" => &["button", "check", "radio", "menu", "tab", "link", "toggle"],
        _ => &[],
    }
}

/// Task 3.4: the sanitized descriptor used to report the present/absent
/// decision — the control hint first, then the window/app hint as a fallback.
fn presence_expected_hint(step: &self::llm_planner::GuiTypedPlanStep) -> Option<String> {
    step.target_control_hint
        .as_deref()
        .or(step.target_window_hint.as_deref())
        .or(step.target_app_hint.as_deref())
        .map(str::trim)
        .filter(|hint| !hint.is_empty())
        .map(str::to_string)
}

/// Task 3.4 (Requirement 2.3/2.4, Property 2/8): tolerant presence predicate —
/// is the step's expected target OBSERVABLE on the fresh context? Window/app
/// steps use
/// [`window_or_app_observable`](self::perception::GuiObservationSnapshot::window_or_app_observable);
/// control steps use
/// [`control_descriptor_observable`](self::perception::GuiObservationSnapshot::control_descriptor_observable),
/// which matches by role + label and is TOLERANT of a changed `control_id` after
/// a re-render. This is presence EVIDENCE only — it never authorizes execution
/// (the resolver still gates that) — and is decided from real observation, never
/// the action kind.
/// Task 0 ladder (degenerate-plan guard): whether a strictly-valid LLM plan
/// actually PURSUES the prompt's explicit concrete app action. Returns `true`
/// (accept the plan) for every goal EXCEPT an `open_app` / `switch_window`
/// contract that names a target app but whose plan contains NO `OpenApp` /
/// `SwitchWindow` step — that plan would never act, so it is rerouted to the
/// deterministic plan. The check is deliberately narrow (only the two concrete
/// app-launch/activation actions with a resolved target) so it never overrides
/// a legitimate clarification or an observe/analyze plan, and never lenient-
/// scrapes — it only detects an action the plan structurally omits.
fn llm_plan_pursues_goal_action(
    plan: &self::llm_planner::GuiLlmPlan,
    request: &GuiLlmPlannerRequest,
) -> bool {
    let goal = request.contract.action_type.as_str();
    let needs_step = match goal {
        "open_app" => "OpenApp",
        "switch_window" => "SwitchWindow",
        // Any other goal: do not apply the degenerate-plan guard.
        _ => return true,
    };
    // Only guard when the contract resolved a concrete target app/window — with
    // no target the planner may legitimately ask for clarification.
    let has_target = request
        .contract
        .target_app_hint
        .as_deref()
        .map(|v| !v.trim().is_empty())
        .unwrap_or(false)
        || request
            .contract
            .target_window_hint
            .as_deref()
            .map(|v| !v.trim().is_empty())
            .unwrap_or(false);
    if !has_target {
        return true;
    }
    // Accept only when the plan actually contains the matching action step
    // (OpenApp also satisfies a switch_window goal — "open or switch to").
    plan.typed_steps.iter().any(|step| {
        step.step_type == needs_step
            || (needs_step == "SwitchWindow" && step.step_type == "OpenApp")
            || (needs_step == "OpenApp" && step.step_type == "SwitchWindow")
    })
}

/// Task 3 (Issue #1, Requirement 3): whether a real window-focus backend handler
/// can run in the current session. The window-focus chain
/// ([`select_focus_backends`]) is ordered compositor-native-first.
///
/// - [`WindowFocusBackend::GnomeBridge`] is the compositor-native
///   activate-by-window-identity path. On Wayland (and GNOME-on-X11) KRIA
///   activates the target window via `gio launch <.desktop>` — the SAME
///   mechanism the `open_application` tool already uses, which raises/activates
///   an existing single-instance window on Wayland (the X11-only
///   `xdotool windowactivate` path fails there). It is therefore reachable when
///   the `gio` binary exists on PATH AND the session is a known graphical one
///   (`wayland`/`x11`) AND the `gui_cog_wayland_focus` flag is ON. This makes the
///   routing prefer GnomeBridge over a blind Alt+Tab fallback for SwitchWindow.
/// - [`WindowFocusBackend::Portal`] is not implemented yet → always unavailable.
/// - [`WindowFocusBackend::UinputAltTab`] is the input-substrate-backed key-based
///   fallback; available when the deterministic input substrate can execute.
/// - [`WindowFocusBackend::X11Wmctrl`] handler is not wired through this predicate
///   (the legacy X11 path executes elsewhere) → unavailable here.
///
/// The `flag_enabled` parameter mirrors the `gui_cog_wayland_focus` gate: while
/// the flag is OFF the Wayland-native GnomeBridge path is never reported
/// available, so SwitchWindow behavior is preserved byte-for-byte (the routing
/// itself is also gated on the flag; this is the defensive, testable contract).
/// Availability is NEVER fabricated — a backend is reported available only when a
/// real activation path can run; the re-observe verification decides verified vs.
/// inconclusive/failed afterwards.
fn window_focus_backend_available(
    backend: WindowFocusBackend,
    status: &GuiActionBackendStatus,
    flag_enabled: bool,
) -> bool {
    window_focus_backend_available_inner(
        backend,
        &status.session_type,
        status.can_execute_actions,
        flag_enabled,
        gio_binary_available,
    )
}

/// Testable core of [`window_focus_backend_available`]: the `gio` presence probe
/// is injected so the session/flag decision logic can be unit-tested without a
/// live binary on PATH.
fn window_focus_backend_available_inner<F>(
    backend: WindowFocusBackend,
    session_type: &str,
    can_execute_actions: bool,
    flag_enabled: bool,
    gio_present: F,
) -> bool
where
    F: Fn() -> bool,
{
    match backend {
        WindowFocusBackend::UinputAltTab => can_execute_actions,
        WindowFocusBackend::GnomeBridge => {
            if !flag_enabled {
                return false;
            }
            let session = session_type.trim().to_ascii_lowercase();
            (session == "wayland" || session == "x11") && gio_present()
        }
        // Portal not implemented; the X11-only wmctrl handler executes via the
        // legacy path, not this predicate. Never claim availability we cannot back.
        WindowFocusBackend::Portal | WindowFocusBackend::X11Wmctrl => false,
    }
}

/// Whether the `gio` binary (used for the Wayland-native `gio launch <.desktop>`
/// activate-by-identity path) is reachable. Checks the common absolute locations
/// first, then scans `PATH`. Mirrors the trusted lookup used by the Linux OS
/// intent backend (`gio launch` app activation).
fn gio_binary_available() -> bool {
    if ["/usr/bin/gio", "/bin/gio"]
        .iter()
        .any(|candidate| std::path::Path::new(candidate).exists())
    {
        return true;
    }
    if let Some(paths) = std::env::var_os("PATH") {
        for dir in std::env::split_paths(&paths) {
            if dir.join("gio").exists() {
                return true;
            }
        }
    }
    false
}

/// Task 2.1 (Requirement 2): whether the inferred target app is already the
/// ACTIVE window, observable in a VISIBLE but non-active window, or not present
/// at all in the current desktop context — the signal the auto-prerequisite
/// pass ([`apply_auto_prerequisite`]) uses to decide between no-op,
/// `SwitchWindow`, and `OpenApp`. Active-window matching mirrors the resolver's
/// `active_context_matches_app_hint` (active app/label substring + the
/// `browser` generic alias group); the visible-set check reuses the
/// observation's alias-tolerant `window_or_app_observable`.
fn app_observability(context: &GuiContext, app_hint: &str) -> AppObservability {
    if active_context_matches_app_hint(context, app_hint) {
        AppObservability::Active
    } else if context.observation.window_or_app_observable(app_hint) {
        AppObservability::VisibleNotActive
    } else {
        AppObservability::NotPresent
    }
}

/// Whether the inferred app hint matches the ACTIVE/focused window only (Task
/// 2.1). Mirrors the resolver's private `active_context_matches_app_hint`: a
/// case-insensitive substring match against the active window's app_name/label,
/// with the `browser` hint expanded to the common browser alias group.
fn active_context_matches_app_hint(context: &GuiContext, app_hint: &str) -> bool {
    let hint = app_hint.trim().to_lowercase();
    if hint.is_empty() {
        return false;
    }
    let active_app = context
        .active_window
        .app_name
        .as_deref()
        .unwrap_or_default()
        .to_lowercase();
    let active_label = context.active_window.label.to_lowercase();
    if hint == "browser" {
        return ["chrome", "chromium", "firefox", "brave", "browser"]
            .iter()
            .any(|needle| active_app.contains(needle) || active_label.contains(needle));
    }
    active_app.contains(&hint) || active_label.contains(&hint)
}

fn step_target_observable(context: &GuiContext, step: &self::llm_planner::GuiTypedPlanStep) -> bool {
    match step.step_type.as_str() {
        "SwitchWindow" | "OpenApp" => step
            .target_window_hint
            .as_deref()
            .or(step.target_app_hint.as_deref())
            .map(str::trim)
            .filter(|hint| !hint.is_empty())
            .map(|hint| context.observation.window_or_app_observable(hint))
            .unwrap_or(false),
        _ => {
            let label = step
                .target_control_hint
                .as_deref()
                .map(str::trim)
                .unwrap_or("");
            context
                .observation
                .control_descriptor_observable(label, workflow_step_role_groups(&step.step_type))
        }
    }
}

fn json_str(value: &Option<serde_json::Value>, key: &str) -> Option<String> {
    value
        .as_ref()
        .and_then(|object| object.get(key))
        .and_then(serde_json::Value::as_str)
        .map(|text| text.to_string())
}

fn sanitize_event_text(value: &str) -> String {
    value
        .replace('\n', " ")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .chars()
        .take(180)
        .collect()
}

/// Whether the prompt (and/or its extracted control hint) targets a password /
/// secure-entry field, so any typed payload destined for it must be treated as
/// secret at planning time (Task 6.2, Requirement 5/15). Reads ONLY the
/// lowercased prompt + the sanitized control hint — never a raw secret. English-
/// scoped secure-field phrasing per Requirement 26.3.
fn prompt_targets_secure_field(lower_message: &str, control_hint: Option<&str>) -> bool {
    const SECURE_PHRASES: &[&str] = &[
        "password field",
        "password input",
        "password box",
        "password entry",
        "passphrase field",
        "passphrase input",
        "secure entry",
        "secure field",
        "secure input",
        "secure text field",
    ];
    if SECURE_PHRASES
        .iter()
        .any(|phrase| lower_message.contains(phrase))
    {
        return true;
    }
    control_hint
        .map(|hint| is_password_or_secure_field(hint, hint, false))
        .unwrap_or(false)
}

/// Whether the OBSERVED context exposes a password / secure-entry field as an
/// executable target (Task 6.2, Requirement 5/15). When a typed payload would
/// land in such a field, the payload must be treated as secret even if the
/// prompt never says "password" (the secure-entry signal comes from the AT-SPI
/// role, e.g. "password text", reported by the perception layer). Reads ONLY the
/// sanitized observed control descriptors — never a raw secret.
fn context_has_secure_text_field(context: &GuiContext) -> bool {
    let focused_secure = context
        .observation
        .cursor_focus
        .focused_control_role
        .as_deref()
        .map(|role| is_password_or_secure_field(role, "", false))
        .unwrap_or(false);
    focused_secure
        || context
            .executable_text_fields()
            .iter()
            .any(|field| is_password_or_secure_field(&field.role, &field.name, false))
}

fn role_for_action(kind: &GuiActionKind) -> &'static str {
    match kind {
        GuiActionKind::OpenApp => "application",
        GuiActionKind::SwitchWindow => "window",
        GuiActionKind::FocusField | GuiActionKind::FillField | GuiActionKind::TypeText => "text",
        GuiActionKind::ClickControl => "push button",
        GuiActionKind::PressKey | GuiActionKind::Hotkey => "focused_context",
        GuiActionKind::Scroll => "scrollable",
        GuiActionKind::Copy | GuiActionKind::Paste => "focused_context",
        // Task 6.1 typed primitives.
        GuiActionKind::ClearField | GuiActionKind::SelectAll => "text",
        GuiActionKind::SetCheckbox => "check box",
        GuiActionKind::CloseDialog => "dialog",
        GuiActionKind::InAppSearch => "text",
    }
}

fn execution_hint_for_action(kind: &GuiActionKind) -> &'static str {
    match kind {
        GuiActionKind::OpenApp => "open_application",
        GuiActionKind::SwitchWindow => "focus_window",
        GuiActionKind::FocusField | GuiActionKind::ClickControl => "click_ui_element",
        GuiActionKind::FillField | GuiActionKind::TypeText => "fill_form_field",
        GuiActionKind::PressKey | GuiActionKind::Hotkey | GuiActionKind::Copy | GuiActionKind::Paste => {
            "press_shortcut"
        }
        GuiActionKind::Scroll => "scroll",
        // Task 6.1 typed primitives: clear/select-all are keyboard shortcuts on
        // the focused control; checkbox/dialog-close are control clicks;
        // in-app-search focuses and fills the search box. All route through the
        // Wayland-capable input backend (uinput) via these existing hints.
        GuiActionKind::ClearField | GuiActionKind::SelectAll => "press_shortcut",
        GuiActionKind::SetCheckbox | GuiActionKind::CloseDialog => "click_ui_element",
        GuiActionKind::InAppSearch => "fill_form_field",
    }
}

/// Task 2 (Issue #3): deterministic Ctrl+L browser address-bar path.
/// When a TypeText/FillField step carries the browser address-bar sentinel
/// (`BROWSER_ADDRESSBAR_HINT`), the address bar is focused and typed into
/// ATOMICALLY by the executor (Ctrl+L then synthetic-keystroke type via uinput,
/// no a11y / no vision), so route it to `browser_addressbar_type` instead of
/// resolving a control by label. All other actions delegate to
/// `execution_hint_for_action`.
fn gui_execution_hint_for(kind: &GuiActionKind, target_name: &str) -> &'static str {
    if matches!(kind, GuiActionKind::TypeText | GuiActionKind::FillField)
        && target_name == llm_planner::BROWSER_ADDRESSBAR_HINT
    {
        "browser_addressbar_type"
    } else {
        execution_hint_for_action(kind)
    }
}

struct RuntimeState {
    status: String,
    reply: String,
    target_resolution: Option<serde_json::Value>,
    target: Option<serde_json::Value>,
    safety_gate: Option<serde_json::Value>,
    hitl_decision: Option<serde_json::Value>,
    action: Option<serde_json::Value>,
    execution_result: Option<serde_json::Value>,
    action_backend: Option<GuiActionBackendStatus>,
    execution_blocker: Option<serde_json::Value>,
    preconditions: Option<serde_json::Value>,
    verification: Option<GuiVerificationReport>,
    verification_result: Option<GuiPostActionVerificationResult>,
    recovery_assessment: Option<serde_json::Value>,
    recovery_result: Option<serde_json::Value>,
    workflow_run: Option<serde_json::Value>,
    workflow_checkpoint: Option<serde_json::Value>,
    blocker: Option<GuiBlocker>,
    /// Task 9.2: append-only sanitized audit ledger of executed GUI actions.
    /// Populated only when the `gui_cog_safety_polish` flag is ON.
    action_ledger: GuiActionLedger,
}

impl RuntimeState {
    fn new(reply: String) -> Self {
        Self {
            status: "ok".into(),
            reply,
            target_resolution: None,
            target: None,
            safety_gate: None,
            hitl_decision: None,
            action: None,
            execution_result: None,
            action_backend: None,
            execution_blocker: None,
            preconditions: None,
            verification: None,
            verification_result: None,
            recovery_assessment: None,
            recovery_result: None,
            workflow_run: None,
            workflow_checkpoint: None,
            blocker: None,
            action_ledger: GuiActionLedger::new(),
        }
    }
}

/// Task 9.3 (Requirements 10, 11, 12, 13, 14, 15, 22, 23): build the additive
/// `ApprovalLifecycle` telemetry event for an approval-gated proposal. Emitted
/// ONLY when the `gui_cog_safety_polish` flag is ON. It makes the approval
/// lifecycle inspectable — the proposal pauses, a decision arrives (or is
/// awaited), and the action either executes on a FRESH authorizing decision or
/// stays gated/blocked. It surfaces the decision verdict, whether the bound
/// proposal/target hashes matched (`hash_matched`), whether the decision was
/// fresh (`fresh` — not expired/stale), whether it was authorizing, and the
/// carried decision id (the one threaded into execution). It NEVER carries a
/// secret payload, the raw prompt, or coordinates, and it never alters control
/// flow (purely observational telemetry).
fn approval_lifecycle_event(
    safety_gate: &GuiSafetyGateResult,
    decision: Option<&GuiHitlDecision>,
    outcome: &str,
    executed: bool,
    final_status: &str,
    environment: &str,
) -> serde_json::Value {
    let proposal = &safety_gate.proposal;
    let (verdict, decision_id, authorizing, hash_matched, fresh) = match decision {
        Some(decision) => (
            Some(decision.decision.clone()),
            Some(decision.decision_id.clone()),
            decision.can_authorize_step7,
            // A `hash_mismatch_rejected` verdict is precisely the case where the
            // decision's bound hashes did not match the proposal at decision
            // time; any other verdict carries the matching bound hashes.
            decision.decision != "hash_mismatch_rejected"
                && decision.proposal_hash == proposal.proposal_hash
                && decision.target_hash == proposal.target_hash,
            !matches!(decision.decision.as_str(), "expired" | "stale_rejected"),
        ),
        None => (None, None, false, false, false),
    };
    serde_json::json!({
        "type": "ApprovalLifecycle",
        "proposal_id": proposal.proposal_id,
        "request_id": proposal.request_id,
        "proposal_hash": proposal.proposal_hash,
        "target_hash": proposal.target_hash,
        "action_type": proposal.action_type,
        "risk_level": proposal.risk_level,
        "requires_user_approval": proposal.requires_user_approval,
        // The lifecycle always begins paused at the approval gate.
        "paused": true,
        "environment": environment,
        "decision_verdict": verdict,
        "decision_id": decision_id,
        "authorizing": authorizing,
        "hash_matched": hash_matched,
        "fresh": fresh,
        "outcome": outcome,
        "executed": executed,
        "final_status": final_status,
        "prompt_hash": proposal.prompt_hash,
        "can_execute": false,
    })
}

/// Task 1 (Issue #5): whether the deterministic goal-level approval gate is
/// active. Default ON; rollback via `KRIA_GUI_COG_GATE_DETERMINISM` set to a
/// falsy value (`0`/`false`/`no`/`off`/empty), which restores the prior
/// per-step-only approval gating byte-for-byte.
fn gate_determinism_enabled() -> bool {
    match std::env::var("KRIA_GUI_COG_GATE_DETERMINISM") {
        Ok(v) => {
            let v = v.trim().to_ascii_lowercase();
            !matches!(v.as_str(), "0" | "false" | "no" | "off" | "")
        }
        Err(_) => true,
    }
}

/// Task 5 (Issue #12): whether clear, root-cause failure reporting is active.
/// Default ON; rollback via `KRIA_GUI_COG_CLEAR_FAILURE` set to a falsy value
/// (`0`/`false`/`no`/`off`/empty), which restores the prior raw guard-reason
/// messaging byte-for-byte. An absent env value keeps the flag ON.
fn clear_failure_enabled() -> bool {
    clear_failure_enabled_lookup(|key| std::env::var(key).ok())
}

/// Testable core of [`clear_failure_enabled`] with an injectable lookup.
fn clear_failure_enabled_lookup<F>(lookup: F) -> bool
where
    F: Fn(&str) -> Option<String>,
{
    match lookup("KRIA_GUI_COG_CLEAR_FAILURE") {
        Some(v) => !matches!(
            v.trim().to_ascii_lowercase().as_str(),
            "0" | "false" | "no" | "off" | ""
        ),
        None => true,
    }
}

/// Task 6 (Issue #13): whether smart bounded recovery is active. Default ON;
/// rollback via `KRIA_GUI_COG_SMART_RECOVERY` set to a falsy value
/// (`0`/`false`/`no`/`off`/empty), which skips the recovery loop so the turn
/// stops on the unverified step (the pre-recovery behavior). An absent env value
/// keeps the flag ON.
fn smart_recovery_enabled() -> bool {
    smart_recovery_enabled_lookup(|key| std::env::var(key).ok())
}

/// Testable core of [`smart_recovery_enabled`] with an injectable lookup.
fn smart_recovery_enabled_lookup<F>(lookup: F) -> bool
where
    F: Fn(&str) -> Option<String>,
{
    match lookup("KRIA_GUI_COG_SMART_RECOVERY") {
        Some(v) => !matches!(
            v.trim().to_ascii_lowercase().as_str(),
            "0" | "false" | "no" | "off" | ""
        ),
        None => true,
    }
}

/// Issue (multi-step prompt regression): whether the capability ladder, instead
/// of DISCARDING a schema-valid LLM plan that merely omits the leading app
/// action, first tries to REPAIR it by prepending the inferred
/// `OpenApp`/`SwitchWindow` prerequisite (the same `apply_auto_prerequisite`
/// machinery used post-selection) and KEEPS the multi-step LLM plan when the
/// repair makes it pursue the goal action.
///
/// Why this matters: a prompt like "Open Chrome and create a new tab" produces
/// an LLM plan whose primitive ("new tab") may not lead with an explicit
/// `OpenApp`. The prior behavior discarded the whole plan and substituted the
/// open-only deterministic plan, so the second action ("new tab") was silently
/// dropped. With the merge, the app-open step is prepended and the LLM's extra
/// steps survive.
///
/// Default ON; an explicit falsy value (`0`/`false`/`no`/`off`/empty) in
/// `KRIA_GUI_COG_PLAN_PREREQ_MERGE` rolls the behavior back to the prior
/// discard-and-substitute path byte-for-byte.
fn plan_prereq_merge_enabled() -> bool {
    plan_prereq_merge_enabled_lookup(|key| std::env::var(key).ok())
}

/// Testable core of [`plan_prereq_merge_enabled`] with an injectable lookup.
fn plan_prereq_merge_enabled_lookup<F>(lookup: F) -> bool
where
    F: Fn(&str) -> Option<String>,
{
    match lookup("KRIA_GUI_COG_PLAN_PREREQ_MERGE") {
        Some(v) => !matches!(
            v.trim().to_ascii_lowercase().as_str(),
            "0" | "false" | "no" | "off" | ""
        ),
        None => true,
    }
}

/// Task 10 (Issue #8): whether the consolidated honest AT-SPI health signal
/// (`accessibility_resolution_trustworthy`) is surfaced in the observation
/// event. Default ON; an explicit falsy value (`0`/`false`/`no`/`off`/empty) in
/// `KRIA_GUI_COG_ATSPI_HEALTH` rolls back to the prior event payload
/// byte-for-byte (no trust field). Additive-only telemetry — the underlying
/// snapshot/confidence behavior is unchanged either way.
fn atspi_health_enabled() -> bool {
    atspi_health_enabled_lookup(|key| std::env::var(key).ok())
}

/// Testable core of [`atspi_health_enabled`] with an injectable lookup.
fn atspi_health_enabled_lookup<F>(lookup: F) -> bool
where
    F: Fn(&str) -> Option<String>,
{
    match lookup("KRIA_GUI_COG_ATSPI_HEALTH") {
        Some(v) => !matches!(
            v.trim().to_ascii_lowercase().as_str(),
            "0" | "false" | "no" | "off" | ""
        ),
        None => true,
    }
}

/// Task 5 (Issue #12): the user-facing ROOT CAUSE for a workflow stop, mapped
/// from the failing step + the bounded-guard reason. Replaces an opaque
/// "screen state repeated N times" / "re-observe budget reached" with an
/// actionable reason. `kind` is a stable tag for telemetry; `message` is the
/// sanitized user-facing reply. The bounded guard itself is UNCHANGED — only the
/// REPORTED reason is classified.
struct GuiStopRootCause {
    kind: &'static str,
    message: String,
}

/// Classify the upstream root cause of a workflow STOP from the failing step's
/// type + blockers + the raw guard reason. Returns `None` when no clearer
/// mapping applies (the caller then keeps the raw reason). Pure + heuristic over
/// already-sanitized text — no new probes, no behavior change.
fn classify_gui_stop_root_cause(
    step_type: Option<&str>,
    step_blockers: &[String],
    raw_reason: &str,
) -> Option<GuiStopRootCause> {
    let raw = raw_reason.to_ascii_lowercase();
    let blockers = step_blockers.join(" ").to_ascii_lowercase();
    let hay = format!("{raw} {blockers}");
    let step = step_type.unwrap_or("");

    // Ambiguity / clarification (no-guess) takes precedence.
    if hay.contains("clarif") || hay.contains("ambig") || hay.contains("multiple match") {
        return Some(GuiStopRootCause {
            kind: "needs_clarification",
            message: "I need clarification to choose the right target safely, so I stopped without guessing.".into(),
        });
    }

    // PressKey/focus requiring known focus → the app/field wasn't focused.
    if hay.contains("requires known focus") || hay.contains("not focused") || hay.contains("focus lost") {
        return Some(GuiStopRootCause {
            kind: "app_not_focused",
            message: "The target app/field wasn't focused in time, so I stopped safely instead of acting on the wrong window.".into(),
        });
    }

    // Readiness / still loading.
    if step == "WaitForState"
        || hay.contains("did not become ready")
        || hay.contains("not ready")
        || hay.contains("still loading")
        || hay.contains("become observable")
    {
        return Some(GuiStopRootCause {
            kind: "load_not_ready",
            message: "The screen/app wasn't ready (still loading) within the safe wait, so I stopped safely.".into(),
        });
    }

    // Target/control could not be found or resolved (incl. the flapping/budget
    // guards firing on an in-app control-resolution step).
    let target_signal = hay.contains("no longer present")
        || hay.contains("not present on the current screen")
        || hay.contains("could not be resolved")
        || hay.contains("no matching")
        || hay.contains("not safely resolved")
        || hay.contains("target not")
        || raw.contains("screen state repeated")
        || raw.contains("re-observe budget")
        || raw.contains("flapping");
    if target_signal {
        return match step {
            // Text/field steps: usually unresolvable because a11y is off / no
            // real vision yet (Tasks 8/10), not because the screen is "repeating".
            "FocusField" | "TypeText" | "FillField" | "ClearField" | "SelectAll" | "InAppSearch" => {
                Some(GuiStopRootCause {
                    kind: "field_not_resolvable_vision_unavailable",
                    message: "I couldn't reliably locate the input field on screen (accessibility/vision is limited here), so I stopped safely instead of guessing.".into(),
                })
            }
            "ClickControl" | "SetCheckbox" | "CloseDialog" => Some(GuiStopRootCause {
                kind: "target_not_found",
                message: "I couldn't find the requested control on screen, so I stopped safely (no guess).".into(),
            }),
            _ => Some(GuiStopRootCause {
                kind: "target_not_found",
                message: "I couldn't find the requested target on screen, so I stopped safely (no guess).".into(),
            }),
        };
    }
    None
}

/// Task 1 (Issue #5): whether the GOAL (not just an individual step) requires
/// human approval — risk high/critical, OR the goal contract / plan already
/// flagged approval (destructive verb, explicit "after approval", etc.). When
/// true, the workflow MUST pause for HITL before running ANY step so the
/// decision is deterministic and no state changes before approval.
fn goal_requires_approval(
    goal_contract: &GuiGoalContract,
    plan: &self::llm_planner::GuiLlmPlan,
) -> bool {
    goal_contract_requires_approval(goal_contract) || plan.requires_user_approval
}

/// Contract-only half of [`goal_requires_approval`] (no plan needed) so the
/// goal-level approval decision is unit-testable without constructing a plan.
fn goal_contract_requires_approval(goal_contract: &GuiGoalContract) -> bool {
    goal_contract.requires_user_approval
        || matches!(goal_contract.risk_level.as_str(), "high" | "critical")
}

#[cfg(test)]
mod clear_failure_tests {
    //! Task 5 (Issue #12): the bounded-guard stop reason is mapped to an
    //! actionable UPSTREAM root cause (target-not-found / field-not-resolvable /
    //! app-not-focused / needs-clarification / load-not-ready), not an opaque
    //! "screen state repeated N times". Pure classifier + flag plumbing.
    use super::*;

    #[test]
    fn flag_defaults_on_and_rolls_back_on_falsy() {
        assert!(clear_failure_enabled_lookup(|_| None));
        for raw in ["0", "false", "no", "off", "", " OFF "] {
            assert!(!clear_failure_enabled_lookup(|_| Some(raw.to_string())));
        }
        for raw in ["1", "true", "yes", "on", "anything"] {
            assert!(clear_failure_enabled_lookup(|_| Some(raw.to_string())));
        }
    }

    #[test]
    fn flapping_on_click_step_maps_to_target_not_found() {
        let root = classify_gui_stop_root_cause(
            Some("ClickControl"),
            &[],
            "screen state repeated 3 times without progress (flapping threshold 3)",
        )
        .expect("classified");
        assert_eq!(root.kind, "target_not_found");
        assert!(!root.message.to_lowercase().contains("repeated"));
    }

    #[test]
    fn reobserve_budget_on_field_step_maps_to_field_not_resolvable() {
        let root = classify_gui_stop_root_cause(
            Some("FocusField"),
            &[],
            "re-observe budget reached (16 of max 16)",
        )
        .expect("classified");
        assert_eq!(root.kind, "field_not_resolvable_vision_unavailable");
    }

    #[test]
    fn unfocused_presskey_maps_to_app_not_focused() {
        let root = classify_gui_stop_root_cause(
            Some("PressKey"),
            &["PressKey requires known focus or a prior resolved editable target.".into()],
            "target not safely resolved",
        )
        .expect("classified");
        assert_eq!(root.kind, "app_not_focused");
    }

    #[test]
    fn ambiguity_maps_to_needs_clarification() {
        let root = classify_gui_stop_root_cause(
            Some("ClickControl"),
            &["multiple matching controls found".into()],
            "stopped after re-observe",
        )
        .expect("classified");
        assert_eq!(root.kind, "needs_clarification");
    }

    #[test]
    fn wait_for_state_maps_to_load_not_ready() {
        let root = classify_gui_stop_root_cause(
            Some("WaitForState"),
            &[],
            "re-observe budget reached (16 of max 16)",
        )
        .expect("classified");
        assert_eq!(root.kind, "load_not_ready");
    }

    #[test]
    fn no_signal_returns_none_keeping_raw_reason() {
        assert!(classify_gui_stop_root_cause(Some("OpenApp"), &[], "some unrelated reason").is_none());
    }

    #[test]
    fn target_not_present_recovery_reason_maps_to_target_not_found() {
        let root = classify_gui_stop_root_cause(
            Some("ClickControl"),
            &["the resolved target is no longer present".into()],
            "the resolved target is no longer present",
        )
        .expect("classified");
        assert_eq!(root.kind, "target_not_found");
    }
}

#[cfg(test)]
mod smart_recovery_tests {
    //! Task 6 (Issue #13): the `gui_cog_smart_recovery` kill-switch — default ON,
    //! falsy = rollback (recovery skipped). The recovery POLICY itself (bounded,
    //! idempotent-only, risky→stop) lives in `recovery.rs` and is covered by the
    //! `gui_cognition_recovery_tests` suite.
    use super::*;

    #[test]
    fn smart_recovery_flag_defaults_on_and_rolls_back_on_falsy() {
        assert!(smart_recovery_enabled_lookup(|_| None));
        for raw in ["0", "false", "no", "off", "", " OFF "] {
            assert!(!smart_recovery_enabled_lookup(|_| Some(raw.to_string())));
        }
        for raw in ["1", "true", "yes", "on", "anything"] {
            assert!(smart_recovery_enabled_lookup(|_| Some(raw.to_string())));
        }
    }

    #[test]
    fn plan_prereq_merge_flag_defaults_on_and_rolls_back_on_falsy() {
        assert!(plan_prereq_merge_enabled_lookup(|_| None));
        for raw in ["0", "false", "no", "off", "", " OFF "] {
            assert!(!plan_prereq_merge_enabled_lookup(|_| Some(raw.to_string())));
        }
        for raw in ["1", "true", "yes", "on", "anything"] {
            assert!(plan_prereq_merge_enabled_lookup(|_| Some(raw.to_string())));
        }
    }
}

#[cfg(test)]
mod atspi_health_tests {
    //! Task 10 (Issue #8): the `gui_cog_atspi_health` kill-switch (default ON,
    //! falsy = rollback) + the consolidated `resolution_trustworthy` derivation
    //! on the accessibility summary (degraded/partial/unavailable → low-trust).
    use super::*;
    use crate::agent::gui_cognition::perception::GuiAccessibilitySummary;

    #[test]
    fn atspi_health_flag_defaults_on_and_rolls_back_on_falsy() {
        assert!(atspi_health_enabled_lookup(|_| None));
        for raw in ["0", "false", "no", "off", "", " OFF "] {
            assert!(!atspi_health_enabled_lookup(|_| Some(raw.to_string())));
        }
        for raw in ["1", "true", "yes", "on", "anything"] {
            assert!(atspi_health_enabled_lookup(|_| Some(raw.to_string())));
        }
    }

    #[test]
    fn healthy_summary_with_controls_is_resolution_trustworthy() {
        let mut s = GuiAccessibilitySummary {
            available: true,
            overall_status: "healthy".into(),
            control_count: 5,
            ..Default::default()
        };
        assert!(s.resolution_trustworthy());
        // Any partiality removes trust.
        s.omitted_node_count = 1;
        assert!(!s.resolution_trustworthy());
    }

    #[test]
    fn degraded_or_partial_summary_is_not_trustworthy() {
        // Degraded status.
        let degraded = GuiAccessibilitySummary {
            available: true,
            overall_status: "degraded".into(),
            control_count: 5,
            ..Default::default()
        };
        assert!(!degraded.resolution_trustworthy());

        // Healthy but no controls (app a11y off).
        let empty = GuiAccessibilitySummary {
            available: true,
            overall_status: "healthy".into(),
            control_count: 0,
            ..Default::default()
        };
        assert!(!empty.resolution_trustworthy());

        // Healthy but apps skipped.
        let skipped = GuiAccessibilitySummary {
            available: true,
            overall_status: "healthy".into(),
            control_count: 5,
            skipped_app_count: 1,
            ..Default::default()
        };
        assert!(!skipped.resolution_trustworthy());

        // Unavailable (default).
        assert!(!GuiAccessibilitySummary::default().resolution_trustworthy());
    }
}

#[cfg(test)]
mod gate_determinism_tests {
    //! Task 1 (Issue #5): the goal-level approval gate must fire DETERMINISTICALLY
    //! for any approval-required goal, independent of plan shape / target
    //! resolution — so a risky action can never slip through on a code path where
    //! the per-step gate is preceded by a benign step.
    use super::*;

    fn contract(prompt: &str) -> GuiGoalContract {
        extract_gui_goal_contract(prompt, None).contract
    }

    #[test]
    fn approval_prompts_require_goal_level_approval() {
        for p in [
            "Click the Submit button only after approval",
            "Delete the selected file, but ask for my approval",
            "Install the update, but require my approval before applying",
            "Send the email after I approve",
            "Pay the invoice, but ask first",
        ] {
            assert!(
                goal_contract_requires_approval(&contract(p)),
                "approval-required prompt must gate at the goal level: {p}"
            );
        }
    }

    #[test]
    fn benign_prompts_do_not_require_goal_level_approval() {
        for p in [
            "Open the calculator",
            "Scroll down the current page",
            "Switch to the file manager window",
        ] {
            assert!(
                !goal_contract_requires_approval(&contract(p)),
                "benign prompt must NOT require goal-level approval: {p}"
            );
        }
    }

    #[test]
    fn gate_determinism_flag_defaults_on_with_falsy_rollback() {
        let prev = std::env::var("KRIA_GUI_COG_GATE_DETERMINISM").ok();
        std::env::remove_var("KRIA_GUI_COG_GATE_DETERMINISM");
        assert!(gate_determinism_enabled(), "default must be ON");
        std::env::set_var("KRIA_GUI_COG_GATE_DETERMINISM", "0");
        assert!(!gate_determinism_enabled(), "0 must roll back (OFF)");
        std::env::set_var("KRIA_GUI_COG_GATE_DETERMINISM", "off");
        assert!(!gate_determinism_enabled(), "off must roll back (OFF)");
        std::env::set_var("KRIA_GUI_COG_GATE_DETERMINISM", "1");
        assert!(gate_determinism_enabled(), "1 must be ON");
        match prev {
            Some(v) => std::env::set_var("KRIA_GUI_COG_GATE_DETERMINISM", v),
            None => std::env::remove_var("KRIA_GUI_COG_GATE_DETERMINISM"),
        }
    }
}

fn gui_observation_reply(observation: &GuiObservationSnapshot) -> String {
    let text_sample = control_sample(&observation.text_fields, 4);
    let button_sample = control_sample(&observation.buttons, 6);
    format!(
        "GUI Cognition mode is active on the dedicated selected-mode path. Active window: {}. Active-window source: {} ({:.0}% confidence, {} reliability). Visible applications: {}. Visible controls: {} (text fields: {}, buttons: {}, dialogs: {}, other: {}, disabled/hidden: {}). Screenshot: {}. OCR: {} ({} untrusted block summaries). Accessibility: {} ({} nodes, {} controls). Monitors: {}. Focus known: {}. Text fields seen: {}. Buttons seen: {}.",
        observation.active_window_display(),
        observation.active_window.source,
        observation.active_window.confidence * 100.0,
        observation.active_window.reliability,
        observation.visible_app_count,
        observation.visible_control_count(),
        observation.text_fields.len(),
        observation.buttons.len(),
        observation.dialogs.len(),
        observation.other_controls.len(),
        observation.disabled_control_count(),
        if observation.screenshot_available { "available" } else { "unavailable" },
        if observation.ocr_available { "available" } else { "unavailable" },
        observation.ocr_blocks.len(),
        if observation.accessibility_ok { "available" } else { "unavailable" },
        observation.accessibility.node_count,
        observation.accessibility.control_count,
        observation.monitors.len(),
        if observation.cursor_focus.keyboard_focus_known { "yes" } else { "no" },
        if text_sample.is_empty() { "none/names not exposed".into() } else { text_sample.join(", ") },
        if button_sample.is_empty() { "none/names not exposed".into() } else { button_sample.join(", ") },
    )
}

fn observation_completed_event(
    observation: &GuiObservationSnapshot,
    source_blockers: serde_json::Value,
) -> serde_json::Value {
    let mut object = serde_json::Map::new();
    object.insert("type".into(), serde_json::json!("ObservationCompleted"));
    object.insert(
        "observation_id".into(),
        serde_json::json!(observation.observation_id),
    );
    object.insert(
        "active_window".into(),
        serde_json::json!(observation.active_window_label),
    );
    object.insert(
        "active_window_source".into(),
        serde_json::json!(observation.active_window.source),
    );
    object.insert(
        "active_window_confidence".into(),
        serde_json::json!(observation.active_window.confidence),
    );
    object.insert(
        "active_window_reliability".into(),
        serde_json::json!(observation.active_window.reliability),
    );
    object.insert(
        "active_window_blocker".into(),
        serde_json::json!(observation.active_window.blocker),
    );
    object.insert(
        "active_window_authority_source".into(),
        serde_json::json!(observation.active_window.source),
    );
    object.insert(
        "active_window_authority_confidence".into(),
        serde_json::json!(observation.active_window.confidence),
    );
    object.insert(
        "active_window_authority_status".into(),
        serde_json::json!(observation.active_window.authority_status),
    );
    object.insert(
        "gnome_bridge_status".into(),
        serde_json::json!(observation.active_window.gnome_bridge_status),
    );
    object.insert(
        "active_window_app".into(),
        serde_json::json!(observation.active_window.app_name),
    );
    object.insert(
        "active_window_app_id".into(),
        serde_json::json!(observation.active_window.app_id),
    );
    object.insert(
        "active_window_pid".into(),
        serde_json::json!(observation.active_window.pid),
    );
    object.insert(
        "active_window_workspace".into(),
        serde_json::json!(observation.active_window.workspace),
    );
    object.insert(
        "active_window_monitor".into(),
        serde_json::json!(observation.active_window.monitor),
    );
    object.insert(
        "active_window_fullscreen".into(),
        serde_json::json!(observation.active_window.fullscreen),
    );
    object.insert(
        "active_window_minimized".into(),
        serde_json::json!(observation.active_window.minimized),
    );
    object.insert(
        "active_window_observed_at_ms".into(),
        serde_json::json!(observation.active_window.observed_at_ms),
    );
    object.insert(
        "active_window_fallback_chain".into(),
        serde_json::to_value(&observation.active_window.fallback_chain)
            .unwrap_or(serde_json::Value::Null),
    );
    object.insert(
        "active_window_failure_chain".into(),
        serde_json::json!(active_window_failure_chain(observation)),
    );
    object.insert(
        "visible_app_count".into(),
        serde_json::json!(observation.visible_app_count),
    );
    object.insert(
        "visible_control_count".into(),
        serde_json::json!(observation.visible_control_count()),
    );
    object.insert(
        "visible_accessible_control_count".into(),
        serde_json::json!(observation.visible_accessible_control_count()),
    );
    object.insert(
        "disabled_control_count".into(),
        serde_json::json!(observation.disabled_control_count()),
    );
    object.insert(
        "hidden_control_count".into(),
        serde_json::json!(observation.hidden_control_count()),
    );
    object.insert(
        "trusted_control_count".into(),
        serde_json::json!(observation.control_quality_count("trusted")),
    );
    object.insert(
        "partial_control_count".into(),
        serde_json::json!(observation.control_quality_count("partial")),
    );
    object.insert(
        "not_executable_control_count".into(),
        serde_json::json!(observation.control_quality_count("not_executable")),
    );
    object.insert(
        "text_field_count".into(),
        serde_json::json!(observation.text_fields.len()),
    );
    object.insert(
        "button_count".into(),
        serde_json::json!(observation.buttons.len()),
    );
    object.insert(
        "dialog_count".into(),
        serde_json::json!(observation.dialogs.len()),
    );
    object.insert(
        "other_control_count".into(),
        serde_json::json!(observation.other_controls.len()),
    );
    object.insert(
        "ocr_available".into(),
        serde_json::json!(observation.ocr_available),
    );
    object.insert(
        "ocr_block_count".into(),
        serde_json::json!(observation.ocr_blocks.len()),
    );
    object.insert("ocr_trust".into(), serde_json::json!("untrusted"));
    object.insert(
        "ocr_wait_for_screenshot_ms".into(),
        serde_json::json!(observation.ocr_diagnostics.wait_for_screenshot_ms),
    );
    object.insert(
        "ocr_engine_selected".into(),
        serde_json::json!(observation.ocr_diagnostics.engine_selected),
    );
    object.insert(
        "ocr_engine_status".into(),
        serde_json::json!(observation.ocr_diagnostics.engine_status),
    );
    object.insert(
        "ocr_image_status".into(),
        serde_json::json!(observation.ocr_diagnostics.image_status),
    );
    object.insert(
        "ocr_total_ms".into(),
        serde_json::json!(observation.ocr_diagnostics.total_ms),
    );
    object.insert(
        "ocr_fast_path".into(),
        serde_json::json!(observation.ocr_diagnostics.fast_path),
    );
    object.insert(
        "ocr_cache_hit".into(),
        serde_json::json!(observation.ocr_diagnostics.cache_hit),
    );
    object.insert(
        "ocr_roi_count".into(),
        serde_json::json!(observation.ocr_diagnostics.roi_count),
    );
    object.insert(
        "ocr_changed_region_count".into(),
        serde_json::json!(observation.ocr_diagnostics.changed_region_count),
    );
    object.insert(
        "ocr_cold_start_ms".into(),
        serde_json::json!(observation.ocr_diagnostics.cold_start_ms),
    );
    object.insert(
        "ocr_warm_start_ms".into(),
        serde_json::json!(observation.ocr_diagnostics.warm_start_ms),
    );
    object.insert(
        "ocr_benchmark_summary".into(),
        serde_json::json!(observation.ocr_diagnostics.benchmark_summary),
    );
    object.insert(
        "ocr_injection_count".into(),
        serde_json::json!(observation
            .ocr_blocks
            .iter()
            .filter(|block| block.injection_suspected)
            .count()),
    );
    object.insert(
        "ocr_blocker".into(),
        serde_json::json!(observation.capabilities.ocr.blocker),
    );
    object.insert(
        "accessibility_available".into(),
        serde_json::json!(observation.accessibility_ok),
    );
    object.insert(
        "accessibility_source_status".into(),
        serde_json::json!(observation.accessibility.source_status),
    );
    object.insert(
        "accessibility_overall_status".into(),
        serde_json::json!(observation.accessibility.overall_status),
    );
    object.insert(
        "accessibility_overall_confidence".into(),
        serde_json::json!(observation.accessibility.overall_confidence),
    );
    if atspi_health_enabled() {
        // Task 10 (Issue #8): consolidated honest trust signal (additive;
        // flag-OFF omits it → prior event byte-for-byte). False on a degraded/
        // partial/unavailable summary → the resolver/UI prefers extension/vision.
        object.insert(
            "accessibility_resolution_trustworthy".into(),
            serde_json::json!(observation.accessibility.resolution_trustworthy()),
        );
    }
    object.insert(
        "accessibility_app_scores".into(),
        serde_json::to_value(&observation.accessibility.app_scores)
            .unwrap_or(serde_json::Value::Null),
    );
    object.insert(
        "accessibility_stale_node_count".into(),
        serde_json::json!(observation.accessibility.stale_node_count),
    );
    object.insert(
        "accessibility_timeout_count".into(),
        serde_json::json!(observation.accessibility.timeout_count),
    );
    object.insert(
        "accessibility_cache_hit_count".into(),
        serde_json::json!(observation.accessibility.cache_hit_count),
    );
    object.insert(
        "accessibility_stale_cache_rejected_count".into(),
        serde_json::json!(observation.accessibility.stale_cache_rejected_count),
    );
    object.insert(
        "accessibility_node_count".into(),
        serde_json::json!(observation.accessibility.node_count),
    );
    object.insert(
        "accessibility_control_count".into(),
        serde_json::json!(observation.accessibility.control_count),
    );
    object.insert(
        "atspi_snapshot_total_ms".into(),
        serde_json::json!(observation.accessibility.snapshot_total_ms),
    );
    object.insert(
        "atspi_skipped_app_count".into(),
        serde_json::json!(observation.accessibility.skipped_app_count),
    );
    object.insert(
        "atspi_omitted_node_count".into(),
        serde_json::json!(observation.accessibility.omitted_node_count),
    );
    object.insert(
        "accessibility_remediation".into(),
        serde_json::json!(observation.accessibility.remediation),
    );
    object.insert(
        "screenshot_available".into(),
        serde_json::json!(observation.screenshot_available),
    );
    object.insert(
        "screenshot_status".into(),
        serde_json::json!(if observation.screenshot_available {
            "available"
        } else {
            "unavailable"
        }),
    );
    object.insert(
        "screenshot_capture_ms".into(),
        serde_json::json!(probe_duration_ms(observation, "capture_screenshot")),
    );
    object.insert(
        "screenshot_duration_ms".into(),
        serde_json::json!(probe_duration_ms(observation, "capture_screenshot")),
    );
    object.insert(
        "screen_hash_prefix".into(),
        serde_json::json!(observation
            .screen_hash
            .as_ref()
            .map(|hash| hash.chars().take(16).collect::<String>())),
    );
    object.insert(
        "monitor_count".into(),
        serde_json::json!(observation.monitors.len()),
    );
    object.insert(
        "dpi_available".into(),
        serde_json::json!(!observation.monitors.is_empty()),
    );
    object.insert(
        "cursor_focus_known".into(),
        serde_json::json!(observation.cursor_focus.keyboard_focus_known),
    );
    object.insert(
        "focused_window".into(),
        serde_json::json!(observation.cursor_focus.focused_window_label),
    );
    object.insert(
        "focused_app".into(),
        serde_json::json!(observation.cursor_focus.focused_app),
    );
    object.insert(
        "focused_control_id".into(),
        serde_json::json!(observation.cursor_focus.focused_control_id),
    );
    object.insert(
        "focused_control_label".into(),
        serde_json::json!(observation.cursor_focus.focused_control_label),
    );
    object.insert(
        "focused_control_role".into(),
        serde_json::json!(observation.cursor_focus.focused_control_role),
    );
    object.insert(
        "focused_control_bounds".into(),
        serde_json::json!(observation.cursor_focus.focused_control_bounds),
    );
    object.insert(
        "text_cursor_known".into(),
        serde_json::json!(observation.cursor_focus.text_cursor_known),
    );
    object.insert(
        "editable_target_known".into(),
        serde_json::json!(observation.cursor_focus.editable_target_known),
    );
    object.insert(
        "terminal_like".into(),
        serde_json::json!(observation.cursor_focus.terminal_like),
    );
    object.insert(
        "focus_source".into(),
        serde_json::json!(observation.cursor_focus.source),
    );
    object.insert(
        "focus_confidence".into(),
        serde_json::json!(observation.cursor_focus.confidence),
    );
    object.insert(
        "focus_reliability".into(),
        serde_json::json!(observation.cursor_focus.reliability),
    );
    object.insert(
        "focus_adapter_status".into(),
        serde_json::json!(observation.cursor_focus.adapter_status),
    );
    object.insert(
        "focus_latency_ms".into(),
        serde_json::json!(observation.cursor_focus.latency_ms),
    );
    object.insert(
        "focus_failure_chain".into(),
        serde_json::to_value(&observation.cursor_focus.failure_chain)
            .unwrap_or(serde_json::Value::Null),
    );
    object.insert(
        "active_window_probe_ok".into(),
        serde_json::json!(observation.active_window_probe_ok),
    );
    object.insert(
        "desktop_state_probe_ok".into(),
        serde_json::json!(observation.desktop_state_probe_ok),
    );
    object.insert(
        "capabilities_probe_ok".into(),
        serde_json::json!(observation.capabilities_probe_ok),
    );
    object.insert(
        "observation_total_ms".into(),
        serde_json::json!(observation.timing.total_ms),
    );
    object.insert(
        "slowest_probe".into(),
        serde_json::json!(observation.timing.slowest_probe),
    );
    object.insert(
        "slowest_probe_ms".into(),
        serde_json::json!(observation.timing.slowest_probe_ms),
    );
    object.insert(
        "probe_timeout_count".into(),
        serde_json::json!(observation.timing.probe_timeout_count),
    );
    object.insert(
        "probe_timings".into(),
        serde_json::to_value(&observation.timing.probe_timings).unwrap_or(serde_json::Value::Null),
    );
    object.insert(
        "cache_hit".into(),
        serde_json::json!(observation.cache.cache_hit),
    );
    object.insert(
        "cache_age_ms".into(),
        serde_json::json!(observation.cache.cache_age_ms),
    );
    object.insert(
        "cache_policy".into(),
        serde_json::json!(observation.cache.cache_policy),
    );
    object.insert(
        "freshness".into(),
        serde_json::json!(observation.cache.freshness),
    );
    object.insert("source_blockers".into(), source_blockers);
    object.insert(
        "control_samples".into(),
        serde_json::json!(control_detail_sample(observation, 12)),
    );
    object.insert(
        "executable_control_count".into(),
        serde_json::json!(observation
            .all_controls()
            .iter()
            .filter(|control| control.is_executable_candidate())
            .count()),
    );
    object.insert(
        "visual_control_count".into(),
        serde_json::json!(observation.visual_controls.len()),
    );
    object.insert(
        "visual_control_summary".into(),
        visual_control_summary(observation),
    );
    serde_json::Value::Object(object)
}

fn source_blockers_json(observation: &GuiObservationSnapshot) -> serde_json::Value {
    serde_json::json!({
        "active_window": observation.capabilities.active_window.blocker,
        "desktop_state": observation.capabilities.desktop_state.blocker,
        "accessibility": observation.capabilities.accessibility.blocker,
        "screenshot": observation.capabilities.screenshot.blocker,
        "ocr": observation.capabilities.ocr.blocker,
        "monitor": observation.capabilities.monitor.blocker,
        "cursor_focus": observation.capabilities.cursor_focus.blocker,
    })
}

fn perception_summary_json(observation: &GuiObservationSnapshot) -> serde_json::Value {
    let mut value = observation_completed_event(observation, source_blockers_json(observation));
    if let Some(object) = value.as_object_mut() {
        object.remove("type");
        object.insert(
            "capabilities".into(),
            serde_json::to_value(&observation.capabilities).unwrap_or(serde_json::Value::Null),
        );
        object.insert(
            "text_field_sample".into(),
            serde_json::json!(control_sample(&observation.text_fields, 6)),
        );
        object.insert(
            "button_sample".into(),
            serde_json::json!(control_sample(&observation.buttons, 8)),
        );
        object.insert(
            "control_quality_summary".into(),
            control_quality_summary(observation),
        );
    }
    value
}

fn probe_duration_ms(observation: &GuiObservationSnapshot, probe_name: &str) -> Option<u64> {
    observation
        .timing
        .probe_timings
        .iter()
        .find(|timing| timing.probe_name == probe_name)
        .map(|timing| timing.duration_ms)
}

fn control_quality_summary(observation: &GuiObservationSnapshot) -> serde_json::Value {
    serde_json::json!({
        "trusted": observation.control_quality_count("trusted"),
        "partial": observation.control_quality_count("partial"),
        "not_executable": observation.control_quality_count("not_executable"),
        "executable": observation
            .all_controls()
            .iter()
            .filter(|control| control.is_executable_candidate())
            .count(),
    })
}

fn visual_control_summary(observation: &GuiObservationSnapshot) -> serde_json::Value {
    let matched_count = observation
        .visual_controls
        .iter()
        .filter(|control| control.matched_control_id.is_some())
        .count();
    serde_json::json!({
        "detected": observation.visual_controls.len(),
        "matched": matched_count,
        "unmatched": observation.visual_controls.len().saturating_sub(matched_count),
        "button_like": observation
            .visual_controls
            .iter()
            .filter(|control| {
                matches!(
                    control.control_type.as_str(),
                    "button" | "link" | "toggle" | "menu" | "tab"
                )
            })
            .count(),
        "false_positive_risk": "supporting_visual_only",
    })
}

fn active_window_failure_chain(observation: &GuiObservationSnapshot) -> Vec<serde_json::Value> {
    observation
        .active_window
        .fallback_chain
        .iter()
        .filter(|attempt| attempt.status != "matched")
        .map(|attempt| {
            serde_json::json!({
                "source": attempt.source,
                "status": attempt.status,
                "reliability": attempt.reliability,
                "reason": attempt.reason,
            })
        })
        .collect()
}

fn control_detail_sample(
    observation: &GuiObservationSnapshot,
    limit: usize,
) -> Vec<serde_json::Value> {
    observation
        .all_controls()
        .into_iter()
        .take(limit)
        .map(|control| {
            serde_json::json!({
                "id": control.control_id,
                "role": control.role,
                "label": control.name,
                "bounds": control.bounds,
                "enabled": control.enabled,
                "visible": control.visible,
                "focused": control.focused,
                "source": control.source,
                "confidence": control.confidence,
                "quality": control.quality,
                "label_source": control.label_source,
                "state_source": control.state_source,
                "rejection_reason": control.rejection_reason,
                "identity_confidence": control.identity_confidence,
                "bounds_confidence": control.bounds_confidence,
                "state_confidence": control.state_confidence,
                "executable_confidence": control.executable_confidence,
                "sources": control.sources,
            })
        })
        .collect()
}

#[allow(dead_code)]
fn action_summary(execution: &GuiActionExecution) -> serde_json::Value {
    serde_json::json!({
        "success": execution.success,
        "tool": execution.tool,
        "error": execution.error,
        "evidence": execution.evidence,
    })
}

fn action_backend_event(status: &GuiActionBackendStatus) -> serde_json::Value {
    let capabilities =
        serde_json::to_value(&status.capabilities).unwrap_or_else(|_| serde_json::json!({}));
    serde_json::json!({
        "type": "ActionBackendStatus",
        "global_halt_engaged": status.global_halt_engaged,
        "halt_kind": status.halt_kind,
        "halt_reason": status.halt_reason,
        "release_conditions": status.release_conditions,
        "startup_elapsed_ms": status.startup_elapsed_ms,
        "can_observe": status.can_observe,
        "can_plan": status.can_plan,
        "automation_enabled": status.automation_enabled,
        "vision_sidecar": status.vision_sidecar,
        "uinput_daemon": status.uinput_daemon,
        "orchestrator_available": status.orchestrator_available,
        "session_type": status.session_type,
        "xdotool_available": status.xdotool_available,
        "ydotool_available": status.ydotool_available,
        "uinput_available": status.uinput_available,
        "selected_backend": status.selected_backend,
        "backend_selection_reason": status.backend_selection_reason,
        "backend_probe_status": status.backend_probe_status,
        "backend_probe_errors": status.backend_probe_errors,
        "input_backend_kind": status.input_backend_kind,
        "focus_supported": status.focus_supported,
        "typing_supported": status.typing_supported,
        "click_supported": status.click_supported,
        "verification_supported": status.verification_supported,
        "xdotool_usable_for_actions": status.xdotool_usable_for_actions,
        "ydotool_usable_for_actions": status.ydotool_usable_for_actions,
        "uinput_socket_path": status.uinput_socket_path,
        "uinput_socket_accessible": status.uinput_socket_accessible,
        "can_execute_actions": status.can_execute_actions,
        "blockers": status.blockers,
        "capabilities": capabilities,
    })
}

fn action_backend_summary(status: &GuiActionBackendStatus) -> serde_json::Value {
    let capabilities =
        serde_json::to_value(&status.capabilities).unwrap_or_else(|_| serde_json::json!({}));
    serde_json::json!({
        "global_halt_engaged": status.global_halt_engaged,
        "halt_kind": status.halt_kind,
        "halt_reason": status.halt_reason,
        "release_conditions": status.release_conditions,
        "startup_elapsed_ms": status.startup_elapsed_ms,
        "can_observe": status.can_observe,
        "can_plan": status.can_plan,
        "automation_enabled": status.automation_enabled,
        "vision_sidecar": status.vision_sidecar,
        "uinput_daemon": status.uinput_daemon,
        "orchestrator_available": status.orchestrator_available,
        "session_type": status.session_type,
        "xdotool_available": status.xdotool_available,
        "ydotool_available": status.ydotool_available,
        "uinput_available": status.uinput_available,
        "selected_backend": status.selected_backend,
        "backend_selection_reason": status.backend_selection_reason,
        "backend_probe_status": status.backend_probe_status,
        "backend_probe_errors": status.backend_probe_errors,
        "input_backend_kind": status.input_backend_kind,
        "focus_supported": status.focus_supported,
        "typing_supported": status.typing_supported,
        "click_supported": status.click_supported,
        "verification_supported": status.verification_supported,
        "xdotool_usable_for_actions": status.xdotool_usable_for_actions,
        "ydotool_usable_for_actions": status.ydotool_usable_for_actions,
        "uinput_socket_path": status.uinput_socket_path,
        "uinput_socket_accessible": status.uinput_socket_accessible,
        "can_execute_actions": status.can_execute_actions,
        "blockers": status.blockers,
        "capabilities": capabilities,
    })
}

fn verification_summary(report: &GuiVerificationReport) -> serde_json::Value {
    serde_json::json!({
        "status": report.status,
        "confidence": report.confidence,
        "after_observation_id": report.after_observation_id,
    })
}

fn blocker_summary(blocker: &GuiBlocker) -> serde_json::Value {
    serde_json::json!({
        "kind": blocker.kind,
        "reason": blocker.reason,
        "candidate_count": blocker.candidate_count,
        "target_name": blocker.target_name,
        "options": blocker.options,
        "clarification_question": blocker.clarification_question,
    })
}

#[cfg(test)]
mod task3_window_focus_availability_tests {
    //! Task 3 (Issue #1): GnomeBridge availability now reports a real
    //! Wayland-native activation path (`gio launch <.desktop>`) so SwitchWindow
    //! routing prefers the compositor-native activate-by-identity backend over a
    //! blind Alt+Tab fallback. These tests cover the session/flag/gio decision
    //! logic and the selection preference, with the `gio` probe injected.
    use super::{window_focus_backend_available_inner, WindowFocusBackend};
    use super::window_focus::{
        select_focus_backends, select_window_focus_backend, WindowIdentity,
    };
    use super::executor::GuiActionBackendStatus;

    fn status_with(session: &str, can_execute: bool) -> GuiActionBackendStatus {
        let mut status = GuiActionBackendStatus::available("test_backend");
        status.session_type = session.to_string();
        status.can_execute_actions = can_execute;
        status
    }

    #[test]
    fn gnome_bridge_available_on_wayland_when_gio_present_and_flag_on() {
        // Wayland session + gio present + flag ON ⇒ GnomeBridge is reachable.
        assert!(window_focus_backend_available_inner(
            WindowFocusBackend::GnomeBridge,
            "wayland",
            true,
            true,
            || true,
        ));
        // Also reachable on GNOME-on-X11 (gio launch raises the window there too).
        assert!(window_focus_backend_available_inner(
            WindowFocusBackend::GnomeBridge,
            "x11",
            true,
            true,
            || true,
        ));
    }

    #[test]
    fn gnome_bridge_unavailable_without_gio() {
        // gio binary missing ⇒ no Wayland-native activation path ⇒ unavailable
        // (never fabricated).
        assert!(!window_focus_backend_available_inner(
            WindowFocusBackend::GnomeBridge,
            "wayland",
            true,
            true,
            || false,
        ));
    }

    #[test]
    fn gnome_bridge_unavailable_on_unknown_session() {
        for session in ["unknown", "", "tty", "mir"] {
            assert!(
                !window_focus_backend_available_inner(
                    WindowFocusBackend::GnomeBridge,
                    session,
                    true,
                    true,
                    || true,
                ),
                "session {session:?} is not a known graphical session for gio activation"
            );
        }
    }

    #[test]
    fn flag_off_keeps_gnome_bridge_unavailable() {
        // Hard constraint: flag OFF ⇒ GnomeBridge never reported available, so
        // SwitchWindow behavior is preserved byte-for-byte (no Wayland-native
        // path is selected).
        assert!(!window_focus_backend_available_inner(
            WindowFocusBackend::GnomeBridge,
            "wayland",
            true,
            false, // flag OFF
            || true,
        ));
    }

    #[test]
    fn uinput_alt_tab_tracks_can_execute_actions_regardless_of_flag() {
        // UinputAltTab availability is unchanged: it tracks the deterministic
        // input substrate, independent of the Wayland-focus flag.
        for flag in [true, false] {
            assert!(window_focus_backend_available_inner(
                WindowFocusBackend::UinputAltTab,
                "wayland",
                true,
                flag,
                || false,
            ));
            assert!(!window_focus_backend_available_inner(
                WindowFocusBackend::UinputAltTab,
                "wayland",
                false,
                flag,
                || false,
            ));
        }
    }

    #[test]
    fn portal_never_available() {
        assert!(!window_focus_backend_available_inner(
            WindowFocusBackend::Portal,
            "wayland",
            true,
            true,
            || true,
        ));
    }

    #[test]
    fn selection_prefers_gnome_bridge_over_alt_tab_on_wayland_when_available() {
        // End-to-end of the routing predicate: on a healthy Wayland session with
        // gio present + flag ON, the selected focus backend is the
        // compositor-native GnomeBridge, NOT the blind Alt+Tab fallback.
        let status = status_with("wayland", true);
        let identity = WindowIdentity::new(Some("Text Editor"), None);
        let chain = select_focus_backends(&status.session_type, &status);
        let selected = select_window_focus_backend(&chain, &identity, |candidate| {
            window_focus_backend_available_inner(
                candidate,
                &status.session_type,
                status.can_execute_actions,
                true,
                || true,
            )
        })
        .expect("a focus backend should be selected");
        assert_eq!(selected, WindowFocusBackend::GnomeBridge);
    }

    #[test]
    fn selection_falls_back_to_alt_tab_when_gio_missing() {
        // No gio ⇒ GnomeBridge unavailable ⇒ verifiable Alt+Tab fallback chosen.
        let status = status_with("wayland", true);
        let identity = WindowIdentity::new(Some("Text Editor"), None);
        let chain = select_focus_backends(&status.session_type, &status);
        let selected = select_window_focus_backend(&chain, &identity, |candidate| {
            window_focus_backend_available_inner(
                candidate,
                &status.session_type,
                status.can_execute_actions,
                true,
                || false,
            )
        })
        .expect("a focus backend should be selected");
        assert_eq!(selected, WindowFocusBackend::UinputAltTab);
        assert!(selected.requires_verification());
    }

    #[test]
    fn selection_flag_off_does_not_select_gnome_bridge() {
        // Flag OFF ⇒ GnomeBridge unavailable even with gio present; selection
        // falls to the legacy verifiable Alt+Tab substrate.
        let status = status_with("wayland", true);
        let identity = WindowIdentity::new(Some("Text Editor"), None);
        let chain = select_focus_backends(&status.session_type, &status);
        let selected = select_window_focus_backend(&chain, &identity, |candidate| {
            window_focus_backend_available_inner(
                candidate,
                &status.session_type,
                status.can_execute_actions,
                false, // flag OFF
                || true,
            )
        })
        .expect("a focus backend should be selected");
        assert_ne!(selected, WindowFocusBackend::GnomeBridge);
        assert_eq!(selected, WindowFocusBackend::UinputAltTab);
    }
}
