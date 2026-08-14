//! Audio domain: the `AudioControl` desired-state provider (design §3).
//!
//! linux-os-control-production **Task 2.1** — "Migrate audio volume and add
//! getters/mute" (OSC-005, OSC-006, OSC-018, OSC-031).
//!
//! This module replaces the direct `wpctl`/`pactl`/`amixer` subprocess handling
//! that used to live in `tools/system_config.rs`. It composes the F1 runtime:
//!
//! * [`AudioEndpointState`] is a normalized observation
//!   ([`NormalizedObservation`]) whose numeric volume drives
//!   [`ComparatorKind::WithinTolerance`] idempotency + verification, so a
//!   set-volume that is already within the configured percentage tolerance is
//!   `Unchanged` and never re-applied.
//! * [`AudioControl`] implements the generic [`DesiredStateControl`] lifecycle
//!   (observe → apply → verify → rollback). Its `apply`/`rollback` build a
//!   governed [`StructuredCommandRequest`] from the borrowed
//!   [`AdmittedMutationContext`] — the only sanctioned path to a child process
//!   — so no audio code touches `ExecWrapper`/`tokio::process` directly.
//! * The live transport ([`crate::os_control::linux::providers::pipewire`]) is a
//!   raw, deny-live-gated adapter; deny-live tests inject [`FakeAudioTransport`].
//!
//! # Mute / microphone privacy (OSC-018.3)
//!
//! Output volume + output mute are `PublicLocal`. Microphone (input) endpoint
//! state is privacy-sensitive: unmuting, raising the input level, or activating
//! the microphone is RED. This task implements the **output** operations and
//! wires the [`endpoint_data_class`] / [`is_privacy_sensitive`] classification
//! hooks against the shared sensitivity registry so mute state stays coherent;
//! the input-device operations land in Task 3.6.

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::SystemTime;

use async_trait::async_trait;

use crate::os_control::context::{AdmittedMutationContext, HostExecutionContext};
use crate::os_control::contract::{
    CapabilityId, ComparatorKind, DesiredStateControl, Digest, OsEvidenceSource, ProviderId,
    SafeErrorCode, Tolerance, VerificationReliability,
};
use crate::os_control::error::OsControlError;
use crate::os_control::linux::structured_command::{
    CommandPlan, CommandPolicy, StructuredCommandRequest, TrustedExecutable,
};
use crate::os_control::receipt::{
    ApplyOutcome, RedactedObservation, RollbackToken, SatisfyingVerification,
    VerificationContradiction, VerificationReport,
};
use crate::os_control::redaction::DataClass;
use crate::os_control::runtime::NormalizedObservation;

pub mod media;
pub mod parsers;
pub mod selection;
pub mod streams;

/// Deny-live fake transport (Task 0.4 / OSC-033); test composition only.
#[cfg(feature = "os-control-test")]
pub mod fake;


pub use selection::AudioBackend;

/// The frozen maximum byte length of an audio identity token
/// (`AudioEndpointId`, `AudioStreamId`, `AudioProfileId`, `AudioPortId`).
pub const AUDIO_ID_MAX_BYTES: usize = 128;

/// Validate a caller-supplied audio identity token **before** it can become an
/// argv element.
///
/// Rejects rather than escapes (design §5): an empty token, one over the frozen
/// byte bound, one containing a control character, and one beginning with `-`,
/// which a CLI would read as an option rather than a target.
pub fn validate_audio_id(field: &str, raw: &str) -> Result<String, OsControlError> {
    let invalid = |reason: &str| OsControlError::InvalidRequest {
        field: crate::os_control::contract::SafeField::new(field),
        reason: crate::os_control::contract::SafeText::new(reason),
    };
    if raw.is_empty() {
        return Err(invalid("an audio identity token must not be empty"));
    }
    if raw.len() > AUDIO_ID_MAX_BYTES {
        return Err(invalid("audio identity token exceeds the maximum length"));
    }
    if raw.chars().any(char::is_control) {
        return Err(invalid("audio identity token contains a control character"));
    }
    if raw.starts_with('-') {
        return Err(invalid(
            "audio identity token must not begin with `-`: it would be read as a command option",
        ));
    }
    Ok(raw.to_string())
}

macro_rules! audio_id {
    ($name:ident, $field:literal, $doc:literal) => {
        #[doc = $doc]
        ///
        /// Validated on construction: this token reaches a governed argv, so an
        /// option-looking or control-character-bearing value is rejected, never
        /// escaped.
        #[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
        pub struct $name(String);

        impl $name {
            /// Validate and wrap a raw token.
            pub fn parse(raw: impl AsRef<str>) -> Result<Self, OsControlError> {
                Ok(Self(validate_audio_id($field, raw.as_ref().trim())?))
            }

            /// Borrow the token.
            #[must_use]
            pub fn as_str(&self) -> &str {
                &self.0
            }

            /// A correlation-safe digest of the token.
            #[must_use]
            pub fn digest(&self) -> Digest {
                Digest::of_str(&self.0)
            }
        }

        impl std::fmt::Display for $name {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str(&self.0)
            }
        }

        impl serde::Serialize for $name {
            fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
                s.serialize_str(&self.0)
            }
        }
    };
}

audio_id!(
    AudioEndpointId,
    "endpoint",
    "A stable audio endpoint identity (a sink or source *name*, which survives a PipeWire restart — not a node index, which does not)."
);
audio_id!(
    AudioProfileId,
    "profile",
    "A card profile identity (e.g. `output:analog-stereo+input:analog-stereo`)."
);
audio_id!(
    AudioPortId,
    "port",
    "A port identity on an endpoint (e.g. `analog-output-headphones`)."
);
audio_id!(
    AudioStreamId,
    "stream",
    "A per-application audio stream identity — the stream's own node id. Two windows of one application are two different streams, and an application *name* identifies neither."
);
audio_id!(
    MediaPlayerId,
    "player",
    "An MPRIS player identity — its session-bus name (`org.mpris.MediaPlayer2.<app>`), which is stable for the player's lifetime. Its displayed title is not."
);

/// The compile-time maximum percentage tolerance for audio verification
/// (matches the frozen manifest's `AbsolutePercentagePoints.compileTimeMaximum`).
pub const AUDIO_TOLERANCE_MAX: f64 = 5.0;

/// The default absolute percentage tolerance used for idempotency + verification.
pub const AUDIO_TOLERANCE_DEFAULT: f64 = 2.0;

/// Which audio endpoint an operation targets.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AudioEndpointKind {
    /// The default output sink (speakers/headphones). `PublicLocal`.
    Output,
    /// The default input source (microphone). Privacy-sensitive (OSC-018.3).
    Input,
}

/// Which dimension of the endpoint state a request compares against, so the
/// idempotency/verification comparator only considers the field the operation
/// changes (a volume change must not be spuriously contradicted by an unrelated
/// mute reading, and vice versa).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AudioFocus {
    /// Compare the volume percentage (numeric, within tolerance).
    Volume,
    /// Compare the mute state (exact).
    Mute,
    /// Compare which endpoint is the default for its kind (exact).
    ///
    /// Deliberately carries no volume or mute fact: once the default endpoint
    /// changes, any level or mute reading taken beforehand described the
    /// *previous* device, so bundling one into this observation would attach a
    /// stale fact to a fresh receipt.
    DefaultEndpoint,
    /// Compare an endpoint's active card profile (exact).
    Profile,
    /// Compare the full endpoint state (reads / `get_audio_state`).
    Full,
}

/// A normalized audio endpoint observation (design §5). The digest binds the
/// focused dimension so `Exact` comparison of a mute op is not perturbed by
/// volume, while `numeric_value` exposes the volume for `WithinTolerance`.
#[derive(Debug, Clone, PartialEq)]
pub struct AudioEndpointState {
    /// Output/input volume, 0..=100.
    pub volume_percent: u8,
    /// Whether the endpoint is muted.
    pub muted: bool,
    /// Which endpoint is the default for its kind, when that is the focused
    /// dimension. `None` means it was not read — never "there is none".
    pub default_endpoint: Option<AudioEndpointId>,
    /// The endpoint's active card profile, when that is the focused dimension.
    /// `None` means it was not read.
    pub active_profile: Option<AudioProfileId>,
    /// The comparison focus for this observation.
    pub focus: AudioFocus,
}

impl AudioEndpointState {
    /// Construct a focused endpoint observation.
    #[must_use]
    pub fn new(volume_percent: u8, muted: bool, focus: AudioFocus) -> Self {
        Self {
            volume_percent: volume_percent.min(100),
            muted,
            default_endpoint: None,
            active_profile: None,
            focus,
        }
    }

    /// A [`AudioFocus::DefaultEndpoint`]-focused observation.
    #[must_use]
    pub fn default_endpoint(endpoint: AudioEndpointId) -> Self {
        Self {
            volume_percent: 0,
            muted: false,
            default_endpoint: Some(endpoint),
            active_profile: None,
            focus: AudioFocus::DefaultEndpoint,
        }
    }

    /// A [`AudioFocus::Profile`]-focused observation.
    #[must_use]
    pub fn active_profile(profile: AudioProfileId) -> Self {
        Self {
            volume_percent: 0,
            muted: false,
            default_endpoint: None,
            active_profile: Some(profile),
            focus: AudioFocus::Profile,
        }
    }
}

impl NormalizedObservation for AudioEndpointState {
    fn observation_digest(&self) -> Digest {
        match self.focus {
            AudioFocus::Volume => Digest::of_str(&format!("audio:vol:{}", self.volume_percent)),
            AudioFocus::Mute => Digest::of_str(&format!("audio:mute:{}", self.muted)),
            // An unread dimension gets a digest that cannot collide with any
            // real id, so an unread state never compares equal to a desired one.
            AudioFocus::DefaultEndpoint => Digest::of_str(&match &self.default_endpoint {
                Some(id) => format!("audio:default:id:{id}"),
                None => "audio:default:unread".to_string(),
            }),
            AudioFocus::Profile => Digest::of_str(&match &self.active_profile {
                Some(id) => format!("audio:profile:id:{id}"),
                None => "audio:profile:unread".to_string(),
            }),
            AudioFocus::Full => {
                Digest::of_str(&format!("audio:vol:{}:mute:{}", self.volume_percent, self.muted))
            }
        }
    }

    fn numeric_value(&self) -> Option<f64> {
        match self.focus {
            AudioFocus::Volume => Some(self.volume_percent as f64),
            _ => None,
        }
    }
}

/// The concrete audio operation.
///
/// `SetOutputLevel` / `SetOutputMute` are endpoint-agnostic despite their
/// names: the endpoint they act on is [`AudioRequest::endpoint`], so the same
/// two variants drive the speaker and the microphone. The names are kept
/// because they are part of the already-shipped `set_volume` / `set_audio_mute`
/// path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AudioOp {
    /// Read-only state query (`get_audio_state`).
    GetState,
    /// Set the targeted endpoint's volume to a percentage.
    SetOutputLevel(u8),
    /// Set the targeted endpoint's mute state.
    SetOutputMute(bool),
    /// Make a named endpoint the default for its kind.
    SetDefaultEndpoint(AudioEndpointId),
    /// Switch an endpoint's card profile, optionally selecting a port.
    SetProfile {
        /// The endpoint (card) whose profile changes.
        endpoint: AudioEndpointId,
        /// The profile to activate.
        profile: AudioProfileId,
        /// An optional port to select once the profile is active.
        port: Option<AudioPortId>,
    },
}

/// A fully-described audio request. It carries the canonical `action`/`params`
/// so the governed [`StructuredCommandRequest`] can bind them against the grant.
#[derive(Debug, Clone)]
pub struct AudioRequest {
    /// The canonical tool/action name the grant was minted against.
    pub action: String,
    /// The canonical tool parameters (must reproduce the grant's params digest).
    pub params: serde_json::Value,
    /// The concrete operation.
    pub op: AudioOp,
    /// The endpoint targeted (output for this task; input arrives in 3.6).
    pub endpoint: AudioEndpointKind,
}

impl AudioRequest {
    /// The comparison focus implied by the operation.
    #[must_use]
    pub fn focus(&self) -> AudioFocus {
        match self.op {
            AudioOp::GetState => AudioFocus::Full,
            AudioOp::SetOutputLevel(_) => AudioFocus::Volume,
            AudioOp::SetOutputMute(_) => AudioFocus::Mute,
            AudioOp::SetDefaultEndpoint(_) => AudioFocus::DefaultEndpoint,
            AudioOp::SetProfile { .. } => AudioFocus::Profile,
        }
    }

    /// The desired end state for a mutation, focused on the changed dimension.
    /// Returns `None` for the read-only [`AudioOp::GetState`].
    #[must_use]
    pub fn desired_state(&self) -> Option<AudioEndpointState> {
        match &self.op {
            AudioOp::GetState => None,
            AudioOp::SetOutputLevel(v) => {
                Some(AudioEndpointState::new(*v, false, AudioFocus::Volume))
            }
            AudioOp::SetOutputMute(m) => Some(AudioEndpointState::new(0, *m, AudioFocus::Mute)),
            AudioOp::SetDefaultEndpoint(endpoint) => {
                Some(AudioEndpointState::default_endpoint(endpoint.clone()))
            }
            AudioOp::SetProfile { profile, .. } => {
                Some(AudioEndpointState::active_profile(profile.clone()))
            }
        }
    }

    /// The idempotency/verification comparator for this operation.
    #[must_use]
    pub fn comparator(&self) -> ComparatorKind {
        match self.op {
            AudioOp::SetOutputLevel(_) => ComparatorKind::WithinTolerance,
            AudioOp::SetOutputMute(_)
            | AudioOp::GetState
            | AudioOp::SetDefaultEndpoint(_)
            | AudioOp::SetProfile { .. } => ComparatorKind::Exact,
        }
    }

    /// The numeric tolerance for this operation (volume only).
    #[must_use]
    pub fn tolerance(&self) -> Option<Tolerance> {
        match self.op {
            AudioOp::SetOutputLevel(_) => Some(Tolerance {
                abs: AUDIO_TOLERANCE_DEFAULT,
            }),
            _ => None,
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Privacy classification hooks (OSC-018.3)
// ─────────────────────────────────────────────────────────────────────────────

/// The data-sensitivity class of an endpoint's state. Output is `PublicLocal`;
/// microphone (input) state is privacy-sensitive metadata.
#[must_use]
pub fn endpoint_data_class(endpoint: AudioEndpointKind) -> DataClass {
    match endpoint {
        AudioEndpointKind::Output => DataClass::PublicLocal,
        AudioEndpointKind::Input => DataClass::SensitiveMetadata,
    }
}

/// Whether an operation is privacy-sensitive (RED) under OSC-018.3: unmuting,
/// raising the level of, or activating the **microphone**. Output operations are
/// never privacy-sensitive. This is the classification hook the input-device
/// operations (Task 3.6) build their approval requirement on.
#[must_use]
pub fn is_privacy_sensitive(endpoint: AudioEndpointKind, op: &AudioOp) -> bool {
    if endpoint != AudioEndpointKind::Input {
        return false;
    }
    match op {
        // Unmuting or raising the mic level activates capture → privacy-sensitive.
        AudioOp::SetOutputMute(muted) => !muted,
        AudioOp::SetOutputLevel(_) => true,
        // Switching which microphone is the default starts capturing from a
        // different device — a privacy event even though no level changed.
        AudioOp::SetDefaultEndpoint(_) => true,
        // A card profile can enable or disable the capture path entirely.
        AudioOp::SetProfile { .. } => true,
        AudioOp::GetState => false,
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Transport seam
// ─────────────────────────────────────────────────────────────────────────────

/// The raw audio transport seam. The live implementation
/// ([`crate::os_control::linux::providers::pipewire::LivePipewireAudio`]) is a
/// deny-live-gated adapter over `wpctl`/`pactl`/`amixer`; deny-live tests inject
/// [`FakeAudioTransport`]. Reads run a query command and parse it; `dispatch`
/// runs a governed [`StructuredCommandRequest`].
#[async_trait]
pub trait AudioTransport: Send + Sync {
    /// The stable provider identity (never model prose).
    fn provider_id(&self) -> ProviderId;

    /// The selected backend (records which of `wpctl`/`pactl`/`amixer` is used).
    fn selected_backend(&self) -> AudioBackend;

    /// Read the current endpoint state (volume + mute). A parse ambiguity must
    /// surface as an error, never a fabricated state.
    async fn read_state(
        &self,
        ctx: &HostExecutionContext,
        endpoint: AudioEndpointKind,
    ) -> Result<(u8, bool), OsControlError>;

    /// Read which endpoint is currently the default for `endpoint`'s kind.
    ///
    /// Errors when it cannot be determined. There is no "no default" answer to
    /// report: a session always has one, so silence means the read failed.
    async fn read_default_endpoint(
        &self,
        ctx: &HostExecutionContext,
        endpoint: AudioEndpointKind,
    ) -> Result<AudioEndpointId, OsControlError>;

    /// Read `endpoint`'s active card profile.
    ///
    /// `Ok(None)` means the endpoint is genuinely not a present card — a
    /// different fact from `Err`, which means the profile could not be read.
    async fn read_active_profile(
        &self,
        ctx: &HostExecutionContext,
        endpoint: &AudioEndpointId,
    ) -> Result<Option<AudioProfileId>, OsControlError>;

    /// Enumerate live per-application streams. An empty vector means nothing is
    /// playing; that is an answer, not a failure.
    async fn list_streams(
        &self,
        ctx: &HostExecutionContext,
    ) -> Result<Vec<streams::AudioStreamInfo>, OsControlError>;

    /// Read one stream by its node id.
    ///
    /// `Ok(None)` means the stream is gone — the application closed it between
    /// the listing and this read. That is a *different fact* from `Err`, which
    /// means the stream table could not be read at all.
    async fn read_stream(
        &self,
        ctx: &HostExecutionContext,
        stream: &AudioStreamId,
    ) -> Result<Option<streams::AudioStreamInfo>, OsControlError>;

    /// Enumerate MPRIS media players on the session bus. An empty vector means
    /// no player is running.
    async fn list_media_players(
        &self,
        ctx: &HostExecutionContext,
    ) -> Result<Vec<media::MediaPlayerInfo>, OsControlError>;

    /// Read one MPRIS player by its bus name.
    ///
    /// `Ok(None)` means the player left the bus — distinct from `Err`, which
    /// means the bus could not be read.
    async fn read_media_player(
        &self,
        ctx: &HostExecutionContext,
        player: &MediaPlayerId,
    ) -> Result<Option<media::MediaPlayerInfo>, OsControlError>;

    /// Dispatch a governed structured command (the only path to a process).
    async fn dispatch(
        &self,
        ctx: &AdmittedMutationContext<'_>,
        request: &StructuredCommandRequest,
    ) -> Result<ApplyOutcome, OsControlError>;
}

/// The maximum number of governed steps one audio apply may perform.
const AUDIO_MAX_APPLY_STEPS: usize = 2;

/// One governed step of an apply: which trusted CLI runs, with which argv, under
/// which stable step label (the label is what a partial-effect receipt names).
struct AudioStep {
    executable: TrustedExecutable,
    args: Vec<String>,
    step: &'static str,
}

/// The rollback snapshot captured before an apply, so a contradiction can be
/// compensated back to the exact prior state.
#[derive(Debug, Clone)]
struct RollbackSnapshot {
    before: AudioEndpointState,
    op: AudioOp,
    action: String,
    params: serde_json::Value,
    endpoint: AudioEndpointKind,
}

/// The `AudioControl` desired-state provider (design §3, §4). Generic over the
/// [`AudioTransport`] so the same governed logic runs over the live PipeWire
/// adapter and the deny-live fake.
pub struct AudioControl<T: AudioTransport> {
    transport: T,
    policy: CommandPolicy,
    tolerance: f64,
    /// Prior-state snapshots keyed by session id, captured in `apply` for
    /// `rollback`. Interior mutability because the provider is shared (`&self`);
    /// audio ops are serialized by the endpoint resource lease.
    snapshots: Mutex<HashMap<String, RollbackSnapshot>>,
    /// Prior per-stream state, keyed the same way. Separate from the endpoint
    /// map because a stream is a different resource with a different lease.
    stream_snapshots: Mutex<HashMap<String, streams::StreamRollbackSnapshot>>,
    /// Prior player state keyed by action id, so a relative playback action can
    /// be verified against what was true immediately before it ran.
    media_snapshots: Mutex<HashMap<String, media::MediaSnapshot>>,
}

impl<T: AudioTransport> AudioControl<T> {
    /// Compose an `AudioControl` over a transport with the default tolerance.
    #[must_use]
    pub fn new(transport: T) -> Self {
        Self::with_tolerance(transport, AUDIO_TOLERANCE_DEFAULT)
    }

    /// Compose with an explicit tolerance (clamped to [`AUDIO_TOLERANCE_MAX`]).
    #[must_use]
    pub fn with_tolerance(transport: T, tolerance: f64) -> Self {
        Self {
            transport,
            policy: CommandPolicy::new(),
            tolerance: tolerance.clamp(0.0, AUDIO_TOLERANCE_MAX),
            snapshots: Mutex::new(HashMap::new()),
            stream_snapshots: Mutex::new(HashMap::new()),
            media_snapshots: Mutex::new(HashMap::new()),
        }
    }

    /// The selected backend (for the `backend` result field).
    #[must_use]
    pub fn backend(&self) -> AudioBackend {
        self.transport.selected_backend()
    }

    /// Borrow the underlying transport (used by tests to inspect captured argv).
    #[must_use]
    pub fn transport(&self) -> &T {
        &self.transport
    }

    /// The provider identity.
    #[must_use]
    pub fn provider_id(&self) -> ProviderId {
        self.transport.provider_id()
    }

    /// The configured tolerance.
    #[must_use]
    pub fn tolerance(&self) -> f64 {
        self.tolerance
    }

    /// The evidence source for verification observations from this backend. The
    /// CLI backends are structured-command queries; a future native PipeWire
    /// property read would be `AuthoritativeServiceState`.
    fn evidence_source(&self) -> OsEvidenceSource {
        OsEvidenceSource::StructuredCommandQuery
    }

    /// Build the governed structured command for a mutating operation on a
    /// caller-chosen trusted executable.
    fn build_command_with(
        &self,
        ctx: &AdmittedMutationContext<'_>,
        action: &str,
        params: &serde_json::Value,
        executable: TrustedExecutable,
        args: Vec<String>,
    ) -> Result<StructuredCommandRequest, OsControlError> {
        let plan = CommandPlan::new(
            CapabilityId::new(action),
            action.to_string(),
            params.clone(),
            executable,
            args,
        );
        StructuredCommandRequest::from_admitted(ctx, plan, &self.policy)
    }

    /// The ordered governed steps that realise `request`.
    ///
    /// Almost every operation is a single step. Only a profile switch that also
    /// selects a port is two, because the control surface exposes them as two
    /// separate commands — and a receipt must be able to say that the first
    /// committed and the second did not.
    fn apply_steps(&self, request: &AudioRequest) -> Result<Vec<AudioStep>, OsControlError> {
        let backend = self.transport.selected_backend();
        let endpoint = request.endpoint;
        match &request.op {
            AudioOp::SetOutputLevel(v) => Ok(vec![AudioStep {
                executable: backend.trusted_executable()?,
                args: selection::set_volume_argv(backend, endpoint, *v),
                step: "set_level",
            }]),
            AudioOp::SetOutputMute(m) => Ok(vec![AudioStep {
                executable: backend.trusted_executable()?,
                args: selection::set_mute_argv(backend, endpoint, *m),
                step: "set_mute",
            }]),
            AudioOp::SetDefaultEndpoint(target) => Ok(vec![AudioStep {
                executable: selection::pulse_executable()?,
                args: selection::set_default_endpoint_argv(endpoint, target.as_str()),
                step: "set_default_endpoint",
            }]),
            AudioOp::SetProfile {
                endpoint: card,
                profile,
                port,
            } => {
                let mut steps = vec![AudioStep {
                    executable: selection::pulse_executable()?,
                    args: selection::set_card_profile_argv(card.as_str(), profile.as_str()),
                    step: "set_card_profile",
                }];
                if let Some(port) = port {
                    steps.push(AudioStep {
                        executable: selection::pulse_executable()?,
                        args: selection::set_sink_port_argv(card.as_str(), port.as_str()),
                        step: "set_port",
                    });
                }
                Ok(steps)
            }
            AudioOp::GetState => Err(OsControlError::InvalidRequest {
                field: crate::os_control::contract::SafeField::new("op"),
                reason: crate::os_control::contract::SafeText::new(
                    "a state read has no apply step",
                ),
            }),
        }
    }

    /// Run an ordered step list, reporting a partial effect honestly when a
    /// later step fails after an earlier one committed.
    async fn dispatch_steps(
        &self,
        ctx: &AdmittedMutationContext<'_>,
        action: &str,
        params: &serde_json::Value,
        steps: Vec<AudioStep>,
    ) -> Result<ApplyOutcome, OsControlError> {
        let mut completed: Vec<crate::os_control::contract::SafeStepId> = Vec::new();
        let mut last = None;
        for step in steps {
            let label = crate::os_control::contract::SafeStepId::new(step.step);
            let command =
                self.build_command_with(ctx, action, params, step.executable, step.args)?;
            match self.transport.dispatch(ctx, &command).await {
                Ok(outcome) => {
                    completed.push(label);
                    last = Some(outcome);
                }
                Err(error) => {
                    // Nothing committed yet → the failure is the whole story.
                    let mut committed = completed.into_iter();
                    let Some(first) = committed.next() else {
                        return Err(error);
                    };
                    // Something did commit: never report this as a clean failure.
                    let tail = crate::os_control::contract::BoundedVec::from_iter_capped(
                        committed,
                        AUDIO_MAX_APPLY_STEPS,
                    );
                    return Ok(ApplyOutcome::PartiallyApplied(
                        crate::os_control::receipt::PartialDispatch::new(
                            None,
                            crate::os_control::contract::NonEmptyBoundedVec::new(first, tail),
                            label,
                            crate::os_control::receipt::PartialEffectCause::StepFailedAfterCommit,
                            crate::os_control::contract::BoundedVec::new(),
                        ),
                    ));
                }
            }
        }
        last.ok_or_else(|| OsControlError::InvalidRequest {
            field: crate::os_control::contract::SafeField::new("op"),
            reason: crate::os_control::contract::SafeText::new("no apply step was produced"),
        })
    }

    fn satisfying(
        &self,
        observed: &AudioEndpointState,
    ) -> SatisfyingVerification<AudioEndpointState> {
        SatisfyingVerification::new(
            self.evidence_source(),
            VerificationReliability::Strong,
            self.transport.provider_id(),
            RedactedObservation::new(observed.clone(), observed.observation_digest()),
            None,
            SystemTime::now(),
            0,
        )
    }

    /// Observe exactly the dimension the request focuses on — and nothing else.
    ///
    /// A default-endpoint or profile observation deliberately carries no volume
    /// or mute reading. Once the default endpoint changes, a level read before
    /// the switch described the *previous* device, so attaching one here would
    /// put a stale fact on a fresh receipt.
    async fn observe_focused(
        &self,
        ctx: &HostExecutionContext,
        request: &AudioRequest,
    ) -> Result<AudioEndpointState, OsControlError> {
        match &request.op {
            AudioOp::SetDefaultEndpoint(_) => {
                let current = self
                    .transport
                    .read_default_endpoint(ctx, request.endpoint)
                    .await?;
                Ok(AudioEndpointState::default_endpoint(current))
            }
            AudioOp::SetProfile { endpoint, .. } => {
                match self.transport.read_active_profile(ctx, endpoint).await? {
                    Some(profile) => Ok(AudioEndpointState::active_profile(profile)),
                    // The card is not present. That is a fact about the target,
                    // not a failed read, so it must not be reported as either a
                    // successful observation or an unavailable provider.
                    None => Err(OsControlError::TargetChanged),
                }
            }
            _ => {
                let (volume, muted) = self.transport.read_state(ctx, request.endpoint).await?;
                Ok(AudioEndpointState::new(volume, muted, request.focus()))
            }
        }
    }
}

#[async_trait]
impl<T: AudioTransport> DesiredStateControl<AudioRequest, AudioEndpointState> for AudioControl<T> {
    async fn observe(
        &self,
        ctx: &HostExecutionContext,
        request: &AudioRequest,
    ) -> Result<AudioEndpointState, OsControlError> {
        self.observe_focused(ctx, request).await
    }

    async fn apply(
        &self,
        ctx: &AdmittedMutationContext<'_>,
        request: &AudioRequest,
        _desired: &AudioEndpointState,
    ) -> Result<ApplyOutcome, OsControlError> {
        // Capture the pre-apply state so a contradiction can be rolled back to
        // the exact prior value of the focused dimension.
        if let Ok(before) = self.observe_focused(ctx.observation(), request).await {
            let session = ctx.grant().session_id().to_string();
            self.snapshots.lock().expect("audio snapshots poisoned").insert(
                session,
                RollbackSnapshot {
                    before,
                    op: request.op.clone(),
                    action: request.action.clone(),
                    params: request.params.clone(),
                    endpoint: request.endpoint,
                },
            );
        }

        let steps = self.apply_steps(request)?;
        self.dispatch_steps(ctx, &request.action, &request.params, steps)
            .await
    }

    async fn verify(
        &self,
        ctx: &HostExecutionContext,
        request: &AudioRequest,
        desired: &AudioEndpointState,
    ) -> Result<VerificationReport<AudioEndpointState>, OsControlError> {
        let observed = self.observe_focused(ctx, request).await?;

        let satisfied = match request.comparator() {
            ComparatorKind::WithinTolerance => {
                (observed.volume_percent as f64 - desired.volume_percent as f64).abs()
                    <= self.tolerance
            }
            _ => observed.observation_digest() == desired.observation_digest(),
        };

        if satisfied {
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
        token: &RollbackToken,
    ) -> Result<ApplyOutcome, OsControlError> {
        let snapshot = self
            .snapshots
            .lock()
            .expect("audio snapshots poisoned")
            .get(token.session_id().as_str())
            .cloned();

        let Some(snapshot) = snapshot else {
            // No captured prior state → the effect is unobservable for compensation.
            return Ok(ApplyOutcome::Uncertain(
                crate::os_control::receipt::UncertainDispatch::new(
                    None,
                    crate::os_control::receipt::UncertainEffectCause::Unobservable,
                    crate::os_control::contract::BoundedVec::new(),
                ),
            ));
        };

        // Restore only the dimension the original op changed, from the exact
        // prior value. An op whose prior value was never read has no inverse and
        // must say so rather than dispatch a guess.
        let inverse = match &snapshot.op {
            AudioOp::SetOutputLevel(_) => Some(AudioOp::SetOutputLevel(
                snapshot.before.volume_percent,
            )),
            AudioOp::SetOutputMute(_) => Some(AudioOp::SetOutputMute(snapshot.before.muted)),
            AudioOp::SetDefaultEndpoint(_) => snapshot
                .before
                .default_endpoint
                .clone()
                .map(AudioOp::SetDefaultEndpoint),
            AudioOp::SetProfile { endpoint, .. } => {
                snapshot
                    .before
                    .active_profile
                    .clone()
                    .map(|profile| AudioOp::SetProfile {
                        endpoint: endpoint.clone(),
                        profile,
                        // The port the profile previously exposed was not read,
                        // so no port is re-selected: a fabricated port would be
                        // a new change dressed up as a compensation.
                        port: None,
                    })
            }
            AudioOp::GetState => None,
        };

        let Some(inverse) = inverse else {
            return Ok(ApplyOutcome::Uncertain(
                crate::os_control::receipt::UncertainDispatch::new(
                    None,
                    crate::os_control::receipt::UncertainEffectCause::Unobservable,
                    crate::os_control::contract::BoundedVec::new(),
                ),
            ));
        };

        let request = AudioRequest {
            action: snapshot.action.clone(),
            params: snapshot.params.clone(),
            op: inverse,
            endpoint: snapshot.endpoint,
        };
        let steps = self.apply_steps(&request)?;
        self.dispatch_steps(ctx, &snapshot.action, &snapshot.params, steps)
            .await
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Receipt → tool-result mapping (existing `set_volume` fields stay compatible)
// ─────────────────────────────────────────────────────────────────────────────

use crate::os_control::receipt::{ActionLifecycle, MutationReceipt};

/// Map a governed [`MutationReceipt`] to the **existing** `set_volume` result
/// fields (`volume`, `backend`, `changed`, `already_in_desired_state`), plus
/// additive `lifecycle`/`verified` fields. Preserving these keeps the migrated
/// tool wire-compatible with the pre-migration handler (design §3, Task 2.1).
#[must_use]
pub fn set_volume_result(
    receipt: &MutationReceipt<AudioEndpointState>,
    requested_percent: u8,
    backend: AudioBackend,
) -> serde_json::Value {
    let lifecycle = receipt.lifecycle();
    serde_json::json!({
        "volume": requested_percent,
        "backend": backend.as_str(),
        "changed": receipt.changed(),
        "already_in_desired_state": matches!(lifecycle, ActionLifecycle::Unchanged),
        "lifecycle": lifecycle.as_str(),
        "verified": receipt.verification().is_some(),
    })
}

/// Map a governed [`MutationReceipt`] to the `set_audio_mute` result fields
/// (mirrors `set_volume`'s shape, with `muted` in place of `volume`).
#[must_use]
pub fn set_mute_result(
    receipt: &MutationReceipt<AudioEndpointState>,
    requested_muted: bool,
    backend: AudioBackend,
) -> serde_json::Value {
    let lifecycle = receipt.lifecycle();
    serde_json::json!({
        "muted": requested_muted,
        "backend": backend.as_str(),
        "changed": receipt.changed(),
        "already_in_desired_state": matches!(lifecycle, ActionLifecycle::Unchanged),
        "lifecycle": lifecycle.as_str(),
        "verified": receipt.verification().is_some(),
    })
}

/// Map a read-only [`AudioEndpointState`] to the `get_audio_state` result fields.
#[must_use]
pub fn audio_state_result(state: &AudioEndpointState, backend: AudioBackend) -> serde_json::Value {
    serde_json::json!({
        "volume": state.volume_percent,
        "muted": state.muted,
        "backend": backend.as_str(),
    })
}

// ─────────────────────────────────────────────────────────────────────────────
// `HostOsControl::audio()` port seam (design §4)
// ─────────────────────────────────────────────────────────────────────────────

/// The dyn-compatible audio domain port design §4 names
/// `fn audio(&self) -> &dyn AudioControl` on `HostOsControl`. Because the
/// concrete [`AudioControl`] provider struct above is generic over its
/// [`AudioTransport`], `HostOsControl::audio()` returns this object-safe
/// supertrait instead so any transport (live PipeWire/wpctl/pactl/amixer, or a
/// deny-live fake) can be composed behind one erased reference. Every
/// [`AudioControl<T>`] implements it automatically via the blanket impl below.
///
/// # Why session media control lives on this port
///
/// MPRIS playback control (`list_media_players`, `control_media_playback`) is
/// exposed here rather than through a separate `HostOsControl::media()`
/// accessor: adding one would mean editing the runtime seam and the live
/// composition root, which this task does not own. The media slice is otherwise
/// a self-contained domain ([`media`]) with its own request, observation and
/// desired-state lifecycle, so promoting it to its own port later is a move, not
/// a rewrite.
#[async_trait]
pub trait AudioControlPort:
    DesiredStateControl<AudioRequest, AudioEndpointState>
    + DesiredStateControl<streams::AudioStreamRequest, streams::AudioStreamState>
    + DesiredStateControl<media::MediaRequest, media::MediaPlayerState>
{
    /// The composed volume backend label (for the `backend` result field).
    fn backend_label(&self) -> AudioBackend;

    /// Read which endpoint is currently the default for its kind.
    async fn default_endpoint(
        &self,
        ctx: &HostExecutionContext,
        endpoint: AudioEndpointKind,
    ) -> Result<AudioEndpointId, OsControlError>;

    /// One deterministic page of live per-application streams.
    async fn stream_page(
        &self,
        ctx: &HostExecutionContext,
        cursor: Option<&str>,
        limit: Option<usize>,
    ) -> Result<streams::AudioStreamPage, OsControlError>;

    /// One deterministic page of MPRIS media players.
    async fn media_player_page(
        &self,
        ctx: &HostExecutionContext,
        cursor: Option<&str>,
        limit: Option<usize>,
    ) -> Result<media::MediaPlayerPage, OsControlError>;
}

#[async_trait]
impl<T: AudioTransport> AudioControlPort for AudioControl<T> {
    fn backend_label(&self) -> AudioBackend {
        self.transport.selected_backend()
    }

    async fn default_endpoint(
        &self,
        ctx: &HostExecutionContext,
        endpoint: AudioEndpointKind,
    ) -> Result<AudioEndpointId, OsControlError> {
        self.transport.read_default_endpoint(ctx, endpoint).await
    }

    async fn stream_page(
        &self,
        ctx: &HostExecutionContext,
        cursor: Option<&str>,
        limit: Option<usize>,
    ) -> Result<streams::AudioStreamPage, OsControlError> {
        let all = self.transport.list_streams(ctx).await?;
        streams::page(all, cursor, limit)
    }

    async fn media_player_page(
        &self,
        ctx: &HostExecutionContext,
        cursor: Option<&str>,
        limit: Option<usize>,
    ) -> Result<media::MediaPlayerPage, OsControlError> {
        let all = self.transport.list_media_players(ctx).await?;
        media::page(all, cursor, limit)
    }
}

#[cfg(all(test, feature = "os-control-test"))]
mod tests {
    use super::*;

    #[test]
    fn output_state_is_public_and_not_privacy_sensitive() {
        assert_eq!(endpoint_data_class(AudioEndpointKind::Output), DataClass::PublicLocal);
        assert!(!is_privacy_sensitive(AudioEndpointKind::Output, &AudioOp::SetOutputMute(false)));
        assert!(!is_privacy_sensitive(AudioEndpointKind::Output, &AudioOp::SetOutputLevel(80)));
    }

    #[test]
    fn microphone_activation_is_privacy_sensitive() {
        assert_eq!(endpoint_data_class(AudioEndpointKind::Input), DataClass::SensitiveMetadata);
        // Unmuting the mic (muted=false) activates capture → RED.
        assert!(is_privacy_sensitive(AudioEndpointKind::Input, &AudioOp::SetOutputMute(false)));
        // Muting the mic is not privacy-sensitive.
        assert!(!is_privacy_sensitive(AudioEndpointKind::Input, &AudioOp::SetOutputMute(true)));
        // Raising the mic level is privacy-sensitive.
        assert!(is_privacy_sensitive(AudioEndpointKind::Input, &AudioOp::SetOutputLevel(50)));
    }

    #[test]
    fn volume_observation_uses_numeric_within_tolerance() {
        let desired = AudioEndpointState::new(60, false, AudioFocus::Volume);
        assert_eq!(desired.numeric_value(), Some(60.0));
        // Digest ignores mute for a volume-focused state.
        let same_vol_diff_mute = AudioEndpointState::new(60, true, AudioFocus::Volume);
        assert_eq!(
            desired.observation_digest(),
            same_vol_diff_mute.observation_digest()
        );
    }

    #[test]
    fn mute_observation_uses_exact_digest_ignoring_volume() {
        let muted = AudioEndpointState::new(60, true, AudioFocus::Mute);
        let muted_other_vol = AudioEndpointState::new(10, true, AudioFocus::Mute);
        assert_eq!(muted.numeric_value(), None);
        assert_eq!(
            muted.observation_digest(),
            muted_other_vol.observation_digest()
        );
        let unmuted = AudioEndpointState::new(60, false, AudioFocus::Mute);
        assert_ne!(muted.observation_digest(), unmuted.observation_digest());
    }
}
