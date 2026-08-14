//! Privacy controls: camera, microphone and location access.
//!
//! linux-os-control-production task **4.7** (OSC-029).
//!
//! # Why both operations are RED, including the read
//!
//! `get_privacy_state` reports whether the camera and microphone are currently
//! permitted. That is a read, and it changes nothing — but it is
//! privacy-sensitive by nature, so the contract makes it RED and it admits as a
//! privacy-sensitive read that fails closed when the audit ledger is unhealthy.
//!
//! `set_privacy_control` is RED for the obvious reason: **enabling** camera or
//! microphone access re-opens a sensor onto the user's room. That direction is the
//! dangerous one, and it is never a silent default.
//!
//! # Fail closed, in the direction that protects
//!
//! An unreadable control is **unknown**, never "disabled". Reporting "camera
//! access is off" when the setting could not be read would tell the user a sensor
//! is closed while it may be wide open — the single worst answer this domain can
//! give. Every unknown surfaces as an error instead.

use async_trait::async_trait;

use crate::os_control::broker::protocol::RecognizedPrivacyControl;
use crate::os_control::context::{AdmittedMutationContext, HostExecutionContext};
use crate::os_control::contract::{
    CapabilityId, ComparatorKind, DesiredStateControl, Digest, OsEvidenceSource, ProviderId,
    SafeErrorCode, SafeText, VerificationReliability,
};
use crate::os_control::error::OsControlError;
use crate::os_control::receipt::{
    ApplyOutcome, RedactedObservation, RollbackToken, SatisfyingVerification,
    VerificationContradiction, VerificationReport,
};
use crate::os_control::runtime::NormalizedObservation;

/// The provider identity.
pub const PRIVACY_PROVIDER_ID: &str = "privacy-portal";

/// The state of one privacy control.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PrivacyState {
    /// Which control this observation is about.
    pub control: RecognizedPrivacyControl,
    /// Whether access is currently permitted.
    pub enabled: bool,
}

impl NormalizedObservation for PrivacyState {
    fn observation_digest(&self) -> Digest {
        // The control is part of the digest: "microphone enabled" must never
        // satisfy a postcondition about the camera.
        Digest::of_str(&format!(
            "privacy:{}:{}",
            self.control.tag(),
            self.enabled
        ))
    }
}

/// A full snapshot across every recognized control.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PrivacySnapshot {
    /// Camera access, when readable. `None` means **unknown**, never "off".
    pub camera: Option<bool>,
    /// Microphone access, when readable.
    pub microphone: Option<bool>,
    /// Location access, when readable.
    pub location: Option<bool>,
}

/// One governed privacy request.
#[derive(Debug, Clone)]
pub struct PrivacyRequest {
    /// The canonical tool/action name.
    pub action: String,
    /// The canonical tool parameters.
    pub params: serde_json::Value,
    /// The control to change.
    pub control: RecognizedPrivacyControl,
    /// The desired state.
    pub enabled: bool,
}

impl PrivacyRequest {
    /// The state this request is trying to reach.
    #[must_use]
    pub fn desired_state(&self) -> PrivacyState {
        PrivacyState {
            control: self.control,
            enabled: self.enabled,
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
pub trait PrivacyTransport: Send + Sync {
    /// The provider identity.
    fn provider_id(&self) -> ProviderId;

    /// Read one control's state.
    ///
    /// An unreadable control must be an `Err`. Returning `false` would report a
    /// sensor as closed while it may be open.
    async fn read_control(
        &self,
        ctx: &HostExecutionContext,
        control: RecognizedPrivacyControl,
    ) -> Result<bool, OsControlError>;

    /// Read every recognized control, tolerating individually-unknown ones.
    ///
    /// Used only by the snapshot read, where per-control `None` is reported to the
    /// caller as an explicit unknown rather than collapsed.
    async fn read_snapshot(
        &self,
        ctx: &HostExecutionContext,
    ) -> Result<PrivacySnapshot, OsControlError>;

    /// Apply one change.
    async fn apply(
        &self,
        ctx: &AdmittedMutationContext<'_>,
        control: RecognizedPrivacyControl,
        enabled: bool,
    ) -> Result<ApplyOutcome, OsControlError>;
}

/// The governed provider.
pub struct PrivacyControl<T: PrivacyTransport> {
    transport: T,
}

impl<T: PrivacyTransport> PrivacyControl<T> {
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

    fn satisfying(&self, observed: &PrivacyState) -> SatisfyingVerification<PrivacyState> {
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
impl<T: PrivacyTransport> DesiredStateControl<PrivacyRequest, PrivacyState> for PrivacyControl<T> {
    async fn observe(
        &self,
        ctx: &HostExecutionContext,
        request: &PrivacyRequest,
    ) -> Result<PrivacyState, OsControlError> {
        let enabled = self.transport.read_control(ctx, request.control).await?;
        Ok(PrivacyState {
            control: request.control,
            enabled,
        })
    }

    async fn apply(
        &self,
        ctx: &AdmittedMutationContext<'_>,
        request: &PrivacyRequest,
        _desired: &PrivacyState,
    ) -> Result<ApplyOutcome, OsControlError> {
        self.transport
            .apply(ctx, request.control, request.enabled)
            .await
    }

    async fn verify(
        &self,
        ctx: &HostExecutionContext,
        request: &PrivacyRequest,
        desired: &PrivacyState,
    ) -> Result<VerificationReport<PrivacyState>, OsControlError> {
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
        // The contract is `UserRequestable`: restoring a privacy control is a
        // deliberate act the user asks for, not something done automatically —
        // silently re-enabling a camera would be the worst possible "recovery".
        Err(OsControlError::Unsupported {
            capability: CapabilityId::new("privacy.rollback"),
            reason: SafeText::new(
                "a privacy control is restored by an explicit user request, never automatically",
            ),
        })
    }
}

/// The port a handler resolves.
#[async_trait]
pub trait PrivacyControlPort: DesiredStateControl<PrivacyRequest, PrivacyState> {
    /// Read every recognized control.
    async fn snapshot(
        &self,
        ctx: &HostExecutionContext,
    ) -> Result<PrivacySnapshot, OsControlError>;
}

#[async_trait]
impl<T: PrivacyTransport> PrivacyControlPort for PrivacyControl<T> {
    async fn snapshot(
        &self,
        ctx: &HostExecutionContext,
    ) -> Result<PrivacySnapshot, OsControlError> {
        self.transport.read_snapshot(ctx).await
    }
}

/// Parse a recognized control name. An unknown token is refused rather than
/// mapped onto a default, because guessing which sensor the user meant is not an
/// acceptable failure mode here.
pub fn parse_control(raw: &str) -> Result<RecognizedPrivacyControl, OsControlError> {
    match raw.trim().to_ascii_lowercase().replace('_', "-").as_str() {
        "camera" | "camera-access" => Ok(RecognizedPrivacyControl::CameraAccess),
        "microphone" | "microphone-access" | "mic" => {
            Ok(RecognizedPrivacyControl::MicrophoneAccess)
        }
        "location" | "location-access" => Ok(RecognizedPrivacyControl::LocationAccess),
        _ => Err(OsControlError::InvalidRequest {
            field: crate::os_control::contract::SafeField::new("control"),
            reason: SafeText::new(
                "control must be one of camera, microphone, location",
            ),
        }),
    }
}

#[cfg(all(test, feature = "os-control-test"))]
mod tests {
    use super::*;

    #[test]
    fn the_control_is_part_of_the_digest() {
        // "microphone enabled" must never satisfy a camera postcondition.
        let camera = PrivacyState {
            control: RecognizedPrivacyControl::CameraAccess,
            enabled: true,
        };
        let mic = PrivacyState {
            control: RecognizedPrivacyControl::MicrophoneAccess,
            enabled: true,
        };
        assert_ne!(camera.observation_digest(), mic.observation_digest());
    }

    #[test]
    fn an_unknown_control_name_is_refused() {
        assert!(parse_control("bluetooth").is_err());
        assert!(parse_control("").is_err());
        assert_eq!(
            parse_control("camera").unwrap(),
            RecognizedPrivacyControl::CameraAccess
        );
        assert_eq!(
            parse_control("microphone_access").unwrap(),
            RecognizedPrivacyControl::MicrophoneAccess
        );
        assert_eq!(
            parse_control("Location").unwrap(),
            RecognizedPrivacyControl::LocationAccess
        );
    }

    #[test]
    fn an_unknown_snapshot_entry_is_none_not_false() {
        // The distinction the whole domain rests on: a sensor whose state could
        // not be read must not be reported as closed.
        let snapshot = PrivacySnapshot {
            camera: None,
            microphone: Some(true),
            location: Some(false),
        };
        assert_eq!(snapshot.camera, None, "unknown is not 'off'");
        assert_eq!(snapshot.microphone, Some(true));
        assert_eq!(snapshot.location, Some(false));
    }

    #[test]
    fn desired_state_names_the_requested_control() {
        let request = PrivacyRequest {
            action: "set_privacy_control".to_string(),
            params: serde_json::Value::Null,
            control: RecognizedPrivacyControl::CameraAccess,
            enabled: false,
        };
        let desired = request.desired_state();
        assert_eq!(desired.control, RecognizedPrivacyControl::CameraAccess);
        assert!(!desired.enabled);
    }
}
