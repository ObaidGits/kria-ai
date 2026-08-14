//! Display configuration and night light.
//!
//! linux-os-control-production task **5.1** (OSC-019).
//!
//! # Why this is a separate slice from brightness
//!
//! Brightness is a single scalar with a trivial postcondition. A **monitor
//! configuration** is not: applying one can leave the user staring at a black
//! screen with no way to click anything, which makes it the most dangerous
//! non-destructive operation in the whole system.
//!
//! # The revert is the compositor's, not ours
//!
//! GNOME's `org.gnome.Mutter.DisplayConfig.ApplyMonitorsConfig` takes a *method*
//! argument. Applied as **temporary**, the compositor itself reverts the layout
//! after a short timeout unless `ConfirmDisplayChange` arrives first.
//!
//! Delegating to that is deliberate and much safer than implementing our own
//! timer:
//!
//! * a timer inside KRIA dies with the KRIA process, and a crash mid-apply would
//!   leave the user permanently on a broken layout;
//! * the compositor's revert runs even if KRIA is gone, because the compositor is
//!   the thing that owns the display.
//!
//! So `set_display_configuration` always applies **temporary**, and
//! `confirm_display_configuration` is the separate, deliberate second step. A
//! caller that never confirms loses nothing: the screen comes back on its own.
//!
//! # Night light
//!
//! A colour-temperature shift with no safety hazard, but its state must still
//! fail closed: an unreadable current setting is *unknown*, never "off". Reporting
//! "off" would let an enable request verify as already satisfied and do nothing.

use async_trait::async_trait;

use crate::os_control::context::{AdmittedMutationContext, HostExecutionContext};
use crate::os_control::contract::{
    CapabilityId, ComparatorKind, DesiredStateControl, Digest, OsEvidenceSource, ProviderId,
    SafeErrorCode, SafeField, SafeText, VerificationReliability,
};
use crate::os_control::error::OsControlError;
use crate::os_control::receipt::{
    ApplyOutcome, RedactedObservation, RollbackToken, SatisfyingVerification,
    VerificationContradiction, VerificationReport,
};
use crate::os_control::runtime::NormalizedObservation;

/// The provider identity for this slice.
pub const DISPLAY_CONFIG_PROVIDER_ID: &str = "display-configuration";

/// Which fact an observation carries. Part of the digest, so a night-light fact
/// can never satisfy a configuration postcondition.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DisplayConfigFocus {
    /// The night-light enabled state.
    NightLight,
    /// The applied monitor configuration.
    Configuration,
    /// Whether a temporary configuration is awaiting confirmation.
    PendingConfirmation,
}

impl DisplayConfigFocus {
    fn tag(self) -> &'static str {
        match self {
            Self::NightLight => "night-light",
            Self::Configuration => "configuration",
            Self::PendingConfirmation => "pending-confirmation",
        }
    }
}

/// A normalized observation of this slice.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DisplayConfigState {
    /// Which fact this carries.
    pub focus: DisplayConfigFocus,
    /// Night light on/off, when that is the focus.
    pub night_light: Option<bool>,
    /// The serial of the applied monitor configuration, when known.
    pub config_serial: Option<u32>,
    /// Whether a temporary configuration is awaiting confirmation.
    pub awaiting_confirmation: bool,
}

impl NormalizedObservation for DisplayConfigState {
    fn observation_digest(&self) -> Digest {
        Digest::of_str(&format!(
            "display-config:{}:{}:{}:{}",
            self.focus.tag(),
            self.night_light
                .map_or_else(|| "-".to_string(), |v| v.to_string()),
            self.config_serial
                .map_or_else(|| "-".to_string(), |v| v.to_string()),
            self.awaiting_confirmation,
        ))
    }
}

/// A monitor layout, as a reviewed opaque selection.
///
/// Deliberately **not** a free-form structure a model can author: a caller names
/// a layout the compositor already reports as available, identified by its
/// serial. Letting a model compose arbitrary mode/position tuples is how you get
/// an unusable screen.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MonitorLayoutSelection {
    /// The configuration serial the caller read from the compositor.
    pub serial: u32,
    /// The named layout to apply.
    pub layout_id: String,
}

/// What to do.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DisplayConfigOp {
    /// Turn night light on or off.
    SetNightLight(bool),
    /// Apply a monitor layout **temporarily**; the compositor reverts it unless
    /// confirmed.
    ApplyConfiguration(MonitorLayoutSelection),
    /// Confirm the pending temporary configuration, making it permanent.
    ConfirmConfiguration,
}

impl DisplayConfigOp {
    /// The fact this operation is judged against.
    #[must_use]
    pub fn focus(&self) -> DisplayConfigFocus {
        match self {
            Self::SetNightLight(_) => DisplayConfigFocus::NightLight,
            Self::ApplyConfiguration(_) => DisplayConfigFocus::Configuration,
            Self::ConfirmConfiguration => DisplayConfigFocus::PendingConfirmation,
        }
    }
}

/// One governed request.
#[derive(Debug, Clone)]
pub struct DisplayConfigRequest {
    /// The canonical tool/action name.
    pub action: String,
    /// The canonical tool parameters.
    pub params: serde_json::Value,
    /// The operation.
    pub op: DisplayConfigOp,
}

impl DisplayConfigRequest {
    /// The state this request is trying to reach.
    #[must_use]
    pub fn desired_state(&self, observed: &DisplayConfigState) -> DisplayConfigState {
        match &self.op {
            DisplayConfigOp::SetNightLight(enabled) => DisplayConfigState {
                focus: DisplayConfigFocus::NightLight,
                night_light: Some(*enabled),
                config_serial: observed.config_serial,
                awaiting_confirmation: observed.awaiting_confirmation,
            },
            DisplayConfigOp::ApplyConfiguration(selection) => DisplayConfigState {
                focus: DisplayConfigFocus::Configuration,
                night_light: observed.night_light,
                // The compositor bumps the serial when a configuration applies.
                config_serial: Some(selection.serial.saturating_add(1)),
                // Applying temporarily leaves the change AWAITING confirmation —
                // that is the postcondition, not a permanent layout.
                awaiting_confirmation: true,
            },
            DisplayConfigOp::ConfirmConfiguration => DisplayConfigState {
                focus: DisplayConfigFocus::PendingConfirmation,
                night_light: observed.night_light,
                config_serial: observed.config_serial,
                // Confirmation's whole postcondition: nothing is pending anymore.
                awaiting_confirmation: false,
            },
        }
    }

    /// The comparator.
    #[must_use]
    pub fn comparator(&self) -> ComparatorKind {
        ComparatorKind::Exact
    }
}

/// Facts read from the compositor.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DisplayConfigFacts {
    /// Night light enabled, when readable.
    pub night_light: Option<bool>,
    /// The current configuration serial.
    pub config_serial: Option<u32>,
    /// Whether a temporary configuration is awaiting confirmation.
    pub awaiting_confirmation: bool,
}

/// The raw transport.
#[async_trait]
pub trait DisplayConfigTransport: Send + Sync {
    /// The provider identity.
    fn provider_id(&self) -> ProviderId;

    /// Read the current facts. An unreadable night-light setting must surface as
    /// `None`, never as `Some(false)`.
    async fn read_facts(
        &self,
        ctx: &HostExecutionContext,
    ) -> Result<DisplayConfigFacts, OsControlError>;

    /// Apply one operation.
    async fn apply(
        &self,
        ctx: &AdmittedMutationContext<'_>,
        op: &DisplayConfigOp,
    ) -> Result<ApplyOutcome, OsControlError>;
}

/// The governed provider.
pub struct DisplayConfigControl<T: DisplayConfigTransport> {
    transport: T,
}

impl<T: DisplayConfigTransport> DisplayConfigControl<T> {
    /// Compose over a transport.
    #[must_use]
    pub fn new(transport: T) -> Self {
        Self { transport }
    }

    /// Borrow the transport (tests assert against it).
    #[must_use]
    pub fn transport(&self) -> &T {
        &self.transport
    }

    fn state_from(
        &self,
        facts: &DisplayConfigFacts,
        focus: DisplayConfigFocus,
    ) -> DisplayConfigState {
        DisplayConfigState {
            focus,
            night_light: facts.night_light,
            config_serial: facts.config_serial,
            awaiting_confirmation: facts.awaiting_confirmation,
        }
    }

    fn satisfying(
        &self,
        observed: &DisplayConfigState,
    ) -> SatisfyingVerification<DisplayConfigState> {
        let digest = observed.observation_digest();
        SatisfyingVerification::new(
            OsEvidenceSource::AuthoritativeServiceState,
            VerificationReliability::Strong,
            self.transport.provider_id(),
            RedactedObservation::new(observed.clone(), digest),
            None,
            std::time::SystemTime::now(),
            0,
        )
    }
}

#[async_trait]
impl<T: DisplayConfigTransport> DesiredStateControl<DisplayConfigRequest, DisplayConfigState>
    for DisplayConfigControl<T>
{
    async fn observe(
        &self,
        ctx: &HostExecutionContext,
        request: &DisplayConfigRequest,
    ) -> Result<DisplayConfigState, OsControlError> {
        let facts = self.transport.read_facts(ctx).await?;

        // A night-light request against an unreadable setting fails closed here,
        // before a decision is made against a fabricated "off".
        if matches!(request.op, DisplayConfigOp::SetNightLight(_)) && facts.night_light.is_none() {
            return Err(OsControlError::Unavailable {
                provider: Some(self.transport.provider_id()),
                reason: SafeText::new(
                    "the night-light setting could not be read; it is unknown, not off",
                ),
                retryable: true,
            });
        }
        // Confirming when nothing is pending is not a failure to report here — the
        // governed layer sees the postcondition already holds and reports Unchanged.
        Ok(self.state_from(&facts, request.op.focus()))
    }

    async fn apply(
        &self,
        ctx: &AdmittedMutationContext<'_>,
        request: &DisplayConfigRequest,
        _desired: &DisplayConfigState,
    ) -> Result<ApplyOutcome, OsControlError> {
        if matches!(request.op, DisplayConfigOp::ConfirmConfiguration) {
            // Confirming a configuration that is not pending would tell the user a
            // layout is permanent when the compositor is about to revert it.
            let facts = self.transport.read_facts(ctx.observation()).await?;
            if !facts.awaiting_confirmation {
                return Err(OsControlError::InvalidRequest {
                    field: SafeField::new("confirm"),
                    reason: SafeText::new(
                        "no display configuration is awaiting confirmation",
                    ),
                });
            }
        }
        self.transport.apply(ctx, &request.op).await
    }

    async fn verify(
        &self,
        ctx: &HostExecutionContext,
        request: &DisplayConfigRequest,
        desired: &DisplayConfigState,
    ) -> Result<VerificationReport<DisplayConfigState>, OsControlError> {
        let facts = self.transport.read_facts(ctx).await?;
        let observed = self.state_from(&facts, request.op.focus());

        // A configuration apply is verified by the change being PENDING, not by
        // the layout being permanent — the compositor may still revert it, and
        // claiming permanence would be wrong until confirmation lands.
        if observed.observation_digest() == desired.observation_digest() {
            Ok(VerificationReport::Satisfied(self.satisfying(&observed)))
        } else {
            Ok(VerificationReport::Contradicted(
                VerificationContradiction::new(
                    desired.observation_digest(),
                    Some(observed.observation_digest()),
                    SafeErrorCode::from_static("os_control.incident.contradicted"),
                ),
            ))
        }
    }

    async fn rollback(
        &self,
        ctx: &AdmittedMutationContext<'_>,
        _token: &RollbackToken,
    ) -> Result<ApplyOutcome, OsControlError> {
        // Night light is trivially reversible, but a configuration is not rolled
        // back by us: the compositor's own revert is the recovery path, and racing
        // it with a second apply is how you end up with an unusable screen.
        let _ = ctx;
        Err(OsControlError::Unsupported {
            capability: CapabilityId::new("display_configuration.rollback"),
            reason: SafeText::new(
                "a display configuration is recovered by the compositor's own timed revert, not by \
                 a second apply from here",
            ),
        })
    }
}

/// The port a handler resolves.
#[async_trait]
pub trait DisplayConfigControlPort:
    DesiredStateControl<DisplayConfigRequest, DisplayConfigState>
{
    /// Read the current facts.
    async fn facts(
        &self,
        ctx: &HostExecutionContext,
    ) -> Result<DisplayConfigFacts, OsControlError>;
}

#[async_trait]
impl<T: DisplayConfigTransport> DisplayConfigControlPort for DisplayConfigControl<T> {
    async fn facts(
        &self,
        ctx: &HostExecutionContext,
    ) -> Result<DisplayConfigFacts, OsControlError> {
        self.transport.read_facts(ctx).await
    }
}

#[cfg(all(test, feature = "os-control-test"))]
mod tests {
    use super::*;

    fn facts(night: Option<bool>, serial: Option<u32>, pending: bool) -> DisplayConfigFacts {
        DisplayConfigFacts {
            night_light: night,
            config_serial: serial,
            awaiting_confirmation: pending,
        }
    }

    fn state(focus: DisplayConfigFocus, f: &DisplayConfigFacts) -> DisplayConfigState {
        DisplayConfigState {
            focus,
            night_light: f.night_light,
            config_serial: f.config_serial,
            awaiting_confirmation: f.awaiting_confirmation,
        }
    }

    #[test]
    fn focus_is_part_of_the_digest() {
        let f = facts(Some(true), Some(3), false);
        assert_ne!(
            state(DisplayConfigFocus::NightLight, &f).observation_digest(),
            state(DisplayConfigFocus::Configuration, &f).observation_digest(),
            "a night-light fact must not satisfy a configuration postcondition"
        );
    }

    #[test]
    fn applying_a_configuration_desires_pending_not_permanent() {
        let observed = state(DisplayConfigFocus::Configuration, &facts(None, Some(4), false));
        let request = DisplayConfigRequest {
            action: "set_display_configuration".to_string(),
            params: serde_json::Value::Null,
            op: DisplayConfigOp::ApplyConfiguration(MonitorLayoutSelection {
                serial: 4,
                layout_id: "external-only".to_string(),
            }),
        };
        let desired = request.desired_state(&observed);
        assert!(
            desired.awaiting_confirmation,
            "a temporary apply is only verified as PENDING; the compositor may still revert it"
        );
        assert_eq!(desired.config_serial, Some(5));
    }

    #[test]
    fn confirming_desires_nothing_pending() {
        let observed = state(
            DisplayConfigFocus::PendingConfirmation,
            &facts(None, Some(5), true),
        );
        let request = DisplayConfigRequest {
            action: "confirm_display_configuration".to_string(),
            params: serde_json::Value::Null,
            op: DisplayConfigOp::ConfirmConfiguration,
        };
        let desired = request.desired_state(&observed);
        assert!(!desired.awaiting_confirmation);
    }

    #[test]
    fn night_light_desired_state_flips_only_the_night_light() {
        let observed = state(DisplayConfigFocus::NightLight, &facts(Some(false), Some(2), false));
        let request = DisplayConfigRequest {
            action: "set_night_light".to_string(),
            params: serde_json::Value::Null,
            op: DisplayConfigOp::SetNightLight(true),
        };
        let desired = request.desired_state(&observed);
        assert_eq!(desired.night_light, Some(true));
        assert_eq!(
            desired.config_serial, observed.config_serial,
            "an unrelated fact must be carried through unchanged"
        );
    }

    #[test]
    fn an_unknown_night_light_state_is_distinct_from_off() {
        let unknown = state(DisplayConfigFocus::NightLight, &facts(None, None, false));
        let off = state(DisplayConfigFocus::NightLight, &facts(Some(false), None, false));
        assert_ne!(
            unknown.observation_digest(),
            off.observation_digest(),
            "'could not read' must never compare equal to 'off'"
        );
    }
}
