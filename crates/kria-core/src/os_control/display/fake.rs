//! Deny-live fake [`DisplayTransport`] (OSC-019, OSC-031, OSC-032, OSC-033),
//! Task 2.2.
//!
//! Compiled only under `os-control-test`. It models a backlight as a single
//! in-memory brightness cell: no `gdbus`, no `brightnessctl`, no `xrandr`, no
//! child process of any kind. `dispatch` records the governed command as its
//! redacted [`StructuredCommandSummary`] and **applies the effect to the cell**
//! by parsing the percentage back out of the governed argv, so an
//! observe → apply → re-observe → verify → rollback lifecycle converges on the
//! fake's own state rather than on a scripted sequence.
//!
//! # `0` is a brightness; "unknown" is not (OSC-031)
//!
//! A screen at 0 % is a perfectly valid, fully-observed state. "The backlight
//! could not be read" is a *different fact*, and conflating the two would let a
//! `set_brightness` verify against a fabricated reading. The two are therefore
//! represented separately and both are scriptable:
//!
//! * [`FakeDisplayTransport::read_ok`]`(0)` — observed, and it is zero;
//! * [`FakeDisplayTransport::read_unknown`] — one read whose value is genuinely
//!   indeterminate (an ambiguous parse) → [`OsControlError::Unavailable`];
//! * [`FakeDisplayTransport::read_failure`] — every read fails (a wedged bus);
//! * a fake with nothing scripted at all — the cell starts *unknown*, never at
//!   `0`, so an unscripted read reports `Unavailable` instead of inventing a
//!   dark screen.
//!
//! # Read ordering
//!
//! One governed mutation performs several reads in a fixed order
//! (pre-observation → under-lease re-observation → pre-apply snapshot →
//! post-apply re-observation → verify), so scripted reads are a FIFO queue.
//! When the queue drains, reads fall through to the modelled cell — which
//! `dispatch` has by then updated — rather than to a canned constant.
//!
//! # What this fake deliberately does NOT model
//!
//! An apply-then-confirm display *configuration* change (mode/resolution/output
//! layout) auto-reverts when it is never confirmed, which is what keeps a user
//! from being stranded at a black screen with nothing to click. The display
//! surface migrated in Task 2.2 is brightness only — [`super::DisplayOp`] is
//! `GetState | SetBrightness` and [`DisplayTransport`] has no
//! configure/confirm pair — so there is no such transaction here to model, and
//! inventing one would mean adding a port method the suite does not require.
//! The reversal path that *does* exist is the governed rollback on a
//! post-apply contradiction, and that one is modelled honestly: the fake's
//! cell really is written back to the pre-apply percentage by the rollback
//! dispatch.

use std::collections::VecDeque;
use std::sync::Mutex;

use async_trait::async_trait;

use crate::os_control::context::{AdmittedMutationContext, HostExecutionContext};
use crate::os_control::contract::{BoundedVec, Digest, ProviderId, SafeText};
use crate::os_control::error::OsControlError;
use crate::os_control::linux::structured_command::{
    StructuredCommandRequest, StructuredCommandSummary,
};
use crate::os_control::receipt::{AppliedDispatch, ApplyOutcome};

use super::selection::BrightnessBackend;
use super::DisplayTransport;

/// Provider identity reported by the fake transport. Matches the `provider`
/// named by the deny-live lifecycle suite's [`crate::os_control::MutationPlan`]
/// so the receipt, the rollback token and the transport all agree on one
/// identity.
pub const FAKE_DISPLAY_PROVIDER_ID: &str = "display-fake-brightnessctl";

/// One scripted read. `Unknown` is a first-class outcome, not a sentinel value:
/// it is the only way to express "the backlight could not be read" without
/// borrowing a real percentage such as `0` to mean it.
#[derive(Debug, Clone)]
enum ScriptedRead {
    /// The backlight was read and it is at this percentage (`0` included).
    Percent(u8),
    /// The read completed but the value was indeterminate (ambiguous parse).
    Unknown(String),
}

/// How a scripted dispatch should fail.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FakeDisplayFault {
    /// The OS authority refused the write (sysfs backlight not writable, no
    /// polkit rule for the GNOME property).
    PermissionDenied,
    /// The backend is not present in this session at all.
    Unavailable,
}

/// A scripted, in-memory backlight.
pub struct FakeDisplayTransport {
    backend: BrightnessBackend,
    /// Ordered scripted reads. Consumed front-to-back.
    scripted: Mutex<VecDeque<ScriptedRead>>,
    /// The modelled backlight. `None` is *unknown*, which is why a fresh fake
    /// with nothing scripted reports `Unavailable` rather than `0`.
    cell: Mutex<Option<u8>>,
    /// Sticky: every read fails with this reason.
    read_failure: Option<String>,
    /// Scripted `dispatch` outcome (defaults to `Applied`).
    outcome: Mutex<Option<ApplyOutcome>>,
    /// Sticky: `dispatch` fails with this fault and applies no effect.
    dispatch_fault: Option<FakeDisplayFault>,
    /// Redacted projections of the governed commands, in order. Never executed.
    dispatched: Mutex<Vec<StructuredCommandSummary>>,
    /// The exact argv of each dispatch, kept so the fake can apply the effect
    /// to its own cell (and so a test can assert the governed argv shape).
    dispatched_args: Mutex<Vec<Vec<String>>>,
    /// How many reads were served, including failures.
    reads: Mutex<usize>,
}

impl FakeDisplayTransport {
    /// A fake on `backend` with nothing scripted: the modelled brightness is
    /// **unknown**, so a read fails closed until [`Self::read_ok`] is called.
    #[must_use]
    pub fn new(backend: BrightnessBackend) -> Self {
        Self {
            backend,
            scripted: Mutex::new(VecDeque::new()),
            cell: Mutex::new(None),
            read_failure: None,
            outcome: Mutex::new(None),
            dispatch_fault: None,
            dispatched: Mutex::new(Vec::new()),
            dispatched_args: Mutex::new(Vec::new()),
            reads: Mutex::new(0),
        }
    }

    /// Builder: queue the next read as `percent`. `0` is a real, observed
    /// brightness — use [`Self::read_unknown`] for "could not be read".
    #[must_use]
    pub fn read_ok(self, percent: u8) -> Self {
        self.scripted
            .lock()
            .expect("scripted mutex")
            .push_back(ScriptedRead::Percent(percent.min(100)));
        self
    }

    /// Builder: queue the next read as *indeterminate*. Distinct from
    /// `read_ok(0)`: the observation never happened, so verification must not
    /// be allowed to compare against it.
    #[must_use]
    pub fn read_unknown(self, reason: impl Into<String>) -> Self {
        self.scripted
            .lock()
            .expect("scripted mutex")
            .push_back(ScriptedRead::Unknown(reason.into()));
        self
    }

    /// Builder: make **every** read fail, proving an ambiguous parse never
    /// becomes a fabricated state.
    #[must_use]
    pub fn read_failure(mut self, reason: impl Into<String>) -> Self {
        self.read_failure = Some(reason.into());
        self
    }

    /// Builder: seed the modelled backlight without consuming a scripted read
    /// (the state the display was already in before this test).
    #[must_use]
    pub fn with_brightness(self, percent: u8) -> Self {
        *self.cell.lock().expect("cell mutex") = Some(percent.min(100));
        self
    }

    /// Builder: script the outcome `dispatch` returns.
    #[must_use]
    pub fn dispatch_outcome(self, outcome: ApplyOutcome) -> Self {
        *self.outcome.lock().expect("outcome mutex") = Some(outcome);
        self
    }

    /// Builder: make `dispatch` fail with `fault` and apply no effect, so the
    /// cell is left exactly as it was.
    #[must_use]
    pub fn dispatch_fault(mut self, fault: FakeDisplayFault) -> Self {
        self.dispatch_fault = Some(fault);
        self
    }

    /// The redacted, digest-only projections of the commands this fake captured
    /// instead of executing, in order. This is the only "process" evidence a
    /// display test ever sees.
    #[must_use]
    pub fn captured(&self) -> Vec<StructuredCommandSummary> {
        self.dispatched.lock().expect("dispatch mutex").clone()
    }

    /// The exact argv of each captured dispatch, in order.
    #[must_use]
    pub fn captured_args(&self) -> Vec<Vec<String>> {
        self.dispatched_args.lock().expect("argv mutex").clone()
    }

    /// How many dispatches were requested.
    #[must_use]
    pub fn dispatch_count(&self) -> usize {
        self.dispatched.lock().expect("dispatch mutex").len()
    }

    /// How many reads were served (successful or not).
    #[must_use]
    pub fn read_count(&self) -> usize {
        *self.reads.lock().expect("reads mutex")
    }

    /// The modelled brightness. `None` means genuinely unknown — never `0`.
    #[must_use]
    pub fn brightness(&self) -> Option<u8> {
        *self.cell.lock().expect("cell mutex")
    }

    fn unavailable(&self, reason: impl Into<String>, retryable: bool) -> OsControlError {
        OsControlError::Unavailable {
            provider: Some(ProviderId::new(FAKE_DISPLAY_PROVIDER_ID)),
            reason: SafeText::new(reason.into()),
            retryable,
        }
    }

    /// Serve one read against the scripted queue and the modelled cell. Kept
    /// free of [`HostExecutionContext`] (which the fake ignores) so the
    /// "0 is not unknown" rule is unit-testable without a governed chain.
    fn serve_read(&self) -> Result<u8, OsControlError> {
        *self.reads.lock().expect("reads mutex") += 1;

        if let Some(reason) = &self.read_failure {
            return Err(self.unavailable(reason.clone(), true));
        }

        // A scripted read always wins, so a test can drive a TOCTOU change or a
        // post-apply contradiction the model would not produce on its own.
        let next = self.scripted.lock().expect("scripted mutex").pop_front();
        match next {
            Some(ScriptedRead::Percent(percent)) => {
                *self.cell.lock().expect("cell mutex") = Some(percent);
                Ok(percent)
            }
            // An indeterminate read must NOT touch the cell: an unknown reading
            // is not evidence, and overwriting the last known value with it
            // would quietly fabricate state for the *next* read.
            Some(ScriptedRead::Unknown(reason)) => Err(self.unavailable(reason, false)),
            // Queue drained: serve the modelled backlight, which `dispatch`
            // keeps current. Unknown stays an error, never a default of 0.
            None => self.brightness().ok_or_else(|| {
                self.unavailable("no display brightness scripted on the fake transport", false)
            }),
        }
    }

    /// Recover the percentage a governed argv would actually have applied, per
    /// backend. `None` when the argv cannot be interpreted — in which case the
    /// modelled brightness becomes *unknown* rather than optimistically
    /// assuming the request landed.
    fn applied_percent(backend: BrightnessBackend, args: &[String]) -> Option<u8> {
        match backend {
            // ["call", "--system", …, "SetBrightness", "ssu", "backlight",
            //  <device>, <raw value>]
            //
            // The raw value is in the DEVICE's units, so a percentage cannot be
            // recovered from argv alone. The fake models the documented test
            // maximum of 100 so a scaled value round-trips; a value above that is
            // reported as unknown rather than clamped, because clamping would hide
            // a scaling bug.
            BrightnessBackend::LogindSession => args
                .last()
                .and_then(|value| value.parse::<u32>().ok())
                .and_then(|raw| u8::try_from(raw).ok()),
            // ["set", "75%"]
            BrightnessBackend::Brightnessctl => args
                .iter()
                .find_map(|arg| arg.strip_suffix('%'))
                .and_then(|value| value.parse::<u8>().ok()),
            // [..., "Brightness", "<int32 75>"]
            BrightnessBackend::GnomeSettingsDaemon => args.iter().find_map(|arg| {
                arg.strip_prefix("<int32 ")
                    .and_then(|rest| rest.strip_suffix('>'))
                    .and_then(|value| value.trim().parse::<u8>().ok())
            }),
            // ["--output", <connector>, "--brightness", "0.75"]
            BrightnessBackend::XrandrGamma => args
                .iter()
                .position(|arg| arg == "--brightness")
                .and_then(|index| args.get(index + 1))
                .and_then(|value| value.parse::<f64>().ok())
                .map(|fraction| (fraction * 100.0).round().clamp(0.0, 100.0) as u8),
        }
    }
}

#[async_trait]
impl DisplayTransport for FakeDisplayTransport {
    fn provider_id(&self) -> ProviderId {
        ProviderId::new(FAKE_DISPLAY_PROVIDER_ID)
    }

    fn selected_backend(&self) -> BrightnessBackend {
        self.backend
    }

    async fn read_brightness(&self, _ctx: &HostExecutionContext) -> Result<u8, OsControlError> {
        self.serve_read()
    }

    async fn dispatch(
        &self,
        _ctx: &AdmittedMutationContext<'_>,
        request: &StructuredCommandRequest,
    ) -> Result<ApplyOutcome, OsControlError> {
        // Recorded, never executed: no child process is spawned.
        self.dispatched
            .lock()
            .expect("dispatch mutex")
            .push(request.safe_summary());
        self.dispatched_args
            .lock()
            .expect("argv mutex")
            .push(request.args().to_vec());

        if let Some(fault) = self.dispatch_fault {
            // A refused write changes nothing, so the cell is left untouched.
            return Err(match fault {
                FakeDisplayFault::PermissionDenied => OsControlError::PermissionDenied {
                    authority: SafeText::new("fake-display-authority"),
                    remediation: SafeText::new("grant backlight write access"),
                },
                FakeDisplayFault::Unavailable => {
                    self.unavailable("fake display backend absent", false)
                }
            });
        }

        // Apply the effect to the model. An argv the fake cannot interpret
        // leaves the brightness genuinely unknown rather than assuming success.
        let applied = Self::applied_percent(self.backend, request.args());
        *self.cell.lock().expect("cell mutex") = applied;

        if let Some(outcome) = self.outcome.lock().expect("outcome mutex").clone() {
            return Ok(outcome);
        }
        Ok(ApplyOutcome::Applied(AppliedDispatch::new(
            Some(Digest::of_str(crate::os_control::testing::FAKE_RECEIPT_TAG)),
            BoundedVec::new(),
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_unscripted_fake_is_unknown_not_dark() {
        let unknown = FakeDisplayTransport::new(BrightnessBackend::Brightnessctl);
        assert_eq!(unknown.brightness(), None, "unknown, and not 0");
        assert!(matches!(
            unknown.serve_read(),
            Err(OsControlError::Unavailable { .. })
        ));
    }

    #[test]
    fn zero_is_a_real_brightness_and_reads_as_zero() {
        let at_zero = FakeDisplayTransport::new(BrightnessBackend::Brightnessctl).read_ok(0);
        assert_eq!(at_zero.serve_read().unwrap(), 0);
        assert_eq!(at_zero.brightness(), Some(0));
    }

    #[test]
    fn an_unknown_read_errors_and_does_not_overwrite_the_last_known_value() {
        let transport = FakeDisplayTransport::new(BrightnessBackend::Brightnessctl)
            .read_ok(40)
            .read_unknown("ambiguous brightnessctl output");

        assert_eq!(transport.serve_read().unwrap(), 40);
        assert!(matches!(
            transport.serve_read(),
            Err(OsControlError::Unavailable { .. })
        ));
        // An unknown reading is not evidence: it neither becomes 0 nor erases
        // the last real reading.
        assert_eq!(transport.brightness(), Some(40));
        assert_eq!(transport.read_count(), 2);
    }

    #[test]
    fn a_sticky_read_failure_never_degrades_into_a_value() {
        let transport = FakeDisplayTransport::new(BrightnessBackend::Brightnessctl)
            .with_brightness(55)
            .read_failure("backlight bus wedged");
        assert!(matches!(
            transport.serve_read(),
            Err(OsControlError::Unavailable { .. })
        ));
    }

    #[test]
    fn applied_percent_is_recovered_from_each_backend_argv() {
        use crate::os_control::display::selection::set_brightness_argv;

        for backend in BrightnessBackend::PREFERENCE {
            // logind is the one backend whose argv cannot be built from a
            // percentage: `SetBrightness` takes a raw device value, so the device
            // and its maximum must be resolved first. `set_brightness_argv`
            // therefore yields nothing for it BY DESIGN, and the round-trip is
            // asserted against `logind_set_brightness_argv` instead.
            if backend == BrightnessBackend::LogindSession {
                assert!(
                    set_brightness_argv(backend, 75).is_empty(),
                    "logind must not produce an argv without a resolved device"
                );
                // With a maximum of 100 the raw value equals the percentage.
                let argv =
                    crate::os_control::display::selection::logind_set_brightness_argv(
                        "test0", 100, 75,
                    );
                assert_eq!(
                    FakeDisplayTransport::applied_percent(backend, &argv),
                    Some(75),
                    "logind argv must round-trip through the fake"
                );
                continue;
            }
            let argv = set_brightness_argv(backend, 75);
            assert_eq!(
                FakeDisplayTransport::applied_percent(backend, &argv),
                Some(75),
                "{backend:?} argv must be interpretable by the fake"
            );
            let dark = set_brightness_argv(backend, 0);
            assert_eq!(
                FakeDisplayTransport::applied_percent(backend, &dark),
                Some(0),
                "{backend:?} must round-trip a 0 % (valid) brightness"
            );
        }
    }

    #[test]
    fn an_uninterpretable_argv_leaves_brightness_unknown_not_assumed() {
        assert_eq!(
            FakeDisplayTransport::applied_percent(
                BrightnessBackend::Brightnessctl,
                &["set".to_string(), "max".to_string()]
            ),
            None
        );
    }
}
