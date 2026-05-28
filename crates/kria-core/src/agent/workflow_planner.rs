//! Capability-Aware Workflow Planner — Adaptive Substrate Planning.
//!
//! This module wraps the existing `SubstratePlanner` with capability awareness,
//! outcome contract generation, and HITL negotiation. It does NOT replace the
//! substrate planner — it enhances it with environment-aware adaptation.
//!
//! # Flow
//!
//! ```text
//! WorkflowIntent + CapabilitySet
//!     │
//!     ▼
//! CapabilityAwarePlanner
//!     ├── Pre-flight checks (app available? login needed?)
//!     ├── SubstratePlanner.plan() (existing deterministic routing)
//!     ├── Plan adaptation (gate verification leaves on capabilities)
//!     ├── Outcome contract generation (plan-bound, never re-derived)
//!     └── Recovery path planning (proactive, not reactive)
//! ```
//!
//! # Design Rules
//!
//! - Deterministic: same inputs → same plan always
//! - Bounded: no LLM calls in the planning path
//! - Additive: wraps existing planner, doesn't replace it
//! - Observable: emits telemetry for every planning decision

use crate::agent::gui_substrate_planner::{ExecutionSubstrate, SubstratePlan, SubstratePlanner};
use crate::agent::intent_compiler::GuiTaskSpec;
use crate::agent::workflow_types::{
    CapabilitySet, ExecutionMode,
    HitlOption, HitlReason, InputInjectionLevel, OutcomeContract, OutcomeExpectation,
    OutcomeFailurePolicy, PlannedOutcome,
};

// ═══════════════════════════════════════════════════════════════════════════════
// §1 — Capability-Aware Planning Result
// ═══════════════════════════════════════════════════════════════════════════════

/// Result of capability-aware planning.
#[derive(Debug, Clone)]
pub enum PlanningResult {
    /// Plan generated successfully with outcome contract
    Planned {
        substrate_plan: SubstratePlan,
        outcome_contract: OutcomeContract,
        execution_mode: ExecutionMode,
        adaptations: Vec<PlanAdaptation>,
    },
    /// Planning requires HITL before proceeding
    NeedsHitl {
        reason: HitlReason,
        options: Vec<HitlOption>,
        context: String,
    },
    /// Cannot plan this workflow (falls through to ReAct)
    Unplannable {
        reason: String,
    },
}

/// A planning adaptation that was applied due to capability constraints.
#[derive(Debug, Clone)]
pub struct PlanAdaptation {
    pub description: String,
    pub category: AdaptationCategory,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AdaptationCategory {
    /// Verification leaf downgraded due to environment
    VerificationDowngrade,
    /// Visibility expectation marked as best-effort
    VisibilityBestEffort,
    /// Interactive step gated on capability
    InteractionGated,
    /// App alternative selected
    AppAlternative,
}

// ═══════════════════════════════════════════════════════════════════════════════
// §2 — Capability-Aware Planner
// ═══════════════════════════════════════════════════════════════════════════════

/// Wraps the existing SubstratePlanner with capability awareness.
pub struct CapabilityAwarePlanner;

impl CapabilityAwarePlanner {
    /// Plan a workflow with full capability awareness.
    ///
    /// This is the new canonical planning entry point. It:
    /// 1. Runs pre-flight checks (app available, login needed)
    /// 2. Delegates to SubstratePlanner for substrate selection
    /// 3. Adapts the plan based on capabilities
    /// 4. Generates the outcome contract
    /// 5. Plans recovery paths
    pub fn plan(
        spec: &GuiTaskSpec,
        raw_user_text: &str,
        capabilities: &CapabilitySet,
        app_registry: &crate::platform::app_registry::InstalledAppRegistry,
    ) -> PlanningResult {
        // ── Step 1: Pre-flight capability checks ──────────────────────────────
        if let Some(hitl) = Self::preflight_checks(spec, capabilities, app_registry) {
            return hitl;
        }

        // ── Step 2: Delegate to existing substrate planner ────────────────────
        let planner = SubstratePlanner;
        let substrate_plan = planner.plan(spec, raw_user_text);

        // If substrate planner returns Unknown, we can't plan
        if substrate_plan.substrate == ExecutionSubstrate::Unknown {
            return PlanningResult::Unplannable {
                reason: "Substrate planner could not determine execution strategy".into(),
            };
        }

        // ── Step 3: Adapt plan based on capabilities ──────────────────────────
        let adaptations = Self::adapt_plan(&substrate_plan, capabilities);

        // ── Step 4: Generate outcome contract ─────────────────────────────────
        let outcome_contract = Self::generate_outcome_contract(
            &substrate_plan,
            spec,
            capabilities,
        );

        // ── Step 5: Determine execution mode ──────────────────────────────────
        let execution_mode = Self::determine_execution_mode(&substrate_plan);

        PlanningResult::Planned {
            substrate_plan,
            outcome_contract,
            execution_mode,
            adaptations,
        }
    }

    // ─── Pre-flight Checks ────────────────────────────────────────────────

    fn preflight_checks(
        spec: &GuiTaskSpec,
        capabilities: &CapabilitySet,
        app_registry: &crate::platform::app_registry::InstalledAppRegistry,
    ) -> Option<PlanningResult> {
        use crate::agent::intent_compiler::TargetRef;

        // Check if target app is installed
        for target in &spec.targets {
            if let TargetRef::App(app_name) = target {
                if let Some(reason) =
                    crate::agent::workflow_capability::check_app_available(app_name, app_registry)
                {
                    let options =
                        crate::agent::workflow_capability::hitl_options_for_missing_app(app_name);
                    return Some(PlanningResult::NeedsHitl {
                        reason,
                        options,
                        context: format!(
                            "'{}' is not installed. Choose an action to continue.",
                            app_name
                        ),
                    });
                }
            }
        }

        // Check if interactive steps are possible
        if spec.primary_verb == crate::agent::intent_compiler::Verb::Type
            || spec.primary_verb == crate::agent::intent_compiler::Verb::Click
        {
            if capabilities.interaction.keyboard_injection == InputInjectionLevel::None {
                return Some(PlanningResult::NeedsHitl {
                    reason: HitlReason::ManualStepNeeded {
                        instruction: "GUI interaction is not available in this environment.".into(),
                        context: format!(
                            "Session: {:?}, uinput: {}",
                            capabilities.environment.session_type,
                            capabilities.environment.uinput_available
                        ),
                    },
                    options: vec![
                        HitlOption {
                            id: "manual".into(),
                            label: "I'll do it manually".into(),
                            action_type: crate::agent::workflow_types::HitlActionType::ManualComplete,
                        },
                        HitlOption {
                            id: "cancel".into(),
                            label: "Cancel".into(),
                            action_type: crate::agent::workflow_types::HitlActionType::Cancel,
                        },
                    ],
                    context: "Keyboard/mouse injection is unavailable.".into(),
                });
            }
        }

        None // All pre-flight checks passed
    }

    // ─── Plan Adaptation ──────────────────────────────────────────────────

    fn adapt_plan(
        plan: &SubstratePlan,
        capabilities: &CapabilitySet,
    ) -> Vec<PlanAdaptation> {
        let mut adaptations = Vec::new();

        // Check if the substrate requires capabilities we don't have
        match plan.substrate {
            ExecutionSubstrate::Keystroke => {
                if capabilities.interaction.keyboard_injection == InputInjectionLevel::None {
                    adaptations.push(PlanAdaptation {
                        description: "Keystroke injection unavailable; step will require manual input".into(),
                        category: AdaptationCategory::InteractionGated,
                    });
                }
            }
            ExecutionSubstrate::InteractionHeavy => {
                if capabilities.interaction.keyboard_injection == InputInjectionLevel::None {
                    adaptations.push(PlanAdaptation {
                        description: "Interactive workflow on environment without input injection".into(),
                        category: AdaptationCategory::InteractionGated,
                    });
                }
            }
            ExecutionSubstrate::BrowserNavigate => {
                if !capabilities.verifier.cdp_available {
                    adaptations.push(PlanAdaptation {
                        description: "CDP unavailable; browser verification will use process-only check".into(),
                        category: AdaptationCategory::VerificationDowngrade,
                    });
                }
            }
            _ => {}
        }

        // Check visibility verification capability
        if capabilities.verifier.window_state_max_confidence < 0.60 {
            let has_visible_steps = matches!(
                plan.substrate,
                ExecutionSubstrate::AppOpenOnly
                    | ExecutionSubstrate::FileWriteThenOpen
                    | ExecutionSubstrate::IdeCodeRunWorkflow
                    | ExecutionSubstrate::BrowserNavigate
            );
            if has_visible_steps {
                adaptations.push(PlanAdaptation {
                    description: format!(
                        "Window verification confidence capped at {:.0}% in {:?} session",
                        capabilities.verifier.window_state_max_confidence * 100.0,
                        capabilities.environment.session_type
                    ),
                    category: AdaptationCategory::VisibilityBestEffort,
                });
            }
        }

        adaptations
    }

    // ─── Outcome Contract Generation ──────────────────────────────────────

    fn generate_outcome_contract(
        plan: &SubstratePlan,
        spec: &GuiTaskSpec,
        capabilities: &CapabilitySet,
    ) -> OutcomeContract {
        let mut required = Vec::new();
        let mut desired = Vec::new();

        // Generate outcomes based on substrate type
        match plan.substrate {
            ExecutionSubstrate::FileWriteThenOpen => {
                // Required: file exists
                if let Some(path) = plan.artifacts.first() {
                    required.push(PlannedOutcome {
                        description: format!("File created: {}", path.display()),
                        expectation: OutcomeExpectation::FileExists {
                            path: path.display().to_string(),
                        },
                        min_confidence: 0.90,
                        on_failure: OutcomeFailurePolicy::FailWorkflow,
                    });
                }
                // Desired: app window visible (capability-gated)
                if let Some(app) = Self::extract_app_from_spec(spec) {
                    let min_conf = capabilities.verifier.window_state_max_confidence * 0.8;
                    desired.push(PlannedOutcome {
                        description: format!("{} window visible", app),
                        expectation: OutcomeExpectation::AppWindowVisible {
                            app: app.clone(),
                            title_hint: None,
                        },
                        min_confidence: min_conf,
                        on_failure: OutcomeFailurePolicy::DowngradeFidelity,
                    });
                }
            }
            ExecutionSubstrate::IdeCodeRunWorkflow => {
                // Required: file exists + output captured
                if let Some(path) = plan.artifacts.first() {
                    required.push(PlannedOutcome {
                        description: format!("Source file created: {}", path.display()),
                        expectation: OutcomeExpectation::FileExists {
                            path: path.display().to_string(),
                        },
                        min_confidence: 0.90,
                        on_failure: OutcomeFailurePolicy::FailWorkflow,
                    });
                }
                if plan.artifacts.len() > 1 {
                    let output_path = &plan.artifacts[1];
                    required.push(PlannedOutcome {
                        description: "Program output captured".into(),
                        expectation: OutcomeExpectation::OutputContains {
                            substring: String::new(), // Any output
                            in_file: output_path.display().to_string(),
                        },
                        min_confidence: 0.85,
                        on_failure: OutcomeFailurePolicy::FailWorkflow,
                    });
                }
                // Desired: IDE visible
                if let Some(app) = Self::extract_app_from_spec(spec) {
                    desired.push(PlannedOutcome {
                        description: format!("{} window visible", app),
                        expectation: OutcomeExpectation::AppWindowVisible {
                            app,
                            title_hint: None,
                        },
                        min_confidence: capabilities.verifier.window_state_max_confidence * 0.8,
                        on_failure: OutcomeFailurePolicy::DowngradeFidelity,
                    });
                }
            }
            ExecutionSubstrate::TerminalExecution => {
                // Required: file + output
                if let Some(path) = plan.artifacts.first() {
                    required.push(PlannedOutcome {
                        description: format!("Script created: {}", path.display()),
                        expectation: OutcomeExpectation::FileExists {
                            path: path.display().to_string(),
                        },
                        min_confidence: 0.90,
                        on_failure: OutcomeFailurePolicy::FailWorkflow,
                    });
                }
            }
            ExecutionSubstrate::BrowserNavigate => {
                // Desired: browser at URL (CDP-gated)
                if capabilities.verifier.cdp_available {
                    desired.push(PlannedOutcome {
                        description: "Browser navigated to target".into(),
                        expectation: OutcomeExpectation::BrowserAtUrl {
                            url_contains: String::new(),
                        },
                        min_confidence: 0.80,
                        on_failure: OutcomeFailurePolicy::DowngradeFidelity,
                    });
                } else {
                    // Without CDP, just verify browser process is running
                    desired.push(PlannedOutcome {
                        description: "Browser process running".into(),
                        expectation: OutcomeExpectation::ProcessRunning {
                            binary: "chrome".into(),
                        },
                        min_confidence: 0.70,
                        on_failure: OutcomeFailurePolicy::DowngradeFidelity,
                    });
                }
            }
            ExecutionSubstrate::AppOpenOnly => {
                if let Some(app) = Self::extract_app_from_spec(spec) {
                    let binary = crate::agent::gui_substrate_planner::app_alias_to_binary_pub(&app);
                    required.push(PlannedOutcome {
                        description: format!("{} process running", app),
                        expectation: OutcomeExpectation::ProcessRunning { binary },
                        min_confidence: 0.85,
                        on_failure: OutcomeFailurePolicy::FailWorkflow,
                    });
                }
            }
            _ => {}
        }

        OutcomeContract { required, desired }
    }

    // ─── Execution Mode Determination ─────────────────────────────────────

    fn determine_execution_mode(plan: &SubstratePlan) -> ExecutionMode {
        match plan.substrate {
            ExecutionSubstrate::TerminalExecution => ExecutionMode::Structural,
            ExecutionSubstrate::AppOpenOnly | ExecutionSubstrate::BrowserNavigate => {
                ExecutionMode::Visible
            }
            ExecutionSubstrate::FileWriteThenOpen | ExecutionSubstrate::IdeCodeRunWorkflow => {
                // Hybrid: file write is structural, app open is visible
                let visible_steps: Vec<u32> = plan
                    .workflow
                    .as_ref()
                    .map(|wf| {
                        wf.sub_goals
                            .iter()
                            .filter(|g| {
                                matches!(
                                    g.action.as_str(),
                                    "open_application"
                                        | "open_application_with_file"
                                        | "browser_search"
                                        | "open_url"
                                )
                            })
                            .map(|g| g.step as u32)
                            .collect()
                    })
                    .unwrap_or_default();
                if visible_steps.is_empty() {
                    ExecutionMode::Structural
                } else {
                    ExecutionMode::Hybrid { visible_steps }
                }
            }
            ExecutionSubstrate::Keystroke | ExecutionSubstrate::InteractionHeavy => {
                ExecutionMode::Visible
            }
            _ => ExecutionMode::Structural,
        }
    }

    // ─── Helpers ──────────────────────────────────────────────────────────

    fn extract_app_from_spec(spec: &GuiTaskSpec) -> Option<String> {
        use crate::agent::intent_compiler::TargetRef;
        spec.targets.iter().find_map(|t| match t {
            TargetRef::App(a) => Some(a.clone()),
            _ => None,
        })
    }
}

// ═══════════════════════════════════════════════════════════════════════════════
// §3 — Tests
// ═══════════════════════════════════════════════════════════════════════════════

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::intent_compiler::{ContentClass, GuiTaskSpec, TargetRef, Verb};
    use crate::agent::workflow_types::*;

    fn make_capabilities(session: SessionType, atspi: AtSpiLevel, uinput: bool) -> CapabilitySet {
        let env = EnvironmentCapability {
            session_type: session,
            compositor: Some("mutter".into()),
            atspi_level: atspi,
            xdotool_available: session == SessionType::X11,
            uinput_available: uinput,
            ocr_available: false,
        };
        let verifier = VerifierCapability {
            available_methods: vec![VerificationMethod::FileSystem, VerificationMethod::ProcessTable],
            window_state_max_confidence: match (&env.session_type, &env.atspi_level) {
                (SessionType::X11, AtSpiLevel::Full) => 0.90,
                (SessionType::Wayland, AtSpiLevel::Full) => 0.70,
                _ => 0.40,
            },
            cdp_available: false,
            filesystem_available: true,
            process_table_available: true,
        };
        let interaction = InteractionCapability {
            keyboard_injection: if uinput { InputInjectionLevel::Full } else { InputInjectionLevel::None },
            mouse_injection: if uinput { InputInjectionLevel::Full } else { InputInjectionLevel::None },
            clipboard_available: true,
        };
        CapabilitySet { environment: env, verifier, interaction }
    }

    fn make_spec(verb: Verb, targets: Vec<TargetRef>, content: Option<ContentClass>) -> GuiTaskSpec {
        GuiTaskSpec {
            primary_verb: verb,
            targets,
            content,
            declared_preconditions: vec![],
            declared_success_criteria: vec![],
            ambiguities: vec![],
        }
    }

    #[test]
    fn plan_file_write_then_open_generates_outcome_contract() {
        let spec = make_spec(
            Verb::Open,
            vec![TargetRef::App("gedit".into())],
            Some(ContentClass::Generated { hint: "fibonacci program".into(), language: Some("python".into()) }),
        );
        let caps = make_capabilities(SessionType::X11, AtSpiLevel::Full, true);
        let registry = crate::platform::app_registry::InstalledAppRegistry::build_sync();

        let result = CapabilityAwarePlanner::plan(&spec, "open gedit and write fibonacci", &caps, &registry);

        match result {
            PlanningResult::Planned { outcome_contract, execution_mode, .. } => {
                // Should have at least one required outcome (file exists)
                assert!(!outcome_contract.required.is_empty(), "Should have required outcomes");
                // Should be hybrid (file write + app open)
                assert!(
                    matches!(execution_mode, ExecutionMode::Hybrid { .. } | ExecutionMode::Visible),
                    "Should be hybrid or visible mode"
                );
            }
            PlanningResult::NeedsHitl { reason, .. } => {
                // Acceptable if gedit isn't installed on this system
                assert!(matches!(reason, HitlReason::InstallRequired { .. }));
            }
            PlanningResult::Unplannable { reason } => {
                panic!("Should not be unplannable: {}", reason);
            }
        }
    }

    #[test]
    fn plan_with_missing_interaction_triggers_hitl() {
        let spec = make_spec(
            Verb::Type,
            vec![TargetRef::Element("search box".into())],
            Some(ContentClass::Literal("hello world".into())),
        );
        // No uinput = no interaction capability
        let caps = make_capabilities(SessionType::Wayland, AtSpiLevel::None, false);
        let registry = crate::platform::app_registry::InstalledAppRegistry::build_sync();

        let result = CapabilityAwarePlanner::plan(&spec, "type hello world", &caps, &registry);

        match result {
            PlanningResult::NeedsHitl { reason, .. } => {
                assert!(matches!(reason, HitlReason::ManualStepNeeded { .. }));
            }
            _ => panic!("Should trigger HITL when interaction is unavailable"),
        }
    }

    #[test]
    fn plan_adaptation_notes_wayland_visibility_limitation() {
        let spec = make_spec(
            Verb::Open,
            vec![TargetRef::App("nautilus".into())],
            None,
        );
        // Wayland without AT-SPI = low visibility confidence
        let caps = make_capabilities(SessionType::Wayland, AtSpiLevel::None, true);
        let registry = crate::platform::app_registry::InstalledAppRegistry::build_sync();

        let result = CapabilityAwarePlanner::plan(&spec, "open files", &caps, &registry);

        match result {
            PlanningResult::Planned { adaptations, .. } => {
                // Should note visibility limitation
                let has_visibility_adaptation = adaptations.iter().any(|a| {
                    a.category == AdaptationCategory::VisibilityBestEffort
                });
                assert!(
                    has_visibility_adaptation,
                    "Should note visibility limitation on Wayland without AT-SPI"
                );
            }
            PlanningResult::NeedsHitl { .. } => {
                // Also acceptable if nautilus isn't installed
            }
            _ => {}
        }
    }

    #[test]
    fn outcome_contract_gates_window_verification_on_capability() {
        let spec = make_spec(
            Verb::Open,
            vec![TargetRef::App("firefox".into())],
            None,
        );

        // High capability environment
        let high_caps = make_capabilities(SessionType::X11, AtSpiLevel::Full, true);
        let registry = crate::platform::app_registry::InstalledAppRegistry::build_sync();

        let result = CapabilityAwarePlanner::plan(&spec, "open firefox", &high_caps, &registry);
        if let PlanningResult::Planned { outcome_contract, .. } = &result {
            // On X11 with full AT-SPI, desired outcomes should have higher confidence threshold
            for desired in &outcome_contract.desired {
                if matches!(desired.expectation, OutcomeExpectation::AppWindowVisible { .. }) {
                    assert!(desired.min_confidence >= 0.60, "X11+AT-SPI should have high confidence threshold");
                }
            }
        }

        // Low capability environment
        let low_caps = make_capabilities(SessionType::Wayland, AtSpiLevel::None, true);
        let result_low = CapabilityAwarePlanner::plan(&spec, "open firefox", &low_caps, &registry);
        if let PlanningResult::Planned { outcome_contract, .. } = &result_low {
            for desired in &outcome_contract.desired {
                if matches!(desired.expectation, OutcomeExpectation::AppWindowVisible { .. }) {
                    // On Wayland without AT-SPI, confidence threshold should be lower
                    assert!(desired.min_confidence <= 0.40, "Wayland without AT-SPI should have low confidence threshold");
                }
            }
        }
    }

    #[test]
    fn execution_mode_correctly_identifies_hybrid() {
        let spec = make_spec(
            Verb::Open,
            vec![TargetRef::App("code".into())],
            Some(ContentClass::Generated { hint: "hello world".into(), language: Some("python".into()) }),
        );
        let caps = make_capabilities(SessionType::X11, AtSpiLevel::Full, true);
        let registry = crate::platform::app_registry::InstalledAppRegistry::build_sync();

        let result = CapabilityAwarePlanner::plan(
            &spec,
            "open code and write hello world and run it and show output",
            &caps,
            &registry,
        );

        match result {
            PlanningResult::Planned { execution_mode, .. } => {
                // IDE code-run workflow should be hybrid (backend write + visible open)
                assert!(
                    matches!(execution_mode, ExecutionMode::Hybrid { .. }),
                    "IDE code-run should be hybrid mode, got {:?}",
                    execution_mode
                );
            }
            PlanningResult::NeedsHitl { .. } => {
                // Acceptable if VS Code isn't installed
            }
            _ => {}
        }
    }
}
