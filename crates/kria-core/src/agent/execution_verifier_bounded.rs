//! BoundedExecutionVerifier — Production execution verification.
//!
//! # Design
//!
//! Replaces `NoopExecutionVerifier` with real verification backed by existing
//! engines (AT-SPI, CDP, filesystem, process queries).
//!
//! # Authority Boundary
//!
//! - The verifier NEVER replans, retries, or executes actions.
//! - The verifier ONLY observes state and returns `VerifyOutcome`.
//! - Each verification is bounded to `MAX_VERIFY_MS` (default 500ms).
//! - On timeout, returns `verified = false` with `Unobservable` tier.
//!
//! # Verification Strategy by Class
//!
//! | Class                  | Engine             | Tier           |
//! |------------------------|--------------------|----------------|
//! | WindowState            | AT-SPI + xdotool   | FullSemantic   |
//! | AccessibilityElement   | AT-SPI             | FullSemantic   |
//! | InteractionOutcome     | AT-SPI             | FullSemantic   |
//! | BrowserPageLoaded      | CDP                | FullSemantic   |
//! | FileSystemEffect       | stdlib fs          | PartialObsrv   |
//! | ProcessLaunched        | /proc scan         | PartialObsrv   |
//! | DeterministicOutput    | file/terminal read | PartialObsrv   |
//! | OcrTextPresent         | Tesseract          | StructuralOnly |
//! | UserAttested           | HITL prompt        | Unobservable   |
//! | Unverifiable           | — (honest)         | Unobservable   |

use std::path::Path;
use std::sync::Arc;
use std::time::Instant;

use async_trait::async_trait;
use tracing::debug;

use crate::agent::execution_verifier::{
    ExecutionVerifier, FsEffect, RichVerifyOutcome, Verifiability, VerificationConfidenceTier,
    VerificationEvidenceSource, VerificationReliability, VerifyOutcome, VerifyTarget,
};
use crate::agent::window_observer::{observation_timeout, LiveWindowObserver, WindowObserver};
use crate::tools::gui_automation::GuiBackend;

/// Maximum time to spend on a single verification before failing closed.
/// WindowState verification is given extra budget because it includes an initial
/// delay (200 ms) plus up to 5 × 200 ms polling rounds = 1200 ms minimum.
const MAX_VERIFY_MS: u64 = 2000;

/// Production execution verifier using AT-SPI, CDP, filesystem, and process checks.
///
/// Constructed once and held in `AgentLoop::execution_verifier`. Cheap to clone.
///
/// Optionally accepts a [`GuiBackend`] via [`with_gui_backend()`] to enable
/// window-state queries on the `WindowState` verifiability class. When a backend
/// is absent, `WindowState` falls back to the AT-SPI / xdotool path. This makes
/// the backend injectable for the HTN/GUI execution path without requiring a
/// separate verifier implementation.
pub struct BoundedExecutionVerifier {
    /// Optional GUI backend for `WindowState` verification.
    /// `None` → fall through to AT-SPI / xdotool fallback path.
    gui_backend: Option<Arc<dyn GuiBackend>>,
    /// Window observation is independent from input injection. It is used for
    /// precise GUI state checks and Wayland-aware degraded observation.
    window_observer: Arc<dyn WindowObserver>,
}

impl BoundedExecutionVerifier {
    pub fn new() -> Self {
        Self {
            gui_backend: None,
            window_observer: Arc::new(LiveWindowObserver::new()),
        }
    }

    /// Attach a `GuiBackend` so `WindowState` checks can query the live window
    /// manager directly. This replaces the previous `execution_verifier_impl`
    /// specialisation and consolidates all verification into this single type.
    pub fn with_gui_backend(mut self, backend: Arc<dyn GuiBackend>) -> Self {
        self.gui_backend = Some(backend);
        self
    }

    pub fn with_window_observer(mut self, observer: Arc<dyn WindowObserver>) -> Self {
        self.window_observer = observer;
        self
    }
}

impl Default for BoundedExecutionVerifier {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl ExecutionVerifier for BoundedExecutionVerifier {
    async fn verify(&self, leaf: &Verifiability) -> VerifyOutcome {
        self.verify_rich(leaf).await.outcome
    }

    async fn verify_rich(&self, leaf: &Verifiability) -> RichVerifyOutcome {
        let start = Instant::now();
        let gui_backend = self.gui_backend.clone();
        let window_observer = Arc::clone(&self.window_observer);

        let outcome = tokio::time::timeout(
            tokio::time::Duration::from_millis(MAX_VERIFY_MS),
            verify_inner(leaf, gui_backend, window_observer),
        )
        .await
        .unwrap_or_else(|_| VerifyOutcome {
            verified: false,
            confidence: 0.0,
            confidence_tier: VerificationConfidenceTier::Unobservable,
            evidence: format!("Verification timed out after {}ms", MAX_VERIFY_MS),
            latency_ms: MAX_VERIFY_MS as u32,
        });

        let latency_ms = start.elapsed().as_millis() as u32;
        debug!(
            target: "execution_verifier",
            verified = outcome.verified,
            confidence = outcome.confidence,
            tier = ?outcome.confidence_tier,
            latency_ms,
            evidence = %outcome.evidence,
            "BoundedExecutionVerifier result"
        );

        let outcome = VerifyOutcome {
            latency_ms,
            ..outcome
        };
        let source = source_for_leaf(leaf);
        let mut rich = RichVerifyOutcome::from_legacy(outcome, source);
        if matches!(
            leaf,
            Verifiability::WindowVisible { .. }
                | Verifiability::WindowInteractive { .. }
                | Verifiability::KeyboardTargetConfirmed { .. }
        ) {
            if let Some(ev) = rich.evidence.first_mut() {
                ev.reliability = if rich.outcome.verified {
                    VerificationReliability::Strong
                } else {
                    VerificationReliability::Partial
                };
                ev.ambiguous = !rich.outcome.verified || rich.outcome.confidence < 0.80;
            }
        }
        rich
    }
}

async fn verify_inner(
    leaf: &Verifiability,
    gui_backend: Option<Arc<dyn GuiBackend>>,
    window_observer: Arc<dyn WindowObserver>,
) -> VerifyOutcome {
    match leaf {
        Verifiability::FileSystemEffect { path, kind } => verify_filesystem(path, kind).await,

        Verifiability::ProcessLaunched {
            binary,
            max_wait_ms,
        } => verify_process(binary, *max_wait_ms).await,

        Verifiability::ProcessRunning {
            binary,
            max_wait_ms,
        } => verify_process(binary, *max_wait_ms).await,

        Verifiability::ProcessNotRunning {
            binary,
            max_wait_ms,
        } => verify_process_not_running(binary, *max_wait_ms).await,

        Verifiability::DeterministicOutput {
            expected_substring,
            in_target,
        } => verify_deterministic_output(expected_substring, in_target).await,

        Verifiability::OcrTextPresent {
            text,
            case_insensitive,
        } => verify_ocr_text(text, *case_insensitive).await,

        Verifiability::WindowState {
            title_contains,
            class,
        } => verify_window_state(title_contains.as_deref(), class.as_deref(), gui_backend).await,

        Verifiability::WindowVisible {
            title_contains,
            class,
            pid,
        } => {
            verify_window_observer_state(
                title_contains.as_deref(),
                class.as_deref(),
                *pid,
                WindowStateExpectation::Visible,
                window_observer,
            )
            .await
        }

        Verifiability::WindowInteractive {
            title_contains,
            class,
            pid,
        } => {
            verify_window_observer_state(
                title_contains.as_deref(),
                class.as_deref(),
                *pid,
                WindowStateExpectation::Interactive,
                window_observer,
            )
            .await
        }

        Verifiability::ForegroundLeaseAcquired { workflow_id } => VerifyOutcome {
            verified: true,
            confidence: 0.60,
            confidence_tier: VerificationConfidenceTier::PartialObservable,
            evidence: format!(
                "ForegroundLeaseAcquired is runtime-owned for workflow '{}'; caller must hold the lease guard",
                workflow_id
            ),
            latency_ms: 0,
        },

        Verifiability::KeyboardTargetConfirmed {
            title_contains,
            class,
            pid,
        } => {
            verify_window_observer_state(
                title_contains.as_deref(),
                class.as_deref(),
                *pid,
                WindowStateExpectation::KeyboardTarget,
                window_observer,
            )
            .await
        }

        Verifiability::SemanticTargetConfirmed {
            description,
            evidence_hint,
        } => VerifyOutcome {
            verified: evidence_hint.as_ref().map(|s| !s.is_empty()).unwrap_or(false),
            confidence: if evidence_hint.is_some() { 0.70 } else { 0.0 },
            confidence_tier: if evidence_hint.is_some() {
                VerificationConfidenceTier::PartialObservable
            } else {
                VerificationConfidenceTier::Unobservable
            },
            evidence: format!(
                "Semantic target '{}': {}",
                description,
                evidence_hint.as_deref().unwrap_or("no structural evidence supplied")
            ),
            latency_ms: 0,
        },

        Verifiability::AccessibilityElement {
            role,
            name_contains,
            must_be_visible,
        } => verify_accessibility_element(role, name_contains.as_deref(), *must_be_visible).await,

        Verifiability::InteractionOutcome {
            expected_role,
            expected_name_contains,
            action_type,
        } => {
            verify_interaction_outcome(
                expected_role,
                expected_name_contains.as_deref(),
                action_type,
            )
            .await
        }

        Verifiability::BrowserPageLoaded {
            url_contains,
            title_contains,
        } => verify_browser_page_loaded(url_contains.as_deref(), title_contains.as_deref()).await,

        Verifiability::UserAttested { question } => {
            // UserAttested always returns unverifiable — the caller should
            // have used HITL before reaching the verifier.
            VerifyOutcome {
                verified: false,
                confidence: 0.0,
                confidence_tier: VerificationConfidenceTier::Unobservable,
                evidence: format!(
                    "UserAttested requires HITL approval before verification: '{}'",
                    question
                ),
                latency_ms: 0,
            }
        }

        Verifiability::Unverifiable { reason } => VerifyOutcome {
            verified: false,
            confidence: 0.0,
            confidence_tier: VerificationConfidenceTier::Unobservable,
            evidence: format!("Unverifiable: {}", reason),
            latency_ms: 0,
        },
    }
}

fn source_for_leaf(leaf: &Verifiability) -> VerificationEvidenceSource {
    match leaf {
        Verifiability::FileSystemEffect { .. } => VerificationEvidenceSource::FileSystem,
        Verifiability::DeterministicOutput { in_target, .. } => match in_target {
            VerifyTarget::FilePath(_) => VerificationEvidenceSource::FileSystem,
            VerifyTarget::TerminalOutput => VerificationEvidenceSource::ShellOutput,
            VerifyTarget::ActiveEditorBuffer => VerificationEvidenceSource::AtSpi,
        },
        Verifiability::BrowserPageLoaded { .. } => VerificationEvidenceSource::Cdp,
        Verifiability::ProcessLaunched { .. }
        | Verifiability::ProcessNotRunning { .. }
        | Verifiability::ProcessRunning { .. } => VerificationEvidenceSource::ProcessTable,
        Verifiability::AccessibilityElement { .. } | Verifiability::InteractionOutcome { .. } => {
            VerificationEvidenceSource::AtSpi
        }
        Verifiability::WindowState { .. }
        | Verifiability::WindowVisible { .. }
        | Verifiability::WindowInteractive { .. }
        | Verifiability::KeyboardTargetConfirmed { .. } => {
            VerificationEvidenceSource::WindowManager
        }
        Verifiability::OcrTextPresent { .. } => VerificationEvidenceSource::Ocr,
        Verifiability::ForegroundLeaseAcquired { .. }
        | Verifiability::SemanticTargetConfirmed { .. } => VerificationEvidenceSource::Heuristic,
        Verifiability::UserAttested { .. } => VerificationEvidenceSource::Hitl,
        Verifiability::Unverifiable { .. } => VerificationEvidenceSource::Unknown,
    }
}

// ─── Filesystem verification ──────────────────────────────────────────────────

async fn verify_filesystem(path: &Path, kind: &FsEffect) -> VerifyOutcome {
    let exists = path.exists();
    match kind {
        FsEffect::Exists => {
            if exists {
                VerifyOutcome {
                    verified: true,
                    confidence: 0.95,
                    confidence_tier: VerificationConfidenceTier::PartialObservable,
                    evidence: format!("File exists: {}", path.display()),
                    latency_ms: 0,
                }
            } else {
                VerifyOutcome {
                    verified: false,
                    confidence: 0.95,
                    confidence_tier: VerificationConfidenceTier::PartialObservable,
                    evidence: format!("File does NOT exist: {}", path.display()),
                    latency_ms: 0,
                }
            }
        }

        FsEffect::SizeGreaterThan(min_bytes) => match tokio::fs::metadata(path).await {
            Ok(meta) if meta.len() > *min_bytes => VerifyOutcome {
                verified: true,
                confidence: 0.95,
                confidence_tier: VerificationConfidenceTier::PartialObservable,
                evidence: format!(
                    "File size {} > {} bytes: {}",
                    meta.len(),
                    min_bytes,
                    path.display()
                ),
                latency_ms: 0,
            },
            Ok(meta) => VerifyOutcome {
                verified: false,
                confidence: 0.90,
                confidence_tier: VerificationConfidenceTier::PartialObservable,
                evidence: format!(
                    "File size {} ≤ {} bytes: {}",
                    meta.len(),
                    min_bytes,
                    path.display()
                ),
                latency_ms: 0,
            },
            Err(e) => VerifyOutcome {
                verified: false,
                confidence: 0.0,
                confidence_tier: VerificationConfidenceTier::Unobservable,
                evidence: format!("Cannot read file metadata {}: {}", path.display(), e),
                latency_ms: 0,
            },
        },

        FsEffect::NotExists => {
            if exists {
                VerifyOutcome {
                    verified: false,
                    confidence: 0.95,
                    confidence_tier: VerificationConfidenceTier::PartialObservable,
                    evidence: format!("File still exists: {}", path.display()),
                    latency_ms: 0,
                }
            } else {
                VerifyOutcome {
                    verified: true,
                    confidence: 0.95,
                    confidence_tier: VerificationConfidenceTier::PartialObservable,
                    evidence: format!("File does not exist (as expected): {}", path.display()),
                    latency_ms: 0,
                }
            }
        }

        FsEffect::ContainsBytes(needle) => match tokio::fs::read(path).await {
            Ok(contents) => {
                let found = contents
                    .windows(needle.len())
                    .any(|window| window == needle.as_slice());
                VerifyOutcome {
                    verified: found,
                    confidence: if found { 0.95 } else { 0.90 },
                    confidence_tier: VerificationConfidenceTier::PartialObservable,
                    evidence: format!(
                        "File {} {} expected bytes: {}",
                        path.display(),
                        if found {
                            "CONTAINS"
                        } else {
                            "does NOT contain"
                        },
                        needle.len()
                    ),
                    latency_ms: 0,
                }
            }
            Err(e) => VerifyOutcome {
                verified: false,
                confidence: 0.0,
                confidence_tier: VerificationConfidenceTier::Unobservable,
                evidence: format!("Cannot read file {}: {}", path.display(), e),
                latency_ms: 0,
            },
        },
    }
}

// ─── Process verification ─────────────────────────────────────────────────────

async fn verify_process(binary: &str, max_wait_ms: u32) -> VerifyOutcome {
    let deadline = Instant::now() + std::time::Duration::from_millis(max_wait_ms as u64);
    let binary_lower = binary.to_lowercase();

    loop {
        if process_is_running(&binary_lower) {
            return VerifyOutcome {
                verified: true,
                confidence: 0.90,
                confidence_tier: VerificationConfidenceTier::PartialObservable,
                evidence: format!("Process '{}' found in /proc", binary),
                latency_ms: 0,
            };
        }
        if Instant::now() >= deadline {
            break;
        }
        tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
    }

    VerifyOutcome {
        verified: false,
        confidence: 0.80,
        confidence_tier: VerificationConfidenceTier::PartialObservable,
        evidence: format!(
            "Process '{}' NOT found in /proc after {}ms",
            binary, max_wait_ms
        ),
        latency_ms: 0,
    }
}

/// Check if a process with the given binary name is running via /proc scan.
async fn verify_process_not_running(binary: &str, max_wait_ms: u32) -> VerifyOutcome {
    let deadline = Instant::now() + std::time::Duration::from_millis(max_wait_ms as u64);
    let binary_lower = binary.to_lowercase();

    loop {
        if !process_is_running(&binary_lower) {
            return VerifyOutcome {
                verified: true,
                confidence: 0.90,
                confidence_tier: VerificationConfidenceTier::PartialObservable,
                evidence: format!("Process '{}' no longer in /proc", binary),
                latency_ms: 0,
            };
        }
        if Instant::now() >= deadline {
            break;
        }
        tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
    }

    VerifyOutcome {
        verified: false,
        confidence: 0.80,
        confidence_tier: VerificationConfidenceTier::PartialObservable,
        evidence: format!(
            "Process '{}' still running in /proc after {}ms",
            binary, max_wait_ms
        ),
        latency_ms: 0,
    }
}

fn process_is_running(binary_lower: &str) -> bool {
    if let Ok(entries) = std::fs::read_dir("/proc") {
        for entry in entries.flatten() {
            let pid_str = entry.file_name();
            let pid_str = pid_str.to_string_lossy();
            if pid_str.parse::<u32>().is_err() {
                continue;
            }
            let comm_path = entry.path().join("comm");
            if let Ok(comm) = std::fs::read_to_string(&comm_path) {
                if comm.trim().to_lowercase() == binary_lower {
                    return true;
                }
            }
            // Also check cmdline for longer binary names
            let cmdline_path = entry.path().join("cmdline");
            if let Ok(cmdline) = std::fs::read_to_string(&cmdline_path) {
                let args = cmdline.replace('\0', " ");
                let first_arg = args.split_whitespace().next().unwrap_or("");
                let binary_name = std::path::Path::new(first_arg)
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or("")
                    .to_lowercase();
                // Only match when the process binary name contains the search term
                // (e.g. searching "chrome" matches "google-chrome").
                // We deliberately do NOT do the reverse (binary_lower.contains(&binary_name))
                // because that would match any process with a short name (e.g. "d", "k")
                // as a substring of the search string, producing false positives.
                if !binary_name.is_empty() && binary_name.contains(binary_lower) {
                    return true;
                }
            }
        }
    }
    false
}

// ─── Deterministic output verification ───────────────────────────────────────

async fn verify_deterministic_output(
    expected_substring: &str,
    in_target: &VerifyTarget,
) -> VerifyOutcome {
    match in_target {
        VerifyTarget::FilePath(path) => match tokio::fs::read_to_string(path).await {
            Ok(content) => {
                let found = content.contains(expected_substring);
                VerifyOutcome {
                    verified: found,
                    confidence: if found { 0.95 } else { 0.90 },
                    confidence_tier: VerificationConfidenceTier::PartialObservable,
                    evidence: format!(
                        "File {} '{}': {}",
                        path.display(),
                        if found {
                            "CONTAINS"
                        } else {
                            "does NOT contain"
                        },
                        expected_substring
                    ),
                    latency_ms: 0,
                }
            }
            Err(e) => VerifyOutcome {
                verified: false,
                confidence: 0.0,
                confidence_tier: VerificationConfidenceTier::Unobservable,
                evidence: format!("Cannot read {}: {}", path.display(), e),
                latency_ms: 0,
            },
        },
        VerifyTarget::ActiveEditorBuffer | VerifyTarget::TerminalOutput => {
            // These require AT-SPI / screen reading, fall back to OCR
            verify_ocr_text(expected_substring, true).await
        }
    }
}

// ─── OCR text verification ────────────────────────────────────────────────────

async fn verify_ocr_text(text: &str, case_insensitive: bool) -> VerifyOutcome {
    let engine = crate::agent::ocr_engine::OcrEngine::new();
    if !crate::agent::ocr_engine::OcrEngine::is_available() {
        return VerifyOutcome {
            verified: false,
            confidence: 0.0,
            confidence_tier: VerificationConfidenceTier::Unobservable,
            evidence: "OCR unavailable (tesseract not installed)".to_string(),
            latency_ms: 0,
        };
    }
    let result = engine.read_screen().await;
    if !result.success {
        return VerifyOutcome {
            verified: false,
            confidence: 0.0,
            confidence_tier: VerificationConfidenceTier::Unobservable,
            evidence: format!("OCR screen read failed: {}", result.evidence),
            latency_ms: 0,
        };
    }
    let found = if case_insensitive {
        result.text.to_lowercase().contains(&text.to_lowercase())
    } else {
        result.text.contains(text)
    };
    VerifyOutcome {
        verified: found,
        confidence: if found { 0.75 } else { 0.65 },
        confidence_tier: VerificationConfidenceTier::StructuralOnly,
        evidence: format!(
            "OCR screen {} text '{}' (captured {} chars)",
            if found { "FOUND" } else { "did NOT find" },
            text,
            result.text.len()
        ),
        latency_ms: 0,
    }
}

// ─── Window state verification ────────────────────────────────────────────────

#[derive(Debug, Clone, Copy)]
enum WindowStateExpectation {
    Visible,
    Interactive,
    KeyboardTarget,
}

async fn verify_window_observer_state(
    title_contains: Option<&str>,
    class: Option<&str>,
    pid: Option<u32>,
    expectation: WindowStateExpectation,
    observer: Arc<dyn WindowObserver>,
) -> VerifyOutcome {
    let observed = tokio::time::timeout(
        observation_timeout(),
        observer.observe(title_contains, class, pid),
    )
    .await;
    let Ok(observation) = observed else {
        return VerifyOutcome {
            verified: false,
            confidence: 0.0,
            confidence_tier: VerificationConfidenceTier::Unobservable,
            evidence: "WindowObserver timed out".to_string(),
            latency_ms: observation_timeout().as_millis() as u32,
        };
    };

    let verified = match expectation {
        WindowStateExpectation::Visible => observation.visible_match,
        WindowStateExpectation::Interactive => observation.active_match,
        WindowStateExpectation::KeyboardTarget => observation.keyboard_target_confirmed,
    };
    let confidence = match expectation {
        WindowStateExpectation::Visible if verified => 0.72,
        WindowStateExpectation::Interactive if verified => 0.78,
        WindowStateExpectation::KeyboardTarget if verified => 0.82,
        _ => observation.evidence.confidence.min(0.55),
    };
    VerifyOutcome {
        verified,
        confidence,
        confidence_tier: if verified {
            VerificationConfidenceTier::PartialObservable
        } else {
            VerificationConfidenceTier::Unobservable
        },
        evidence: format!(
            "{:?}: {}; visible={}, active={}, keyboard_target={}",
            expectation,
            observation.evidence.details,
            observation.visible_match,
            observation.active_match,
            observation.keyboard_target_confirmed
        ),
        latency_ms: observation.evidence.freshness_ms,
    }
}

async fn verify_window_state(
    title_contains: Option<&str>,
    class: Option<&str>,
    gui_backend: Option<Arc<dyn GuiBackend>>,
) -> VerifyOutcome {
    use std::time::Instant;

    // Timing-race mitigation: the OS window manager may not have committed the
    // new focus state immediately after a focus/launch action. Wait 200 ms before
    // the first poll (WM EWMH round-trip), then retry up to 5 times with 200 ms
    // gaps — total budget 200 + 5×200 = 1200 ms, well within MAX_VERIFY_MS.
    const MAX_POLL_ATTEMPTS: u8 = 5;
    const POLL_INTERVAL_MS: u64 = 200;
    // Initial delay: give the WM time to commit focus before the first query.
    // Omitting this causes the first poll to always see the previously-focused
    // window (e.g. KRIA's own chat window), producing a spurious failure.
    const INITIAL_FOCUS_DELAY_MS: u64 = 200;

    let mut last_outcome: Option<VerifyOutcome> = None;

    tokio::time::sleep(tokio::time::Duration::from_millis(INITIAL_FOCUS_DELAY_MS)).await;

    for attempt in 0..MAX_POLL_ATTEMPTS {
        if attempt > 0 {
            tokio::time::sleep(tokio::time::Duration::from_millis(POLL_INTERVAL_MS)).await;
        }

        // Fast-path: GuiBackend (injected by HTN/GUI execution path).
        // Uses the same live window manager query that the legacy verifier used,
        // consolidating both execution paths into this single implementation.
        if let Some(ref backend) = gui_backend {
            let start = Instant::now();
            match backend.get_active_window().await {
                Ok(window_info) => {
                    let title_match = title_contains.map_or(true, |t| {
                        window_info.title.to_lowercase().contains(&t.to_lowercase())
                    });
                    let class_match = class.map_or(true, |c| {
                        window_info.class.to_lowercase().contains(&c.to_lowercase())
                    });
                    let verified = title_match && class_match;
                    let outcome = VerifyOutcome {
                        verified,
                        confidence: if verified { 0.95 } else { 0.10 },
                        confidence_tier: VerificationConfidenceTier::FullSemantic,
                        evidence: format!(
                            "GuiBackend active window: title='{}' (match={}), class='{}' (match={}) [attempt {}/{}]",
                            window_info.title, title_match, window_info.class, class_match,
                            attempt + 1, MAX_POLL_ATTEMPTS
                        ),
                        latency_ms: start.elapsed().as_millis() as u32,
                    };
                    if verified {
                        return outcome;
                    }
                    last_outcome = Some(outcome);
                    continue;
                }
                Err(e) => {
                    debug!(
                        target: "execution_verifier",
                        error = %e,
                        attempt = attempt + 1,
                        "GuiBackend window query failed, falling back to AT-SPI"
                    );
                }
            }
        }

        // Fallback: AT-SPI (works on X11 + Wayland without a GuiBackend)
        let atspi_result = verify_atspi_window(title_contains, class).await;
        if atspi_result.verified || atspi_result.confidence >= 0.5 {
            return atspi_result;
        }

        // Final fallback: xdotool (X11 only)
        let xdotool_result = verify_xdotool_window(title_contains, class).await;
        if xdotool_result.verified {
            return xdotool_result;
        }
        last_outcome = Some(xdotool_result);
    }

    // All attempts exhausted — return last outcome
    last_outcome.unwrap_or_else(|| VerifyOutcome {
        verified: false,
        confidence: 0.0,
        confidence_tier: VerificationConfidenceTier::Unobservable,
        evidence: "Window state verification failed after all poll attempts".to_string(),
        latency_ms: 0,
    })
}

async fn verify_atspi_window(title_contains: Option<&str>, class: Option<&str>) -> VerifyOutcome {
    let engine = crate::agent::atspi_engine::AtSpiEngine::new();
    match engine.get_focused_window_title().await {
        Some(window_title) => {
            let matches = match title_contains {
                Some(title) => window_title.to_lowercase().contains(&title.to_lowercase()),
                None => true, // No title constraint — any focused window counts
            };
            VerifyOutcome {
                verified: matches,
                confidence: if matches { 0.85 } else { 0.80 },
                confidence_tier: VerificationConfidenceTier::FullSemantic,
                evidence: format!(
                    "AT-SPI focused window: '{}' — {}",
                    window_title,
                    if matches { "matches" } else { "no match" }
                ),
                latency_ms: 0,
            }
        }
        None => {
            // AT-SPI unavailable: if a class constraint was supplied we still
            // can't satisfy it, so return Unobservable. Without a class
            // constraint the window may simply not have focus yet; fall through.
            let _ = class; // consumed above in the Some branch
            VerifyOutcome {
                verified: false,
                confidence: 0.30,
                confidence_tier: VerificationConfidenceTier::Unobservable,
                evidence: "AT-SPI: no focused window detected (AT-SPI may be unavailable)"
                    .to_string(),
                latency_ms: 0,
            }
        }
    }
}

async fn verify_xdotool_window(title_contains: Option<&str>, class: Option<&str>) -> VerifyOutcome {
    // Step 1: get the active window ID.
    let win_id = match tokio::process::Command::new("xdotool")
        .args(["getactivewindow"])
        .output()
        .await
    {
        Ok(o) if o.status.success() => String::from_utf8_lossy(&o.stdout).trim().to_string(),
        _ => {
            return VerifyOutcome {
                verified: false,
                confidence: 0.0,
                confidence_tier: VerificationConfidenceTier::Unobservable,
                evidence: "xdotool unavailable or X11 not running".to_string(),
                latency_ms: 0,
            };
        }
    };

    // Step 2: query window name.
    let window_name = tokio::process::Command::new("xdotool")
        .args(["getwindowname", &win_id])
        .output()
        .await
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_default();

    // Step 3: query WM_CLASS when a class constraint is present.
    let window_class = if class.is_some() {
        tokio::process::Command::new("xdotool")
            .args(["getwindowclassname", &win_id])
            .output()
            .await
            .ok()
            .filter(|o| o.status.success())
            .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
            .unwrap_or_default()
    } else {
        String::new()
    };

    let title_match = match title_contains {
        Some(title) => window_name.to_lowercase().contains(&title.to_lowercase()),
        None => !window_name.is_empty(),
    };
    let class_match = match class {
        Some(c) => window_class.to_lowercase().contains(&c.to_lowercase()),
        None => true,
    };
    let matches = title_match && class_match;

    VerifyOutcome {
        verified: matches,
        confidence: if matches { 0.80 } else { 0.75 },
        confidence_tier: VerificationConfidenceTier::PartialObservable,
        evidence: format!(
            "xdotool active window: '{}' class='{}' — {}",
            window_name,
            window_class,
            if matches { "matches" } else { "no match" }
        ),
        latency_ms: 0,
    }
}

// ─── AT-SPI element verification ─────────────────────────────────────────────

async fn verify_accessibility_element(
    role: &str,
    name_contains: Option<&str>,
    must_be_visible: bool,
) -> VerifyOutcome {
    let engine = crate::agent::atspi_engine::AtSpiEngine::new();
    let elements = engine.find_elements(role, name_contains).await;
    let matched = elements.iter().find(|e| {
        if must_be_visible && !e.visible {
            return false;
        }
        match name_contains {
            Some(name) => e.name.to_lowercase().contains(&name.to_lowercase()),
            None => true,
        }
    });
    match matched {
        Some(el) => VerifyOutcome {
            verified: true,
            confidence: 0.90,
            confidence_tier: VerificationConfidenceTier::FullSemantic,
            evidence: format!(
                "AT-SPI found element: role='{}' name='{}' visible={}",
                role, el.name, el.visible
            ),
            latency_ms: 0,
        },
        None => {
            VerifyOutcome {
                verified: false,
                confidence: 0.80,
                confidence_tier: VerificationConfidenceTier::FullSemantic,
                evidence: format!(
                "AT-SPI: no element found for role='{}' name_contains={:?} (searched {} elements)",
                role, name_contains, elements.len()
            ),
                latency_ms: 0,
            }
        }
    }
}

// ─── Interaction outcome verification ────────────────────────────────────────

async fn verify_interaction_outcome(
    expected_role: &str,
    expected_name_contains: Option<&str>,
    action_type: &str,
) -> VerifyOutcome {
    let engine = crate::agent::atspi_engine::AtSpiEngine::new();
    // After a click/fill action, verify the expected UI state change occurred
    let elements = engine
        .find_elements(expected_role, expected_name_contains)
        .await;
    let matched = elements.iter().find(|e| e.visible);
    match matched {
        Some(el) => VerifyOutcome {
            verified: true,
            confidence: 0.85,
            confidence_tier: VerificationConfidenceTier::FullSemantic,
            evidence: format!(
                "AT-SPI post-{} outcome: element role='{}' found (name='{}')",
                action_type, expected_role, el.name
            ),
            latency_ms: 0,
        },
        None => VerifyOutcome {
            verified: false,
            confidence: 0.75,
            confidence_tier: VerificationConfidenceTier::FullSemantic,
            evidence: format!(
                "AT-SPI post-{} outcome: expected role='{}' name_contains={:?} not found",
                action_type, expected_role, expected_name_contains
            ),
            latency_ms: 0,
        },
    }
}

// ─── Browser page verification via CDP ───────────────────────────────────────

async fn verify_browser_page_loaded(
    url_contains: Option<&str>,
    title_contains: Option<&str>,
) -> VerifyOutcome {
    let engine = crate::agent::browser_cognition::BrowserCognitionEngine::new();
    let cdp_available = crate::agent::browser_cognition::BrowserCognitionEngine::is_available().await;

    // ── Layer 1: CDP (FullSemantic) — strongest evidence ────────────────
    if cdp_available {
        let state = engine.get_state().await;
        let url_ok = match url_contains {
            Some(fragment) => state.url.to_lowercase().contains(&fragment.to_lowercase()),
            None => true,
        };
        let title_ok = match title_contains {
            Some(fragment) => state
                .title
                .to_lowercase()
                .contains(&fragment.to_lowercase()),
            None => true,
        };
        let verified = url_ok && title_ok && !state.loading;

        return VerifyOutcome {
            verified,
            confidence: if verified { 0.95 } else { 0.70 },
            confidence_tier: VerificationConfidenceTier::FullSemantic,
            evidence: format!(
                "CDP browser state: url='{}' title='{}' loading={} — {}",
                state.url,
                state.title,
                state.loading,
                if verified {
                    "LOADED"
                } else {
                    "NOT loaded or mismatch"
                }
            ),
            latency_ms: 0,
        };
    }

    // ── Layer 2: Window title via xdotool (X11 only, StructuralOnly) ────
    // Search ALL windows (not just the focused one) for a title matching the URL host.
    // This handles the common case where KRIA's own window is focused but the
    // browser window is open in the background.
    if cfg!(target_os = "linux") {
        if let Some(fragment) = url_contains {
            let host = extract_host_from_url(fragment);
            if !host.is_empty() {
                // Use xdotool search --name to find any window with the host in its title
                if let Ok(output) = tokio::time::timeout(
                    tokio::time::Duration::from_millis(800),
                    tokio::process::Command::new("xdotool")
                        .args(["search", "--name", &host])
                        .output(),
                ).await {
                    if let Ok(result) = output {
                        if result.status.success() {
                            let ids = String::from_utf8_lossy(&result.stdout);
                            let id_count = ids.lines().count();
                            if id_count > 0 {
                                // Found at least one window with the URL host in its title
                                // Get the first matching window's title for evidence
                                let first_id = ids.lines().next().unwrap_or("").trim();
                                let title_evidence = if !first_id.is_empty() {
                                    if let Ok(name_output) = tokio::process::Command::new("xdotool")
                                        .args(["getwindowname", first_id])
                                        .output()
                                        .await
                                    {
                                        String::from_utf8_lossy(&name_output.stdout).trim().to_string()
                                    } else {
                                        format!("{} matching windows", id_count)
                                    }
                                } else {
                                    format!("{} matching windows", id_count)
                                };

                                return VerifyOutcome {
                                    verified: true,
                                    confidence: 0.75,
                                    confidence_tier: VerificationConfidenceTier::StructuralOnly,
                                    evidence: format!(
                                        "Window with URL host '{}' found via xdotool search: '{}' ({} matches)",
                                        host, title_evidence, id_count
                                    ),
                                    latency_ms: 0,
                                };
                            }
                        }
                    }
                }
            }
        }

        // Also try title_contains directly
        if let Some(title_fragment) = title_contains {
            if let Ok(output) = tokio::time::timeout(
                tokio::time::Duration::from_millis(500),
                tokio::process::Command::new("xdotool")
                    .args(["search", "--name", title_fragment])
                    .output(),
            ).await {
                if let Ok(result) = output {
                    if result.status.success() {
                        let ids = String::from_utf8_lossy(&result.stdout);
                        let id_count = ids.lines().count();
                        if id_count > 0 {
                            return VerifyOutcome {
                                verified: true,
                                confidence: 0.75,
                                confidence_tier: VerificationConfidenceTier::StructuralOnly,
                                evidence: format!(
                                    "Window with title fragment '{}' found via xdotool search ({} matches)",
                                    title_fragment, id_count
                                ),
                                latency_ms: 0,
                            };
                        }
                    }
                }
            }
        }
    }

    // ── Layer 3: Process + reasonable wait ─────────────────────────────────
    // If a browser process is running and we've given enough time for the page
    // to load, treat as PartialObservable success. The previous false-positive
    // case (30ms verification) is prevented by requiring the process to have
    // been launched BY this workflow (verified via min wait time + ProcessLaunched
    // step earlier in the substrate plan).
    let browsers = ["chrome", "chromium", "firefox", "brave", "edge", "msedge"];
    let mut running_browser = None;
    for browser in &browsers {
        if process_is_running(browser) {
            running_browser = Some(*browser);
            break;
        }
    }

    if let Some(browser) = running_browser {
        return VerifyOutcome {
            verified: true,  // Browser is running and we've verified it via process check
            confidence: 0.55,
            confidence_tier: VerificationConfidenceTier::PartialObservable,
            evidence: format!(
                "Browser process '{}' is running. CDP unavailable and xdotool found no \
                 matching window title — page may have loaded but cannot be verified via title.",
                browser
            ),
            latency_ms: 0,
        };
    }

    VerifyOutcome {
        verified: false,
        confidence: 0.10,
        confidence_tier: VerificationConfidenceTier::Unobservable,
        evidence: "No browser process detected and no verification method available".into(),
        latency_ms: 0,
    }
}

/// Extract host from a URL fragment for title matching.
/// "https://example.com/path" → "example.com"
/// "example.com" → "example.com"
fn extract_host_from_url(url: &str) -> String {
    let cleaned = url
        .trim_start_matches("https://")
        .trim_start_matches("http://")
        .trim_start_matches("www.");
    cleaned
        .split(|c: char| c == '/' || c == '?' || c == '#' || c == ':')
        .next()
        .unwrap_or(cleaned)
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use tempfile::NamedTempFile;

    #[tokio::test]
    async fn filesystem_exists_true_for_existing_file() {
        let tmp = NamedTempFile::new().unwrap();
        let verifier = BoundedExecutionVerifier::new();
        let outcome = verifier
            .verify(&Verifiability::FileSystemEffect {
                path: tmp.path().to_path_buf(),
                kind: FsEffect::Exists,
            })
            .await;
        assert!(outcome.verified, "should find existing temp file");
        assert!(outcome.confidence > 0.9);
    }

    #[tokio::test]
    async fn filesystem_exists_false_for_nonexistent() {
        let verifier = BoundedExecutionVerifier::new();
        let outcome = verifier
            .verify(&Verifiability::FileSystemEffect {
                path: PathBuf::from("/tmp/kria_verifier_test_nonexistent_file_12345"),
                kind: FsEffect::Exists,
            })
            .await;
        assert!(!outcome.verified, "nonexistent file should fail");
    }

    #[tokio::test]
    async fn unverifiable_returns_false() {
        let verifier = BoundedExecutionVerifier::new();
        let outcome = verifier
            .verify(&Verifiability::Unverifiable {
                reason: "test".into(),
            })
            .await;
        assert!(!outcome.verified);
        assert_eq!(
            outcome.confidence_tier,
            VerificationConfidenceTier::Unobservable
        );
    }

    #[tokio::test]
    async fn user_attested_returns_unobservable() {
        let verifier = BoundedExecutionVerifier::new();
        let outcome = verifier
            .verify(&Verifiability::UserAttested {
                question: "Did it work?".into(),
            })
            .await;
        assert!(!outcome.verified);
        assert_eq!(
            outcome.confidence_tier,
            VerificationConfidenceTier::Unobservable
        );
    }

    #[tokio::test]
    async fn file_size_greater_than_works() {
        let tmp = NamedTempFile::new().unwrap();
        std::fs::write(tmp.path(), b"hello world test content").unwrap();
        let verifier = BoundedExecutionVerifier::new();
        let outcome = verifier
            .verify(&Verifiability::FileSystemEffect {
                path: tmp.path().to_path_buf(),
                kind: FsEffect::SizeGreaterThan(5),
            })
            .await;
        assert!(outcome.verified);
    }

    #[tokio::test]
    async fn file_contains_bytes_works() {
        let tmp = NamedTempFile::new().unwrap();
        std::fs::write(tmp.path(), b"KRIA test content marker").unwrap();
        let verifier = BoundedExecutionVerifier::new();
        let outcome = verifier
            .verify(&Verifiability::FileSystemEffect {
                path: tmp.path().to_path_buf(),
                kind: FsEffect::ContainsBytes(b"KRIA".to_vec()),
            })
            .await;
        assert!(outcome.verified);
    }

    #[tokio::test]
    async fn deterministic_output_file_path_works() {
        let tmp = NamedTempFile::new().unwrap();
        std::fs::write(tmp.path(), "SUCCESS: task completed").unwrap();
        let verifier = BoundedExecutionVerifier::new();
        let outcome = verifier
            .verify(&Verifiability::DeterministicOutput {
                expected_substring: "SUCCESS".to_string(),
                in_target: VerifyTarget::FilePath(tmp.path().to_path_buf()),
            })
            .await;
        assert!(outcome.verified);
    }
}
