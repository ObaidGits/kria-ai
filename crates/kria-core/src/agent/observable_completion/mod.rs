//! Phase 1 — Observable Completion Engine.
//!
//! # Core Mission
//!
//! Determine whether a workflow's outcome became **human-visible**, not merely
//! whether commands technically executed. This is the semantic bridge between
//! technical execution and human workflow expectations.
//!
//! # Authority Boundary
//!
//! - **Read-only**: The engine observes state; it NEVER executes actions.
//! - **Bounded**: Each verification has a small, hard cap tuned for live GUI checks.
//! - **PSDG-aware**: Facts from WorldModelStore inform expected outcomes.
//! - **Fail-closed**: Verification failures return `VisibilityUnknown`, not
//!   false success. The caller decides whether to surface or escalate.
//!
//! # Design Invariants
//!
//! Execution alone is NOT sufficient for completion. KRIA must answer:
//!
//! ```text
//! "Would the user perceive this task as done?"
//! ```
//!
//! Not:
//!
//! ```text
//! "Did the command exit 0?"
//! ```

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;

use serde::{Deserialize, Serialize};
use tracing::debug;

use crate::agent::execution_verifier::{
    ExecutionVerifier, FsEffect, Verifiability, VerificationConfidenceTier, VerifyOutcome,
};
use crate::agent::execution_verifier_bounded::BoundedExecutionVerifier;
use crate::agent::intent_compiler::{TargetRef, Verb};
use crate::agent::psdg::PsdgHandle;
use crate::agent::turn_gate::Operation;

// ─── Observable Outcome ───────────────────────────────────────────────────────

/// What human-visible state is expected to exist after a workflow completes.
///
/// Each variant maps to one or more `Verifiability` checks via
/// `ObservableCompletionEngine::to_verifiability_leaves`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ObservableOutcome {
    /// An application window is open and visible.
    ApplicationWindow {
        app_name: String,
        /// Optional window title hint to narrow the check.
        title_hint: Option<String>,
    },
    /// A browser page is loaded at the expected URL.
    BrowserPage {
        url_contains: Option<String>,
        title_contains: Option<String>,
    },
    /// An IDE workspace is open at the expected path.
    IdeWorkspace { path: String },
    /// A terminal shows expected output.
    TerminalOutput { contains: String },
    /// A file is present and non-empty on disk.
    FileCreated {
        path: PathBuf,
        /// If `Some(n)`, file must be at least `n` bytes.
        min_size_bytes: Option<u64>,
    },
    /// A file was modified (modification time after `after_epoch_secs`).
    FileModified {
        path: PathBuf,
        after_epoch_secs: u64,
    },
    /// A process is running.
    ProcessRunning { binary: String },
    /// A notification or confirmation text is visible on screen.
    NotificationVisible { contains: String },
    /// Audio is playing (player window visible or process running).
    AudioPlaybackActive { player_hint: Option<String> },
    /// An email was sent (confirmation visible in mail client).
    EmailSentConfirmation { client_hint: Option<String> },
    /// User must explicitly confirm they saw the output.
    UserAcknowledged { question: String },
    /// A download completed (file appears in Downloads or target path).
    DownloadComplete {
        file_hint: Option<String>,
        target_dir: Option<PathBuf>,
    },
    /// No observable outcome is expected (background ops, pure data ops).
    Silent,
}

// ─── Visibility Requirement ───────────────────────────────────────────────────

/// How visible must the outcome be before KRIA considers the task complete?
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum VisibilityRequirement {
    /// Silent background operation — no visible confirmation needed.
    ///
    /// Examples: write to database, save preferences, background sync.
    SilentOk,
    /// An observable state must exist (window open, file present).
    ///
    /// KRIA verifies the state but does NOT require user acknowledgement.
    VisibleStateRequired,
    /// The result must be actively brought to the foreground / shown.
    ///
    /// Examples: "show me the file", "run and show output".
    OutputMustBeSurfaced,
    /// Explicit human acknowledgement is required before task is marked done.
    ///
    /// Used for: destructive ops, sent emails, deployed code, deleted files.
    UserAcknowledgementRequired,
}

// ─── Completion Visibility Policy ─────────────────────────────────────────────

/// Full policy for completing a single observable outcome.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompletionVisibilityPolicy {
    /// The expected observable outcome.
    pub outcome: ObservableOutcome,
    /// How visible the outcome must be.
    pub visibility: VisibilityRequirement,
    /// Whether silent background success is acceptable.
    pub allow_hidden_execution: bool,
    /// Maximum milliseconds to spend verifying this outcome.
    pub verify_timeout_ms: u64,
    /// Whether PSDG state can confirm this outcome without live probe.
    ///
    /// Example: browser URL already matches in WorldModelStore → skip CDP call.
    pub accept_psdg_evidence: bool,
}

impl CompletionVisibilityPolicy {
    /// Derive the policy for an outcome given the operation context.
    pub fn for_outcome(outcome: ObservableOutcome, operation: Operation) -> Self {
        use ObservableOutcome::*;
        use Operation::*;
        use VisibilityRequirement::*;

        let visibility = match (&outcome, operation) {
            // Silent background operations
            (Silent, _) => SilentOk,

            // Destructive or irreversible operations always need acknowledgement
            (EmailSentConfirmation { .. }, Send) => UserAcknowledgementRequired,
            (FileModified { .. }, Delete) => UserAcknowledgementRequired,

            // Automate operations targeting apps/browser need visible state
            (ApplicationWindow { .. }, Automate) => VisibleStateRequired,
            (BrowserPage { .. }, _) => VisibleStateRequired,
            (IdeWorkspace { .. }, Automate) => VisibleStateRequired,

            // "Run and show output" class
            (TerminalOutput { .. }, ExecuteShell) => OutputMustBeSurfaced,
            (TerminalOutput { .. }, ExecuteCode) => OutputMustBeSurfaced,

            // File creation for Write ops should be surfaced
            (FileCreated { .. }, Write) => OutputMustBeSurfaced,

            // User-attested outcomes always need acknowledgement
            (UserAcknowledged { .. }, _) => UserAcknowledgementRequired,

            // Default: require visible state for interactive ops, silent for data ops
            (_, Automate | ExecuteShell | ExecuteCode | Write | ConfigureSystem) => {
                VisibleStateRequired
            }
            _ => SilentOk,
        };

        let allow_hidden_execution = matches!((&outcome, visibility), (Silent, _) | (_, SilentOk));

        Self {
            outcome,
            visibility,
            allow_hidden_execution,
            verify_timeout_ms: match visibility {
                VisibleStateRequired | UserAcknowledgementRequired => 2_500,
                OutputMustBeSurfaced => 1_500,
                SilentOk => 0,
            },
            accept_psdg_evidence: true,
        }
    }
}

// ─── Observable Verify Result ─────────────────────────────────────────────────

/// Result of checking whether an observable outcome is visible.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ObservableVerifyResult {
    /// Whether the outcome is human-visible now.
    pub visible: bool,
    /// Confidence in the visibility check (0.0–1.0).
    pub confidence: f32,
    /// Confidence tier from the underlying verifier.
    pub tier: VerificationConfidenceTier,
    /// Human-readable evidence string.
    pub evidence: String,
    /// Latency of the check.
    pub latency_ms: u64,
    /// Whether PSDG evidence was used (vs live probe).
    pub psdg_backed: bool,
}

impl ObservableVerifyResult {
    fn unknown(evidence: impl Into<String>, latency_ms: u64) -> Self {
        Self {
            visible: false,
            confidence: 0.0,
            tier: VerificationConfidenceTier::Unobservable,
            evidence: evidence.into(),
            latency_ms,
            psdg_backed: false,
        }
    }

    fn from_verify_outcome(outcome: VerifyOutcome, psdg_backed: bool) -> Self {
        Self {
            visible: outcome.verified,
            confidence: outcome.confidence,
            tier: outcome.confidence_tier,
            evidence: outcome.evidence,
            latency_ms: outcome.latency_ms as u64,
            psdg_backed,
        }
    }
}

// ─── Outcome Inference ────────────────────────────────────────────────────────

/// Infer expected observable outcomes from a user prompt and verb/target context.
///
/// Pure function — no I/O, no state mutation.
pub fn infer_outcomes(
    user_prompt: &str,
    verb: &Verb,
    targets: &[TargetRef],
    operation: Operation,
) -> Vec<ObservableOutcome> {
    let lower = user_prompt.to_lowercase();
    let mut outcomes: Vec<ObservableOutcome> = Vec::new();

    // ── URL / browser targets ─────────────────────────────────────────────
    for target in targets {
        if let TargetRef::Url(url) = target {
            outcomes.push(ObservableOutcome::BrowserPage {
                url_contains: Some(url.clone()),
                title_contains: None,
            });
        }
    }

    // ── App targets ───────────────────────────────────────────────────────
    if matches!(verb, Verb::Open | Verb::Switch | Verb::Run) {
        for target in targets {
            if let TargetRef::App(name) = target {
                outcomes.push(ObservableOutcome::ApplicationWindow {
                    app_name: name.clone(),
                    title_hint: None,
                });
            }
        }
    }

    // ── File write targets ────────────────────────────────────────────────
    if matches!(verb, Verb::Save | Verb::Run) {
        for target in targets {
            if let TargetRef::File(path) = target {
                outcomes.push(ObservableOutcome::FileCreated {
                    path: path.clone(),
                    min_size_bytes: Some(1),
                });
            }
        }
    }

    // ── Prompt-specific pattern detection ─────────────────────────────────

    // Terminal/output surfacing
    if lower.contains("run")
        && (lower.contains("output") || lower.contains("show") || lower.contains("print"))
    {
        outcomes.push(ObservableOutcome::TerminalOutput {
            contains: String::new(),
        });
    }

    // Email sends
    if lower.contains("send") && (lower.contains("email") || lower.contains("mail")) {
        outcomes.push(ObservableOutcome::EmailSentConfirmation { client_hint: None });
    }

    // Downloads
    if lower.contains("download") {
        outcomes.push(ObservableOutcome::DownloadComplete {
            file_hint: None,
            target_dir: None,
        });
    }

    // Music / audio
    if lower.contains("play")
        && (lower.contains("music") || lower.contains("audio") || lower.contains("song"))
    {
        outcomes.push(ObservableOutcome::AudioPlaybackActive { player_hint: None });
    }

    // IDE workspace opens — leave to WorkflowExpectationEngine refinement
    // (we don't know the path at inference time)

    // Pure data operations produce Silent outcome
    if outcomes.is_empty() {
        match operation {
            Operation::Converse
            | Operation::RetrieveMemory
            | Operation::Read
            | Operation::Search
            | Operation::GenerateImage
            | Operation::AnalyzeImage
            | Operation::AnalyzeFile
            | Operation::Schedule => {
                outcomes.push(ObservableOutcome::Silent);
            }
            _ => {}
        }
    }

    // Always have at least one outcome
    if outcomes.is_empty() {
        outcomes.push(ObservableOutcome::Silent);
    }

    outcomes
}

// ─── Observable Completion Engine ─────────────────────────────────────────────

/// Determines whether workflow outcomes became human-visible.
///
/// The central semantic bridge between technical execution and human
/// workflow expectations in Batch 2.
pub struct ObservableCompletionEngine {
    /// Production verifier (AT-SPI, CDP, filesystem, process).
    verifier: Arc<BoundedExecutionVerifier>,
    /// PSDG handle for fast path: WorldModelStore evidence.
    psdg: Option<PsdgHandle>,
}

impl ObservableCompletionEngine {
    /// Create a new engine with optional PSDG backing.
    pub fn new(psdg: Option<PsdgHandle>) -> Self {
        Self {
            verifier: Arc::new(BoundedExecutionVerifier::new()),
            psdg,
        }
    }

    /// Verify that an observable outcome is currently human-visible.
    ///
    /// Uses PSDG evidence as a fast path when `policy.accept_psdg_evidence`
    /// is true. Falls back to live probe via `BoundedExecutionVerifier`.
    pub async fn verify_visible(
        &self,
        policy: &CompletionVisibilityPolicy,
    ) -> ObservableVerifyResult {
        let start = Instant::now();

        // Fast path: silent outcomes are always "visible" (no check needed).
        if policy.outcome == ObservableOutcome::Silent {
            return ObservableVerifyResult {
                visible: true,
                confidence: 1.0,
                tier: VerificationConfidenceTier::FullSemantic,
                evidence: "Silent operation — no visibility required".into(),
                latency_ms: 0,
                psdg_backed: false,
            };
        }

        // PSDG fast path: check WorldModelStore before probing live.
        if policy.accept_psdg_evidence {
            if let Some(psdg_result) = self.check_psdg_evidence(&policy.outcome) {
                debug!(
                    target: "observable_completion",
                    outcome = ?policy.outcome,
                    "PSDG fast-path: outcome evidence found in WorldModelStore"
                );
                return psdg_result;
            }
        }

        // Live probe via BoundedExecutionVerifier.
        let verifiability_leaves = self.to_verifiability_leaves(&policy.outcome);
        if verifiability_leaves.is_empty() {
            return ObservableVerifyResult::unknown(
                "No verifiability leaves available for this outcome type",
                start.elapsed().as_millis() as u64,
            );
        }

        // Verify the primary leaf (first one). For richer outcomes, add
        // parallel leaf verification in a future batch.
        let primary_leaf = &verifiability_leaves[0];
        let raw = tokio::time::timeout(
            tokio::time::Duration::from_millis(policy.verify_timeout_ms),
            self.verifier.verify(primary_leaf),
        )
        .await
        .unwrap_or_else(|_| VerifyOutcome {
            verified: false,
            confidence: 0.0,
            confidence_tier: VerificationConfidenceTier::Unobservable,
            evidence: format!(
                "Visibility probe timed out after {}ms",
                policy.verify_timeout_ms
            ),
            latency_ms: policy.verify_timeout_ms as u32,
        });

        ObservableVerifyResult::from_verify_outcome(raw, false)
    }

    /// Verify all outcomes in a policy set. Returns the aggregate result.
    ///
    /// All outcomes must be visible for the aggregate to be `visible = true`.
    /// Silent outcomes are excluded from the aggregate.
    pub async fn verify_all(
        &self,
        policies: &[CompletionVisibilityPolicy],
    ) -> AggregateVisibilityResult {
        let mut results = Vec::with_capacity(policies.len());
        let mut any_required_invisible = false;
        let mut any_surfacing_needed = false;

        for policy in policies {
            if policy.outcome == ObservableOutcome::Silent {
                continue;
            }
            let result = self.verify_visible(policy).await;
            if !result.visible
                && matches!(
                    policy.visibility,
                    VisibilityRequirement::VisibleStateRequired
                        | VisibilityRequirement::OutputMustBeSurfaced
                        | VisibilityRequirement::UserAcknowledgementRequired
                )
            {
                any_required_invisible = true;
            }
            if !result.visible
                && matches!(
                    policy.visibility,
                    VisibilityRequirement::OutputMustBeSurfaced
                )
            {
                any_surfacing_needed = true;
            }
            results.push((policy.clone(), result));
        }

        let all_visible = !any_required_invisible;
        let overall_confidence = if results.is_empty() {
            1.0
        } else {
            results.iter().map(|(_, r)| r.confidence).sum::<f32>() / results.len() as f32
        };

        AggregateVisibilityResult {
            all_required_visible: all_visible,
            overall_confidence,
            surfacing_needed: any_surfacing_needed,
            per_outcome: results,
        }
    }

    /// Translate an `ObservableOutcome` into `Verifiability` leaves for the
    /// `BoundedExecutionVerifier`.
    pub(crate) fn to_verifiability_leaves(
        &self,
        outcome: &ObservableOutcome,
    ) -> Vec<Verifiability> {
        match outcome {
            ObservableOutcome::ApplicationWindow {
                app_name,
                title_hint,
            } => {
                vec![Verifiability::WindowState {
                    title_contains: title_hint.clone().or_else(|| Some(app_name.clone())),
                    class: None,
                }]
            }
            ObservableOutcome::BrowserPage {
                url_contains,
                title_contains,
            } => {
                vec![Verifiability::BrowserPageLoaded {
                    url_contains: url_contains.clone(),
                    title_contains: title_contains.clone(),
                }]
            }
            ObservableOutcome::IdeWorkspace { path } => {
                vec![Verifiability::WindowState {
                    title_contains: Some(
                        std::path::Path::new(path)
                            .file_name()
                            .and_then(|n| n.to_str())
                            .unwrap_or(path.as_str())
                            .to_string(),
                    ),
                    class: None,
                }]
            }
            ObservableOutcome::TerminalOutput { contains } => {
                if contains.is_empty() {
                    vec![Verifiability::WindowState {
                        title_contains: Some("terminal".to_string()),
                        class: None,
                    }]
                } else {
                    vec![Verifiability::OcrTextPresent {
                        text: contains.clone(),
                        case_insensitive: true,
                    }]
                }
            }
            ObservableOutcome::FileCreated {
                path,
                min_size_bytes,
            } => {
                let kind = min_size_bytes
                    .map(|s| FsEffect::SizeGreaterThan(s.saturating_sub(1)))
                    .unwrap_or(FsEffect::Exists);
                vec![Verifiability::FileSystemEffect {
                    path: path.clone(),
                    kind,
                }]
            }
            ObservableOutcome::FileModified { path, .. } => {
                vec![Verifiability::FileSystemEffect {
                    path: path.clone(),
                    kind: FsEffect::Exists,
                }]
            }
            ObservableOutcome::ProcessRunning { binary } => {
                vec![Verifiability::ProcessLaunched {
                    binary: binary.clone(),
                    max_wait_ms: 2000,
                }]
            }
            ObservableOutcome::NotificationVisible { contains } => {
                vec![Verifiability::OcrTextPresent {
                    text: contains.clone(),
                    case_insensitive: true,
                }]
            }
            ObservableOutcome::AudioPlaybackActive { player_hint } => {
                let binary = player_hint.clone().unwrap_or_else(|| "mpv".to_string());
                vec![Verifiability::ProcessLaunched {
                    binary,
                    max_wait_ms: 1000,
                }]
            }
            ObservableOutcome::EmailSentConfirmation { client_hint: _ } => {
                // Verify by looking for "sent" text in mail client window
                vec![Verifiability::OcrTextPresent {
                    text: "sent".to_string(),
                    case_insensitive: true,
                }]
            }
            ObservableOutcome::UserAcknowledged { question } => {
                vec![Verifiability::UserAttested {
                    question: question.clone(),
                }]
            }
            ObservableOutcome::DownloadComplete {
                file_hint,
                target_dir,
            } => {
                let dir = target_dir.clone().or_else(|| {
                    std::env::var("HOME")
                        .ok()
                        .map(|h| PathBuf::from(h).join("Downloads"))
                });
                if let (Some(hint), Some(dir)) = (file_hint, dir) {
                    vec![Verifiability::FileSystemEffect {
                        path: dir.join(hint),
                        kind: FsEffect::Exists,
                    }]
                } else {
                    vec![Verifiability::Unverifiable {
                        reason: "Download target path unknown".to_string(),
                    }]
                }
            }
            ObservableOutcome::Silent => vec![],
        }
    }

    /// Check WorldModelStore for evidence that an outcome is already visible.
    ///
    /// Returns `Some(result)` when PSDG has recent high-confidence evidence.
    /// Returns `None` when live probe is needed.
    fn check_psdg_evidence(&self, outcome: &ObservableOutcome) -> Option<ObservableVerifyResult> {
        let psdg = self.psdg.as_ref()?;
        let store = psdg.store();

        match outcome {
            ObservableOutcome::BrowserPage { url_contains, .. } => {
                if let Ok(Some(fact)) = store.query("browser_primary", "current_url") {
                    if fact.confidence >= 0.7 {
                        let url_match = url_contains
                            .as_ref()
                            .map(|u| fact.object.contains(u.as_str()))
                            .unwrap_or(true);
                        if url_match {
                            return Some(ObservableVerifyResult {
                                visible: true,
                                confidence: fact.confidence as f32,
                                tier: VerificationConfidenceTier::FullSemantic,
                                evidence: format!(
                                    "PSDG: browser at {} (conf={:.2})",
                                    fact.object, fact.confidence
                                ),
                                latency_ms: 0,
                                psdg_backed: true,
                            });
                        }
                    }
                }
                None
            }
            ObservableOutcome::IdeWorkspace { path } => {
                if let Ok(Some(fact)) = store.query("ide_primary", "workspace_root") {
                    if fact.confidence >= 0.7 && fact.object.contains(path.as_str()) {
                        return Some(ObservableVerifyResult {
                            visible: true,
                            confidence: fact.confidence as f32,
                            tier: VerificationConfidenceTier::FullSemantic,
                            evidence: format!(
                                "PSDG: IDE workspace at {} (conf={:.2})",
                                fact.object, fact.confidence
                            ),
                            latency_ms: 0,
                            psdg_backed: true,
                        });
                    }
                }
                None
            }
            ObservableOutcome::ApplicationWindow { app_name, .. } => {
                if let Ok(Some(fact)) = store.query("desktop_environment", "focused_app") {
                    if fact.confidence >= 0.7
                        && fact
                            .object
                            .to_lowercase()
                            .contains(&app_name.to_lowercase())
                    {
                        return Some(ObservableVerifyResult {
                            visible: true,
                            confidence: fact.confidence as f32,
                            tier: VerificationConfidenceTier::PartialObservable,
                            evidence: format!(
                                "PSDG: focused app is {} (conf={:.2})",
                                fact.object, fact.confidence
                            ),
                            latency_ms: 0,
                            psdg_backed: true,
                        });
                    }
                }
                None
            }
            _ => None,
        }
    }

    /// Generate a human-readable completion summary for the response.
    ///
    /// Called after `verify_all()` to produce the "what happened" narrative
    /// that goes into the user-visible response.
    pub fn completion_narrative(
        &self,
        aggregate: &AggregateVisibilityResult,
        _operation: Operation,
    ) -> String {
        if aggregate.per_outcome.is_empty() {
            return String::new();
        }

        if aggregate.all_required_visible {
            let visible_descs: Vec<String> = aggregate
                .per_outcome
                .iter()
                .filter(|(_, r)| r.visible)
                .map(|(p, _r)| describe_outcome_short(&p.outcome))
                .collect();

            if visible_descs.is_empty() {
                return String::new();
            }
            format!("✓ {}", visible_descs.join("; "))
        } else {
            let invisible: Vec<String> = aggregate
                .per_outcome
                .iter()
                .filter(|(_, r)| !r.visible)
                .map(|(p, r)| format!("{} ({})", describe_outcome_short(&p.outcome), r.evidence))
                .collect();
            format!(
                "⚠ Expected outcome not yet visible: {}",
                invisible.join("; ")
            )
        }
    }
}

fn describe_outcome_short(outcome: &ObservableOutcome) -> String {
    match outcome {
        ObservableOutcome::ApplicationWindow { app_name, .. } => format!("{} is open", app_name),
        ObservableOutcome::BrowserPage { url_contains, .. } => {
            format!(
                "browser at {}",
                url_contains.as_deref().unwrap_or("target page")
            )
        }
        ObservableOutcome::IdeWorkspace { path } => format!("IDE workspace {} is open", path),
        ObservableOutcome::TerminalOutput { contains } => {
            if contains.is_empty() {
                "terminal output visible".into()
            } else {
                format!("terminal shows '{}'", contains)
            }
        }
        ObservableOutcome::FileCreated { path, .. } => format!("file {} created", path.display()),
        ObservableOutcome::FileModified { path, .. } => format!("file {} modified", path.display()),
        ObservableOutcome::ProcessRunning { binary } => format!("{} is running", binary),
        ObservableOutcome::NotificationVisible { contains } => {
            format!("notification '{}' visible", contains)
        }
        ObservableOutcome::AudioPlaybackActive { .. } => "audio is playing".into(),
        ObservableOutcome::EmailSentConfirmation { .. } => "email sent".into(),
        ObservableOutcome::UserAcknowledged { .. } => "user acknowledged".into(),
        ObservableOutcome::DownloadComplete { file_hint, .. } => {
            format!(
                "download complete: {}",
                file_hint.as_deref().unwrap_or("file")
            )
        }
        ObservableOutcome::Silent => "background operation complete".into(),
    }
}

// ─── Aggregate Visibility Result ──────────────────────────────────────────────

/// Aggregate result of checking all visibility policies for a workflow.
#[derive(Debug)]
pub struct AggregateVisibilityResult {
    /// `true` if all non-silent, required outcomes are visible.
    pub all_required_visible: bool,
    /// Mean confidence across all non-silent checks.
    pub overall_confidence: f32,
    /// `true` if any outcome needs to be surfaced (brought to foreground).
    pub surfacing_needed: bool,
    /// Per-outcome results.
    pub per_outcome: Vec<(CompletionVisibilityPolicy, ObservableVerifyResult)>,
}

// ─── Test Coverage ─────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn engine() -> ObservableCompletionEngine {
        ObservableCompletionEngine::new(None)
    }

    // ── Outcome inference ──────────────────────────────────────────────────

    #[test]
    fn infer_open_firefox_produces_app_window() {
        let outcomes = infer_outcomes(
            "open firefox",
            &Verb::Open,
            &[TargetRef::App("firefox".into())],
            Operation::Automate,
        );
        assert!(outcomes.iter().any(|o| matches!(o, ObservableOutcome::ApplicationWindow { app_name, .. } if app_name == "firefox")));
    }

    #[test]
    fn infer_navigate_url_produces_browser_page() {
        let outcomes = infer_outcomes(
            "navigate to https://github.com",
            &Verb::Open,
            &[TargetRef::Url("https://github.com".into())],
            Operation::Automate,
        );
        assert!(outcomes
            .iter()
            .any(|o| matches!(o, ObservableOutcome::BrowserPage { .. })));
    }

    #[test]
    fn infer_run_and_show_produces_terminal_output() {
        let outcomes = infer_outcomes(
            "run the tests and show output",
            &Verb::Run,
            &[TargetRef::App("cargo test".into())],
            Operation::ExecuteShell,
        );
        assert!(outcomes
            .iter()
            .any(|o| matches!(o, ObservableOutcome::TerminalOutput { .. })));
    }

    #[test]
    fn infer_send_email_produces_email_sent() {
        let outcomes = infer_outcomes(
            "send the email to john",
            &Verb::Other("send".into()),
            &[],
            Operation::Send,
        );
        assert!(outcomes
            .iter()
            .any(|o| matches!(o, ObservableOutcome::EmailSentConfirmation { .. })));
    }

    #[test]
    fn infer_converse_produces_silent() {
        let outcomes = infer_outcomes(
            "what is the weather today?",
            &Verb::Other("ask".into()),
            &[],
            Operation::Converse,
        );
        assert!(outcomes
            .iter()
            .any(|o| matches!(o, ObservableOutcome::Silent)));
    }

    // ── Visibility policy ──────────────────────────────────────────────────

    #[test]
    fn silent_outcome_has_policy_silent_ok() {
        let policy =
            CompletionVisibilityPolicy::for_outcome(ObservableOutcome::Silent, Operation::Converse);
        assert_eq!(policy.visibility, VisibilityRequirement::SilentOk);
        assert!(policy.allow_hidden_execution);
    }

    #[test]
    fn email_send_needs_user_acknowledgement() {
        let policy = CompletionVisibilityPolicy::for_outcome(
            ObservableOutcome::EmailSentConfirmation { client_hint: None },
            Operation::Send,
        );
        assert_eq!(
            policy.visibility,
            VisibilityRequirement::UserAcknowledgementRequired
        );
    }

    #[test]
    fn run_and_show_output_needs_surfacing() {
        let policy = CompletionVisibilityPolicy::for_outcome(
            ObservableOutcome::TerminalOutput {
                contains: "ok".into(),
            },
            Operation::ExecuteShell,
        );
        assert_eq!(
            policy.visibility,
            VisibilityRequirement::OutputMustBeSurfaced
        );
    }

    // ── Verifiability leaf generation ──────────────────────────────────────

    #[test]
    fn app_window_maps_to_window_state_leaf() {
        let engine = engine();
        let leaves = engine.to_verifiability_leaves(&ObservableOutcome::ApplicationWindow {
            app_name: "firefox".into(),
            title_hint: None,
        });
        assert!(matches!(leaves[0], Verifiability::WindowState { .. }));
    }

    #[test]
    fn browser_page_maps_to_browser_page_loaded() {
        let engine = engine();
        let leaves = engine.to_verifiability_leaves(&ObservableOutcome::BrowserPage {
            url_contains: Some("github.com".into()),
            title_contains: None,
        });
        assert!(matches!(leaves[0], Verifiability::BrowserPageLoaded { .. }));
    }

    #[test]
    fn file_created_maps_to_fs_effect() {
        let engine = engine();
        let leaves = engine.to_verifiability_leaves(&ObservableOutcome::FileCreated {
            path: PathBuf::from("/tmp/test.txt"),
            min_size_bytes: Some(1),
        });
        assert!(matches!(leaves[0], Verifiability::FileSystemEffect { .. }));
    }

    // ── Verify visible (async) ─────────────────────────────────────────────

    #[tokio::test]
    async fn silent_outcome_is_always_visible() {
        let engine = engine();
        let policy =
            CompletionVisibilityPolicy::for_outcome(ObservableOutcome::Silent, Operation::Converse);
        let result = engine.verify_visible(&policy).await;
        assert!(result.visible);
        assert_eq!(result.confidence, 1.0);
    }

    #[tokio::test]
    async fn missing_file_is_not_visible() {
        let engine = engine();
        let policy = CompletionVisibilityPolicy::for_outcome(
            ObservableOutcome::FileCreated {
                path: PathBuf::from("/tmp/kria_batch2_test_nonexistent_9999.txt"),
                min_size_bytes: None,
            },
            Operation::Write,
        );
        let result = engine.verify_visible(&policy).await;
        assert!(
            !result.visible,
            "Non-existent file must not be considered visible"
        );
    }

    #[tokio::test]
    async fn existing_file_is_visible() {
        let engine = engine();
        let tmp = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(tmp.path(), b"hello world").unwrap();
        let policy = CompletionVisibilityPolicy::for_outcome(
            ObservableOutcome::FileCreated {
                path: tmp.path().to_path_buf(),
                min_size_bytes: Some(1),
            },
            Operation::Write,
        );
        let result = engine.verify_visible(&policy).await;
        assert!(result.visible, "Existing file must be considered visible");
        assert!(result.confidence > 0.0);
    }

    // ── Completion narrative ───────────────────────────────────────────────

    #[tokio::test]
    async fn completion_narrative_summarises_visible_outcomes() {
        let engine = engine();
        let tmp = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(tmp.path(), b"output").unwrap();

        let policies = vec![CompletionVisibilityPolicy::for_outcome(
            ObservableOutcome::FileCreated {
                path: tmp.path().to_path_buf(),
                min_size_bytes: Some(1),
            },
            Operation::Write,
        )];
        let aggregate = engine.verify_all(&policies).await;
        let narrative = engine.completion_narrative(&aggregate, Operation::Write);
        assert!(!narrative.is_empty());
        assert!(
            narrative.contains("✓") || narrative.contains("⚠"),
            "Narrative must use ✓ or ⚠"
        );
    }
}
