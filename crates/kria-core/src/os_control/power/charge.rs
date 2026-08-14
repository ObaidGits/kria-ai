//! Battery charge thresholds, applied through the privileged broker.
//!
//! linux-os-control-production task **5.4** (OSC-020).
//!
//! # Why this goes through the broker
//!
//! Charge thresholds live in `sysfs` under `/sys/class/power_supply/...`, which is
//! root-owned. Writing them needs privilege, and this architecture never escalates
//! in-process: the request becomes a typed
//! [`BrokerOperation::SetBatteryChargeThresholds`] that the small privileged
//! service performs after Polkit authorizes it. Until that service is installed,
//! this reports `Unavailable` — never a silent no-op, and never a `sudo` fallback.
//!
//! # Why the pair is validated together
//!
//! A lower bound at or above the upper bound is rejected by the kernel, so writing
//! them one at a time can leave the machine with only one value applied. The pair
//! is validated up front and the broker writes the upper bound first, so no
//! transient invalid combination exists.

use async_trait::async_trait;

use crate::os_control::broker::client::BrokerTransport;
use crate::os_control::broker::protocol::{BoundedPercent, BrokerOperation, ChargeThresholdAdapterId};
use crate::os_control::context::{AdmittedMutationContext, HostExecutionContext};
use crate::os_control::contract::{
    CapabilityId, ComparatorKind, DesiredStateControl, Digest, OsEvidenceSource, ProviderId,
    SafeErrorCode, SafeText, VerificationReliability,
};
use crate::os_control::error::OsControlError;
use crate::os_control::hardware::validate_charge_thresholds;
use crate::os_control::receipt::{
    ApplyOutcome, RedactedObservation, RollbackToken, SatisfyingVerification,
    VerificationContradiction, VerificationReport,
};
use crate::os_control::runtime::NormalizedObservation;

/// The provider identity.
pub const CHARGE_PROVIDER_ID: &str = "power-charge-thresholds";

/// The observed threshold pair.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChargeThresholdState {
    /// The lower (start-charging) bound, when readable.
    ///
    /// `None` is **unknown**, never zero: a machine whose thresholds cannot be read
    /// must not look like one set to start charging at 0%.
    pub lower: Option<u8>,
    /// The upper (stop-charging) bound, when readable.
    pub upper: Option<u8>,
}

impl NormalizedObservation for ChargeThresholdState {
    fn observation_digest(&self) -> Digest {
        Digest::of_str(&format!(
            "charge:{}:{}",
            self.lower.map_or_else(|| "-".to_string(), |v| v.to_string()),
            self.upper.map_or_else(|| "-".to_string(), |v| v.to_string()),
        ))
    }
}

/// One governed threshold request.
#[derive(Debug, Clone)]
pub struct ChargeThresholdRequest {
    /// The canonical tool/action name.
    pub action: String,
    /// The canonical tool parameters.
    pub params: serde_json::Value,
    /// The validated lower bound.
    pub lower: u8,
    /// The validated upper bound.
    pub upper: u8,
}

impl ChargeThresholdRequest {
    /// Build a request, validating the pair together.
    pub fn new(
        action: impl Into<String>,
        params: serde_json::Value,
        lower: u8,
        upper: u8,
    ) -> Result<Self, OsControlError> {
        let (lower, upper) = validate_charge_thresholds(lower, upper)?;
        Ok(Self {
            action: action.into(),
            params,
            lower,
            upper,
        })
    }

    /// The state this request is trying to reach.
    #[must_use]
    pub fn desired_state(&self) -> ChargeThresholdState {
        ChargeThresholdState {
            lower: Some(self.lower),
            upper: Some(self.upper),
        }
    }

    /// The comparator.
    #[must_use]
    pub fn comparator(&self) -> ComparatorKind {
        ComparatorKind::Exact
    }
}

/// The raw transport.
#[async_trait]
pub trait ChargeThresholdTransport: Send + Sync {
    /// The provider identity.
    fn provider_id(&self) -> ProviderId;

    /// The recognized adapter this host uses.
    async fn adapter(
        &self,
        ctx: &HostExecutionContext,
    ) -> Result<ChargeThresholdAdapterId, OsControlError>;

    /// Read the current pair.
    async fn read_thresholds(
        &self,
        ctx: &HostExecutionContext,
    ) -> Result<ChargeThresholdState, OsControlError>;

    /// Apply the pair through the broker.
    async fn dispatch(
        &self,
        ctx: &AdmittedMutationContext<'_>,
        adapter: ChargeThresholdAdapterId,
        lower: u8,
        upper: u8,
    ) -> Result<ApplyOutcome, OsControlError>;
}

/// The governed provider.
pub struct ChargeThresholdControl<T: ChargeThresholdTransport> {
    transport: T,
}

impl<T: ChargeThresholdTransport> ChargeThresholdControl<T> {
    /// Compose over a transport.
    #[must_use]
    pub fn new(transport: T) -> Self {
        Self { transport }
    }

    /// Borrow the transport.
    #[must_use]
    pub fn transport(&self) -> &T {
        &self.transport
    }

    fn satisfying(
        &self,
        observed: &ChargeThresholdState,
    ) -> SatisfyingVerification<ChargeThresholdState> {
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
impl<T: ChargeThresholdTransport>
    DesiredStateControl<ChargeThresholdRequest, ChargeThresholdState> for ChargeThresholdControl<T>
{
    async fn observe(
        &self,
        ctx: &HostExecutionContext,
        _request: &ChargeThresholdRequest,
    ) -> Result<ChargeThresholdState, OsControlError> {
        self.transport.read_thresholds(ctx).await
    }

    async fn apply(
        &self,
        ctx: &AdmittedMutationContext<'_>,
        request: &ChargeThresholdRequest,
        _desired: &ChargeThresholdState,
    ) -> Result<ApplyOutcome, OsControlError> {
        let adapter = self.transport.adapter(ctx.observation()).await?;
        self.transport
            .dispatch(ctx, adapter, request.lower, request.upper)
            .await
    }

    async fn verify(
        &self,
        ctx: &HostExecutionContext,
        request: &ChargeThresholdRequest,
        desired: &ChargeThresholdState,
    ) -> Result<VerificationReport<ChargeThresholdState>, OsControlError> {
        let observed = self.observe(ctx, request).await?;
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
        _ctx: &AdmittedMutationContext<'_>,
        _token: &RollbackToken,
    ) -> Result<ApplyOutcome, OsControlError> {
        // The contract is `UserRequestable`: the previous pair is not retained, so
        // restoring it is a fresh user-initiated request rather than an automatic
        // compensation.
        Err(OsControlError::Unsupported {
            capability: CapabilityId::new("charge_thresholds.rollback"),
            reason: SafeText::new(
                "the previous threshold pair is not retained; restoring it is a new request",
            ),
        })
    }
}

/// The port a handler resolves.
pub trait ChargeThresholdControlPort:
    DesiredStateControl<ChargeThresholdRequest, ChargeThresholdState>
{
}

impl<T: ChargeThresholdTransport> ChargeThresholdControlPort for ChargeThresholdControl<T> {}

/// The real broker-backed transport.
pub struct RealChargeThresholdTransport<B: BrokerTransport + Send + Sync> {
    broker: B,
}

impl<B: BrokerTransport + Send + Sync> RealChargeThresholdTransport<B> {
    /// Compose over a broker transport.
    #[must_use]
    pub fn new(broker: B) -> Self {
        Self { broker }
    }
}

#[async_trait]
impl<B: BrokerTransport + Send + Sync> ChargeThresholdTransport
    for RealChargeThresholdTransport<B>
{
    fn provider_id(&self) -> ProviderId {
        ProviderId::new(CHARGE_PROVIDER_ID)
    }

    async fn adapter(
        &self,
        _ctx: &HostExecutionContext,
    ) -> Result<ChargeThresholdAdapterId, OsControlError> {
        crate::os_control::access::deny_live_transport(
            crate::os_control::access::RawTransportKind::Process,
        );
        // ThinkPad exposes the ACPI-specific pair; anything else uses the standard
        // sysfs names. The set is closed, so no request can direct a privileged
        // write at an arbitrary node.
        if std::path::Path::new("/sys/devices/platform/thinkpad_acpi").exists() {
            Ok(ChargeThresholdAdapterId::ThinkpadAcpi)
        } else {
            Ok(ChargeThresholdAdapterId::SysfsStandard)
        }
    }

    async fn read_thresholds(
        &self,
        _ctx: &HostExecutionContext,
    ) -> Result<ChargeThresholdState, OsControlError> {
        crate::os_control::access::deny_live_transport(
            crate::os_control::access::RawTransportKind::Process,
        );
        // Reading sysfs needs no privilege. An unreadable value stays `None`
        // (unknown) rather than becoming a number nobody measured.
        let read = |name: &str| -> Option<u8> {
            for battery in ["BAT0", "BAT1"] {
                let path = format!("/sys/class/power_supply/{battery}/{name}");
                if let Ok(text) = std::fs::read_to_string(&path) {
                    if let Ok(value) = text.trim().parse::<u8>() {
                        return Some(value);
                    }
                }
            }
            None
        };
        Ok(ChargeThresholdState {
            lower: read("charge_control_start_threshold"),
            upper: read("charge_control_end_threshold"),
        })
    }

    async fn dispatch(
        &self,
        ctx: &AdmittedMutationContext<'_>,
        adapter: ChargeThresholdAdapterId,
        lower: u8,
        upper: u8,
    ) -> Result<ApplyOutcome, OsControlError> {
        let lower_percent = BoundedPercent::new(lower).map_err(|_| OsControlError::InvalidRequest {
            field: crate::os_control::contract::SafeField::new("lower"),
            reason: SafeText::new("lower is out of range"),
        })?;
        let upper_percent = BoundedPercent::new(upper).map_err(|_| OsControlError::InvalidRequest {
            field: crate::os_control::contract::SafeField::new("upper"),
            reason: SafeText::new("upper is out of range"),
        })?;
        let operation = BrokerOperation::SetBatteryChargeThresholds {
            adapter,
            lower_percent,
            upper_percent,
        };
        let caller = caller_credentials();
        let request = crate::os_control::broker::build_broker_request(
            ctx,
            &caller,
            format!("charge-thresholds-{lower}-{upper}"),
            operation,
        )?;
        crate::os_control::broker::dispatch_via_broker_bound(
            &self.broker,
            &request,
            caller.connection_nonce.as_str(),
        )
    }
}

/// The broker caller's own local peer credentials.
fn caller_credentials() -> crate::os_control::broker::PeerCredentials {
    crate::os_control::broker::PeerCredentials {
        // SAFETY: these libc calls always succeed and have no error condition.
        uid: unsafe { libc::getuid() },
        gid: unsafe { libc::getgid() },
        pid: unsafe { libc::getpid() },
        connection_nonce: format!("charge-{}", uuid::Uuid::new_v4()),
    }
}

#[cfg(all(test, feature = "os-control-test"))]
mod tests {
    use super::*;

    #[test]
    fn the_pair_is_validated_on_construction() {
        // An inverted pair would be half-applied by the kernel.
        assert!(ChargeThresholdRequest::new("t", serde_json::Value::Null, 80, 75).is_err());
        assert!(ChargeThresholdRequest::new("t", serde_json::Value::Null, 0, 10).is_err());
        let ok = ChargeThresholdRequest::new("t", serde_json::Value::Null, 75, 80).unwrap();
        assert_eq!((ok.lower, ok.upper), (75, 80));
    }

    #[test]
    fn an_unreadable_threshold_is_unknown_not_zero() {
        let unknown = ChargeThresholdState {
            lower: None,
            upper: None,
        };
        let zeroed = ChargeThresholdState {
            lower: Some(0),
            upper: Some(0),
        };
        assert_ne!(
            unknown.observation_digest(),
            zeroed.observation_digest(),
            "'could not read' must not look like 'start charging at 0%'"
        );
    }

    #[test]
    fn desired_state_carries_the_validated_pair() {
        let request = ChargeThresholdRequest::new("t", serde_json::Value::Null, 60, 85).unwrap();
        let desired = request.desired_state();
        assert_eq!(desired.lower, Some(60));
        assert_eq!(desired.upper, Some(85));
    }
}
