//! Deny-live fake [`AudioTransport`] (OSC-010, OSC-033), Tasks 2.1 / 3.6.
//!
//! Compiled only under `os-control-test`. Reads are served from a scripted
//! `(volume, muted)` pair and `dispatch` records the structured command instead
//! of running it, so no `wpctl`/`pactl`/`amixer` process is ever spawned and no
//! real endpoint changes. A read can be scripted to fail so the "parse ambiguity
//! must surface as an error, never a fabricated state" rule is testable.

use std::collections::VecDeque;
use std::sync::Mutex;

use async_trait::async_trait;

use crate::os_control::context::{AdmittedMutationContext, HostExecutionContext};
use crate::os_control::contract::{BoundedVec, Digest, ProviderId, SafeText};
use crate::os_control::error::OsControlError;
use crate::os_control::linux::structured_command::StructuredCommandRequest;
use crate::os_control::receipt::{AppliedDispatch, ApplyOutcome};

use super::media::MediaPlayerInfo;
use super::selection::AudioBackend;
use super::streams::AudioStreamInfo;
use super::{
    AudioEndpointId, AudioEndpointKind, AudioProfileId, AudioStreamId, AudioTransport,
    MediaPlayerId,
};

/// Provider identity reported by the fake transport.
pub const FAKE_AUDIO_PROVIDER_ID: &str = "fake-audio";

/// A scripted, in-memory audio transport.
///
/// Reads are a FIFO queue because one governed mutation performs several in a
/// fixed order (pre-observation → under-lease re-observation → pre-apply
/// snapshot → post-apply re-observation → verify). Script them with successive
/// [`Self::read_ok`] calls. When the queue is exhausted the last scripted value
/// is reused, so a test that only cares about one steady state can script once.
pub struct FakeAudioTransport {
    backend: AudioBackend,
    scripted: Mutex<VecDeque<(u8, bool)>>,
    last: Mutex<Option<(u8, bool)>>,
    read_failure: Option<String>,
    outcome: Mutex<Option<ApplyOutcome>>,
    dispatched: Mutex<Vec<StructuredCommandRequest>>,
    reads: Mutex<Vec<AudioEndpointKind>>,
    /// Which endpoint the fake reports as default, per kind. Unscripted means a
    /// read fails closed rather than inventing a device.
    defaults: Mutex<Vec<(AudioEndpointKind, AudioEndpointId)>>,
    /// Scripted card profiles. An entry mapping to `None` models a card that is
    /// genuinely absent, which is a different fact from an unscripted read.
    profiles: Mutex<Vec<(String, Option<AudioProfileId>)>>,
    /// Scripted stream table. `None` means unscripted → fail closed.
    streams: Mutex<Option<Vec<AudioStreamInfo>>>,
    /// Scripted MPRIS player table. `None` means unscripted → fail closed.
    players: Mutex<Option<Vec<MediaPlayerInfo>>>,
}

impl FakeAudioTransport {
    /// A fake on `backend` with nothing scripted yet; a read fails closed until
    /// [`Self::read_ok`] is called.
    #[must_use]
    pub fn new(backend: AudioBackend) -> Self {
        Self {
            backend,
            scripted: Mutex::new(VecDeque::new()),
            last: Mutex::new(None),
            read_failure: None,
            outcome: Mutex::new(None),
            dispatched: Mutex::new(Vec::new()),
            reads: Mutex::new(Vec::new()),
            defaults: Mutex::new(Vec::new()),
            profiles: Mutex::new(Vec::new()),
            streams: Mutex::new(None),
            players: Mutex::new(None),
        }
    }

    /// Builder: report `id` as the default endpoint of `kind`.
    #[must_use]
    pub fn default_endpoint_ok(self, kind: AudioEndpointKind, id: AudioEndpointId) -> Self {
        self.defaults.lock().expect("defaults mutex").push((kind, id));
        self
    }

    /// Builder: script a card's active profile. `None` models a card that is not
    /// present at all.
    #[must_use]
    pub fn profile_ok(self, card: impl Into<String>, profile: Option<AudioProfileId>) -> Self {
        self.profiles
            .lock()
            .expect("profiles mutex")
            .push((card.into(), profile));
        self
    }

    /// Builder: script the whole stream table (an empty table is a valid answer).
    #[must_use]
    pub fn streams_ok(self, streams: Vec<AudioStreamInfo>) -> Self {
        *self.streams.lock().expect("streams mutex") = Some(streams);
        self
    }

    /// Builder: script the whole MPRIS player table.
    #[must_use]
    pub fn players_ok(self, players: Vec<MediaPlayerInfo>) -> Self {
        *self.players.lock().expect("players mutex") = Some(players);
        self
    }

    /// Builder: queue the next read as `percent` volume and `muted`.
    #[must_use]
    pub fn read_ok(self, percent: u8, muted: bool) -> Self {
        self.scripted
            .lock()
            .expect("scripted mutex")
            .push_back((percent, muted));
        self
    }

    /// Builder: make every read fail, proving an ambiguous parse never becomes a
    /// fabricated state.
    #[must_use]
    pub fn read_failure(mut self, reason: impl Into<String>) -> Self {
        self.read_failure = Some(reason.into());
        self
    }

    /// Builder: script the outcome `dispatch` returns.
    #[must_use]
    pub fn dispatch_outcome(self, outcome: ApplyOutcome) -> Self {
        *self.outcome.lock().expect("outcome mutex") = Some(outcome);
        self
    }

    /// The structured commands this fake captured instead of executing, in order.
    #[must_use]
    pub fn captured(&self) -> Vec<StructuredCommandRequest> {
        self.dispatched.lock().expect("dispatch mutex").clone()
    }

    /// How many dispatches were requested.
    #[must_use]
    pub fn dispatch_count(&self) -> usize {
        self.dispatched.lock().expect("dispatch mutex").len()
    }

    /// The endpoints read, in order.
    #[must_use]
    pub fn reads(&self) -> Vec<AudioEndpointKind> {
        self.reads.lock().expect("reads mutex").clone()
    }

    /// How many reads were served.
    #[must_use]
    pub fn read_count(&self) -> usize {
        self.reads.lock().expect("reads mutex").len()
    }

    /// The error an unscripted read returns. Never a value: a fake that invented
    /// state would let a test prove a mutation verified against a fact nobody read.
    fn unscripted(&self, reason: &str) -> OsControlError {
        OsControlError::Unavailable {
            provider: Some(ProviderId::new(FAKE_AUDIO_PROVIDER_ID)),
            reason: SafeText::new(reason),
            retryable: false,
        }
    }
}

#[async_trait]
impl AudioTransport for FakeAudioTransport {
    fn provider_id(&self) -> ProviderId {
        ProviderId::new(FAKE_AUDIO_PROVIDER_ID)
    }

    fn selected_backend(&self) -> AudioBackend {
        self.backend
    }

    async fn read_state(
        &self,
        _ctx: &HostExecutionContext,
        endpoint: AudioEndpointKind,
    ) -> Result<(u8, bool), OsControlError> {
        self.reads.lock().expect("reads mutex").push(endpoint);
        if let Some(reason) = &self.read_failure {
            return Err(OsControlError::Unavailable {
                provider: Some(ProviderId::new(FAKE_AUDIO_PROVIDER_ID)),
                reason: SafeText::new(reason.clone()),
                retryable: true,
            });
        }
        // Serve the next scripted read; when the queue is drained, hold the last
        // observed value (a steady state) rather than inventing a new one.
        let next = self.scripted.lock().expect("scripted mutex").pop_front();
        let mut last = self.last.lock().expect("last mutex");
        if let Some(value) = next {
            *last = Some(value);
        }
        // Fail closed when nothing was ever scripted: never invent a state.
        last.ok_or_else(|| OsControlError::Unavailable {
            provider: Some(ProviderId::new(FAKE_AUDIO_PROVIDER_ID)),
            reason: SafeText::new("no audio state scripted on the fake transport"),
            retryable: false,
        })
    }

    async fn read_default_endpoint(
        &self,
        _ctx: &HostExecutionContext,
        endpoint: AudioEndpointKind,
    ) -> Result<AudioEndpointId, OsControlError> {
        self.reads.lock().expect("reads mutex").push(endpoint);
        self.defaults
            .lock()
            .expect("defaults mutex")
            .iter()
            .find(|(kind, _)| *kind == endpoint)
            .map(|(_, id)| id.clone())
            .ok_or_else(|| self.unscripted("no default endpoint scripted on the fake transport"))
    }

    async fn read_active_profile(
        &self,
        _ctx: &HostExecutionContext,
        endpoint: &AudioEndpointId,
    ) -> Result<Option<AudioProfileId>, OsControlError> {
        self.profiles
            .lock()
            .expect("profiles mutex")
            .iter()
            .find(|(card, _)| card == endpoint.as_str())
            .map(|(_, profile)| profile.clone())
            .ok_or_else(|| self.unscripted("no card profile scripted on the fake transport"))
    }

    async fn list_streams(
        &self,
        _ctx: &HostExecutionContext,
    ) -> Result<Vec<AudioStreamInfo>, OsControlError> {
        self.streams
            .lock()
            .expect("streams mutex")
            .clone()
            .ok_or_else(|| self.unscripted("no stream table scripted on the fake transport"))
    }

    async fn read_stream(
        &self,
        _ctx: &HostExecutionContext,
        stream: &AudioStreamId,
    ) -> Result<Option<AudioStreamInfo>, OsControlError> {
        let table = self
            .streams
            .lock()
            .expect("streams mutex")
            .clone()
            .ok_or_else(|| self.unscripted("no stream table scripted on the fake transport"))?;
        // Absent from a table that WAS read is "the stream is gone", not "unknown".
        Ok(table.into_iter().find(|info| &info.stream == stream))
    }

    async fn list_media_players(
        &self,
        _ctx: &HostExecutionContext,
    ) -> Result<Vec<MediaPlayerInfo>, OsControlError> {
        self.players
            .lock()
            .expect("players mutex")
            .clone()
            .ok_or_else(|| self.unscripted("no player table scripted on the fake transport"))
    }

    async fn read_media_player(
        &self,
        _ctx: &HostExecutionContext,
        player: &MediaPlayerId,
    ) -> Result<Option<MediaPlayerInfo>, OsControlError> {
        let table = self
            .players
            .lock()
            .expect("players mutex")
            .clone()
            .ok_or_else(|| self.unscripted("no player table scripted on the fake transport"))?;
        Ok(table.into_iter().find(|info| &info.player == player))
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
            .push(request.clone());
        if let Some(outcome) = self.outcome.lock().expect("outcome mutex").clone() {
            return Ok(outcome);
        }
        Ok(ApplyOutcome::Applied(AppliedDispatch::new(
            Some(Digest::of_str(crate::os_control::testing::FAKE_RECEIPT_TAG)),
            BoundedVec::new(),
        )))
    }
}
