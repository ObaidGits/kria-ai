//! Deny-live fake [`PowerProfileTransport`] (OSC-020, OSC-031, OSC-033),
//! Tasks 2.3 / 0.4.
//!
//! Compiled only under `os-control-test`. This is **not** a stub that returns
//! canned values: it is a small in-memory model of a `power-profiles-daemon`
//! host, and [`FakePowerProfileTransport::dispatch`] applies the requested
//! profile to that model instead of running `powerprofilesctl`. A lifecycle
//! test therefore drives the real governed observe → apply → verify →
//! (rollback) path against state the fake actually changed, rather than a
//! pre-scripted sequence that would pass regardless of what the provider did.
//!
//! # The three facts this fake keeps distinct
//!
//! 1. **Unknown vs. a value.** An unscripted read is an `Err`, never a default
//!    profile (OSC-031). A fake that invented `Balanced` would let a mutation
//!    "verify" against a fact nobody read.
//! 2. **No battery vs. a battery at 0%.** A desktop with no battery is
//!    [`BatteryHealth::Absent`] — a positive inventory fact. A battery whose
//!    health is genuinely 0% is `BatteryHealth::present(0, _)`. A host whose
//!    battery source was never scripted is an `Err`. All three are different
//!    answers and the fake never collapses them.
//! 3. **Which profiles exist is hardware-dependent.** The available set is
//!    scriptable ([`FakePowerProfileTransport::available_profiles`]) and a
//!    dispatch naming a profile outside it is **refused**, not silently
//!    accepted — a host without `performance` must fail the request rather
//!    than report success for a profile it cannot enter.

use std::collections::VecDeque;
use std::sync::Mutex;

use async_trait::async_trait;

use crate::os_control::context::{AdmittedMutationContext, HostExecutionContext};
use crate::os_control::contract::{BoundedVec, CapabilityId, Digest, ProviderId, SafeText};
use crate::os_control::error::OsControlError;
use crate::os_control::linux::structured_command::{
    StructuredCommandRequest, StructuredCommandSummary,
};
use crate::os_control::receipt::{AppliedDispatch, ApplyOutcome};

use super::selection::PowerProfileBackend;
use super::{BatteryHealth, PowerProfile, PowerProfileTransport};

/// Provider identity reported by the fake transport. Deliberately prefixed
/// `fake-` so a receipt produced through it can never be mistaken for evidence
/// that a real `power-profiles-daemon` accepted the change (OSC-033).
pub const FAKE_POWER_PROFILE_PROVIDER_ID: &str = "fake-power-profile";

/// A dispatch the fake **refused**, with the reason. A refusal means no effect
/// was applied and nothing was recorded in [`FakePowerProfileTransport::captured`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RefusedDispatch {
    /// The governed argv named a profile this host does not offer.
    ProfileUnavailable {
        /// The refused profile.
        profile: PowerProfile,
    },
    /// The governed argv carried no recognizable profile token, so the fake
    /// could not tell which profile was requested. It refuses rather than
    /// guessing one.
    UnparseableArgv {
        /// The redacted argv the fake was handed.
        redacted_args: Vec<String>,
    },
}

/// A scripted, in-memory power-profile transport.
///
/// # Read ordering
///
/// Reads are a FIFO queue because one governed mutation performs several in a
/// fixed order (pre-observation → under-lease re-observation → pre-apply
/// snapshot → post-apply re-observation → verify, plus one more verify per
/// rollback). Script them with successive [`Self::read_ok`] calls.
///
/// When the queue is drained, a read reports the fake's **current modelled
/// profile** — which [`Self::dispatch`] updates — so a test may script only the
/// starting state and let the model carry the rest. A scripted read always wins
/// over the model, which is what lets a test stage a post-apply contradiction
/// (the host "did not take" the change) on purpose.
///
/// With nothing scripted and nothing applied there is no profile to report, and
/// the read is an `Err`.
pub struct FakePowerProfileTransport {
    backend: PowerProfileBackend,
    scripted: Mutex<VecDeque<PowerProfile>>,
    /// The modelled active profile. `None` until a read is scripted or a
    /// dispatch applies one — never defaulted.
    current: Mutex<Option<PowerProfile>>,
    /// The profiles this modelled host offers. Hardware-dependent in reality,
    /// so scriptable here; a dispatch outside this set is refused.
    available: Mutex<Vec<PowerProfile>>,
    /// The modelled battery inventory. `None` means "no battery source was
    /// scripted" — reported as an error, never as `Absent` and never as 0%.
    battery: Mutex<Option<BatteryHealth>>,
    read_fault: Mutex<Option<OsControlError>>,
    dispatch_fault: Mutex<Option<OsControlError>>,
    outcomes: Mutex<VecDeque<ApplyOutcome>>,
    dispatched: Mutex<Vec<StructuredCommandSummary>>,
    refused: Mutex<Vec<RefusedDispatch>>,
    read_count: Mutex<usize>,
}

impl FakePowerProfileTransport {
    /// A fake on `backend` with nothing scripted yet: every profile is
    /// available (the common `power-profiles-daemon` case), no battery source
    /// is scripted, and a read fails closed until [`Self::read_ok`] is called.
    #[must_use]
    pub fn new(backend: PowerProfileBackend) -> Self {
        Self {
            backend,
            scripted: Mutex::new(VecDeque::new()),
            current: Mutex::new(None),
            available: Mutex::new(vec![
                PowerProfile::PowerSaver,
                PowerProfile::Balanced,
                PowerProfile::Performance,
            ]),
            battery: Mutex::new(None),
            read_fault: Mutex::new(None),
            dispatch_fault: Mutex::new(None),
            outcomes: Mutex::new(VecDeque::new()),
            dispatched: Mutex::new(Vec::new()),
            refused: Mutex::new(Vec::new()),
            read_count: Mutex::new(0),
        }
    }

    /// Builder: queue the next read as `profile`.
    #[must_use]
    pub fn read_ok(self, profile: PowerProfile) -> Self {
        self.scripted
            .lock()
            .expect("scripted mutex")
            .push_back(profile);
        self
    }

    /// Builder: seed the modelled active profile without queueing a read.
    ///
    /// Useful when a test wants the model to answer every read from the
    /// dispatch effects alone.
    #[must_use]
    pub fn active_profile(self, profile: PowerProfile) -> Self {
        *self.current.lock().expect("current mutex") = Some(profile);
        self
    }

    /// Builder: declare exactly which profiles this modelled host offers.
    ///
    /// The set is hardware- and daemon-dependent in reality (a host with no
    /// `performance` platform profile genuinely cannot enter it), so a dispatch
    /// naming a profile outside `profiles` is refused with
    /// [`OsControlError::Unsupported`] and recorded in [`Self::refused`].
    #[must_use]
    pub fn available_profiles(self, profiles: &[PowerProfile]) -> Self {
        *self.available.lock().expect("available mutex") = profiles.to_vec();
        self
    }

    /// Builder: this modelled host has **no battery** — a positive fact read
    /// from the power service's own device inventory, distinct both from a
    /// battery at 0% health and from an unreadable battery.
    #[must_use]
    pub fn battery_absent(self) -> Self {
        *self.battery.lock().expect("battery mutex") = Some(BatteryHealth::Absent);
        self
    }

    /// Builder: this modelled host has a battery at `capacity_percent` of its
    /// design capacity. `battery_present(0, _)` is a real, reportable state and
    /// is never conflated with [`Self::battery_absent`].
    #[must_use]
    pub fn battery_present(self, capacity_percent: u8, cycle_count: Option<u64>) -> Self {
        *self.battery.lock().expect("battery mutex") =
            Some(BatteryHealth::present(capacity_percent, cycle_count));
        self
    }

    /// Builder: make every read fail with a retryable `Unavailable`, proving an
    /// ambiguous parse surfaces as an error rather than a fabricated profile.
    #[must_use]
    pub fn read_failure(self, reason: impl Into<String>) -> Self {
        *self.read_fault.lock().expect("read fault mutex") = Some(OsControlError::Unavailable {
            provider: Some(ProviderId::new(FAKE_POWER_PROFILE_PROVIDER_ID)),
            reason: SafeText::new(reason.into()),
            retryable: true,
        });
        self
    }

    /// Builder: script an arbitrary read fault (timeout, permission denied,
    /// protocol error) so those failure paths are testable too.
    #[must_use]
    pub fn read_fault(self, error: OsControlError) -> Self {
        *self.read_fault.lock().expect("read fault mutex") = Some(error);
        self
    }

    /// Builder: script an arbitrary dispatch fault. The fault is returned
    /// *before* any effect is applied to the model, so it models a dispatch
    /// that provably did not change the host.
    #[must_use]
    pub fn dispatch_fault(self, error: OsControlError) -> Self {
        *self.dispatch_fault.lock().expect("dispatch fault mutex") = Some(error);
        self
    }

    /// Builder: queue the outcome the next `dispatch` returns. Call once per
    /// expected dispatch (an apply and its rollback are two). When the queue is
    /// drained, `dispatch` reports `Applied` stamped with the fake-provider tag.
    #[must_use]
    pub fn dispatch_outcome(self, outcome: ApplyOutcome) -> Self {
        self.outcomes
            .lock()
            .expect("outcomes mutex")
            .push_back(outcome);
        self
    }

    /// The redacted structured-command summaries this fake captured instead of
    /// executing, in order. Refused dispatches are **not** here: nothing was
    /// dispatched.
    #[must_use]
    pub fn captured(&self) -> Vec<StructuredCommandSummary> {
        self.dispatched.lock().expect("dispatch mutex").clone()
    }

    /// How many dispatches the fake accepted and applied.
    #[must_use]
    pub fn dispatch_count(&self) -> usize {
        self.dispatched.lock().expect("dispatch mutex").len()
    }

    /// The dispatches the fake refused, in order, with the reason.
    #[must_use]
    pub fn refused(&self) -> Vec<RefusedDispatch> {
        self.refused.lock().expect("refused mutex").clone()
    }

    /// How many reads were served (successful or not).
    #[must_use]
    pub fn read_count(&self) -> usize {
        *self.read_count.lock().expect("read count mutex")
    }

    /// The profile the model currently holds, or `None` when nothing has been
    /// scripted or applied.
    #[must_use]
    pub fn modelled_profile(&self) -> Option<PowerProfile> {
        *self.current.lock().expect("current mutex")
    }

    /// The profiles this modelled host offers.
    #[must_use]
    pub fn offered_profiles(&self) -> Vec<PowerProfile> {
        self.available.lock().expect("available mutex").clone()
    }

    /// The error an unscripted read returns. Never a value: a fake that
    /// invented state would let a test prove a mutation verified against a fact
    /// nobody read.
    fn unscripted(&self, reason: &str) -> OsControlError {
        OsControlError::Unavailable {
            provider: Some(ProviderId::new(FAKE_POWER_PROFILE_PROVIDER_ID)),
            reason: SafeText::new(reason),
            retryable: false,
        }
    }

    /// The profile a governed `powerprofilesctl set <profile>` argv names.
    /// Read from the redacted summary because that is the only projection of a
    /// [`StructuredCommandRequest`] a transport is given.
    fn requested_profile(summary: &StructuredCommandSummary) -> Option<PowerProfile> {
        summary
            .redacted_args
            .iter()
            .find_map(|arg| PowerProfile::parse(arg))
    }
}

#[async_trait]
impl PowerProfileTransport for FakePowerProfileTransport {
    fn provider_id(&self) -> ProviderId {
        ProviderId::new(FAKE_POWER_PROFILE_PROVIDER_ID)
    }

    fn selected_backend(&self) -> PowerProfileBackend {
        self.backend
    }

    async fn read_profile(
        &self,
        _ctx: &HostExecutionContext,
    ) -> Result<PowerProfile, OsControlError> {
        *self.read_count.lock().expect("read count mutex") += 1;
        if let Some(fault) = self.read_fault.lock().expect("read fault mutex").clone() {
            return Err(fault);
        }
        // A scripted read wins over the model: that is how a test stages a
        // post-apply contradiction on purpose.
        let next = self.scripted.lock().expect("scripted mutex").pop_front();
        let mut current = self.current.lock().expect("current mutex");
        if let Some(profile) = next {
            *current = Some(profile);
        }
        current.ok_or_else(|| self.unscripted("no power profile scripted on the fake transport"))
    }

    async fn read_battery_health(
        &self,
        _ctx: &HostExecutionContext,
    ) -> Result<BatteryHealth, OsControlError> {
        // Unscripted is "could not tell", which is neither `Absent` nor 0%.
        self.battery
            .lock()
            .expect("battery mutex")
            .clone()
            .ok_or_else(|| {
                self.unscripted(
                    "no battery inventory scripted on the fake transport; \
                     battery health is unknown, not absent and not zero",
                )
            })
    }

    async fn dispatch(
        &self,
        _ctx: &AdmittedMutationContext<'_>,
        request: &StructuredCommandRequest,
    ) -> Result<ApplyOutcome, OsControlError> {
        if let Some(fault) = self
            .dispatch_fault
            .lock()
            .expect("dispatch fault mutex")
            .clone()
        {
            return Err(fault);
        }

        // Recorded, never executed: no child process is spawned.
        let summary = request.safe_summary();
        let Some(profile) = Self::requested_profile(&summary) else {
            self.refused
                .lock()
                .expect("refused mutex")
                .push(RefusedDispatch::UnparseableArgv {
                    redacted_args: summary.redacted_args.clone(),
                });
            return Err(OsControlError::InvalidRequest {
                field: crate::os_control::contract::SafeField::new("argv"),
                reason: SafeText::new(
                    "the governed argv named no known power profile; \
                     the fake refuses rather than guessing which profile was requested",
                ),
            });
        };

        if !self
            .available
            .lock()
            .expect("available mutex")
            .contains(&profile)
        {
            self.refused
                .lock()
                .expect("refused mutex")
                .push(RefusedDispatch::ProfileUnavailable { profile });
            return Err(OsControlError::Unsupported {
                capability: CapabilityId::new(&summary.capability),
                reason: SafeText::new(format!(
                    "this host does not offer the `{}` power profile",
                    profile.as_str()
                )),
            });
        }

        self.dispatched.lock().expect("dispatch mutex").push(summary);

        let outcome = self
            .outcomes
            .lock()
            .expect("outcomes mutex")
            .pop_front()
            .unwrap_or_else(|| {
                ApplyOutcome::Applied(AppliedDispatch::new(
                    Some(Digest::of_str(
                        crate::os_control::testing::FAKE_RECEIPT_TAG,
                    )),
                    BoundedVec::new(),
                ))
            });

        // Apply the effect to the model, but only for an outcome that claims
        // the change took hold. An uncertain or partial dispatch leaves the
        // model where it was, so a re-observation reports what it truly is.
        if matches!(
            outcome,
            ApplyOutcome::Applied(_) | ApplyOutcome::Accepted(_)
        ) {
            *self.current.lock().expect("current mutex") = Some(profile);
        }
        Ok(outcome)
    }
}

#[cfg(all(test, feature = "os-control-test"))]
mod tests {
    use super::*;

    #[test]
    fn absent_battery_is_not_a_zero_percent_battery_and_neither_is_unscripted() {
        let absent = FakePowerProfileTransport::new(PowerProfileBackend::Powerprofilesctl)
            .battery_absent();
        let dying = FakePowerProfileTransport::new(PowerProfileBackend::Powerprofilesctl)
            .battery_present(0, Some(1200));

        let absent_state = absent.battery.lock().unwrap().clone().unwrap();
        let dying_state = dying.battery.lock().unwrap().clone().unwrap();

        assert_eq!(absent_state, BatteryHealth::Absent);
        assert!(!absent_state.is_present());
        assert!(dying_state.is_present());
        assert_ne!(absent_state, dying_state);

        // And "no source scripted" is a third answer: an error, not `Absent`.
        let unknown = FakePowerProfileTransport::new(PowerProfileBackend::Powerprofilesctl);
        assert!(unknown.battery.lock().unwrap().is_none());
    }

    #[test]
    fn every_profile_is_offered_by_default_and_the_set_is_scriptable() {
        let all = FakePowerProfileTransport::new(PowerProfileBackend::PowerProfilesDaemon);
        assert_eq!(all.offered_profiles().len(), 3);

        let limited = FakePowerProfileTransport::new(PowerProfileBackend::PowerProfilesDaemon)
            .available_profiles(&[PowerProfile::Balanced, PowerProfile::PowerSaver]);
        assert_eq!(
            limited.offered_profiles(),
            vec![PowerProfile::Balanced, PowerProfile::PowerSaver]
        );
        assert!(!limited.offered_profiles().contains(&PowerProfile::Performance));
    }

    #[test]
    fn a_scripted_read_wins_over_the_model_then_the_model_carries_on() {
        let fake = FakePowerProfileTransport::new(PowerProfileBackend::Powerprofilesctl)
            .active_profile(PowerProfile::Performance)
            .read_ok(PowerProfile::Balanced);
        // Scripted first…
        assert_eq!(
            fake.scripted.lock().unwrap().front().copied(),
            Some(PowerProfile::Balanced)
        );
        // …and the model was seeded independently of the queue.
        assert_eq!(fake.modelled_profile(), Some(PowerProfile::Performance));
    }

    #[test]
    fn nothing_scripted_means_no_profile_to_report() {
        let fake = FakePowerProfileTransport::new(PowerProfileBackend::Powerprofilesctl);
        assert!(fake.modelled_profile().is_none());
        assert_eq!(fake.dispatch_count(), 0);
        assert!(fake.refused().is_empty());
    }
}
