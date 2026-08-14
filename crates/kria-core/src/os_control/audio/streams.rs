//! Per-application audio streams (Task 5.2, OSC-018).
//!
//! A *stream* is one application's live playback connection to an endpoint —
//! what a mixer shows as a per-app slider. It is a separate desired-state
//! lifecycle from the endpoint slice in [`super`] because it has a separate
//! identity and a separate lifetime.
//!
//! # Identity
//!
//! A stream is addressed by its **node id** and by nothing else. An application
//! name is not an identity: two windows of one browser are two streams that
//! report the same name, and a name is not unique even across different
//! applications. Every operation here therefore takes an [`AudioStreamId`]; the
//! human label rides along as descriptive metadata that is never matched on.
//!
//! # Disappearing is not failing
//!
//! A stream ends whenever its application stops playing, which can happen
//! between a listing and a mutation. That is a fact about the target, reported
//! as [`OsControlError::TargetChanged`] — deliberately distinct from
//! `Unavailable` (the stream table could not be read) and from a failed
//! mutation (the stream was there and the change did not take). A verification
//! whose target vanished is [`VerificationReport::Inconclusive`], never
//! "satisfied".

use std::time::SystemTime;

use async_trait::async_trait;

use crate::os_control::context::{AdmittedMutationContext, HostExecutionContext};
use crate::os_control::contract::{
    ComparatorKind, DesiredStateControl, Digest, SafeErrorCode, SafeField, SafeText, Tolerance,
    VerificationReliability,
};
use crate::os_control::error::OsControlError;
use crate::os_control::receipt::{
    ApplyOutcome, RedactedObservation, RollbackToken, SatisfyingVerification,
    VerificationContradiction, VerificationReport,
};
use crate::os_control::runtime::NormalizedObservation;

use super::{
    selection, AudioControl, AudioEndpointId, AudioStep, AudioStreamId, AudioTransport,
    AUDIO_TOLERANCE_DEFAULT,
};

/// The default number of streams in one page.
pub const STREAM_PAGE_DEFAULT_ITEMS: usize = 50;
/// The frozen maximum number of streams in one page.
pub const STREAM_PAGE_MAX_ITEMS: usize = 256;

fn invalid(field: &str, reason: &str) -> OsControlError {
    OsControlError::InvalidRequest {
        field: SafeField::new(field),
        reason: SafeText::new(reason),
    }
}

/// One live per-application stream.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AudioStreamInfo {
    /// The stream's node id — its only identity.
    pub stream: AudioStreamId,
    /// A descriptive application label. Never an identity.
    pub app: String,
    /// The endpoint the stream is routed to. `None` means the stream exists but
    /// is not currently routed anywhere — a fact about an idle stream, not an
    /// unknown.
    pub endpoint: Option<AudioEndpointId>,
    /// The stream's own volume, 0..=100 (independent of the endpoint's).
    pub level_percent: u8,
    /// Whether the stream is muted.
    pub muted: bool,
}

/// Which dimension of a stream a request compares against.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StreamFocus {
    /// Compare the stream's level (numeric, within tolerance).
    Level,
    /// Compare the stream's mute state (exact).
    Mute,
    /// Compare the whole stream (reads).
    Full,
}

/// A normalized observation of one stream. The digest binds the stream's
/// identity as well as the focused dimension, so an observation of a *different*
/// stream can never satisfy this stream's postcondition.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AudioStreamState {
    /// The observed stream's identity.
    pub stream: AudioStreamId,
    /// The stream's level, 0..=100.
    pub level_percent: u8,
    /// Whether the stream is muted.
    pub muted: bool,
    /// The endpoint the stream is routed to, when read.
    pub endpoint: Option<AudioEndpointId>,
    /// The comparison focus.
    pub focus: StreamFocus,
}

impl AudioStreamState {
    /// A focused stream observation.
    #[must_use]
    pub fn new(stream: AudioStreamId, level_percent: u8, muted: bool, focus: StreamFocus) -> Self {
        Self {
            stream,
            level_percent: level_percent.min(100),
            muted,
            endpoint: None,
            focus,
        }
    }

    /// The observation an info record represents.
    #[must_use]
    pub fn from_info(info: &AudioStreamInfo, focus: StreamFocus) -> Self {
        Self {
            stream: info.stream.clone(),
            level_percent: info.level_percent,
            muted: info.muted,
            endpoint: info.endpoint.clone(),
            focus,
        }
    }
}

impl NormalizedObservation for AudioStreamState {
    fn observation_digest(&self) -> Digest {
        let id = self.stream.as_str();
        Digest::of_str(&match self.focus {
            StreamFocus::Level => format!("audio:stream:{id}:level:{}", self.level_percent),
            StreamFocus::Mute => format!("audio:stream:{id}:mute:{}", self.muted),
            StreamFocus::Full => format!(
                "audio:stream:{id}:level:{}:mute:{}",
                self.level_percent, self.muted
            ),
        })
    }

    fn numeric_value(&self) -> Option<f64> {
        match self.focus {
            StreamFocus::Level => Some(self.level_percent as f64),
            _ => None,
        }
    }
}

/// The concrete per-stream operation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AudioStreamOp {
    /// Set the stream's own volume.
    SetLevel(u8),
    /// Set the stream's mute state.
    SetMute(bool),
}

/// A fully-described per-stream request.
#[derive(Debug, Clone)]
pub struct AudioStreamRequest {
    /// The canonical tool/action name the grant was minted against.
    pub action: String,
    /// The canonical tool parameters (must reproduce the grant's params digest).
    pub params: serde_json::Value,
    /// The stream being changed.
    pub stream: AudioStreamId,
    /// The operation.
    pub op: AudioStreamOp,
}

impl AudioStreamRequest {
    /// The comparison focus implied by the operation.
    #[must_use]
    pub fn focus(&self) -> StreamFocus {
        match self.op {
            AudioStreamOp::SetLevel(_) => StreamFocus::Level,
            AudioStreamOp::SetMute(_) => StreamFocus::Mute,
        }
    }

    /// The desired end state, focused on the changed dimension and bound to this
    /// stream's identity.
    #[must_use]
    pub fn desired_state(&self) -> AudioStreamState {
        match self.op {
            AudioStreamOp::SetLevel(v) => {
                AudioStreamState::new(self.stream.clone(), v, false, StreamFocus::Level)
            }
            AudioStreamOp::SetMute(m) => {
                AudioStreamState::new(self.stream.clone(), 0, m, StreamFocus::Mute)
            }
        }
    }

    /// The idempotency/verification comparator.
    #[must_use]
    pub fn comparator(&self) -> ComparatorKind {
        match self.op {
            AudioStreamOp::SetLevel(_) => ComparatorKind::WithinTolerance,
            AudioStreamOp::SetMute(_) => ComparatorKind::Exact,
        }
    }

    /// The numeric tolerance (level only).
    #[must_use]
    pub fn tolerance(&self) -> Option<Tolerance> {
        match self.op {
            AudioStreamOp::SetLevel(_) => Some(Tolerance {
                abs: AUDIO_TOLERANCE_DEFAULT,
            }),
            AudioStreamOp::SetMute(_) => None,
        }
    }

    /// The governed argv for this operation.
    #[must_use]
    pub fn argv(&self) -> Vec<String> {
        match self.op {
            AudioStreamOp::SetLevel(v) => {
                selection::set_stream_volume_argv(self.stream.as_str(), v)
            }
            AudioStreamOp::SetMute(m) => selection::set_stream_mute_argv(self.stream.as_str(), m),
        }
    }

    /// The stable step label for a receipt.
    #[must_use]
    pub fn step(&self) -> &'static str {
        match self.op {
            AudioStreamOp::SetLevel(_) => "set_stream_level",
            AudioStreamOp::SetMute(_) => "set_stream_mute",
        }
    }
}

/// One deterministic page of streams.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AudioStreamPage {
    /// The page's streams, ordered by stream id.
    pub items: Vec<AudioStreamInfo>,
    /// The cursor that continues this listing, when it was truncated.
    pub next_cursor: Option<String>,
    /// Whether more streams exist beyond this page.
    pub truncated: bool,
}

/// Mint an integrity-checked page cursor.
#[must_use]
pub fn encode_cursor(offset: usize) -> String {
    let check = Digest::of_str(&format!("audio-stream-cursor:{offset}"));
    format!("as1.{offset}.{}", &check.as_hex()[..16])
}

/// Decode a page cursor this build minted.
pub fn decode_cursor(cursor: &str) -> Result<usize, OsControlError> {
    let field = "cursor";
    if cursor.len() > 512 {
        return Err(invalid(field, "cursor exceeds the maximum length"));
    }
    let mut parts = cursor.split('.');
    match parts.next() {
        Some("as1") => {}
        _ => return Err(invalid(field, "cursor was not minted by this build")),
    }
    let offset: usize = parts
        .next()
        .and_then(|raw| raw.parse().ok())
        .ok_or_else(|| invalid(field, "cursor offset is not a number"))?;
    let check = parts
        .next()
        .ok_or_else(|| invalid(field, "cursor is missing its integrity check"))?;
    if parts.next().is_some() {
        return Err(invalid(field, "cursor has trailing content"));
    }
    let expected = Digest::of_str(&format!("audio-stream-cursor:{offset}"));
    if check != &expected.as_hex()[..16] {
        return Err(invalid(field, "cursor failed its integrity check"));
    }
    Ok(offset)
}

/// Page a stream listing deterministically (ordered by stream id).
pub fn page(
    mut all: Vec<AudioStreamInfo>,
    cursor: Option<&str>,
    limit: Option<usize>,
) -> Result<AudioStreamPage, OsControlError> {
    let limit = match limit {
        None => STREAM_PAGE_DEFAULT_ITEMS,
        Some(0) => return Err(invalid("limit", "limit must be at least 1")),
        Some(n) if n > STREAM_PAGE_MAX_ITEMS => {
            return Err(invalid("limit", "limit exceeds the maximum page size"))
        }
        Some(n) => n,
    };
    let offset = match cursor {
        None => 0,
        Some(raw) => decode_cursor(raw)?,
    };

    all.sort_by(|a, b| a.stream.cmp(&b.stream));
    if offset > all.len() {
        return Err(invalid("cursor", "cursor points past the end of the listing"));
    }
    let end = (offset + limit).min(all.len());
    let items = all[offset..end].to_vec();
    let truncated = end < all.len();
    Ok(AudioStreamPage {
        items,
        next_cursor: truncated.then(|| encode_cursor(end)),
        truncated,
    })
}

/// The prior stream state captured before an apply, so a contradiction can be
/// compensated back to the exact prior value.
#[derive(Debug, Clone)]
pub(super) struct StreamRollbackSnapshot {
    pub(super) before: AudioStreamState,
    pub(super) op: AudioStreamOp,
    pub(super) action: String,
    pub(super) params: serde_json::Value,
    pub(super) stream: AudioStreamId,
}

impl<T: AudioTransport> AudioControl<T> {
    /// Observe one stream, or report honestly that it is gone.
    async fn observe_stream(
        &self,
        ctx: &HostExecutionContext,
        request: &AudioStreamRequest,
    ) -> Result<AudioStreamState, OsControlError> {
        match self.transport.read_stream(ctx, &request.stream).await? {
            Some(info) => Ok(AudioStreamState::from_info(&info, request.focus())),
            // The stream ended. Not an unavailable provider, and not a failed
            // mutation — the target itself is gone.
            None => Err(OsControlError::TargetChanged),
        }
    }
}

#[async_trait]
impl<T: AudioTransport> DesiredStateControl<AudioStreamRequest, AudioStreamState>
    for AudioControl<T>
{
    async fn observe(
        &self,
        ctx: &HostExecutionContext,
        request: &AudioStreamRequest,
    ) -> Result<AudioStreamState, OsControlError> {
        self.observe_stream(ctx, request).await
    }

    async fn apply(
        &self,
        ctx: &AdmittedMutationContext<'_>,
        request: &AudioStreamRequest,
        _desired: &AudioStreamState,
    ) -> Result<ApplyOutcome, OsControlError> {
        if let Ok(before) = self.observe_stream(ctx.observation(), request).await {
            let session = ctx.grant().session_id().to_string();
            self.stream_snapshots
                .lock()
                .expect("audio stream snapshots poisoned")
                .insert(
                    session,
                    StreamRollbackSnapshot {
                        before,
                        op: request.op.clone(),
                        action: request.action.clone(),
                        params: request.params.clone(),
                        stream: request.stream.clone(),
                    },
                );
        }

        let steps = vec![AudioStep {
            executable: selection::pulse_executable()?,
            args: request.argv(),
            step: request.step(),
        }];
        self.dispatch_steps(ctx, &request.action, &request.params, steps)
            .await
    }

    async fn verify(
        &self,
        ctx: &HostExecutionContext,
        request: &AudioStreamRequest,
        desired: &AudioStreamState,
    ) -> Result<VerificationReport<AudioStreamState>, OsControlError> {
        let observed = match self.transport.read_stream(ctx, &request.stream).await? {
            Some(info) => AudioStreamState::from_info(&info, request.focus()),
            // The stream ended before it could be verified. The postcondition is
            // now unobservable — reporting it satisfied would claim a fact nobody
            // read, and reporting it contradicted would blame a mutation that may
            // well have applied.
            None => {
                return Ok(VerificationReport::Inconclusive {
                    reason: SafeText::new(
                        "the stream ended before verification; its state is no longer observable",
                    ),
                })
            }
        };

        let satisfied = match request.comparator() {
            ComparatorKind::WithinTolerance => {
                observed.stream == desired.stream
                    && (observed.level_percent as f64 - desired.level_percent as f64).abs()
                        <= self.tolerance
            }
            _ => observed.observation_digest() == desired.observation_digest(),
        };

        if satisfied {
            Ok(VerificationReport::Satisfied(SatisfyingVerification::new(
                crate::os_control::contract::OsEvidenceSource::StructuredCommandQuery,
                VerificationReliability::Strong,
                self.transport.provider_id(),
                RedactedObservation::new(observed.clone(), observed.observation_digest()),
                None,
                SystemTime::now(),
                0,
            )))
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
            .stream_snapshots
            .lock()
            .expect("audio stream snapshots poisoned")
            .get(token.session_id().as_str())
            .cloned();

        let Some(snapshot) = snapshot else {
            return Ok(ApplyOutcome::Uncertain(
                crate::os_control::receipt::UncertainDispatch::new(
                    None,
                    crate::os_control::receipt::UncertainEffectCause::Unobservable,
                    crate::os_control::contract::BoundedVec::new(),
                ),
            ));
        };

        let inverse = match snapshot.op {
            AudioStreamOp::SetLevel(_) => AudioStreamOp::SetLevel(snapshot.before.level_percent),
            AudioStreamOp::SetMute(_) => AudioStreamOp::SetMute(snapshot.before.muted),
        };
        let request = AudioStreamRequest {
            action: snapshot.action.clone(),
            params: snapshot.params.clone(),
            stream: snapshot.stream.clone(),
            op: inverse,
        };
        let steps = vec![AudioStep {
            executable: selection::pulse_executable()?,
            args: request.argv(),
            step: request.step(),
        }];
        self.dispatch_steps(ctx, &snapshot.action, &snapshot.params, steps)
            .await
    }
}

#[cfg(test)]
mod stream_tests {
    use super::*;

    fn info(id: &str, level: u8, muted: bool) -> AudioStreamInfo {
        AudioStreamInfo {
            stream: AudioStreamId::parse(id).expect("valid id"),
            app: "Firefox".into(),
            endpoint: Some(AudioEndpointId::parse("49").expect("valid endpoint")),
            level_percent: level,
            muted,
        }
    }

    #[test]
    fn two_streams_of_the_same_app_have_different_postconditions() {
        let a = AudioStreamRequest {
            action: "set_application_volume".into(),
            params: serde_json::json!({}),
            stream: AudioStreamId::parse("78").unwrap(),
            op: AudioStreamOp::SetLevel(40),
        };
        let b = AudioStreamRequest {
            stream: AudioStreamId::parse("91").unwrap(),
            ..a.clone()
        };
        // Same app, same level, different identity → different postcondition, so
        // one stream's observation can never satisfy the other's.
        assert_ne!(
            a.desired_state().observation_digest(),
            b.desired_state().observation_digest()
        );
    }

    #[test]
    fn a_mute_postcondition_ignores_level_but_not_identity() {
        let muted_loud = AudioStreamState::new(
            AudioStreamId::parse("78").unwrap(),
            90,
            true,
            StreamFocus::Mute,
        );
        let muted_quiet = AudioStreamState::new(
            AudioStreamId::parse("78").unwrap(),
            10,
            true,
            StreamFocus::Mute,
        );
        assert_eq!(
            muted_loud.observation_digest(),
            muted_quiet.observation_digest()
        );
        let other_stream = AudioStreamState::new(
            AudioStreamId::parse("91").unwrap(),
            90,
            true,
            StreamFocus::Mute,
        );
        assert_ne!(
            muted_loud.observation_digest(),
            other_stream.observation_digest()
        );
    }

    #[test]
    fn stream_ids_that_would_be_read_as_options_are_rejected() {
        assert!(AudioStreamId::parse("-1").is_err());
        assert!(AudioStreamId::parse("").is_err());
        assert!(AudioStreamId::parse("78\n--force").is_err());
        assert!(AudioStreamId::parse("78").is_ok());
    }

    #[test]
    fn paging_is_deterministic_and_cursor_checked() {
        let all = vec![info("91", 50, false), info("78", 65, false)];
        let first = page(all.clone(), None, Some(1)).expect("page");
        assert_eq!(first.items.len(), 1);
        // Ordered by id, not by discovery order.
        assert_eq!(first.items[0].stream.as_str(), "78");
        assert!(first.truncated);
        let cursor = first.next_cursor.clone().expect("cursor when truncated");
        let second = page(all.clone(), Some(&cursor), Some(1)).expect("second page");
        assert_eq!(second.items[0].stream.as_str(), "91");
        assert!(!second.truncated);
        // A forged cursor is rejected, never silently treated as offset 0.
        assert!(page(all.clone(), Some("as1.0.0000000000000000"), None).is_err());
        assert!(page(all, Some("nope"), None).is_err());
    }

    #[test]
    fn an_empty_stream_list_is_an_answer() {
        let empty = page(Vec::new(), None, None).expect("page");
        assert!(empty.items.is_empty());
        assert!(!empty.truncated);
        assert!(empty.next_cursor.is_none());
    }
}
