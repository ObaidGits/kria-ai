//! Live PipeWire/WirePlumber audio adapter (raw transport seam).
//!
//! linux-os-control-production **Task 2.1** (OSC-018, OSC-031, OSC-033), design
//! §3 (`linux/providers/pipewire.rs`).
//!
//! # Host safety
//!
//! Driving the audio stack (`wpctl`/`pactl`/`amixer`) is a **raw live
//! transport**. Like [`crate::os_control::linux::dbus`] and
//! [`crate::os_control::linux::providers::secret_service`], this adapter:
//!
//! 1. can be constructed **only** with a
//!    [`crate::os_control::access::LiveHostAccessToken`] (mintable solely in a
//!    live composition root under `os-control-live`), so no completion test can
//!    build it; and
//! 2. calls [`crate::os_control::access::deny_live_transport`] **before** any
//!    read or dispatch, so a deny-live (`os-control-test`) build that reached
//!    here would trip the sentinel and abort rather than run a child process.
//!
//! Reads run through [`StructuredQueryRequest`] and mutations through
//! [`StructuredCommandRequest`], so both inherit the same containment: a trusted
//! absolute executable, an exact digested argv, a hermetic environment, a pinned
//! `C` locale, bounded output, a deadline and cancellation. There is no ungoverned
//! subprocess fallback anywhere in this file. Unparseable output fails closed
//! rather than defaulting, because a fabricated observation would let a mutation
//! "verify" against a fact that was never read. Deny-live tests inject
//! [`crate::os_control::audio::fake::FakeAudioTransport`].

use crate::os_control::access::{deny_live_transport, LiveHostAccessToken, RawTransportKind};
use crate::os_control::audio::media::{self, MediaPlayerInfo};
use crate::os_control::audio::selection::{
    self, parse_mute_output, parse_volume_output, query_mute_argv, query_volume_argv,
};
use crate::os_control::audio::streams::AudioStreamInfo;
use crate::os_control::audio::{
    AudioBackend, AudioEndpointId, AudioEndpointKind, AudioProfileId, AudioStreamId, AudioTransport,
    MediaPlayerId,
};
use crate::os_control::context::{AdmittedMutationContext, HostExecutionContext};
use crate::os_control::contract::{CapabilityId, ProviderId, SafeText};
use crate::os_control::error::OsControlError;
use crate::os_control::linux::structured_command::{
    CommandPlan, CommandPolicy, StructuredCommandRequest, TrustedExecutable,
};
use crate::os_control::linux::structured_query::StructuredQueryRequest;
use crate::os_control::receipt::ApplyOutcome;

/// The live PipeWire/WirePlumber audio adapter. Constructible only in a live
/// composition; a value cannot exist under `os-control-test`.
pub struct LivePipewireAudio {
    backend: AudioBackend,
    _seal: (),
}

impl LivePipewireAudio {
    /// Construct in a live composition root over a selected backend. Requires a
    /// [`LiveHostAccessToken`], so no completion test can build one.
    #[must_use]
    pub fn new(_token: &LiveHostAccessToken, backend: AudioBackend) -> Self {
        Self {
            backend,
            _seal: (),
        }
    }

    /// Run one governed observation and return its bounded stdout.
    ///
    /// Reads go through [`StructuredQueryRequest`] rather than a bare
    /// `Command`, so an observation inherits the same trusted-executable, exact
    /// argv, hermetic environment, output-bound, deadline and cancellation
    /// discipline a mutation gets. A read simply has no grant to seal against,
    /// because it changes nothing.
    async fn query(
        &self,
        ctx: &HostExecutionContext,
        action: &str,
        argv: Vec<String>,
    ) -> Result<String, OsControlError> {
        let executable = self.backend.trusted_executable()?;
        self.query_with(ctx, action, executable, argv).await
    }

    /// Run one governed observation on an explicit trusted executable.
    ///
    /// Named endpoints, per-application streams and card profiles are read
    /// through `pactl`, and MPRIS players through `gdbus`, regardless of which
    /// CLI was selected for volume — see
    /// [`crate::os_control::audio::selection`] for why those identities are only
    /// available there. Both still travel this same governed query seam.
    async fn query_with(
        &self,
        ctx: &HostExecutionContext,
        action: &str,
        executable: TrustedExecutable,
        argv: Vec<String>,
    ) -> Result<String, OsControlError> {
        let plan = CommandPlan::new(
            CapabilityId::new(action),
            action,
            serde_json::Value::Null,
            executable,
            argv,
        );
        let request = StructuredQueryRequest::from_observation(ctx, plan, &CommandPolicy::new())?;
        let output = request.run().await?;
        if output.truncated {
            // A truncated observation must never be parsed as if complete.
            return Err(OsControlError::Unavailable {
                provider: Some(self.provider_id()),
                reason: SafeText::new("audio state output was truncated; refusing a partial read"),
                retryable: true,
            });
        }
        Ok(output.stdout)
    }

    /// Guard the operations that need the PulseAudio-compatibility surface.
    ///
    /// `amixer` is only ever selected when neither WirePlumber nor the
    /// PulseAudio-compatibility CLI was found, and ALSA has no concept of a
    /// named default endpoint, a per-application stream or a card profile. That
    /// is a fact about the backend, so it is `Unsupported` — not an
    /// `Unavailable` that invites a pointless retry, and never a fabricated
    /// reading.
    fn require_pulse_surface(&self, capability: &str) -> Result<TrustedExecutable, OsControlError> {
        if self.backend == AudioBackend::Amixer {
            return Err(OsControlError::Unsupported {
                capability: CapabilityId::new(capability),
                reason: SafeText::new(
                    "the ALSA mixer exposes no named endpoints, per-application streams or card profiles, and no PipeWire/PulseAudio control surface was found",
                ),
            });
        }
        selection::pulse_executable()
    }

    /// Read the whole per-application stream table.
    async fn read_stream_table(
        &self,
        ctx: &HostExecutionContext,
    ) -> Result<Vec<AudioStreamInfo>, OsControlError> {
        deny_live_transport(RawTransportKind::Process);
        let executable = self.require_pulse_surface("list_audio_streams")?;
        let stdout = self
            .query_with(
                ctx,
                "list_audio_streams",
                executable,
                selection::query_streams_argv(),
            )
            .await?;
        let raw = selection::parse_sink_inputs(&stdout)?;
        let mut out = Vec::with_capacity(raw.len());
        for stream in raw {
            let endpoint = match stream.endpoint {
                Some(id) => Some(AudioEndpointId::parse(id)?),
                None => None,
            };
            out.push(AudioStreamInfo {
                stream: AudioStreamId::parse(stream.id)?,
                app: stream.app,
                endpoint,
                level_percent: stream.level_percent,
                muted: stream.muted,
            });
        }
        Ok(out)
    }

    /// Read the whole MPRIS player table from the session bus.
    ///
    /// A player that leaves the bus mid-enumeration makes its property read
    /// fail, which fails the whole listing. That is deliberate: silently
    /// dropping it would be indistinguishable from a read that could not be
    /// performed at all.
    async fn read_player_table(
        &self,
        ctx: &HostExecutionContext,
    ) -> Result<Vec<MediaPlayerInfo>, OsControlError> {
        deny_live_transport(RawTransportKind::Process);
        let names = self
            .query_with(
                ctx,
                "list_media_players",
                media::gdbus_executable()?,
                media::list_names_argv(),
            )
            .await?;
        let buses = media::mpris_names(&media::parse_bus_names(&names)?);

        let mut out = Vec::with_capacity(buses.len());
        for bus in buses.into_iter().take(media::MEDIA_PAGE_MAX_ITEMS) {
            let player = media::parse_player_id(&bus)?;
            let status = self
                .query_with(
                    ctx,
                    "list_media_players",
                    media::gdbus_executable()?,
                    media::get_property_argv(&bus, "PlaybackStatus"),
                )
                .await?;
            let metadata = self
                .query_with(
                    ctx,
                    "list_media_players",
                    media::gdbus_executable()?,
                    media::get_property_argv(&bus, "Metadata"),
                )
                .await?;
            out.push(MediaPlayerInfo {
                app: media::app_label(&player),
                player,
                playback_state: media::parse_playback_status(&status)?,
                track_label: media::parse_track_title(&metadata)?,
                track_id: media::parse_track_id(&metadata)?,
            });
        }
        Ok(out)
    }
}

#[async_trait::async_trait]
impl AudioTransport for LivePipewireAudio {
    fn provider_id(&self) -> ProviderId {
        ProviderId::new(format!("pipewire-{}", self.backend.as_str()))
    }

    fn selected_backend(&self) -> AudioBackend {
        self.backend
    }

    async fn read_state(
        &self,
        ctx: &HostExecutionContext,
        endpoint: AudioEndpointKind,
    ) -> Result<(u8, bool), OsControlError> {
        // A state read runs a query child process.
        deny_live_transport(RawTransportKind::Process);

        // The argv is endpoint-specific (default sink vs default source, or the
        // `Master` vs `Capture` ALSA control), so an input read can never return
        // output facts.
        let (percent, mute_from_volume) = self
            .query(ctx, "get_audio_state", query_volume_argv(self.backend, endpoint))
            .await
            .and_then(|out| parse_volume_output(self.backend, &out))?;

        // `wpctl`/`amixer` report mute in the same call; `pactl` needs a second
        // query. A backend that reported neither must fail rather than guess.
        //
        // For the microphone this is a privacy guarantee, not a nicety: reporting
        // "muted" for a mute state nobody read would tell the user their
        // microphone is off while it is live.
        let muted = match mute_from_volume {
            Some(muted) => muted,
            None => {
                let argv = query_mute_argv(self.backend, endpoint).ok_or_else(|| {
                    OsControlError::Unavailable {
                        provider: Some(self.provider_id()),
                        reason: SafeText::new(
                            "backend reported no mute state and offers no mute query",
                        ),
                        retryable: false,
                    }
                })?;
                self.query(ctx, "get_audio_state", argv)
                    .await
                    .and_then(|out| parse_mute_output(self.backend, &out))?
            }
        };

        Ok((percent, muted))
    }

    async fn read_default_endpoint(
        &self,
        ctx: &HostExecutionContext,
        endpoint: AudioEndpointKind,
    ) -> Result<AudioEndpointId, OsControlError> {
        deny_live_transport(RawTransportKind::Process);
        let capability = match endpoint {
            AudioEndpointKind::Output => "set_default_audio_output",
            AudioEndpointKind::Input => "set_default_audio_input",
        };
        let executable = self.require_pulse_surface(capability)?;
        let stdout = self
            .query_with(
                ctx,
                capability,
                executable,
                selection::query_default_endpoint_argv(endpoint),
            )
            .await?;
        AudioEndpointId::parse(selection::parse_default_endpoint(&stdout)?)
    }

    async fn read_active_profile(
        &self,
        ctx: &HostExecutionContext,
        endpoint: &AudioEndpointId,
    ) -> Result<Option<AudioProfileId>, OsControlError> {
        deny_live_transport(RawTransportKind::Process);
        let executable = self.require_pulse_surface("set_audio_device_profile")?;
        let stdout = self
            .query_with(
                ctx,
                "set_audio_device_profile",
                executable,
                selection::query_cards_argv(),
            )
            .await?;
        match selection::parse_active_card_profile(&stdout, endpoint.as_str())? {
            Some(profile) => Ok(Some(AudioProfileId::parse(profile)?)),
            // The card is genuinely not present — an answer, not a failure.
            None => Ok(None),
        }
    }

    async fn list_streams(
        &self,
        ctx: &HostExecutionContext,
    ) -> Result<Vec<AudioStreamInfo>, OsControlError> {
        self.read_stream_table(ctx).await
    }

    async fn read_stream(
        &self,
        ctx: &HostExecutionContext,
        stream: &AudioStreamId,
    ) -> Result<Option<AudioStreamInfo>, OsControlError> {
        // The table was read successfully, so absence from it is the fact that
        // the stream ended — not an unknown.
        let table = self.read_stream_table(ctx).await?;
        Ok(table.into_iter().find(|info| &info.stream == stream))
    }

    async fn list_media_players(
        &self,
        ctx: &HostExecutionContext,
    ) -> Result<Vec<MediaPlayerInfo>, OsControlError> {
        self.read_player_table(ctx).await
    }

    async fn read_media_player(
        &self,
        ctx: &HostExecutionContext,
        player: &MediaPlayerId,
    ) -> Result<Option<MediaPlayerInfo>, OsControlError> {
        deny_live_transport(RawTransportKind::Process);
        // Presence is decided by the bus's own authoritative name list, so a
        // player that left the bus is reported absent rather than as a failed
        // property read.
        let names = self
            .query_with(
                ctx,
                "control_media_playback",
                media::gdbus_executable()?,
                media::list_names_argv(),
            )
            .await?;
        if !media::parse_bus_names(&names)?
            .iter()
            .any(|name| name == player.as_str())
        {
            return Ok(None);
        }

        let status = self
            .query_with(
                ctx,
                "control_media_playback",
                media::gdbus_executable()?,
                media::get_property_argv(player.as_str(), "PlaybackStatus"),
            )
            .await?;
        let metadata = self
            .query_with(
                ctx,
                "control_media_playback",
                media::gdbus_executable()?,
                media::get_property_argv(player.as_str(), "Metadata"),
            )
            .await?;
        Ok(Some(MediaPlayerInfo {
            app: media::app_label(player),
            player: player.clone(),
            playback_state: media::parse_playback_status(&status)?,
            track_label: media::parse_track_title(&metadata)?,
            track_id: media::parse_track_id(&metadata)?,
        }))
    }

    async fn dispatch(
        &self,
        _ctx: &AdmittedMutationContext<'_>,
        request: &StructuredCommandRequest,
    ) -> Result<ApplyOutcome, OsControlError> {
        // The governed request's own launch trips the deny-live sentinel; keep an
        // explicit guard here too so the adapter is unreachable under test.
        deny_live_transport(RawTransportKind::Process);
        request.dispatch().await
    }
}
