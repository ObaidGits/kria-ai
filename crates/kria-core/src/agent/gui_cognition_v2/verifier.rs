//! GUI Cognition — shared external-signal verifier registry (spec Task 1).
//!
//! This is the single most important anti-loop primitive: COMPLETION (in the
//! live loop) and PROOF (in the live test harness) are decided by the SAME
//! external-signal predicates, so what the loop believes and what a test proves
//! cannot diverge (Requirements 15, 22; Properties 11, 12, 19).
//!
//! A [`SubGoalVerifier`] maps each [`SubGoalKind`] to a predicate evaluated over
//! injected [`VerificationProbe`] signals (window/title/OCR/file/output/element),
//! returning a confidence-scored [`Verdict`]:
//!   - `Verified`   — an external signal confirms the sub-goal, confidence ≥ floor.
//!   - `Failed`     — an external signal contradicts it.
//!   - `Unverified` — no usable signal, or confidence below the floor (NEVER a
//!                    silent pass — Requirement 15.2 / 22.3).
//!
//! The probe is the seam over the real desktop substrate (window manager,
//! OCR/grounded sight, filesystem, bridge working-context). Core ships the pure
//! decision logic + a recording fake; the desktop wires the concrete probe.

use async_trait::async_trait;

use super::types::{SubGoal, SubGoalKind};

/// Minimum confidence for a definitive `Verified`/`Failed`; below this the
/// verdict downgrades to `Unverified` (Requirement 22.3).
pub const CONFIDENCE_FLOOR: f32 = 0.6;

/// A single external-signal reading: the observed value and how confident the
/// probe is in it. `None` means the signal was unavailable (→ `Unverified`).
#[derive(Debug, Clone, PartialEq)]
pub struct Signal<T> {
    pub value: T,
    pub confidence: f32,
    pub detail: String,
}

impl<T> Signal<T> {
    pub fn new(value: T, confidence: f32, detail: impl Into<String>) -> Self {
        Self {
            value,
            confidence,
            detail: detail.into(),
        }
    }
}

/// Outcome of verifying one sub-goal.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VerifyOutcome {
    Verified,
    Failed,
    Unverified,
}

/// A confidence-scored verification verdict (Requirement 22.2).
#[derive(Debug, Clone, PartialEq)]
pub struct Verdict {
    pub outcome: VerifyOutcome,
    pub confidence: f32,
    pub detail: String,
}

impl Verdict {
    pub fn verified(confidence: f32, detail: impl Into<String>) -> Self {
        Self {
            outcome: VerifyOutcome::Verified,
            confidence,
            detail: detail.into(),
        }
    }
    pub fn failed(confidence: f32, detail: impl Into<String>) -> Self {
        Self {
            outcome: VerifyOutcome::Failed,
            confidence,
            detail: detail.into(),
        }
    }
    pub fn unverified(detail: impl Into<String>) -> Self {
        Self {
            outcome: VerifyOutcome::Unverified,
            confidence: 0.0,
            detail: detail.into(),
        }
    }

    pub fn is_verified(&self) -> bool {
        self.outcome == VerifyOutcome::Verified
    }

    /// Apply the confidence floor: a definitive verdict whose confidence is below
    /// the floor is downgraded to `Unverified` (never a low-confidence pass/fail).
    fn with_floor(self) -> Self {
        match self.outcome {
            VerifyOutcome::Verified | VerifyOutcome::Failed
                if self.confidence < CONFIDENCE_FLOOR =>
            {
                Verdict::unverified(format!(
                    "low confidence {:.2} < floor {:.2}: {}",
                    self.confidence, CONFIDENCE_FLOOR, self.detail
                ))
            }
            _ => self,
        }
    }
}

/// External-signal seam. Every method returns `Option<Signal<_>>`: `None` when
/// the signal could not be obtained (→ the verifier reports `Unverified`). The
/// desktop implements these against the compositor/sight/filesystem; tests use a
/// recording fake. Implementations SHOULD settle (bounded retry) against a
/// still-changing screen before answering, to avoid race-induced verdicts
/// (Requirement 22.4) — that settling lives in the concrete probe.
#[async_trait]
pub trait VerificationProbe: Send + Sync {
    /// Is a window matching `hint` present AND focused? (window manager / compositor)
    async fn window_present_focused(&self, hint: &str) -> Option<Signal<bool>>;
    /// The active window's title/URL, for navigation checks (browser title/OCR).
    async fn active_window_title(&self) -> Option<Signal<String>>;
    /// Does the on-screen text contain `needle`? (OCR / grounded sight)
    async fn screen_contains(&self, needle: &str) -> Option<Signal<bool>>;
    /// Does `path` exist (and optionally contain `contains`)? (filesystem)
    async fn file_matches(&self, path: &str, contains: Option<&str>) -> Option<Signal<bool>>;
    /// Captured output of the most recent bridged command (working context).
    async fn command_output(&self) -> Option<Signal<String>>;
    /// Is an element labeled `label` observable on the current screen? (grounded sight)
    async fn element_observable(&self, label: &str) -> Option<Signal<bool>>;
}

/// Verify one sub-goal against external signals. Pure decision logic over the
/// injected probe, so it is fully unit-testable with a fake and is the SAME code
/// the loop and the harness call.
pub async fn verify_sub_goal(sub_goal: &SubGoal, probe: &dyn VerificationProbe) -> Verdict {
    let target = sub_goal.target_hint.as_deref().unwrap_or("");
    let verdict = match sub_goal.kind {
        SubGoalKind::OpenApp => match probe.window_present_focused(target).await {
            Some(s) if s.value => {
                Verdict::verified(s.confidence, format!("window focused: {}", s.detail))
            }
            Some(s) => Verdict::failed(s.confidence, format!("window not focused: {}", s.detail)),
            None => Verdict::unverified("no window signal"),
        },
        SubGoalKind::Navigate => match probe.active_window_title().await {
            Some(s) => {
                let hay = s.value.to_ascii_lowercase();
                let needle = sub_goal
                    .expect_contains
                    .as_deref()
                    .unwrap_or(target)
                    .to_ascii_lowercase();
                if !needle.is_empty() && hay.contains(&needle) {
                    Verdict::verified(
                        s.confidence,
                        format!("loaded page title '{}' matches '{}'", s.value, needle),
                    )
                } else {
                    // STRICT (Requirement 15/22): a Navigate is verified ONLY by the
                    // loaded page (window title / URL), NEVER by arbitrary on-screen
                    // text — otherwise the typed-but-unsubmitted address-bar text
                    // would falsely "match" and report success for a page that never
                    // loaded. No OCR fallback here.
                    Verdict::failed(
                        s.confidence,
                        format!(
                            "page title '{}' does not show '{}' (not loaded?)",
                            s.value, needle
                        ),
                    )
                }
            }
            None => Verdict::unverified("no active-title signal"),
        },
        SubGoalKind::RunCommand => match probe.command_output().await {
            Some(s) => match sub_goal.expect_contains.as_deref() {
                Some(exp) if !exp.is_empty() => {
                    if s.value.contains(exp) {
                        Verdict::verified(s.confidence, format!("output contains '{exp}'"))
                    } else {
                        Verdict::failed(s.confidence, format!("output missing '{exp}'"))
                    }
                }
                // No explicit expectation: any captured output proves it ran.
                _ if !s.value.trim().is_empty() => {
                    Verdict::verified(s.confidence, "command produced output")
                }
                _ => Verdict::failed(s.confidence, "no command output"),
            },
            None => Verdict::unverified("no command-output signal"),
        },
        SubGoalKind::WriteFile => {
            match probe
                .file_matches(target, sub_goal.expect_contains.as_deref())
                .await
            {
                Some(s) if s.value => {
                    Verdict::verified(s.confidence, format!("file ok: {}", s.detail))
                }
                Some(s) => {
                    Verdict::failed(s.confidence, format!("file missing/mismatch: {}", s.detail))
                }
                None => Verdict::unverified("no filesystem signal"),
            }
        }
        SubGoalKind::ReadOutput => match probe.command_output().await {
            Some(s) if !s.value.trim().is_empty() => {
                Verdict::verified(s.confidence, "output available")
            }
            Some(s) => Verdict::failed(s.confidence, "empty output"),
            None => Verdict::unverified("no output signal"),
        },
        SubGoalKind::Click => match probe.element_observable(target).await {
            Some(s) if s.value => Verdict::verified(
                s.confidence,
                format!("element/pane observable: {}", s.detail),
            ),
            Some(s) => Verdict::failed(
                s.confidence,
                format!("expected change not observable: {}", s.detail),
            ),
            None => Verdict::unverified("no element/screen signal"),
        },
        SubGoalKind::Type => {
            let needle = sub_goal.expect_contains.as_deref().unwrap_or(target);
            match probe.screen_contains(needle).await {
                Some(s) if s.value => {
                    Verdict::verified(s.confidence, format!("text '{needle}' present"))
                }
                Some(s) => Verdict::failed(s.confidence, format!("text '{needle}' absent")),
                None => Verdict::unverified("no screen-text signal"),
            }
        }
        // Verify/Other have no intrinsic action; a Verify checkpoint with a target
        // is treated like a screen-contains assertion, else Unverified.
        SubGoalKind::Verify | SubGoalKind::Other => {
            if let Some(needle) = sub_goal
                .expect_contains
                .as_deref()
                .or(Some(target))
                .filter(|n| !n.is_empty())
            {
                match probe.screen_contains(needle).await {
                    Some(s) if s.value => {
                        Verdict::verified(s.confidence, format!("checkpoint '{needle}' satisfied"))
                    }
                    Some(s) => {
                        Verdict::failed(s.confidence, format!("checkpoint '{needle}' failed"))
                    }
                    None => Verdict::unverified("no checkpoint signal"),
                }
            } else {
                Verdict::unverified("no checkpoint target")
            }
        }
    };
    verdict.with_floor()
}

/// Trait wrapper so callers can hold a verifier object; the default impl just
/// delegates to [`verify_sub_goal`]. Lets the loop and harness share one type.
#[async_trait]
pub trait SubGoalVerifier: Send + Sync {
    async fn verify(&self, sub_goal: &SubGoal, probe: &dyn VerificationProbe) -> Verdict;
}

/// The standard registry verifier (the production default).
pub struct StandardVerifier;

#[async_trait]
impl SubGoalVerifier for StandardVerifier {
    async fn verify(&self, sub_goal: &SubGoal, probe: &dyn VerificationProbe) -> Verdict {
        verify_sub_goal(sub_goal, probe).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use std::sync::Mutex;

    /// A fully scriptable probe: each method returns whatever the test seeds.
    #[derive(Default)]
    struct FakeProbe {
        window: Mutex<Option<Signal<bool>>>,
        title: Mutex<Option<Signal<String>>>,
        screen: Mutex<HashMap<String, Signal<bool>>>,
        files: Mutex<HashMap<String, Signal<bool>>>,
        output: Mutex<Option<Signal<String>>>,
        elements: Mutex<HashMap<String, Signal<bool>>>,
    }

    #[async_trait]
    impl VerificationProbe for FakeProbe {
        async fn window_present_focused(&self, _hint: &str) -> Option<Signal<bool>> {
            self.window.lock().unwrap().clone()
        }
        async fn active_window_title(&self) -> Option<Signal<String>> {
            self.title.lock().unwrap().clone()
        }
        async fn screen_contains(&self, needle: &str) -> Option<Signal<bool>> {
            self.screen.lock().unwrap().get(needle).cloned()
        }
        async fn file_matches(&self, path: &str, _contains: Option<&str>) -> Option<Signal<bool>> {
            self.files.lock().unwrap().get(path).cloned()
        }
        async fn command_output(&self) -> Option<Signal<String>> {
            self.output.lock().unwrap().clone()
        }
        async fn element_observable(&self, label: &str) -> Option<Signal<bool>> {
            self.elements.lock().unwrap().get(label).cloned()
        }
    }

    fn sg(kind: SubGoalKind, target: &str) -> SubGoal {
        SubGoal::new("t", kind).with_target(target)
    }

    #[tokio::test]
    async fn open_app_verified_when_window_focused() {
        let probe = FakeProbe::default();
        *probe.window.lock().unwrap() = Some(Signal::new(true, 0.95, "Calculator focused"));
        let v = verify_sub_goal(&sg(SubGoalKind::OpenApp, "calculator"), &probe).await;
        assert_eq!(v.outcome, VerifyOutcome::Verified);
        assert!(v.confidence >= CONFIDENCE_FLOOR);
    }

    #[tokio::test]
    async fn open_app_failed_when_not_focused() {
        let probe = FakeProbe::default();
        *probe.window.lock().unwrap() = Some(Signal::new(false, 0.9, "no match"));
        let v = verify_sub_goal(&sg(SubGoalKind::OpenApp, "calculator"), &probe).await;
        assert_eq!(v.outcome, VerifyOutcome::Failed);
    }

    #[tokio::test]
    async fn no_signal_is_unverified_never_pass() {
        let probe = FakeProbe::default(); // window signal unset → None
        let v = verify_sub_goal(&sg(SubGoalKind::OpenApp, "calculator"), &probe).await;
        assert_eq!(v.outcome, VerifyOutcome::Unverified);
    }

    #[tokio::test]
    async fn low_confidence_downgrades_to_unverified() {
        let probe = FakeProbe::default();
        *probe.window.lock().unwrap() = Some(Signal::new(true, 0.4, "blurry"));
        let v = verify_sub_goal(&sg(SubGoalKind::OpenApp, "x"), &probe).await;
        assert_eq!(
            v.outcome,
            VerifyOutcome::Unverified,
            "confidence below floor must not pass"
        );
    }

    #[tokio::test]
    async fn navigate_matches_title() {
        let probe = FakeProbe::default();
        *probe.title.lock().unwrap() = Some(Signal::new("YouTube - Chromium".into(), 0.9, "title"));
        let v = verify_sub_goal(&sg(SubGoalKind::Navigate, "youtube"), &probe).await;
        assert_eq!(v.outcome, VerifyOutcome::Verified);
        // A non-matching target fails.
        let v2 = verify_sub_goal(&sg(SubGoalKind::Navigate, "gmail"), &probe).await;
        assert_eq!(v2.outcome, VerifyOutcome::Failed);
    }

    #[tokio::test]
    async fn run_command_verified_by_expected_output() {
        let probe = FakeProbe::default();
        *probe.output.lock().unwrap() = Some(Signal::new("file1\nfile2\n".into(), 0.9, "ls"));
        let mut g = sg(SubGoalKind::RunCommand, "ls");
        g.expect_contains = Some("file1".into());
        assert_eq!(
            verify_sub_goal(&g, &probe).await.outcome,
            VerifyOutcome::Verified
        );
    }

    #[tokio::test]
    async fn write_file_verified_by_filesystem() {
        let probe = FakeProbe::default();
        probe.files.lock().unwrap().insert(
            "/tmp/pascal.py".into(),
            Signal::new(true, 0.99, "exists with content"),
        );
        let v = verify_sub_goal(&sg(SubGoalKind::WriteFile, "/tmp/pascal.py"), &probe).await;
        assert_eq!(v.outcome, VerifyOutcome::Verified);
    }

    #[tokio::test]
    async fn type_verified_by_screen_text() {
        let probe = FakeProbe::default();
        probe
            .screen
            .lock()
            .unwrap()
            .insert("3328".into(), Signal::new(true, 0.85, "ocr"));
        let mut g = sg(SubGoalKind::Type, "");
        g.expect_contains = Some("3328".into());
        assert_eq!(
            verify_sub_goal(&g, &probe).await.outcome,
            VerifyOutcome::Verified
        );
    }

    #[tokio::test]
    async fn click_verified_by_observable_element() {
        let probe = FakeProbe::default();
        probe
            .elements
            .lock()
            .unwrap()
            .insert("Wi-Fi".into(), Signal::new(true, 0.8, "pane"));
        assert_eq!(
            verify_sub_goal(&sg(SubGoalKind::Click, "Wi-Fi"), &probe)
                .await
                .outcome,
            VerifyOutcome::Verified
        );
    }

    #[tokio::test]
    async fn standard_verifier_delegates() {
        let probe = FakeProbe::default();
        *probe.window.lock().unwrap() = Some(Signal::new(true, 0.9, "ok"));
        let v = StandardVerifier
            .verify(&sg(SubGoalKind::OpenApp, "x"), &probe)
            .await;
        assert!(v.is_verified());
    }
}
