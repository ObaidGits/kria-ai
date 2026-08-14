//! Audio backend selection and captured-argv construction.
//!
//! linux-os-control-production **Task 2.1** (OSC-018, OSC-031), design §3.
//!
//! Provider selection prefers the WirePlumber/PipeWire control CLI (`wpctl`),
//! then the PulseAudio compatibility CLI (`pactl`), then the ALSA mixer
//! (`amixer`) as declared **degraded** providers. Each backend has a fixed,
//! trusted absolute executable and a pure argv builder, so the exact command
//! line is testable without launching a process (the governed argv executor in
//! [`crate::os_control::linux::structured_command`] is the only thing that ever
//! dispatches it).

use crate::os_control::audio::AudioEndpointKind;
use crate::os_control::contract::Digest;
use crate::os_control::linux::structured_command::TrustedExecutable;
use crate::os_control::OsControlError;

/// The concrete host audio backend a provider selected. The string form is kept
/// **compatible with the pre-migration `set_volume` `backend` field**
/// (`"wpctl"` / `"pactl"` / `"amixer"`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AudioBackend {
    /// WirePlumber control CLI (the PipeWire session-manager front-end). Preferred.
    Wpctl,
    /// PulseAudio-compatibility control CLI. Degraded.
    Pactl,
    /// ALSA mixer CLI. Degraded (last resort).
    Amixer,
}

impl AudioBackend {
    /// The full, ordered preference list (most preferred first).
    pub const PREFERENCE: [AudioBackend; 3] =
        [AudioBackend::Wpctl, AudioBackend::Pactl, AudioBackend::Amixer];

    /// The stable label used in the `backend` result field and traces.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            AudioBackend::Wpctl => "wpctl",
            AudioBackend::Pactl => "pactl",
            AudioBackend::Amixer => "amixer",
        }
    }

    /// Whether this backend is a declared **degraded** provider (not the
    /// preferred authoritative path).
    #[must_use]
    pub fn is_degraded(self) -> bool {
        !matches!(self, AudioBackend::Wpctl)
    }

    /// The trusted absolute executable path for this backend.
    #[must_use]
    pub fn executable_path(self) -> &'static str {
        match self {
            AudioBackend::Wpctl => "/usr/bin/wpctl",
            AudioBackend::Pactl => "/usr/bin/pactl",
            AudioBackend::Amixer => "/usr/bin/amixer",
        }
    }

    /// A stable trusted-executable identity used by the fallback adapter. Live
    /// transports compare the on-disk identity against this to detect drift; the
    /// deny-live provider tests use it directly.
    #[must_use]
    pub fn trusted_executable(self) -> Result<TrustedExecutable, OsControlError> {
        TrustedExecutable::new(
            self.executable_path(),
            Digest::of_str(&format!("{}-fallback-v1", self.as_str())),
        )
    }

    /// The default endpoint selector token for this backend.
    ///
    /// Output is the default sink, input the default source. `amixer` has no
    /// endpoint concept at all, so it uses its conventional simple-control
    /// names (`Master` for playback, `Capture` for capture) — the two are
    /// different controls, never the same one read twice.
    #[must_use]
    fn default_target(self, endpoint: AudioEndpointKind) -> &'static str {
        match (self, endpoint) {
            (AudioBackend::Wpctl, AudioEndpointKind::Output) => "@DEFAULT_AUDIO_SINK@",
            (AudioBackend::Wpctl, AudioEndpointKind::Input) => "@DEFAULT_AUDIO_SOURCE@",
            (AudioBackend::Pactl, AudioEndpointKind::Output) => "@DEFAULT_SINK@",
            (AudioBackend::Pactl, AudioEndpointKind::Input) => "@DEFAULT_SOURCE@",
            (AudioBackend::Amixer, AudioEndpointKind::Output) => "Master",
            (AudioBackend::Amixer, AudioEndpointKind::Input) => "Capture",
        }
    }
}

/// Select the most-preferred available backend, or `None` when no session audio
/// backend is present (→ the provider reports `Unavailable`).
#[must_use]
pub fn select_backend(available: &[AudioBackend]) -> Option<AudioBackend> {
    AudioBackend::PREFERENCE
        .into_iter()
        .find(|candidate| available.contains(candidate))
}

/// The argv for reading an endpoint's volume (and, where the backend emits it in
/// the same call, its mute state).
#[must_use]
pub fn query_volume_argv(backend: AudioBackend, endpoint: AudioEndpointKind) -> Vec<String> {
    let target = backend.default_target(endpoint);
    match (backend, endpoint) {
        (AudioBackend::Wpctl, _) => vec!["get-volume".into(), target.into()],
        (AudioBackend::Pactl, AudioEndpointKind::Output) => {
            vec!["get-sink-volume".into(), target.into()]
        }
        (AudioBackend::Pactl, AudioEndpointKind::Input) => {
            vec!["get-source-volume".into(), target.into()]
        }
        (AudioBackend::Amixer, _) => vec!["get".into(), target.into()],
    }
}

/// The argv for reading an endpoint's mute state. `wpctl`/`amixer` report mute
/// alongside the volume query, so a separate query is only needed for `pactl`;
/// those backends return `None` here.
#[must_use]
pub fn query_mute_argv(
    backend: AudioBackend,
    endpoint: AudioEndpointKind,
) -> Option<Vec<String>> {
    let target = backend.default_target(endpoint);
    match (backend, endpoint) {
        (AudioBackend::Pactl, AudioEndpointKind::Output) => {
            Some(vec!["get-sink-mute".into(), target.into()])
        }
        (AudioBackend::Pactl, AudioEndpointKind::Input) => {
            Some(vec!["get-source-mute".into(), target.into()])
        }
        (AudioBackend::Wpctl | AudioBackend::Amixer, _) => None,
    }
}

/// The argv for setting an endpoint's volume to `percent`.
#[must_use]
pub fn set_volume_argv(
    backend: AudioBackend,
    endpoint: AudioEndpointKind,
    percent: u8,
) -> Vec<String> {
    let target = backend.default_target(endpoint);
    let value = format!("{percent}%");
    match (backend, endpoint) {
        (AudioBackend::Wpctl, _) => vec!["set-volume".into(), target.into(), value],
        (AudioBackend::Pactl, AudioEndpointKind::Output) => {
            vec!["set-sink-volume".into(), target.into(), value]
        }
        (AudioBackend::Pactl, AudioEndpointKind::Input) => {
            vec!["set-source-volume".into(), target.into(), value]
        }
        // Keep unmute implicit on an output volume change (matches prior
        // behaviour), but as separate literal argv tokens (no shell string).
        (AudioBackend::Amixer, AudioEndpointKind::Output) => {
            vec!["set".into(), target.into(), value, "unmute".into()]
        }
        // A capture control must NOT be implicitly un-muted by a level change:
        // silently activating the microphone is a privacy event, and the caller
        // asked only for a level.
        (AudioBackend::Amixer, AudioEndpointKind::Input) => {
            vec!["set".into(), target.into(), value]
        }
    }
}

/// The argv for setting an endpoint's mute state.
#[must_use]
pub fn set_mute_argv(
    backend: AudioBackend,
    endpoint: AudioEndpointKind,
    muted: bool,
) -> Vec<String> {
    let target = backend.default_target(endpoint);
    let flag = if muted { "1" } else { "0" };
    match (backend, endpoint) {
        (AudioBackend::Wpctl, _) => {
            vec!["set-mute".into(), target.into(), flag.into()]
        }
        (AudioBackend::Pactl, AudioEndpointKind::Output) => {
            vec!["set-sink-mute".into(), target.into(), flag.into()]
        }
        (AudioBackend::Pactl, AudioEndpointKind::Input) => {
            vec!["set-source-mute".into(), target.into(), flag.into()]
        }
        (AudioBackend::Amixer, AudioEndpointKind::Output) => vec![
            "set".into(),
            target.into(),
            if muted { "mute".into() } else { "unmute".into() },
        ],
        // ALSA capture switches are `cap`/`nocap`, not `mute`/`unmute`.
        (AudioBackend::Amixer, AudioEndpointKind::Input) => vec![
            "set".into(),
            target.into(),
            if muted { "nocap".into() } else { "cap".into() },
        ],
    }
}

#[cfg(all(test, feature = "os-control-test"))]
mod tests {
    use super::*;

    #[test]
    fn selection_matrix_prefers_wpctl_then_pactl_then_amixer() {
        use AudioBackend::*;
        let cases: &[(&[AudioBackend], Option<AudioBackend>)] = &[
            (&[Wpctl, Pactl, Amixer], Some(Wpctl)),
            (&[Pactl, Amixer], Some(Pactl)),
            (&[Amixer], Some(Amixer)),
            (&[Amixer, Pactl], Some(Pactl)), // order-independent
            (&[], None),
        ];
        for (available, expected) in cases {
            assert_eq!(select_backend(available), *expected, "available {available:?}");
        }
    }

    #[test]
    fn degraded_classification() {
        assert!(!AudioBackend::Wpctl.is_degraded());
        assert!(AudioBackend::Pactl.is_degraded());
        assert!(AudioBackend::Amixer.is_degraded());
    }

    #[test]
    fn captured_query_argv_golden() {
        use AudioEndpointKind::{Input, Output};
        assert_eq!(
            query_volume_argv(AudioBackend::Wpctl, Output),
            vec!["get-volume", "@DEFAULT_AUDIO_SINK@"]
        );
        assert_eq!(
            query_volume_argv(AudioBackend::Wpctl, Input),
            vec!["get-volume", "@DEFAULT_AUDIO_SOURCE@"]
        );
        assert_eq!(
            query_volume_argv(AudioBackend::Pactl, Output),
            vec!["get-sink-volume", "@DEFAULT_SINK@"]
        );
        assert_eq!(
            query_volume_argv(AudioBackend::Pactl, Input),
            vec!["get-source-volume", "@DEFAULT_SOURCE@"]
        );
        assert_eq!(query_volume_argv(AudioBackend::Amixer, Output), vec!["get", "Master"]);
        assert_eq!(query_volume_argv(AudioBackend::Amixer, Input), vec!["get", "Capture"]);

        assert_eq!(
            query_mute_argv(AudioBackend::Pactl, Output),
            Some(vec!["get-sink-mute".to_string(), "@DEFAULT_SINK@".to_string()])
        );
        assert_eq!(
            query_mute_argv(AudioBackend::Pactl, Input),
            Some(vec![
                "get-source-mute".to_string(),
                "@DEFAULT_SOURCE@".to_string()
            ])
        );
        assert_eq!(query_mute_argv(AudioBackend::Wpctl, Output), None);
        assert_eq!(query_mute_argv(AudioBackend::Amixer, Input), None);
    }

    #[test]
    fn captured_set_volume_argv_golden() {
        use AudioEndpointKind::{Input, Output};
        assert_eq!(
            set_volume_argv(AudioBackend::Wpctl, Output, 60),
            vec!["set-volume", "@DEFAULT_AUDIO_SINK@", "60%"]
        );
        assert_eq!(
            set_volume_argv(AudioBackend::Wpctl, Input, 60),
            vec!["set-volume", "@DEFAULT_AUDIO_SOURCE@", "60%"]
        );
        assert_eq!(
            set_volume_argv(AudioBackend::Pactl, Output, 0),
            vec!["set-sink-volume", "@DEFAULT_SINK@", "0%"]
        );
        assert_eq!(
            set_volume_argv(AudioBackend::Pactl, Input, 0),
            vec!["set-source-volume", "@DEFAULT_SOURCE@", "0%"]
        );
        assert_eq!(
            set_volume_argv(AudioBackend::Amixer, Output, 100),
            vec!["set", "Master", "100%", "unmute"]
        );
    }

    #[test]
    fn a_capture_level_change_never_implicitly_unmutes_the_microphone() {
        // Raising the mic level must not smuggle in an unmute: activating capture
        // is a privacy event the caller did not request.
        let argv = set_volume_argv(AudioBackend::Amixer, AudioEndpointKind::Input, 100);
        assert_eq!(argv, vec!["set", "Capture", "100%"]);
        assert!(!argv.iter().any(|a| a == "unmute" || a == "cap"));
    }

    #[test]
    fn captured_set_mute_argv_golden() {
        use AudioEndpointKind::{Input, Output};
        assert_eq!(
            set_mute_argv(AudioBackend::Wpctl, Output, true),
            vec!["set-mute", "@DEFAULT_AUDIO_SINK@", "1"]
        );
        assert_eq!(
            set_mute_argv(AudioBackend::Wpctl, Output, false),
            vec!["set-mute", "@DEFAULT_AUDIO_SINK@", "0"]
        );
        assert_eq!(
            set_mute_argv(AudioBackend::Wpctl, Input, true),
            vec!["set-mute", "@DEFAULT_AUDIO_SOURCE@", "1"]
        );
        assert_eq!(
            set_mute_argv(AudioBackend::Pactl, Input, false),
            vec!["set-source-mute", "@DEFAULT_SOURCE@", "0"]
        );
        assert_eq!(
            set_mute_argv(AudioBackend::Amixer, Output, true),
            vec!["set", "Master", "mute"]
        );
        assert_eq!(
            set_mute_argv(AudioBackend::Amixer, Output, false),
            vec!["set", "Master", "unmute"]
        );
        // ALSA capture switches use cap/nocap.
        assert_eq!(
            set_mute_argv(AudioBackend::Amixer, Input, true),
            vec!["set", "Capture", "nocap"]
        );
        assert_eq!(
            set_mute_argv(AudioBackend::Amixer, Input, false),
            vec!["set", "Capture", "cap"]
        );
    }

    #[test]
    fn trusted_executables_are_absolute_and_valid() {
        for backend in AudioBackend::PREFERENCE {
            let exe = backend.trusted_executable().expect("valid trusted executable");
            assert!(exe.path().starts_with('/'));
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Output parsing
// ─────────────────────────────────────────────────────────────────────────────
//
// Every parser is **fail-closed**: unrecognised output is an error, never a
// default. Reporting "volume is 0" because `wpctl` changed its format would let a
// mutation verify against a fabricated observation, which is worse than an honest
// `Unavailable`.

fn unparseable(backend: AudioBackend) -> OsControlError {
    OsControlError::Unavailable {
        provider: Some(crate::os_control::contract::ProviderId::new(format!(
            "audio-{}",
            backend.as_str()
        ))),
        reason: crate::os_control::contract::SafeText::new(
            "audio state output could not be parsed; refusing to assume a value",
        ),
        retryable: true,
    }
}

/// Clamp a parsed percentage into range without silently inventing a value: the
/// backends themselves can report slightly over 100% (software boost), which is a
/// real reading that must map onto the contract's 0–100 scale.
fn clamp_percent(raw: f64) -> u8 {
    raw.round().clamp(0.0, 100.0) as u8
}

/// Parse the volume (and, where the same call reports it, the mute state) from a
/// backend's volume query output.
///
/// * `wpctl get-volume` → `Volume: 0.45` or `Volume: 0.45 [MUTED]`
/// * `pactl get-sink-volume` → `Volume: front-left: 29491 /  45% / -17.31 dB, ...`
/// * `amixer get Master` → `  Front Left: Playback 29491 [45%] [on]`
pub fn parse_volume_output(
    backend: AudioBackend,
    stdout: &str,
) -> Result<(u8, Option<bool>), OsControlError> {
    match backend {
        AudioBackend::Wpctl => {
            // The fraction is authoritative; `[MUTED]` is an optional suffix.
            let line = stdout
                .lines()
                .find(|l| l.trim_start().starts_with("Volume:"))
                .ok_or_else(|| unparseable(backend))?;
            let rest = line.trim_start().trim_start_matches("Volume:").trim();
            let fraction: f64 = rest
                .split_whitespace()
                .next()
                .ok_or_else(|| unparseable(backend))?
                .parse()
                .map_err(|_| unparseable(backend))?;
            let muted = line.contains("[MUTED]");
            Ok((clamp_percent(fraction * 100.0), Some(muted)))
        }
        AudioBackend::Pactl => {
            // Take the FIRST channel percentage. Channels can differ; the contract
            // exposes one scalar, and the first channel is the documented choice.
            let percent = stdout
                .split('/')
                .map(str::trim)
                .find_map(|token| token.strip_suffix('%'))
                .ok_or_else(|| unparseable(backend))?
                .trim()
                .parse::<f64>()
                .map_err(|_| unparseable(backend))?;
            // pactl reports mute in a separate call.
            Ok((clamp_percent(percent), None))
        }
        AudioBackend::Amixer => {
            // `[45%]` plus `[on]`/`[off]` on the same channel line.
            let line = stdout
                .lines()
                .find(|l| l.contains('[') && l.contains("%]"))
                .ok_or_else(|| unparseable(backend))?;
            let start = line.find('[').ok_or_else(|| unparseable(backend))?;
            let end = line[start..]
                .find("%]")
                .map(|i| start + i)
                .ok_or_else(|| unparseable(backend))?;
            let percent: f64 = line[start + 1..end]
                .parse()
                .map_err(|_| unparseable(backend))?;
            // `[off]` means muted. Absent on capture-only controls, so it stays
            // optional rather than defaulting to unmuted.
            let muted = if line.contains("[off]") {
                Some(true)
            } else if line.contains("[on]") {
                Some(false)
            } else {
                None
            };
            Ok((clamp_percent(percent), muted))
        }
    }
}

/// Parse a standalone mute query (`pactl get-sink-mute` → `Mute: yes`).
pub fn parse_mute_output(backend: AudioBackend, stdout: &str) -> Result<bool, OsControlError> {
    let line = stdout
        .lines()
        .find(|l| l.trim_start().starts_with("Mute:"))
        .ok_or_else(|| unparseable(backend))?;
    match line.split(':').nth(1).map(str::trim) {
        Some("yes") => Ok(true),
        Some("no") => Ok(false),
        _ => Err(unparseable(backend)),
    }
}

#[cfg(test)]
mod parse_tests {
    use super::*;

    #[test]
    fn wpctl_volume_and_mute_are_parsed() {
        let (percent, muted) = parse_volume_output(AudioBackend::Wpctl, "Volume: 0.45\n").unwrap();
        assert_eq!(percent, 45);
        assert_eq!(muted, Some(false));

        let (percent, muted) =
            parse_volume_output(AudioBackend::Wpctl, "Volume: 0.30 [MUTED]\n").unwrap();
        assert_eq!(percent, 30);
        assert_eq!(muted, Some(true));
    }

    #[test]
    fn wpctl_software_boost_is_clamped_not_rejected() {
        // PipeWire can report above 1.0; that is a real reading.
        let (percent, _) = parse_volume_output(AudioBackend::Wpctl, "Volume: 1.40").unwrap();
        assert_eq!(percent, 100);
    }

    #[test]
    fn pactl_takes_the_first_channel_percentage() {
        let out = "Volume: front-left: 29491 /  45% / -17.31 dB,   front-right: 29491 / 45% / -17.31 dB";
        let (percent, muted) = parse_volume_output(AudioBackend::Pactl, out).unwrap();
        assert_eq!(percent, 45);
        assert_eq!(muted, None, "pactl reports mute separately");
    }

    #[test]
    fn amixer_volume_and_switch_are_parsed() {
        let out = "Simple mixer control 'Master',0\n  Front Left: Playback 29491 [45%] [-17.31dB] [on]";
        let (percent, muted) = parse_volume_output(AudioBackend::Amixer, out).unwrap();
        assert_eq!(percent, 45);
        assert_eq!(muted, Some(false));

        let off = "  Front Left: Playback 29491 [45%] [off]";
        assert_eq!(
            parse_volume_output(AudioBackend::Amixer, off).unwrap().1,
            Some(true)
        );
    }

    #[test]
    fn pactl_mute_is_parsed() {
        assert!(parse_mute_output(AudioBackend::Pactl, "Mute: yes").unwrap());
        assert!(!parse_mute_output(AudioBackend::Pactl, "Mute: no").unwrap());
    }

    #[test]
    fn unrecognised_output_is_an_error_never_a_default() {
        // The whole point: a format change must not silently become "volume 0".
        for backend in [AudioBackend::Wpctl, AudioBackend::Pactl, AudioBackend::Amixer] {
            assert!(parse_volume_output(backend, "wpctl: unknown command").is_err());
            assert!(parse_volume_output(backend, "").is_err());
        }
        assert!(parse_mute_output(AudioBackend::Pactl, "Mute: maybe").is_err());
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// PulseAudio-compatibility control surface (Tasks 3.6 / 5.2)
// ─────────────────────────────────────────────────────────────────────────────
//
// Named endpoints, per-application streams and card profiles are addressed
// through the PulseAudio-compatibility CLI (`pactl`) on **every** backend, not
// only when `pactl` is the selected volume backend. That is a deliberate
// identity decision, not a convenience:
//
// * `wpctl status` is a human-readable tree. It prints a stream's node id but
//   describes the endpoint that stream is routed to only by its *display name*,
//   so building a stream→endpoint mapping from it would mean joining on a
//   human-visible label — exactly the identity mistake the design forbids.
// * `wpctl` addresses endpoints by node id, which is reassigned on every
//   PipeWire restart. `pactl` addresses them by sink/source *name*
//   (`alsa_output.pci-0000_00_1f.3.analog-stereo`), which survives a restart.
// * ALSA (`amixer`) has no concept of a per-application stream or a card
//   profile at all. That is a fact about ALSA, so those operations report
//   `Unsupported` rather than degrading into a guess.
//
// Under PipeWire this surface is served by `pipewire-pulse`, which is present
// wherever PipeWire is. When `pactl` is genuinely absent the governed query
// simply fails, which surfaces as `Unavailable` — never as an invented reading.

/// The trusted PulseAudio-compatibility CLI used for endpoint, stream and
/// card-profile control.
pub fn pulse_executable() -> Result<TrustedExecutable, OsControlError> {
    TrustedExecutable::new(
        AudioBackend::Pactl.executable_path(),
        Digest::of_str("pactl-fallback-v1"),
    )
}

/// The argv for reading which endpoint is currently the default.
#[must_use]
pub fn query_default_endpoint_argv(endpoint: AudioEndpointKind) -> Vec<String> {
    match endpoint {
        AudioEndpointKind::Output => vec!["get-default-sink".into()],
        AudioEndpointKind::Input => vec!["get-default-source".into()],
    }
}

/// The argv for making `target` the default endpoint of its kind.
#[must_use]
pub fn set_default_endpoint_argv(endpoint: AudioEndpointKind, target: &str) -> Vec<String> {
    match endpoint {
        AudioEndpointKind::Output => vec!["set-default-sink".into(), target.to_string()],
        AudioEndpointKind::Input => vec!["set-default-source".into(), target.to_string()],
    }
}

/// The argv for enumerating live per-application playback streams.
#[must_use]
pub fn query_streams_argv() -> Vec<String> {
    vec!["list".into(), "sink-inputs".into()]
}

/// The argv for setting one stream's volume. The stream is addressed by its
/// numeric node id, never by the application's name.
#[must_use]
pub fn set_stream_volume_argv(stream: &str, percent: u8) -> Vec<String> {
    vec![
        "set-sink-input-volume".into(),
        stream.to_string(),
        format!("{percent}%"),
    ]
}

/// The argv for setting one stream's mute state.
#[must_use]
pub fn set_stream_mute_argv(stream: &str, muted: bool) -> Vec<String> {
    vec![
        "set-sink-input-mute".into(),
        stream.to_string(),
        if muted { "1".into() } else { "0".into() },
    ]
}

/// The argv for enumerating cards and their active profiles.
#[must_use]
pub fn query_cards_argv() -> Vec<String> {
    vec!["list".into(), "cards".into()]
}

/// The argv for switching a card's profile.
#[must_use]
pub fn set_card_profile_argv(card: &str, profile: &str) -> Vec<String> {
    vec![
        "set-card-profile".into(),
        card.to_string(),
        profile.to_string(),
    ]
}

/// The argv for selecting an active port on a sink.
#[must_use]
pub fn set_sink_port_argv(sink: &str, port: &str) -> Vec<String> {
    vec!["set-sink-port".into(), sink.to_string(), port.to_string()]
}

/// One raw per-application stream as the control surface reports it.
///
/// `endpoint` is `None` when the stream is not currently routed to any
/// endpoint. That is a *fact* about an idle stream, distinct from "the routing
/// could not be determined", which is an error.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawStream {
    /// The stream's node id — its only identity.
    pub id: String,
    /// A descriptive application label. Not an identity: two windows of the
    /// same application produce two streams with the same label.
    pub app: String,
    /// The endpoint id the stream is routed to, if it is routed.
    pub endpoint: Option<String>,
    /// The stream's own volume, 0..=100.
    pub level_percent: u8,
    /// Whether the stream is muted.
    pub muted: bool,
}

fn unparseable_pulse(what: &str) -> OsControlError {
    OsControlError::Unavailable {
        provider: Some(crate::os_control::contract::ProviderId::new("audio-pactl")),
        reason: crate::os_control::contract::SafeText::new(format!(
            "{what} output could not be parsed; refusing to assume a value"
        )),
        retryable: true,
    }
}

/// Strip the leading tabs/spaces a `pactl list` block uses for indentation.
fn field(line: &str) -> &str {
    line.trim_start_matches(['\t', ' '])
}

/// Parse `pactl get-default-sink` / `get-default-source`, which print the
/// endpoint's name on a single line.
///
/// Fails closed on empty output: "there is no default endpoint" is not something
/// these commands express by printing nothing, so silence means the read failed.
pub fn parse_default_endpoint(stdout: &str) -> Result<String, OsControlError> {
    let name = stdout
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .ok_or_else(|| unparseable_pulse("default endpoint"))?;
    // A diagnostic line is not an endpoint name.
    if name.contains(' ') || name.starts_with('-') {
        return Err(unparseable_pulse("default endpoint"));
    }
    Ok(name.to_string())
}

/// Parse `pactl list sink-inputs` into one [`RawStream`] per block.
///
/// An **empty** list is a legitimate answer (nothing is playing) and parses to
/// an empty vector. A block that is present but missing its volume, mute or
/// application identity is *not* an empty answer — it is an unreadable one, so
/// it fails closed rather than reporting a stream at 0% or unmuted.
pub fn parse_sink_inputs(stdout: &str) -> Result<Vec<RawStream>, OsControlError> {
    /// The half-built block, so a missing field stays distinguishable from a
    /// defaulted one.
    struct Partial {
        id: String,
        app: Option<String>,
        endpoint: Option<String>,
        level: Option<u8>,
        muted: Option<bool>,
    }

    fn finish(partial: Partial) -> Result<RawStream, OsControlError> {
        let (Some(level), Some(muted)) = (partial.level, partial.muted) else {
            return Err(unparseable_pulse("stream volume/mute"));
        };
        let Some(app) = partial.app else {
            return Err(unparseable_pulse("stream application identity"));
        };
        Ok(RawStream {
            id: partial.id,
            app,
            endpoint: partial.endpoint,
            level_percent: level,
            muted,
        })
    }

    let mut out: Vec<RawStream> = Vec::new();
    let mut current: Option<Partial> = None;
    // Application identity preference: the reverse-DNS id, then the process
    // binary, then the display name. Later, weaker sources never overwrite an
    // earlier, stronger one.
    let mut app_rank: u8 = 0;

    for line in stdout.lines() {
        let trimmed = field(line);

        if let Some(rest) = trimmed.strip_prefix("Sink Input #") {
            if let Some(previous) = current.take() {
                out.push(finish(previous)?);
            }
            let id = rest.trim();
            if id.is_empty() || !id.chars().all(|c| c.is_ascii_digit()) {
                return Err(unparseable_pulse("stream id"));
            }
            app_rank = 0;
            current = Some(Partial {
                id: id.to_string(),
                app: None,
                endpoint: None,
                level: None,
                muted: None,
            });
            continue;
        }

        let Some(block) = current.as_mut() else {
            continue;
        };

        if let Some(rest) = trimmed.strip_prefix("Sink:") {
            let sink = rest.trim();
            if !sink.is_empty() && sink != "n/a" {
                block.endpoint = Some(sink.to_string());
            }
        } else if let Some(rest) = trimmed.strip_prefix("Mute:") {
            block.muted = match rest.trim() {
                "yes" => Some(true),
                "no" => Some(false),
                // An unrecognised mute token must not become "unmuted".
                _ => return Err(unparseable_pulse("stream mute")),
            };
        } else if trimmed.starts_with("Volume:") {
            block.level = Some(
                super::parsers::parse_any_percent(trimmed)
                    .ok_or_else(|| unparseable_pulse("stream volume"))?,
            );
        } else if let Some(value) = property_value(trimmed, "application.id") {
            if app_rank < 3 {
                block.app = Some(value);
                app_rank = 3;
            }
        } else if let Some(value) = property_value(trimmed, "application.process.binary") {
            if app_rank < 2 {
                block.app = Some(value);
                app_rank = 2;
            }
        } else if let Some(value) = property_value(trimmed, "application.name") {
            if app_rank < 1 {
                block.app = Some(value);
                app_rank = 1;
            }
        }
    }

    if let Some(previous) = current.take() {
        out.push(finish(previous)?);
    }
    // Deterministic order so two reads of an unchanged host agree.
    out.sort_by(|a, b| a.id.cmp(&b.id));
    Ok(out)
}

/// Read `key = "value"` from a `pactl` properties line.
fn property_value(line: &str, key: &str) -> Option<String> {
    let rest = line.strip_prefix(key)?.trim_start();
    let rest = rest.strip_prefix('=')?.trim();
    let inner = rest.strip_prefix('"')?.strip_suffix('"')?;
    if inner.is_empty() {
        return None;
    }
    Some(inner.to_string())
}

/// Parse `pactl list cards` for `card`'s active profile.
///
/// `Ok(None)` means the card is genuinely not present — a different fact from
/// `Err`, which means the output could not be read.
pub fn parse_active_card_profile(
    stdout: &str,
    card: &str,
) -> Result<Option<String>, OsControlError> {
    let mut in_card = false;
    let mut saw_any_card = false;

    for line in stdout.lines() {
        let trimmed = field(line);
        if trimmed.starts_with("Card #") {
            saw_any_card = true;
            in_card = false;
            continue;
        }
        if let Some(rest) = trimmed.strip_prefix("Name:") {
            in_card = rest.trim() == card;
            continue;
        }
        if in_card {
            if let Some(rest) = trimmed.strip_prefix("Active Profile:") {
                let profile = rest.trim();
                if profile.is_empty() {
                    return Err(unparseable_pulse("active card profile"));
                }
                return Ok(Some(profile.to_string()));
            }
        }
    }

    if !saw_any_card {
        // Not even a card header: this is not "no cards", it is unreadable
        // output. `pactl list cards` prints nothing at all when there are no
        // cards, so distinguish that one case explicitly.
        if stdout.trim().is_empty() {
            return Ok(None);
        }
        return Err(unparseable_pulse("card list"));
    }
    // Cards exist but none of them is this one.
    Ok(None)
}

#[cfg(test)]
mod pulse_parse_tests {
    use super::*;

    const SINK_INPUTS: &str = "Sink Input #78\n\tDriver: PipeWire\n\tOwner Module: n/a\n\tClient: 76\n\tSink: 49\n\tSample Specification: float32le 2ch 48000Hz\n\tCorked: no\n\tMute: no\n\tVolume: front-left: 42598 /  65% / -7.44 dB,   front-right: 42598 /  65% / -7.44 dB\n\t        balance 0.00\n\tProperties:\n\t\tmedia.class = \"Stream/Output/Audio\"\n\t\tapplication.name = \"Firefox\"\n\t\tapplication.process.binary = \"firefox\"\n\nSink Input #91\n\tDriver: PipeWire\n\tSink: 49\n\tMute: yes\n\tVolume: front-left: 65536 / 100% / 0.00 dB\n\tProperties:\n\t\tapplication.name = \"Firefox\"\n";

    #[test]
    fn two_windows_of_one_app_are_two_distinct_streams() {
        let streams = parse_sink_inputs(SINK_INPUTS).expect("parses");
        assert_eq!(streams.len(), 2);
        // Same application label, different identities: the id is what addresses
        // a stream, never the name.
        assert_eq!(streams[0].app, "firefox");
        assert_eq!(streams[1].app, "Firefox");
        assert_ne!(streams[0].id, streams[1].id);
        assert_eq!(streams[0].id, "78");
        assert_eq!(streams[0].level_percent, 65);
        assert!(!streams[0].muted);
        assert_eq!(streams[0].endpoint.as_deref(), Some("49"));
        assert_eq!(streams[1].id, "91");
        assert!(streams[1].muted);
    }

    #[test]
    fn nothing_playing_is_an_empty_list_not_an_error() {
        assert_eq!(parse_sink_inputs("").expect("empty parses"), Vec::new());
    }

    #[test]
    fn an_unrouted_stream_reports_no_endpoint_rather_than_a_guess() {
        let out = "Sink Input #5\n\tSink: n/a\n\tMute: no\n\tVolume: front-left: 0 / 50% / 0.00 dB\n\tProperties:\n\t\tapplication.name = \"mpv\"\n";
        let streams = parse_sink_inputs(out).expect("parses");
        assert_eq!(streams[0].endpoint, None);
    }

    #[test]
    fn unrecognised_output_is_an_error_never_a_default() {
        // A block whose mute state is missing must not be reported as unmuted.
        let no_mute = "Sink Input #5\n\tSink: 49\n\tVolume: front-left: 0 / 50% / 0.00 dB\n\tProperties:\n\t\tapplication.name = \"mpv\"\n";
        assert!(parse_sink_inputs(no_mute).is_err());
        // An unknown mute token is not "unmuted" either.
        let odd_mute = "Sink Input #5\n\tSink: 49\n\tMute: maybe\n\tVolume: 50%\n";
        assert!(parse_sink_inputs(odd_mute).is_err());
        // A block with no volume is unreadable, not 0%.
        let no_volume = "Sink Input #5\n\tSink: 49\n\tMute: no\n\tProperties:\n\t\tapplication.name = \"mpv\"\n";
        assert!(parse_sink_inputs(no_volume).is_err());
        // A non-numeric stream id is not an identity.
        assert!(parse_sink_inputs("Sink Input #abc\n\tMute: no\n\tVolume: 50%\n").is_err());
        // Default-endpoint reads never invent a name.
        assert!(parse_default_endpoint("").is_err());
        assert!(parse_default_endpoint("Failure: No such entity").is_err());
        // Card output that is neither empty nor a card list is unreadable.
        assert!(parse_active_card_profile("pactl: unknown command", "x").is_err());
    }

    #[test]
    fn default_endpoint_name_is_parsed() {
        assert_eq!(
            parse_default_endpoint("alsa_output.pci-0000_00_1f.3.analog-stereo\n").unwrap(),
            "alsa_output.pci-0000_00_1f.3.analog-stereo"
        );
    }

    #[test]
    fn active_profile_distinguishes_absent_from_unknown() {
        let cards = "Card #50\n\tName: alsa_card.pci-0000_00_1f.3\n\tDriver: PipeWire\n\tActive Profile: output:analog-stereo+input:analog-stereo\n";
        assert_eq!(
            parse_active_card_profile(cards, "alsa_card.pci-0000_00_1f.3").unwrap(),
            Some("output:analog-stereo+input:analog-stereo".to_string())
        );
        // A card that is not present is `None` — an answer, not a failure.
        assert_eq!(parse_active_card_profile(cards, "alsa_card.other").unwrap(), None);
        // No cards at all is also an answer.
        assert_eq!(parse_active_card_profile("", "any").unwrap(), None);
    }

    #[test]
    fn pulse_argv_golden() {
        assert_eq!(
            query_default_endpoint_argv(AudioEndpointKind::Output),
            vec!["get-default-sink"]
        );
        assert_eq!(
            query_default_endpoint_argv(AudioEndpointKind::Input),
            vec!["get-default-source"]
        );
        assert_eq!(
            set_default_endpoint_argv(AudioEndpointKind::Input, "alsa_input.usb-mic"),
            vec!["set-default-source", "alsa_input.usb-mic"]
        );
        assert_eq!(query_streams_argv(), vec!["list", "sink-inputs"]);
        assert_eq!(
            set_stream_volume_argv("78", 40),
            vec!["set-sink-input-volume", "78", "40%"]
        );
        assert_eq!(
            set_stream_mute_argv("78", true),
            vec!["set-sink-input-mute", "78", "1"]
        );
        assert_eq!(query_cards_argv(), vec!["list", "cards"]);
        assert_eq!(
            set_card_profile_argv("alsa_card.pci", "output:hdmi-stereo"),
            vec!["set-card-profile", "alsa_card.pci", "output:hdmi-stereo"]
        );
        assert_eq!(
            set_sink_port_argv("sink-1", "analog-output-headphones"),
            vec!["set-sink-port", "sink-1", "analog-output-headphones"]
        );
    }
}
