//! Deterministic GUI Eval Judge.
//!
//! Evaluates a GuiEvalObservation against a GuiEvalCase using structural
//! signals only — no LLM required. This makes the judge fast, reproducible,
//! and useful in CI environments without a running LLM.
//!
//! ## Verdict Logic
//!
//! 1. **Skip check**: If the display server requirement isn't met, skip.
//! 2. **False-success detection**: If KRIA reported success but no artifacts
//!    exist and no expected tools were called, it's a false success.
//! 3. **Retrieval leakage detection**: If forbidden tools were called, fail.
//! 4. **Artifact verification**: Check that expected files exist with correct content.
//! 5. **Tool call verification**: Check required tools were called.
//! 6. **Response pattern verification**: Check response contains/excludes patterns.
//! 7. **Quality scoring**: Compute a 0.0–1.0 quality score.

use super::invariants::evaluate_invariants;
use super::types::{
    FailureCategory, GuiEvalCase, GuiEvalObservation, GuiEvalPreflightStatus, GuiEvalVerdict,
    GuiEvalVerdictKind,
};

/// Deterministic judge for GUI eval cases.
pub struct GuiEvalJudge;

impl GuiEvalJudge {
    /// Evaluate an observation against a case and produce a verdict.
    pub fn evaluate(&self, case: &GuiEvalCase, obs: &GuiEvalObservation) -> GuiEvalVerdict {
        // ── Step 1: Skip check ────────────────────────────────────────────
        if obs.preflight.status == GuiEvalPreflightStatus::EnvironmentBlocked {
            let reasons = if obs.preflight.blocking_reasons.is_empty() {
                vec![obs.trace.final_response.clone()]
            } else {
                obs.preflight.blocking_reasons.clone()
            };
            return GuiEvalVerdict {
                case_id: case.id.clone(),
                kind: GuiEvalVerdictKind::EnvironmentBlocked,
                failure_category: Some(FailureCategory::EnvironmentBlocked),
                explanation: format!("Environment blocked: {}", reasons.join("; ")),
                evidence: reasons,
                recommended_fix: Some(
                    "Provision the required eval capability or run this case in the correct suite tier."
                        .to_string(),
                ),
                quality_score: 0.0,
            };
        }

        if let Some(reason) = obs.trace.final_response.strip_prefix("EVAL_BLOCKED: ") {
            return GuiEvalVerdict {
                case_id: case.id.clone(),
                kind: GuiEvalVerdictKind::EnvironmentBlocked,
                failure_category: Some(FailureCategory::EnvironmentBlocked),
                explanation: format!("Environment blocked: {}", reason),
                evidence: vec![reason.to_string()],
                recommended_fix: Some(
                    "Provision the required eval capability or run this case in the correct suite tier."
                        .to_string(),
                ),
                quality_score: 0.0,
            };
        }

        if let Some(reason) = obs.trace.final_response.strip_prefix("EVAL_SKIPPED: ") {
            return GuiEvalVerdict {
                case_id: case.id.clone(),
                kind: GuiEvalVerdictKind::Skip,
                failure_category: Some(FailureCategory::Skipped),
                explanation: format!("Skipped: {}", reason),
                evidence: vec![reason.to_string()],
                recommended_fix: None,
                quality_score: 1.0,
            };
        }

        if !case.display_server.is_satisfied() {
            return GuiEvalVerdict {
                case_id: case.id.clone(),
                kind: GuiEvalVerdictKind::Skip,
                failure_category: Some(FailureCategory::Skipped),
                explanation: format!(
                    "Skipped: display server requirement '{}' not met (detected: {})",
                    case.display_server.as_str(),
                    obs.display_server_detected
                ),
                evidence: vec![format!(
                    "Required: {}, Detected: {}",
                    case.display_server.as_str(),
                    obs.display_server_detected
                )],
                recommended_fix: None,
                quality_score: 1.0, // Skip is not a failure
            };
        }

        let mut evidence = Vec::new();
        let mut failures: Vec<(FailureCategory, String)> = Vec::new();

        // ── Step 2: Retrieval leakage detection ───────────────────────────
        for tool in &obs.trace.retrieval_tools_called {
            if case.expected_behavior.forbidden_tools.contains(tool)
                || matches!(
                    tool.as_str(),
                    "web_search" | "search_news" | "searxng_search"
                )
            {
                failures.push((
                    FailureCategory::RetrievalLeakage,
                    format!("Retrieval tool '{}' was called during a GUI workflow", tool),
                ));
                evidence.push(format!("RETRIEVAL LEAK: {} called", tool));
            }
        }

        // ── Step 3: Cloud LLM leakage detection ───────────────────────────
        // FIX #20: Detect any cloud LLM invocation, not just retried ones.
        if obs.trace.cloud_llm_invoked {
            failures.push((
                FailureCategory::CloudLlmLeakage,
                format!(
                    "Cloud LLM was invoked {} time(s) during GUI workflow",
                    obs.trace.llm_retry_count
                ),
            ));
            evidence.push(format!(
                "CLOUD LLM LEAK: {} retries",
                obs.trace.llm_retry_count
            ));
        }

        // ── Step 4: Forbidden tool check ──────────────────────────────────
        for tool in &case.expected_behavior.forbidden_tools {
            if obs.trace.tools_called.contains(tool) {
                failures.push((
                    FailureCategory::RetrievalLeakage,
                    format!("Forbidden tool '{}' was called", tool),
                ));
                evidence.push(format!("FORBIDDEN TOOL: {} called", tool));
            }
        }

        // ── Step 5: Required tool check ───────────────────────────────────
        for tool in &case.expected_behavior.required_tools {
            if !obs.trace.tools_called.contains(tool) {
                failures.push((
                    FailureCategory::WorkflowExecution,
                    format!("Required tool '{}' was not called", tool),
                ));
                evidence.push(format!("MISSING TOOL: {} not called", tool));
            }
        }

        // ── Step 6: Artifact verification ─────────────────────────────────
        if !case.expected_behavior.expected_artifacts.is_empty() {
            if obs.artifacts_found.is_empty() {
                // No artifacts found at all
                if obs.trace.reported_success {
                    // KRIA said it succeeded but nothing was created
                    failures.push((
                        FailureCategory::FalseSuccess,
                        "KRIA reported success but no artifacts were created".to_string(),
                    ));
                    evidence.push(
                        "FALSE SUCCESS: no artifacts found despite reported success".to_string(),
                    );
                } else {
                    failures.push((
                        FailureCategory::WorkflowExecution,
                        "Expected artifacts were not created".to_string(),
                    ));
                    evidence.push("MISSING ARTIFACTS: no files created".to_string());
                }
            } else {
                // Check content of found artifacts
                for artifact_obs in &obs.artifacts_found {
                    if !artifact_obs.content_matches_expected {
                        failures.push((
                            FailureCategory::WorkflowExecution,
                            format!(
                                "Artifact '{}' exists but content doesn't match expected",
                                artifact_obs.path.display()
                            ),
                        ));
                        evidence.push(format!(
                            "WRONG CONTENT: {} (preview: {})",
                            artifact_obs.path.display(),
                            &artifact_obs.content_preview
                                [..artifact_obs.content_preview.len().min(80)]
                        ));
                    } else {
                        evidence.push(format!(
                            "ARTIFACT OK: {} ({} bytes)",
                            artifact_obs.path.display(),
                            artifact_obs.size_bytes
                        ));
                    }
                }
            }
        }

        // ── Step 7: Response pattern checks ───────────────────────────────
        let response_lower = obs.trace.final_response.to_ascii_lowercase();

        for pattern in &case.expected_behavior.forbidden_response_patterns {
            if response_lower.contains(&pattern.to_ascii_lowercase()) {
                failures.push((
                    FailureCategory::FalseSuccess,
                    format!("Response contains forbidden pattern: '{}'", pattern),
                ));
                evidence.push(format!("FORBIDDEN RESPONSE PATTERN: '{}'", pattern));
            }
        }

        if !case.expected_behavior.required_response_patterns.is_empty() {
            let any_required_found = case
                .expected_behavior
                .required_response_patterns
                .iter()
                .any(|p| response_lower.contains(&p.to_ascii_lowercase()));
            if !any_required_found {
                failures.push((
                    FailureCategory::WorkflowExecution,
                    format!(
                        "Response missing required patterns: {:?}",
                        case.expected_behavior.required_response_patterns
                    ),
                ));
                evidence.push(format!(
                    "MISSING RESPONSE PATTERN: none of {:?} found",
                    case.expected_behavior.required_response_patterns
                ));
            }
        }

        // ── Step 8: False-success detection (generic) ─────────────────────
        let false_success_patterns = [
            "done! i completed",
            "i have successfully",
            "task completed successfully",
            "automation task",
        ];
        let has_false_success_phrase = false_success_patterns
            .iter()
            .any(|p| response_lower.contains(p));

        if has_false_success_phrase
            && obs.artifacts_found.is_empty()
            && case.expected_behavior.expect_success
            && !case.expected_behavior.expected_artifacts.is_empty()
        {
            failures.push((
                FailureCategory::FalseSuccess,
                "Response contains success claim but no verifiable artifacts exist".to_string(),
            ));
            evidence.push("FALSE SUCCESS PHRASE detected with no artifacts".to_string());
        }

        // ── Step 9: Substrate check ───────────────────────────────────────
        if let Some(expected_substrate) = &case.expected_behavior.substrate {
            if let Some(actual_substrate) = &obs.trace.substrate_selected {
                if actual_substrate != expected_substrate {
                    failures.push((
                        FailureCategory::SubstratePlanning,
                        format!(
                            "Wrong substrate: expected '{}', got '{}'",
                            expected_substrate, actual_substrate
                        ),
                    ));
                    evidence.push(format!(
                        "WRONG SUBSTRATE: expected={}, actual={}",
                        expected_substrate, actual_substrate
                    ));
                } else {
                    evidence.push(format!("SUBSTRATE OK: {}", actual_substrate));
                }
            } else if case.expected_behavior.expect_success {
                failures.push((
                    FailureCategory::SubstratePlanning,
                    "No substrate was selected (workflow may not have been routed to HTN executor)"
                        .to_string(),
                ));
                evidence.push("NO SUBSTRATE: workflow not routed to HTN executor".to_string());
            }
        }

        // ── Step 10: Compute quality score ────────────────────────────────
        let quality_score = self.compute_quality_score(case, obs, &failures);
        let invariant_report = evaluate_invariants(case, obs);

        // ── Step 11: Build verdict ────────────────────────────────────────
        if failures.is_empty() {
            if invariant_report.has_release_blocking_violation() {
                let evidence = invariant_report.release_blocking_messages();
                return GuiEvalVerdict {
                    case_id: case.id.clone(),
                    kind: GuiEvalVerdictKind::Fail,
                    failure_category: Some(FailureCategory::InvariantViolation),
                    explanation: format!(
                        "FAIL: {} — deterministic invariant violation",
                        case.description
                    ),
                    evidence,
                    recommended_fix: Some(
                        "Fix the violated policy/verifier/lease/HITL invariant before promoting this eval result."
                            .to_string(),
                    ),
                    quality_score: 0.0,
                };
            }

            evidence.push(format!(
                "All {} assertions passed",
                self.count_assertions(case)
            ));
            return GuiEvalVerdict {
                case_id: case.id.clone(),
                kind: GuiEvalVerdictKind::Pass,
                failure_category: None,
                explanation: format!("PASS: {} — all assertions verified", case.description),
                evidence,
                recommended_fix: None,
                quality_score,
            };
        }

        // Sort failures by priority (most critical first)
        let mut sorted_failures = failures;
        sorted_failures.sort_by_key(|(cat, _)| cat.priority());

        let (primary_category, primary_reason) = sorted_failures.remove(0);

        // Determine verdict kind
        let kind = match &primary_category {
            FailureCategory::FalseSuccess => GuiEvalVerdictKind::FalseSuccess,
            FailureCategory::RetrievalLeakage | FailureCategory::CloudLlmLeakage => {
                GuiEvalVerdictKind::RetrievalLeakage
            }
            _ => GuiEvalVerdictKind::Fail,
        };

        let recommended_fix = self.recommend_fix(&primary_category, case, obs);

        GuiEvalVerdict {
            case_id: case.id.clone(),
            kind,
            failure_category: Some(primary_category),
            explanation: format!("FAIL: {} — {}", case.description, primary_reason),
            evidence,
            recommended_fix: Some(recommended_fix),
            quality_score,
        }
    }

    fn compute_quality_score(
        &self,
        case: &GuiEvalCase,
        obs: &GuiEvalObservation,
        failures: &[(FailureCategory, String)],
    ) -> f32 {
        if failures.is_empty() {
            return 1.0;
        }

        let mut score = 1.0f32;

        // Deduct for each failure type
        for (cat, _) in failures {
            score -= match cat {
                FailureCategory::FalseSuccess => 0.5,
                FailureCategory::RetrievalLeakage => 0.3,
                FailureCategory::CloudLlmLeakage => 0.2,
                FailureCategory::AppResolution => 0.4,
                FailureCategory::SemanticParsing => 0.4,
                FailureCategory::SubstratePlanning => 0.3,
                FailureCategory::WorkflowExecution => 0.3,
                FailureCategory::VerificationFailure => 0.2,
                FailureCategory::WindowManagement => 0.15,
                FailureCategory::MissingRecovery => 0.1,
                _ => 0.1,
            };
        }

        // Partial credit for artifacts found
        if !obs.artifacts_found.is_empty() {
            score += 0.1;
        }

        // Partial credit for correct tools called
        let required_called = case
            .expected_behavior
            .required_tools
            .iter()
            .filter(|t| obs.trace.tools_called.contains(t))
            .count();
        let total_required = case.expected_behavior.required_tools.len().max(1);
        score += 0.1 * (required_called as f32 / total_required as f32);

        score.clamp(0.0, 1.0)
    }

    fn count_assertions(&self, case: &GuiEvalCase) -> usize {
        case.expected_behavior.expected_artifacts.len()
            + case.expected_behavior.required_tools.len()
            + case.expected_behavior.forbidden_tools.len()
            + case.expected_behavior.forbidden_response_patterns.len()
            + case.expected_behavior.required_response_patterns.len()
            + if case.expected_behavior.substrate.is_some() {
                1
            } else {
                0
            }
    }

    fn recommend_fix(
        &self,
        category: &FailureCategory,
        case: &GuiEvalCase,
        obs: &GuiEvalObservation,
    ) -> String {
        match category {
            FailureCategory::FalseSuccess => {
                "Fix: Replace hardcoded 'Done!' success strings with verified-outcome reporting. \
                 Only report success when FileSystemEffect or ProcessLaunched verification passes. \
                 See loop_engine/mod.rs execute_workflow result handling."
                    .to_string()
            }
            FailureCategory::RetrievalLeakage => {
                "Fix: Add browser_search/open_application to detect_satisfaction() in turn_memory.rs \
                 so the loop terminates after a successful GUI launch. Also add forced_is_gui_launch \
                 filter to exclude web_search/search_news from the tool schema when GUI is forced."
                    .to_string()
            }
            FailureCategory::CloudLlmLeakage => {
                "Fix: GUI workflows should not invoke the cloud LLM. Check that the HTN executor \
                 path (should_route_to_gui_executor) is correctly routing the prompt before the \
                 ReAct loop starts. Verify intent confidence threshold (MIN_GUI_INTENT_CONFIDENCE=0.6)."
                    .to_string()
            }
            FailureCategory::AppResolution => {
                format!(
                    "Fix: Add '{}' to the conjunction-aware extract_app() in intent_compiler_llm.rs. \
                     Ensure the STOP_TOKENS list includes 'and', 'then', '&'. \
                     Add whole-word matching via contains_word() for bare app names.",
                    case.prompt
                        .split_whitespace()
                        .nth(1)
                        .unwrap_or("unknown")
                )
            }
            FailureCategory::SemanticParsing => {
                "Fix: The RuleIntentCompiler failed to parse the compound prompt. \
                 Check compound_markers in the Verb::Open branch. \
                 Consider routing to LlmIntentCompiler for complex multi-verb prompts."
                    .to_string()
            }
            FailureCategory::SubstratePlanning => {
                format!(
                    "Fix: SubstratePlanner returned Unknown for this spec. \
                     Check that the GuiTaskSpec has primary_verb=Open, a TargetRef::App, \
                     and ContentClass::Generated. \
                     Observed substrate: {:?}",
                    obs.trace.substrate_selected
                )
            }
            FailureCategory::WorkflowExecution => {
                "Fix: A workflow step failed. Check that RegistryToolExecutor uses \
                 execute_with_context (not execute) so tools like write_file work correctly. \
                 Verify the tool is registered in the ToolRegistry."
                    .to_string()
            }
            FailureCategory::VerificationFailure => {
                "Fix: Verification failed. For FileWriteThenOpen workflows, Step 2 \
                 (open_application_with_file) should use VerificationType::None to avoid \
                 X11-only WindowState checks. Step 1 (write_file) should use FileSystemEffect."
                    .to_string()
            }
            FailureCategory::WindowManagement => {
                "Fix: Window state check failed (likely WINDOW_ID_FAILED on Wayland). \
                 Replace WindowState verification with ProcessLaunched or FileSystemEffect \
                 for Wayland-compatible workflows. The uinput-daemon uses xdotool which is X11-only."
                    .to_string()
            }
            FailureCategory::SessionReuse => {
                "Fix: KRIA launched a duplicate app instead of reusing the existing session. \
                 Add is_process_running() check in SubstratePlanner before emitting open_application. \
                 If app is already running, skip the launch step and proceed to file writing."
                    .to_string()
            }
            FailureCategory::AppLifecycle => {
                "Fix: App lifecycle handling failed. Check that open_application_with_file \
                 correctly dispatches via IntentDispatcher with the file as a SafeArg. \
                 Verify the app is in InstalledAppRegistry (check .desktop file scanning)."
                    .to_string()
            }
            _ => format!(
                "Fix: Investigate {} failure in the GUI automation pipeline.",
                category.as_str()
            ),
        }
    }
}
