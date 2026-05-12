//! RFC v2 (P4): Intent-level execution verification.
//!
//! Replaces the "step succeeded once typed" anti-pattern with explicit
//! [`Verifiability`] classes, each with a single bounded check (≤500 ms
//! except `ProcessLaunched`). The verifier NEVER replans and NEVER triggers
//! retries — those concerns live in the executor.
//!
//! See `docs/GUI_INTELLIGENCE_REVIEW.md` §4.5.

use std::path::PathBuf;

/// What kind of filesystem effect the verifier should look for.
#[derive(Debug, Clone)]
pub enum FsEffect {
    Exists,
    ContainsBytes(Vec<u8>),
    SizeGreaterThan(u64),
}

/// Where to look for deterministic output.
#[derive(Debug, Clone)]
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
#[derive(Debug, Clone)]
pub enum Verifiability {
    WindowState {
        title_contains: Option<String>,
        class: Option<String>,
    },
    FileSystemEffect {
        path: PathBuf,
        kind: FsEffect,
    },
    ProcessLaunched {
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
    UserAttested {
        question: String,
    },
    Unverifiable {
        reason: String,
    },
}

/// Outcome of a verification attempt.
#[derive(Debug, Clone)]
pub struct VerifyOutcome {
    pub verified: bool,
    pub confidence: f32,
    pub evidence: String,
    pub latency_ms: u32,
}

/// Verifier contract.
#[async_trait::async_trait]
pub trait ExecutionVerifier: Send + Sync {
    async fn verify(&self, leaf: &Verifiability) -> VerifyOutcome;
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
                evidence: format!("unverifiable: {}", reason),
                latency_ms: 0,
            },
            _ => VerifyOutcome {
                verified: false,
                confidence: 0.0,
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
            Verifiability::WindowState { title_contains: None, class: None },
            Verifiability::FileSystemEffect { path: PathBuf::from("/tmp/x"), kind: FsEffect::Exists },
            Verifiability::Unverifiable { reason: "demo".into() },
        ];
        for leaf in leaves {
            let out = NoopExecutionVerifier.verify(&leaf).await;
            assert!(!out.verified, "noop verifier must never claim success");
        }
    }
}
