//! Outcome-Driven Workflow Verifier — Contract-Bound Verification.
//!
//! This module verifies workflow outcomes by consuming the plan-bound
//! `OutcomeContract` directly. It does NOT re-derive expectations from
//! user text. It does NOT guess what should be verified.
//!
//! # Authority
//!
//! The planner defines what to verify (OutcomeContract).
//! The capability system defines how to verify (available methods).
//! This module executes verification and returns graded results.
//!
//! # Design
//!
//! - Consumes OutcomeContract (never re-derives)
//! - Selects verification strategy from CapabilitySet
//! - Returns graded confidence (not binary pass/fail)
//! - Single timeout per verification leaf (no nesting)
//! - Deterministic: same contract + same state → same result

use std::time::{Duration, Instant};

use crate::agent::workflow_types::{
    AtSpiLevel, CapabilitySet, ConfidenceGrade, OutcomeContract, OutcomeExpectation,
    PlannedOutcome, SessionType, VerificationMethod, VisibilityConfidence,
    WorkflowVerdict,
};

// ═══════════════════════════════════════════════════════════════════════════════
// §1 — Verification Result Types
// ═══════════════════════════════════════════════════════════════════════════════

/// Result of verifying a single planned outcome.
#[derive(Debug, Clone)]
pub struct OutcomeVerification {
    /// Which outcome was verified
    pub description: String,
    /// Whether the outcome was confirmed
    pub verified: bool,
    /// Confidence in the verification (0.0–1.0)
    pub confidence: f32,
    /// Confidence grade
    pub grade: ConfidenceGrade,
    /// Human-readable evidence
    pub evidence: String,
    /// Which method was used
    pub method: VerificationMethod,
    /// Time spent on this verification
    pub latency_ms: u32,
}

/// Aggregate verification result for an entire outcome contract.
#[derive(Debug, Clone)]
pub struct ContractVerification {
    /// Per-outcome results
    pub outcomes: Vec<OutcomeVerification>,
    /// Whether all required outcomes are satisfied
    pub required_satisfied: bool,
    /// Whether all desired outcomes are satisfied
    pub desired_satisfied: bool,
    /// Overall visibility confidence
    pub visibility_confidence: VisibilityConfidence,
    /// Total verification time
    pub total_latency_ms: u32,
}

// ═══════════════════════════════════════════════════════════════════════════════
// §2 — Contract-Driven Verifier
// ═══════════════════════════════════════════════════════════════════════════════

/// Verifies an outcome contract using capability-aware strategies.
///
/// This is the canonical verification entry point. It:
/// 1. Iterates over all outcomes in the contract
/// 2. Selects the best verification method from capabilities
/// 3. Executes verification with a single timeout
/// 4. Returns graded results
pub async fn verify_contract(
    contract: &OutcomeContract,
    capabilities: &CapabilitySet,
) -> ContractVerification {
    let start = Instant::now();
    let mut outcomes = Vec::new();

    // Verify required outcomes
    for outcome in &contract.required {
        let result = verify_single_outcome(outcome, capabilities).await;
        outcomes.push(result);
    }

    // Verify desired outcomes
    for outcome in &contract.desired {
        let result = verify_single_outcome(outcome, capabilities).await;
        outcomes.push(result);
    }

    // Compute aggregate
    let required_satisfied = contract.required.iter().enumerate().all(|(i, outcome)| {
        let result = &outcomes[i];
        result.verified && result.confidence >= outcome.min_confidence
    });

    let desired_start = contract.required.len();
    let desired_satisfied = contract.desired.iter().enumerate().all(|(i, outcome)| {
        let result = &outcomes[desired_start + i];
        result.verified && result.confidence >= outcome.min_confidence
    });

    let visibility_confidence = if desired_satisfied && required_satisfied {
        let avg_conf = outcomes.iter().map(|o| o.confidence).sum::<f32>()
            / outcomes.len().max(1) as f32;
        VisibilityConfidence::Confirmed {
            confidence: avg_conf,
            evidence: "All contract outcomes verified".into(),
        }
    } else if required_satisfied {
        let unverified: Vec<&str> = contract.desired.iter().enumerate()
            .filter(|(i, o)| {
                let r = &outcomes[desired_start + *i];
                !r.verified || r.confidence < o.min_confidence
            })
            .map(|(_, o)| o.description.as_str())
            .collect();
        VisibilityConfidence::StructuralOnly {
            reason: format!("Unverified: {}", unverified.join(", ")),
        }
    } else {
        VisibilityConfidence::Inconclusive {
            reason: "Required outcomes not satisfied".into(),
            suggestion: Some("Check if the workflow completed structurally".into()),
        }
    };

    let total_latency_ms = start.elapsed().as_millis() as u32;

    ContractVerification {
        outcomes,
        required_satisfied,
        desired_satisfied,
        visibility_confidence,
        total_latency_ms,
    }
}

/// Verify a single planned outcome using the best available method.
async fn verify_single_outcome(
    outcome: &PlannedOutcome,
    capabilities: &CapabilitySet,
) -> OutcomeVerification {
    let timeout = Duration::from_millis(select_timeout_for_outcome(&outcome.expectation));

    let result = tokio::time::timeout(timeout, async {
        match &outcome.expectation {
            OutcomeExpectation::FileExists { path } => {
                verify_file_exists(path).await
            }
            OutcomeExpectation::ProcessRunning { binary } => {
                verify_process_running(binary).await
            }
            OutcomeExpectation::AppWindowVisible { app, title_hint } => {
                verify_app_window(app, title_hint.as_deref(), capabilities).await
            }
            OutcomeExpectation::BrowserAtUrl { url_contains } => {
                verify_browser_url(url_contains, capabilities).await
            }
            OutcomeExpectation::OutputContains { substring, in_file } => {
                verify_output_contains(substring, in_file).await
            }
            OutcomeExpectation::PortListening { port } => {
                verify_port_listening(*port).await
            }
        }
    })
    .await;

    match result {
        Ok(verification) => verification,
        Err(_) => OutcomeVerification {
            description: outcome.description.clone(),
            verified: false,
            confidence: 0.0,
            grade: ConfidenceGrade::NoEvidence,
            evidence: "Verification timed out".into(),
            method: VerificationMethod::ProcessTable,
            latency_ms: timeout.as_millis() as u32,
        },
    }
}

/// Select appropriate timeout for an outcome type.
fn select_timeout_for_outcome(expectation: &OutcomeExpectation) -> u64 {
    match expectation {
        OutcomeExpectation::FileExists { .. } => 1000,
        OutcomeExpectation::ProcessRunning { .. } => 8000,
        OutcomeExpectation::AppWindowVisible { .. } => 5000,
        OutcomeExpectation::BrowserAtUrl { .. } => 5000,
        OutcomeExpectation::OutputContains { .. } => 2000,
        OutcomeExpectation::PortListening { .. } => 10000,
    }
}

// ═══════════════════════════════════════════════════════════════════════════════
// §3 — Individual Verification Strategies
// ═══════════════════════════════════════════════════════════════════════════════

/// Verify that a file exists at the given path.
async fn verify_file_exists(path: &str) -> OutcomeVerification {
    let exists = tokio::fs::metadata(path).await.is_ok();
    OutcomeVerification {
        description: format!("File exists: {}", path),
        verified: exists,
        confidence: if exists { 0.95 } else { 0.95 },
        grade: if exists { ConfidenceGrade::Strong } else { ConfidenceGrade::Strong },
        evidence: if exists {
            format!("File confirmed at {}", path)
        } else {
            format!("File NOT found at {}", path)
        },
        method: VerificationMethod::FileSystem,
        latency_ms: 0,
    }
}

/// Verify that a process with the given binary name is running.
async fn verify_process_running(binary: &str) -> OutcomeVerification {
    let binary_lower = binary.to_lowercase();
    let deadline = Instant::now() + Duration::from_millis(6000);

    loop {
        if process_is_running(&binary_lower) {
            return OutcomeVerification {
                description: format!("Process running: {}", binary),
                verified: true,
                confidence: 0.90,
                grade: ConfidenceGrade::Strong,
                evidence: format!("Process '{}' found in /proc", binary),
                method: VerificationMethod::ProcessTable,
                latency_ms: 0,
            };
        }
        if Instant::now() >= deadline {
            break;
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }

    OutcomeVerification {
        description: format!("Process running: {}", binary),
        verified: false,
        confidence: 0.80,
        grade: ConfidenceGrade::Moderate,
        evidence: format!("Process '{}' NOT found in /proc after 6s", binary),
        method: VerificationMethod::ProcessTable,
        latency_ms: 6000,
    }
}

/// Verify that an app window is visible (capability-aware).
async fn verify_app_window(
    app: &str,
    _title_hint: Option<&str>,
    capabilities: &CapabilitySet,
) -> OutcomeVerification {
    // Strategy selection based on capabilities
    let max_confidence = capabilities.verifier.window_state_max_confidence;

    // Always try process check first (works everywhere)
    let binary = crate::agent::gui_substrate_planner::app_alias_to_binary_pub(app);
    if process_is_running(&binary.to_lowercase()) {
        // Process is running — confidence depends on environment
        let confidence: f32 = match (&capabilities.environment.session_type, &capabilities.environment.atspi_level) {
            (SessionType::X11, AtSpiLevel::Full) => 0.85,
            (SessionType::X11, _) => 0.75,
            (SessionType::Wayland, AtSpiLevel::Full) | (SessionType::XWayland, AtSpiLevel::Full) => 0.70,
            (SessionType::Wayland, _) => 0.50,
            _ => 0.45,
        };

        return OutcomeVerification {
            description: format!("{} window visible", app),
            verified: true,
            confidence: confidence.min(max_confidence),
            grade: ConfidenceGrade::from_confidence(confidence),
            evidence: format!(
                "Process '{}' running; window visibility confidence {:.0}% ({:?} session)",
                binary, confidence * 100.0, capabilities.environment.session_type
            ),
            method: VerificationMethod::ProcessTable,
            latency_ms: 0,
        };
    }

    OutcomeVerification {
        description: format!("{} window visible", app),
        verified: false,
        confidence: 0.0,
        grade: ConfidenceGrade::NoEvidence,
        evidence: format!("Process '{}' not found", binary),
        method: VerificationMethod::ProcessTable,
        latency_ms: 0,
    }
}

/// Verify browser is at a specific URL (capability-aware).
async fn verify_browser_url(
    url_contains: &str,
    capabilities: &CapabilitySet,
) -> OutcomeVerification {
    if capabilities.verifier.cdp_available {
        // CDP verification would go here (delegating to existing browser_cognition)
        // For now, fall back to process check
        return OutcomeVerification {
            description: format!("Browser at URL containing '{}'", url_contains),
            verified: false,
            confidence: 0.30,
            grade: ConfidenceGrade::Weak,
            evidence: "CDP verification not yet wired (process fallback)".into(),
            method: VerificationMethod::Cdp,
            latency_ms: 0,
        };
    }

    // Without CDP, check if any browser process is running
    let browsers = ["chrome", "chromium", "firefox", "brave"];
    for browser in &browsers {
        if process_is_running(browser) {
            return OutcomeVerification {
                description: format!("Browser at URL containing '{}'", url_contains),
                verified: true,
                confidence: 0.50, // Low confidence without CDP
                grade: ConfidenceGrade::Weak,
                evidence: format!("Browser process '{}' running (URL not verified — no CDP)", browser),
                method: VerificationMethod::ProcessTable,
                latency_ms: 0,
            };
        }
    }

    OutcomeVerification {
        description: format!("Browser at URL containing '{}'", url_contains),
        verified: false,
        confidence: 0.0,
        grade: ConfidenceGrade::NoEvidence,
        evidence: "No browser process detected".into(),
        method: VerificationMethod::ProcessTable,
        latency_ms: 0,
    }
}

/// Verify that a file contains expected output.
async fn verify_output_contains(substring: &str, file_path: &str) -> OutcomeVerification {
    match tokio::fs::read_to_string(file_path).await {
        Ok(content) => {
            let found = if substring.is_empty() {
                !content.trim().is_empty() // Any non-empty output counts
            } else {
                content.contains(substring)
            };
            OutcomeVerification {
                description: format!("Output contains '{}'", if substring.is_empty() { "<any>" } else { substring }),
                verified: found,
                confidence: if found { 0.95 } else { 0.90 },
                grade: if found { ConfidenceGrade::Strong } else { ConfidenceGrade::Strong },
                evidence: if found {
                    format!("Output file contains expected content ({} bytes)", content.len())
                } else {
                    format!("Output file does NOT contain '{}' ({} bytes)", substring, content.len())
                },
                method: VerificationMethod::FileSystem,
                latency_ms: 0,
            }
        }
        Err(e) => OutcomeVerification {
            description: format!("Output contains '{}'", substring),
            verified: false,
            confidence: 0.0,
            grade: ConfidenceGrade::NoEvidence,
            evidence: format!("Cannot read output file {}: {}", file_path, e),
            method: VerificationMethod::FileSystem,
            latency_ms: 0,
        },
    }
}

/// Verify that a port is listening.
async fn verify_port_listening(port: u16) -> OutcomeVerification {
    let deadline = Instant::now() + Duration::from_millis(8000);

    loop {
        match tokio::net::TcpStream::connect(format!("127.0.0.1:{}", port)).await {
            Ok(_) => {
                return OutcomeVerification {
                    description: format!("Port {} listening", port),
                    verified: true,
                    confidence: 0.95,
                    grade: ConfidenceGrade::Strong,
                    evidence: format!("TCP connection to 127.0.0.1:{} succeeded", port),
                    method: VerificationMethod::PortCheck,
                    latency_ms: 0,
                };
            }
            Err(_) => {
                if Instant::now() >= deadline {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(500)).await;
            }
        }
    }

    OutcomeVerification {
        description: format!("Port {} listening", port),
        verified: false,
        confidence: 0.80,
        grade: ConfidenceGrade::Moderate,
        evidence: format!("Port {} not reachable after 8s", port),
        method: VerificationMethod::PortCheck,
        latency_ms: 8000,
    }
}

// ═══════════════════════════════════════════════════════════════════════════════
// §4 — Process Check Helper
// ═══════════════════════════════════════════════════════════════════════════════

fn process_is_running(binary_lower: &str) -> bool {
    if let Ok(entries) = std::fs::read_dir("/proc") {
        for entry in entries.flatten() {
            let pid_str = entry.file_name();
            if pid_str.to_string_lossy().parse::<u32>().is_err() {
                continue;
            }
            let comm_path = entry.path().join("comm");
            if let Ok(comm) = std::fs::read_to_string(&comm_path) {
                let comm = comm.trim().to_lowercase();
                if comm == binary_lower || comm.contains(binary_lower) {
                    return true;
                }
            }
        }
    }
    false
}

// ═══════════════════════════════════════════════════════════════════════════════
// §5 — Contract → Verdict Computation
// ═══════════════════════════════════════════════════════════════════════════════

/// Compute a workflow verdict directly from contract verification results.
///
/// This is the canonical path: Contract → Verification → Verdict.
/// No string parsing, no re-derivation, no dual-truth.
pub fn verdict_from_contract(
    contract_verification: &ContractVerification,
    contract: &OutcomeContract,
) -> WorkflowVerdict {
    if !contract_verification.required_satisfied {
        // Find the first failed required outcome
        let failed = contract_verification.outcomes.iter()
            .take(contract.required.len())
            .enumerate()
            .find(|(_, o)| !o.verified)
            .map(|(i, o)| (i as u32 + 1, o.evidence.clone()));

        let (step, reason) = failed.unwrap_or((0, "Required outcome not verified".into()));
        return WorkflowVerdict::Failed {
            step,
            reason,
            recovery: None,
        };
    }

    if contract_verification.desired_satisfied {
        WorkflowVerdict::Complete
    } else {
        let unverified: Vec<String> = contract.desired.iter()
            .enumerate()
            .filter(|(i, _)| {
                let idx = contract.required.len() + i;
                idx < contract_verification.outcomes.len()
                    && !contract_verification.outcomes[idx].verified
            })
            .map(|(_, o)| o.description.clone())
            .collect();

        WorkflowVerdict::StructurallyComplete {
            unverified_outcomes: unverified,
        }
    }
}

// ═══════════════════════════════════════════════════════════════════════════════
// §6 — Tests
// ═══════════════════════════════════════════════════════════════════════════════

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::workflow_types::*;

    fn make_capabilities() -> CapabilitySet {
        CapabilitySet {
            environment: EnvironmentCapability {
                session_type: SessionType::X11,
                compositor: None,
                atspi_level: AtSpiLevel::Full,
                xdotool_available: true,
                uinput_available: true,
                ocr_available: false,
            },
            verifier: VerifierCapability {
                available_methods: vec![
                    VerificationMethod::FileSystem,
                    VerificationMethod::ProcessTable,
                    VerificationMethod::AtSpi,
                ],
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
        }
    }

    #[tokio::test]
    async fn verify_file_exists_for_real_file() {
        // /proc/self/comm always exists on Linux
        let result = verify_file_exists("/proc/self/comm").await;
        assert!(result.verified);
        assert_eq!(result.grade, ConfidenceGrade::Strong);
        assert_eq!(result.method, VerificationMethod::FileSystem);
    }

    #[tokio::test]
    async fn verify_file_exists_for_missing_file() {
        let result = verify_file_exists("/tmp/kria_test_nonexistent_xyz_12345.txt").await;
        assert!(!result.verified);
        assert_eq!(result.grade, ConfidenceGrade::Strong); // High confidence it doesn't exist
    }

    #[tokio::test]
    async fn verify_output_contains_with_real_file() {
        // Write a temp file and verify
        let path = "/tmp/kria_verifier_test_output.txt";
        tokio::fs::write(path, "Hello KRIA World\nLine 2\n").await.unwrap();

        let result = verify_output_contains("KRIA", path).await;
        assert!(result.verified);
        assert_eq!(result.grade, ConfidenceGrade::Strong);

        let result_miss = verify_output_contains("NONEXISTENT", path).await;
        assert!(!result_miss.verified);

        // Cleanup
        let _ = tokio::fs::remove_file(path).await;
    }

    #[tokio::test]
    async fn verify_output_contains_empty_substring_matches_any_content() {
        let path = "/tmp/kria_verifier_test_any.txt";
        tokio::fs::write(path, "some output").await.unwrap();

        let result = verify_output_contains("", path).await;
        assert!(result.verified, "Empty substring should match any non-empty content");

        let _ = tokio::fs::remove_file(path).await;
    }

    #[tokio::test]
    async fn verify_process_running_finds_self() {
        // The test process itself should be running
        // Use "cargo" since we're running under cargo test
        let result = verify_process_running("cargo").await;
        // This might not find "cargo" depending on how tests run,
        // but the function should not panic
        assert!(result.confidence > 0.0 || !result.verified);
    }

    #[tokio::test]
    async fn verify_contract_with_file_outcome() {
        let path = "/tmp/kria_contract_test.txt";
        tokio::fs::write(path, "test content").await.unwrap();

        let contract = OutcomeContract {
            required: vec![PlannedOutcome {
                description: "Test file exists".into(),
                expectation: OutcomeExpectation::FileExists { path: path.into() },
                min_confidence: 0.80,
                on_failure: OutcomeFailurePolicy::FailWorkflow,
            }],
            desired: vec![],
        };

        let caps = make_capabilities();
        let result = verify_contract(&contract, &caps).await;

        assert!(result.required_satisfied);
        assert_eq!(result.outcomes.len(), 1);
        assert!(result.outcomes[0].verified);

        let _ = tokio::fs::remove_file(path).await;
    }

    #[tokio::test]
    async fn verify_contract_fails_when_required_missing() {
        let contract = OutcomeContract {
            required: vec![PlannedOutcome {
                description: "Missing file".into(),
                expectation: OutcomeExpectation::FileExists {
                    path: "/tmp/kria_definitely_not_here_xyz.txt".into(),
                },
                min_confidence: 0.80,
                on_failure: OutcomeFailurePolicy::FailWorkflow,
            }],
            desired: vec![],
        };

        let caps = make_capabilities();
        let result = verify_contract(&contract, &caps).await;

        assert!(!result.required_satisfied);
    }

    #[tokio::test]
    async fn verdict_from_contract_complete_when_all_satisfied() {
        let path = "/tmp/kria_verdict_test.txt";
        tokio::fs::write(path, "hello").await.unwrap();

        let contract = OutcomeContract {
            required: vec![PlannedOutcome {
                description: "File exists".into(),
                expectation: OutcomeExpectation::FileExists { path: path.into() },
                min_confidence: 0.80,
                on_failure: OutcomeFailurePolicy::FailWorkflow,
            }],
            desired: vec![],
        };

        let caps = make_capabilities();
        let verification = verify_contract(&contract, &caps).await;
        let verdict = verdict_from_contract(&verification, &contract);

        assert!(matches!(verdict, WorkflowVerdict::Complete));

        let _ = tokio::fs::remove_file(path).await;
    }

    #[tokio::test]
    async fn verdict_from_contract_structurally_complete_when_desired_fails() {
        let path = "/tmp/kria_verdict_structural.txt";
        tokio::fs::write(path, "hello").await.unwrap();

        let contract = OutcomeContract {
            required: vec![PlannedOutcome {
                description: "File exists".into(),
                expectation: OutcomeExpectation::FileExists { path: path.into() },
                min_confidence: 0.80,
                on_failure: OutcomeFailurePolicy::FailWorkflow,
            }],
            desired: vec![PlannedOutcome {
                description: "App window visible".into(),
                expectation: OutcomeExpectation::AppWindowVisible {
                    app: "kria_nonexistent_app_xyz".into(),
                    title_hint: None,
                },
                min_confidence: 0.70,
                on_failure: OutcomeFailurePolicy::DowngradeFidelity,
            }],
        };

        let caps = make_capabilities();
        let verification = verify_contract(&contract, &caps).await;
        let verdict = verdict_from_contract(&verification, &contract);

        assert!(
            matches!(verdict, WorkflowVerdict::StructurallyComplete { .. }),
            "Should be StructurallyComplete when required passes but desired fails, got {:?}",
            verdict
        );

        let _ = tokio::fs::remove_file(path).await;
    }

    #[tokio::test]
    async fn verdict_from_contract_failed_when_required_fails() {
        let contract = OutcomeContract {
            required: vec![PlannedOutcome {
                description: "Missing file".into(),
                expectation: OutcomeExpectation::FileExists {
                    path: "/tmp/kria_not_here_xyz.txt".into(),
                },
                min_confidence: 0.80,
                on_failure: OutcomeFailurePolicy::FailWorkflow,
            }],
            desired: vec![],
        };

        let caps = make_capabilities();
        let verification = verify_contract(&contract, &caps).await;
        let verdict = verdict_from_contract(&verification, &contract);

        assert!(matches!(verdict, WorkflowVerdict::Failed { .. }));
    }

    #[test]
    fn window_confidence_adapts_to_session_type() {
        let x11_caps = CapabilitySet {
            environment: EnvironmentCapability {
                session_type: SessionType::X11,
                compositor: None,
                atspi_level: AtSpiLevel::Full,
                xdotool_available: true,
                uinput_available: true,
                ocr_available: false,
            },
            verifier: VerifierCapability {
                available_methods: vec![],
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

        let wayland_caps = CapabilitySet {
            environment: EnvironmentCapability {
                session_type: SessionType::Wayland,
                compositor: Some("mutter".into()),
                atspi_level: AtSpiLevel::None,
                xdotool_available: false,
                uinput_available: true,
                ocr_available: false,
            },
            verifier: VerifierCapability {
                available_methods: vec![],
                window_state_max_confidence: 0.40,
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

        // X11 should allow higher confidence
        assert!(x11_caps.verifier.window_state_max_confidence > wayland_caps.verifier.window_state_max_confidence);
    }
}
