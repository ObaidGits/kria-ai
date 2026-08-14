//! RFC v2 (P4): Intent-level execution verification.
//!
//! Replaces the "step succeeded once typed" anti-pattern with explicit
//! [`Verifiability`] classes, each with a single bounded check (≤500 ms
//! except `ProcessLaunched`). The verifier NEVER replans and NEVER triggers
//! retries — those concerns live in the executor.
//!
//! See `docs/GUI_INTELLIGENCE_REVIEW.md` §4.5.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::time::SystemTime;

/// What kind of filesystem effect the verifier should look for.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum FsEffect {
    Exists,
    NotExists,
    ContainsBytes(Vec<u8>),
    SizeGreaterThan(u64),
}

/// Where to look for deterministic output.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum VerifyTarget {
    ActiveEditorBuffer,
    TerminalOutput,
    FilePath(PathBuf),
}

/// The explicit verifiability classes a Goal Tree leaf may carry.
///
/// Every leaf the planner emits MUST be tagged with one of these. The
/// `Unverifiable` variant is honest about its nature: it triggers a HITL
/// attestation prompt rather than reporting silent success.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Verifiability {
    /// Legacy active-window check. Prefer the more precise GUI state checks
    /// below for new plans.
    WindowState {
        title_contains: Option<String>,
        class: Option<String>,
    },
    /// A process exists. This is structural, but it does not imply a usable UI.
    ProcessRunning {
        binary: String,
        max_wait_ms: u32,
    },
    /// A window for the target is visible/listed, but not necessarily focused.
    WindowVisible {
        title_contains: Option<String>,
        class: Option<String>,
        pid: Option<u32>,
    },
    /// A window exists and has enough state to receive meaningful interaction.
    WindowInteractive {
        title_contains: Option<String>,
        class: Option<String>,
        pid: Option<u32>,
    },
    /// A foreground lease has been acquired by the workflow runtime. This does
    /// not prove OS focus by itself; it proves KRIA has exclusive GUI ownership.
    ForegroundLeaseAcquired {
        workflow_id: String,
    },
    /// Stronger than WindowState: the runtime believes the current keyboard
    /// target is the intended window/app.
    KeyboardTargetConfirmed {
        title_contains: Option<String>,
        class: Option<String>,
        pid: Option<u32>,
    },
    /// Semantic target confirmation through a structural substrate such as
    /// AT-SPI, CDP, LSP, or filesystem/shell output.
    SemanticTargetConfirmed {
        description: String,
        evidence_hint: Option<String>,
    },
    FileSystemEffect {
        path: PathBuf,
        kind: FsEffect,
    },
    ProcessLaunched {
        binary: String,
        max_wait_ms: u32,
    },
    ProcessNotRunning {
        binary: String,
        max_wait_ms: u32,
    },
    DeterministicOutput {
        expected_substring: String,
        in_target: VerifyTarget,
    },
    OcrTextPresent {
        text: String,
        case_insensitive: bool,
    },
    /// Verify that an accessible UI element exists in the accessibility tree.
    ///
    /// Uses AT-SPI — works on X11 and Wayland natively.
    /// Provides semantic verification that a UI element is present and visible.
    AccessibilityElement {
        role: String,
        name_contains: Option<String>,
        must_be_visible: bool,
    },
    /// Verify an interaction outcome, e.g. a dialog appeared or element vanished.
    InteractionOutcome {
        expected_role: String,
        expected_name_contains: Option<String>,
        action_type: String, // e.g. "click", "fill"
    },
    /// Verify that a browser page has loaded using CDP.
    BrowserPageLoaded {
        url_contains: Option<String>,
        title_contains: Option<String>,
    },
    UserAttested {
        question: String,
    },
    Unverifiable {
        reason: String,
    },
}

/// Confidence tier of the verification.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum VerificationConfidenceTier {
    FullSemantic,
    PartialObservable,
    StructuralOnly,
    Unobservable,
}

impl Default for VerificationConfidenceTier {
    fn default() -> Self {
        Self::Unobservable
    }
}

/// Outcome of a verification attempt.
#[derive(Debug, Clone)]
pub struct VerifyOutcome {
    pub verified: bool,
    pub confidence: f32,
    pub confidence_tier: VerificationConfidenceTier,
    pub evidence: String,
    pub latency_ms: u32,
}

/// Where a verification observation came from. Ordered roughly from most
/// structural/trustworthy to weakest fallback.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum VerificationEvidenceSource {
    FileSystem,
    ShellOutput,
    Cdp,
    Lsp,
    ProcessTable,
    AtSpi,
    WindowManager,
    Ocr,
    Heuristic,
    Hitl,
    Unknown,
}

impl VerificationEvidenceSource {
    pub fn authority_rank(&self) -> u8 {
        match self {
            Self::FileSystem | Self::ShellOutput => 100,
            Self::Cdp | Self::Lsp => 90,
            Self::ProcessTable => 80,
            Self::AtSpi => 70,
            Self::WindowManager => 55,
            Self::Ocr => 35,
            Self::Heuristic => 20,
            Self::Hitl => 15,
            Self::Unknown => 0,
        }
    }

    /// Bridge an OS-control evidence source into this shared verifier taxonomy
    /// (additive; linux-os-control-production Task 1.7, OSC-005). Existing
    /// variants and their ranks are unchanged. The authoritative *OS-state*
    /// ordering is defined by [`os_control_authority_rank`], not by
    /// [`Self::authority_rank`], because the GUI ranks treat filesystem and shell
    /// output equally whereas OS-state verification must rank authoritative
    /// service/filesystem state strictly above structured-command (shell) output.
    #[must_use]
    pub fn from_os_evidence(source: crate::os_control::contract::OsEvidenceSource) -> Self {
        use crate::os_control::contract::OsEvidenceSource as S;
        match source {
            S::AuthoritativeServiceState => Self::FileSystem,
            S::IndependentProviderQuery => Self::ProcessTable,
            S::StructuredCommandQuery => Self::ShellOutput,
            S::UserAttestation => Self::Hitl,
        }
    }
}

/// OS-state evidence authority rank (additive; linux-os-control-production Task
/// 1.7, OSC-005 §13). Authoritative service/property or filesystem state
/// strictly outranks an independent provider query, which strictly outranks
/// structured-command (shell) query output, which outranks user attestation.
/// Because these ranks are strictly ordered, **shell output can never outrank
/// authoritative OS state**.
#[must_use]
pub fn os_control_authority_rank(source: crate::os_control::contract::OsEvidenceSource) -> u8 {
    use crate::os_control::contract::OsEvidenceSource as S;
    match source {
        S::AuthoritativeServiceState => 100,
        S::IndependentProviderQuery => 80,
        S::StructuredCommandQuery => 50,
        S::UserAttestation => 20,
    }
}

/// Evidence reliability is deliberately coarse. The runtime should not pretend
/// it has precise confidence when desktop state is only partially observable.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum VerificationReliability {
    Authoritative,
    Strong,
    Partial,
    Weak,
    Unobservable,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VerificationEvidence {
    pub source: VerificationEvidenceSource,
    pub reliability: VerificationReliability,
    pub confidence: f32,
    pub semantic_meaning: String,
    pub observed_at: SystemTime,
    /// Freshness of the observation when emitted. Callers should treat stale
    /// desktop/window evidence as weaker than structural evidence.
    pub freshness_ms: u32,
    pub ambiguous: bool,
    pub details: String,
}

/// Evidence-rich verifier result. This is the newer contract used by orchestration
/// and recovery. `outcome` is kept for backward compatibility with legacy code.
#[derive(Debug, Clone)]
pub struct RichVerifyOutcome {
    pub outcome: VerifyOutcome,
    pub evidence: Vec<VerificationEvidence>,
}

impl RichVerifyOutcome {
    pub fn from_legacy(outcome: VerifyOutcome, source: VerificationEvidenceSource) -> Self {
        let reliability = match outcome.confidence_tier {
            VerificationConfidenceTier::FullSemantic => VerificationReliability::Strong,
            VerificationConfidenceTier::PartialObservable => VerificationReliability::Partial,
            VerificationConfidenceTier::StructuralOnly => VerificationReliability::Weak,
            VerificationConfidenceTier::Unobservable => VerificationReliability::Unobservable,
        };
        let evidence = VerificationEvidence {
            source,
            reliability,
            confidence: outcome.confidence,
            semantic_meaning: if outcome.verified {
                "verification_satisfied".to_string()
            } else {
                "verification_not_satisfied".to_string()
            },
            observed_at: SystemTime::now(),
            freshness_ms: outcome.latency_ms,
            ambiguous: outcome.confidence < 0.70 || !outcome.verified,
            details: outcome.evidence.clone(),
        };
        Self {
            outcome,
            evidence: vec![evidence],
        }
    }

    pub fn strongest_evidence(&self) -> Option<&VerificationEvidence> {
        self.evidence.iter().max_by_key(|e| {
            (
                e.source.authority_rank(),
                (e.confidence.clamp(0.0, 1.0) * 100.0) as u8,
            )
        })
    }
}

/// Verifier contract.
#[async_trait::async_trait]
pub trait ExecutionVerifier: Send + Sync {
    async fn verify(&self, leaf: &Verifiability) -> VerifyOutcome;

    async fn verify_rich(&self, leaf: &Verifiability) -> RichVerifyOutcome {
        let source = match leaf {
            Verifiability::FileSystemEffect { .. } => VerificationEvidenceSource::FileSystem,
            Verifiability::DeterministicOutput { in_target, .. } => match in_target {
                VerifyTarget::FilePath(_) => VerificationEvidenceSource::FileSystem,
                VerifyTarget::TerminalOutput => VerificationEvidenceSource::ShellOutput,
                VerifyTarget::ActiveEditorBuffer => VerificationEvidenceSource::AtSpi,
            },
            Verifiability::ProcessLaunched { .. }
            | Verifiability::ProcessNotRunning { .. }
            | Verifiability::ProcessRunning { .. } => VerificationEvidenceSource::ProcessTable,
            Verifiability::AccessibilityElement { .. }
            | Verifiability::InteractionOutcome { .. } => VerificationEvidenceSource::AtSpi,
            Verifiability::BrowserPageLoaded { .. } => VerificationEvidenceSource::Cdp,
            Verifiability::OcrTextPresent { .. } => VerificationEvidenceSource::Ocr,
            Verifiability::WindowState { .. }
            | Verifiability::WindowVisible { .. }
            | Verifiability::WindowInteractive { .. }
            | Verifiability::KeyboardTargetConfirmed { .. } => {
                VerificationEvidenceSource::WindowManager
            }
            Verifiability::ForegroundLeaseAcquired { .. } => VerificationEvidenceSource::Heuristic,
            Verifiability::SemanticTargetConfirmed { .. } => VerificationEvidenceSource::Heuristic,
            Verifiability::UserAttested { .. } => VerificationEvidenceSource::Hitl,
            Verifiability::Unverifiable { .. } => VerificationEvidenceSource::Unknown,
        };
        RichVerifyOutcome::from_legacy(self.verify(leaf).await, source)
    }
}

/// Placeholder verifier. Returns `verified = true` only for `Unverifiable`
/// (with a low confidence and a clear evidence string) so the executor still
/// knows to escalate. Replaced in P4.
pub struct NoopExecutionVerifier;

#[async_trait::async_trait]
impl ExecutionVerifier for NoopExecutionVerifier {
    async fn verify(&self, leaf: &Verifiability) -> VerifyOutcome {
        match leaf {
            Verifiability::Unverifiable { reason } => VerifyOutcome {
                verified: false,
                confidence: 0.0,
                confidence_tier: VerificationConfidenceTier::Unobservable,
                evidence: format!("unverifiable: {}", reason),
                latency_ms: 0,
            },
            Verifiability::AccessibilityElement { role, name_contains, .. } => VerifyOutcome {
                verified: false,
                confidence: 0.0,
                confidence_tier: VerificationConfidenceTier::Unobservable,
                evidence: format!(
                    "AccessibilityElement verification for {} '{}' requires BoundedExecutionVerifier",
                    role, name_contains.as_deref().unwrap_or("any")
                ),
                latency_ms: 0,
            },
            Verifiability::InteractionOutcome { action_type, .. } => VerifyOutcome {
                verified: false,
                confidence: 0.0,
                confidence_tier: VerificationConfidenceTier::Unobservable,
                evidence: format!("InteractionOutcome ({}) requires BoundedExecutionVerifier", action_type),
                latency_ms: 0,
            },
            Verifiability::BrowserPageLoaded { url_contains, .. } => VerifyOutcome {
                verified: false,
                confidence: 0.0,
                confidence_tier: VerificationConfidenceTier::Unobservable,
                evidence: format!(
                    "BrowserPageLoaded verification requires BoundedExecutionVerifier (url={:?})",
                    url_contains
                ),
                latency_ms: 0,
            },
            Verifiability::ProcessNotRunning { binary, .. } => VerifyOutcome {
                verified: false,
                confidence: 0.0,
                confidence_tier: VerificationConfidenceTier::Unobservable,
                evidence: format!("ProcessNotRunning ({}) requires BoundedExecutionVerifier", binary),
                latency_ms: 0,
            },
            Verifiability::ProcessRunning { binary, .. } => VerifyOutcome {
                verified: false,
                confidence: 0.0,
                confidence_tier: VerificationConfidenceTier::Unobservable,
                evidence: format!("ProcessRunning ({}) requires BoundedExecutionVerifier", binary),
                latency_ms: 0,
            },
            Verifiability::WindowVisible { .. }
            | Verifiability::WindowInteractive { .. }
            | Verifiability::ForegroundLeaseAcquired { .. }
            | Verifiability::KeyboardTargetConfirmed { .. }
            | Verifiability::SemanticTargetConfirmed { .. } => VerifyOutcome {
                verified: false,
                confidence: 0.0,
                confidence_tier: VerificationConfidenceTier::Unobservable,
                evidence: "Precise GUI state verification requires BoundedExecutionVerifier".into(),
                latency_ms: 0,
            },
            _ => VerifyOutcome {
                verified: false,
                confidence: 0.0,
                confidence_tier: VerificationConfidenceTier::Unobservable,
                evidence: "P4 verifier not yet implemented".into(),
                latency_ms: 0,
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn noop_verifier_never_falsely_reports_success() {
        let leaves = vec![
            Verifiability::WindowState {
                title_contains: None,
                class: None,
            },
            Verifiability::FileSystemEffect {
                path: PathBuf::from("/tmp/x"),
                kind: FsEffect::Exists,
            },
            Verifiability::Unverifiable {
                reason: "demo".into(),
            },
        ];
        for leaf in leaves {
            let out = NoopExecutionVerifier.verify(&leaf).await;
            assert!(!out.verified, "noop verifier must never claim success");
        }
    }

    #[test]
    fn evidence_authority_prefers_structural_sources() {
        assert!(
            VerificationEvidenceSource::FileSystem.authority_rank()
                > VerificationEvidenceSource::WindowManager.authority_rank()
        );
        assert!(
            VerificationEvidenceSource::AtSpi.authority_rank()
                > VerificationEvidenceSource::Ocr.authority_rank()
        );
    }

    #[tokio::test]
    async fn rich_verify_default_wraps_legacy_outcome() {
        let out = NoopExecutionVerifier
            .verify_rich(&Verifiability::ProcessRunning {
                binary: "code".into(),
                max_wait_ms: 1,
            })
            .await;
        assert!(!out.outcome.verified);
        assert_eq!(
            out.evidence[0].source,
            VerificationEvidenceSource::ProcessTable
        );
    }
}
