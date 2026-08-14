//! MPRIS session media control (Task 5.2, OSC-018).
//!
//! `org.mpris.MediaPlayer2.*` on the **session** bus is the only standard way to
//! ask "what is playing, and can you pause it" without knowing anything about
//! the application. This module is the desired-state slice for it:
//! [`list_players`](MediaControlOps::list_players)-style enumeration and the six
//! frozen playback actions.
//!
//! # Identity
//!
//! A player is addressed by its **bus name** (`org.mpris.MediaPlayer2.vlc`,
//! `org.mpris.MediaPlayer2.firefox.instance_1_9`), which is stable for as long
//! as the player is on the bus. Its `Identity` property and its track title are
//! display strings that change while the same player keeps playing, so neither
//! is ever matched on. A bus name is validated before it can reach argv.
//!
//! # Transport
//!
//! Reads and mutations both go through the governed argv seam using the D-Bus
//! CLI (`gdbus`), not a raw bus connection. The live audio adapter is
//! constructed without a `zbus::Connection` — handing it one would mean changing
//! the live composition root, which this task does not own — and a governed
//! `gdbus` call has the same containment as any other structured command: a
//! trusted absolute executable, an exact digested argv, a hermetic environment,
//! a pinned `C` locale, bounded output, a deadline and cancellation.
//!
//! # What a receipt may claim
//!
//! `play`, `pause` and `stop` have an exact typed postcondition
//! (`PlaybackStatus`). `toggle`, `next` and `previous` are *relative*: their
//! postcondition is a change from the state observed before the action, so the
//! desired state is a sentinel that no observation can ever equal (which stops
//! the runtime from short-circuiting them as already-satisfied) and the
//! verification compares against the captured prior state. When no prior state
//! was captured, or a player publishes no track identity, the verification is
//! [`VerificationReport::Inconclusive`] — never "satisfied".

use std::time::SystemTime;

use async_trait::async_trait;

use crate::os_control::context::{AdmittedMutationContext, HostExecutionContext};
use crate::os_control::contract::{
    ComparatorKind, DesiredStateControl, Digest, OsEvidenceSource, SafeErrorCode, SafeField,
    SafeText, VerificationReliability,
};
use crate::os_control::error::OsControlError;
use crate::os_control::linux::structured_command::TrustedExecutable;
use crate::os_control::receipt::{
    ApplyOutcome, RedactedObservation, RollbackToken, SatisfyingVerification,
    VerificationContradiction, VerificationReport,
};
use crate::os_control::runtime::NormalizedObservation;

use super::{AudioControl, AudioStep, AudioTransport, MediaPlayerId};

/// The MPRIS bus-name prefix every player publishes.
pub const MPRIS_PREFIX: &str = "org.mpris.MediaPlayer2.";
/// The single well-known MPRIS object path.
pub const MPRIS_PATH: &str = "/org/mpris/MediaPlayer2";
/// The player interface.
pub const MPRIS_PLAYER_IFACE: &str = "org.mpris.MediaPlayer2.Player";

/// The default number of players in one page.
pub const MEDIA_PAGE_DEFAULT_ITEMS: usize = 50;
/// The frozen maximum number of players in one page.
pub const MEDIA_PAGE_MAX_ITEMS: usize = 256;
/// The maximum byte length of a reported track label.
pub const TRACK_LABEL_MAX_BYTES: usize = 128;

fn invalid(field: &str, reason: &str) -> OsControlError {
    OsControlError::InvalidRequest {
        field: SafeField::new(field),
        reason: SafeText::new(reason),
    }
}

fn unparseable(what: &str) -> OsControlError {
    OsControlError::Unavailable {
        provider: Some(crate::os_control::contract::ProviderId::new("media-gdbus")),
        reason: SafeText::new(format!(
            "{what} output could not be parsed; refusing to assume a value"
        )),
        retryable: true,
    }
}

/// The trusted D-Bus CLI used for MPRIS reads and playback commands.
pub fn gdbus_executable() -> Result<TrustedExecutable, OsControlError> {
    TrustedExecutable::new("/usr/bin/gdbus", Digest::of_str("gdbus-mpris-v1"))
}

/// Validate an MPRIS bus name before it becomes an argv element.
///
/// Rejects (never rewrites) anything that is not a well-formed
/// `org.mpris.MediaPlayer2.*` bus name: a bus name is about to be handed to
/// `--dest`, so a value carrying an option prefix, whitespace or a control
/// character is refused outright.
pub fn parse_player_id(raw: &str) -> Result<MediaPlayerId, OsControlError> {
    let id = MediaPlayerId::parse(raw)?;
    let name = id.as_str();
    if !name.starts_with(MPRIS_PREFIX) {
        return Err(invalid(
            "player",
            "a media player is identified by its MPRIS bus name (org.mpris.MediaPlayer2.*), not by its displayed title",
        ));
    }
    if name.len() == MPRIS_PREFIX.len() {
        return Err(invalid("player", "MPRIS bus name has no player suffix"));
    }
    if !name
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
    {
        return Err(invalid(
            "player",
            "MPRIS bus name contains a character a D-Bus name cannot hold",
        ));
    }
    Ok(id)
}

/// The application label derived from a player's bus name.
///
/// Derived from the *stable* bus name rather than the player's `Identity`
/// property, so it cannot change while the same player keeps playing. Two
/// instances of one application share this label and are still two distinct
/// players.
#[must_use]
pub fn app_label(player: &MediaPlayerId) -> String {
    let suffix = player
        .as_str()
        .strip_prefix(MPRIS_PREFIX)
        .unwrap_or(player.as_str());
    suffix
        .split_once(".instance")
        .map_or(suffix, |(head, _)| head)
        .to_string()
}

/// The six frozen playback actions.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MediaPlaybackAction {
    /// Start or resume playback.
    Play,
    /// Pause playback, keeping the position.
    Pause,
    /// Flip between playing and paused.
    Toggle,
    /// Advance to the next track.
    Next,
    /// Return to the previous track.
    Previous,
    /// Stop playback.
    Stop,
}

impl MediaPlaybackAction {
    /// Parse the frozen enum token. An unknown token is rejected, never mapped
    /// onto a "closest" action.
    pub fn parse(raw: &str) -> Result<Self, OsControlError> {
        match raw {
            "play" => Ok(Self::Play),
            "pause" => Ok(Self::Pause),
            "toggle" => Ok(Self::Toggle),
            "next" => Ok(Self::Next),
            "previous" => Ok(Self::Previous),
            "stop" => Ok(Self::Stop),
            _ => Err(invalid(
                "action",
                "action must be one of play, pause, toggle, next, previous, stop",
            )),
        }
    }

    /// The MPRIS member this action invokes.
    #[must_use]
    pub fn member(self) -> &'static str {
        match self {
            Self::Play => "Play",
            Self::Pause => "Pause",
            Self::Toggle => "PlayPause",
            Self::Next => "Next",
            Self::Previous => "Previous",
            Self::Stop => "Stop",
        }
    }

    /// The exact `PlaybackStatus` this action must produce, when it has one.
    ///
    /// `toggle`, `next` and `previous` return `None`: their effect is relative to
    /// whatever was playing before, so there is no absolute status to demand.
    #[must_use]
    pub fn expected_status(self) -> Option<&'static str> {
        match self {
            Self::Play => Some("Playing"),
            Self::Pause => Some("Paused"),
            Self::Stop => Some("Stopped"),
            Self::Toggle | Self::Next | Self::Previous => None,
        }
    }

    /// The stable step label for a receipt.
    #[must_use]
    pub fn step(self) -> &'static str {
        match self {
            Self::Play => "media_play",
            Self::Pause => "media_pause",
            Self::Toggle => "media_toggle",
            Self::Next => "media_next",
            Self::Previous => "media_previous",
            Self::Stop => "media_stop",
        }
    }
}

/// One MPRIS player as the session bus reports it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MediaPlayerInfo {
    /// The player's bus name — its only identity.
    pub player: MediaPlayerId,
    /// A label derived from the bus name. Descriptive only.
    pub app: String,
    /// `Playing`, `Paused` or `Stopped`, exactly as MPRIS defines them.
    pub playback_state: String,
    /// The current track's title, when the player publishes one. `None` means it
    /// publishes none — not "unknown".
    pub track_label: Option<String>,
    /// The current track's MPRIS track id, used to prove a `next`/`previous`
    /// actually advanced. `None` when the player publishes no track identity.
    pub track_id: Option<String>,
}

/// Which dimension of a player a request compares against.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MediaFocus {
    /// An absolute `PlaybackStatus` postcondition (`play` / `pause` / `stop`).
    Status,
    /// A postcondition relative to the prior state (`toggle` / `next` /
    /// `previous`).
    Relative,
}

/// A normalized observation of one player.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MediaPlayerState {
    /// The observed player's identity.
    pub player: MediaPlayerId,
    /// The observed playback status, when this observation carries one.
    pub playback_state: Option<String>,
    /// The observed track id, when the player publishes one.
    pub track_id: Option<String>,
    /// The comparison focus.
    pub focus: MediaFocus,
    /// Whether this value is the *desired* sentinel for a relative action rather
    /// than something that was observed. A sentinel never equals an observation.
    pending: bool,
}

impl MediaPlayerState {
    /// An observed absolute-status state.
    #[must_use]
    pub fn status(player: MediaPlayerId, playback_state: impl Into<String>) -> Self {
        Self {
            player,
            playback_state: Some(playback_state.into()),
            track_id: None,
            focus: MediaFocus::Status,
            pending: false,
        }
    }

    /// The state an observed player represents under `focus`.
    #[must_use]
    pub fn from_info(info: &MediaPlayerInfo, focus: MediaFocus) -> Self {
        Self {
            player: info.player.clone(),
            playback_state: Some(info.playback_state.clone()),
            track_id: info.track_id.clone(),
            focus,
            pending: false,
        }
    }

    /// The desired sentinel for a relative action.
    ///
    /// Deliberately unequal to every possible observation: a relative action must
    /// never be short-circuited as "already in the desired state", because
    /// "already advanced to the next track" is not a state anything can be in.
    #[must_use]
    pub fn pending(player: MediaPlayerId) -> Self {
        Self {
            player,
            playback_state: None,
            track_id: None,
            focus: MediaFocus::Relative,
            pending: true,
        }
    }
}

impl NormalizedObservation for MediaPlayerState {
    fn observation_digest(&self) -> Digest {
        let id = self.player.as_str();
        Digest::of_str(&if self.pending {
            format!("media:{id}:relative:pending")
        } else {
            match self.focus {
                MediaFocus::Status => format!(
                    "media:{id}:status:{}",
                    self.playback_state.as_deref().unwrap_or("unread")
                ),
                MediaFocus::Relative => format!(
                    "media:{id}:relative:observed:{}:{}",
                    self.playback_state.as_deref().unwrap_or("unread"),
                    self.track_id.as_deref().unwrap_or("no-track")
                ),
            }
        })
    }

    fn numeric_value(&self) -> Option<f64> {
        None
    }
}

/// A fully-described media request.
#[derive(Debug, Clone)]
pub struct MediaRequest {
    /// The canonical tool/action name the grant was minted against.
    pub action: String,
    /// The canonical tool parameters (must reproduce the grant's params digest).
    pub params: serde_json::Value,
    /// The player being controlled.
    pub player: MediaPlayerId,
    /// The playback action.
    pub playback: MediaPlaybackAction,
}

impl MediaRequest {
    /// The comparison focus implied by the action.
    #[must_use]
    pub fn focus(&self) -> MediaFocus {
        match self.playback.expected_status() {
            Some(_) => MediaFocus::Status,
            None => MediaFocus::Relative,
        }
    }

    /// The desired end state.
    #[must_use]
    pub fn desired_state(&self) -> MediaPlayerState {
        match self.playback.expected_status() {
            Some(status) => MediaPlayerState::status(self.player.clone(), status),
            None => MediaPlayerState::pending(self.player.clone()),
        }
    }

    /// Playback status and track identity are exact facts, never numeric.
    #[must_use]
    pub fn comparator(&self) -> ComparatorKind {
        ComparatorKind::Exact
    }

    /// The governed argv that invokes this action.
    #[must_use]
    pub fn argv(&self) -> Vec<String> {
        call_member_argv(self.player.as_str(), MPRIS_PLAYER_IFACE, self.playback.member())
    }
}

/// The argv for invoking a member on a player.
#[must_use]
pub fn call_member_argv(bus: &str, iface: &str, member: &str) -> Vec<String> {
    vec![
        "call".into(),
        "--session".into(),
        "--dest".into(),
        bus.to_string(),
        "--object-path".into(),
        MPRIS_PATH.to_string(),
        "--method".into(),
        format!("{iface}.{member}"),
    ]
}

/// The argv for listing every name on the session bus.
#[must_use]
pub fn list_names_argv() -> Vec<String> {
    vec![
        "call".into(),
        "--session".into(),
        "--dest".into(),
        "org.freedesktop.DBus".into(),
        "--object-path".into(),
        "/org/freedesktop/DBus".into(),
        "--method".into(),
        "org.freedesktop.DBus.ListNames".into(),
    ]
}

/// The argv for reading one player property.
#[must_use]
pub fn get_property_argv(bus: &str, property: &str) -> Vec<String> {
    let mut argv = call_member_argv(bus, "org.freedesktop.DBus.Properties", "Get");
    argv.push(MPRIS_PLAYER_IFACE.to_string());
    argv.push(property.to_string());
    argv
}

/// One deterministic page of players.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MediaPlayerPage {
    /// The page's players, ordered by bus name.
    pub items: Vec<MediaPlayerInfo>,
    /// The cursor that continues this listing, when it was truncated.
    pub next_cursor: Option<String>,
    /// Whether more players exist beyond this page.
    pub truncated: bool,
}

/// Mint an integrity-checked page cursor.
#[must_use]
pub fn encode_cursor(offset: usize) -> String {
    let check = Digest::of_str(&format!("media-player-cursor:{offset}"));
    format!("mp1.{offset}.{}", &check.as_hex()[..16])
}

/// Decode a page cursor this build minted.
pub fn decode_cursor(cursor: &str) -> Result<usize, OsControlError> {
    let field = "cursor";
    if cursor.len() > 512 {
        return Err(invalid(field, "cursor exceeds the maximum length"));
    }
    let mut parts = cursor.split('.');
    match parts.next() {
        Some("mp1") => {}
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
    let expected = Digest::of_str(&format!("media-player-cursor:{offset}"));
    if check != &expected.as_hex()[..16] {
        return Err(invalid(field, "cursor failed its integrity check"));
    }
    Ok(offset)
}

/// Page a player listing deterministically (ordered by bus name).
pub fn page(
    mut all: Vec<MediaPlayerInfo>,
    cursor: Option<&str>,
    limit: Option<usize>,
) -> Result<MediaPlayerPage, OsControlError> {
    let limit = match limit {
        None => MEDIA_PAGE_DEFAULT_ITEMS,
        Some(0) => return Err(invalid("limit", "limit must be at least 1")),
        Some(n) if n > MEDIA_PAGE_MAX_ITEMS => {
            return Err(invalid("limit", "limit exceeds the maximum page size"))
        }
        Some(n) => n,
    };
    let offset = match cursor {
        None => 0,
        Some(raw) => decode_cursor(raw)?,
    };

    all.sort_by(|a, b| a.player.cmp(&b.player));
    if offset > all.len() {
        return Err(invalid("cursor", "cursor points past the end of the listing"));
    }
    let end = (offset + limit).min(all.len());
    let items = all[offset..end].to_vec();
    let truncated = end < all.len();
    Ok(MediaPlayerPage {
        items,
        next_cursor: truncated.then(|| encode_cursor(end)),
        truncated,
    })
}

// ─────────────────────────────────────────────────────────────────────────────
// gdbus output parsing — fail-closed
// ─────────────────────────────────────────────────────────────────────────────

/// Read the single-quoted string that follows `needle`, honouring `\'` escapes.
fn quoted_after(hay: &str, needle: &str) -> Option<String> {
    let rest = &hay[hay.find(needle)? + needle.len()..];
    let rest = rest.trim_start();
    // A variant value may be typed, e.g. `<objectpath '/track/1'>`.
    let rest = rest.strip_prefix('<').unwrap_or(rest).trim_start();
    let rest = match rest.split_once('\'') {
        Some((prefix, after)) if prefix.chars().all(|c| c.is_ascii_alphanumeric() || c == ' ') => {
            after
        }
        _ => return None,
    };

    let mut out = String::new();
    let mut chars = rest.chars();
    while let Some(c) = chars.next() {
        match c {
            '\\' => out.push(chars.next()?),
            '\'' => return Some(out),
            _ => out.push(c),
        }
    }
    // Unterminated: the value cannot be read.
    None
}

/// Parse a `gdbus` `ListNames` reply into the bus names it holds.
///
/// The session bus always holds at least `org.freedesktop.DBus`, so an empty
/// result means the reply could not be read, not that the bus is empty.
pub fn parse_bus_names(stdout: &str) -> Result<Vec<String>, OsControlError> {
    let mut names = Vec::new();
    let mut rest = stdout;
    while let Some(start) = rest.find('\'') {
        let after = &rest[start + 1..];
        let Some(end) = after.find('\'') else {
            return Err(unparseable("session bus name list"));
        };
        names.push(after[..end].to_string());
        rest = &after[end + 1..];
    }
    if names.is_empty() {
        return Err(unparseable("session bus name list"));
    }
    Ok(names)
}

/// Parse a `gdbus` string-property reply such as `(<'Playing'>,)`.
pub fn parse_playback_status(stdout: &str) -> Result<String, OsControlError> {
    let raw = quoted_after(stdout, "<").ok_or_else(|| unparseable("playback status"))?;
    // MPRIS defines exactly three statuses. An unrecognised one is not silently
    // mapped onto "Stopped".
    match raw.as_str() {
        "Playing" | "Paused" | "Stopped" => Ok(raw),
        _ => Err(unparseable("playback status")),
    }
}

/// Parse the track title out of a `gdbus` `Metadata` reply.
///
/// `Ok(None)` means the player publishes no title — a fact, not a failure.
pub fn parse_track_title(stdout: &str) -> Result<Option<String>, OsControlError> {
    if !stdout.contains("'xesam:title'") {
        return Ok(None);
    }
    let title =
        quoted_after(stdout, "'xesam:title':").ok_or_else(|| unparseable("track title"))?;
    let bounded: String = title
        .chars()
        .filter(|c| !c.is_control())
        .take(TRACK_LABEL_MAX_BYTES)
        .collect();
    if bounded.is_empty() {
        return Ok(None);
    }
    Ok(Some(bounded))
}

/// Parse the MPRIS track id out of a `gdbus` `Metadata` reply.
///
/// `Ok(None)` means the player publishes no track identity, which makes a
/// `next`/`previous` unverifiable rather than verified.
pub fn parse_track_id(stdout: &str) -> Result<Option<String>, OsControlError> {
    if !stdout.contains("'mpris:trackid'") {
        return Ok(None);
    }
    let id =
        quoted_after(stdout, "'mpris:trackid':").ok_or_else(|| unparseable("track identity"))?;
    if id.is_empty() {
        return Ok(None);
    }
    Ok(Some(id))
}

/// Keep only the MPRIS players from a bus-name list, in deterministic order.
#[must_use]
pub fn mpris_names(names: &[String]) -> Vec<String> {
    let mut players: Vec<String> = names
        .iter()
        .filter(|name| name.starts_with(MPRIS_PREFIX))
        .cloned()
        .collect();
    players.sort();
    players.dedup();
    players
}

/// The prior player state captured before an apply, so a relative action can be
/// verified against what was actually true beforehand.
#[derive(Debug, Clone)]
pub(super) struct MediaSnapshot {
    pub(super) before: MediaPlayerState,
    pub(super) playback: MediaPlaybackAction,
}

#[async_trait]
impl<T: AudioTransport> DesiredStateControl<MediaRequest, MediaPlayerState> for AudioControl<T> {
    async fn observe(
        &self,
        ctx: &HostExecutionContext,
        request: &MediaRequest,
    ) -> Result<MediaPlayerState, OsControlError> {
        match self
            .transport
            .read_media_player(ctx, &request.player)
            .await?
        {
            Some(info) => Ok(MediaPlayerState::from_info(&info, request.focus())),
            // The player left the bus. A fact about the target, not a failed read.
            None => Err(OsControlError::TargetChanged),
        }
    }

    async fn apply(
        &self,
        ctx: &AdmittedMutationContext<'_>,
        request: &MediaRequest,
        _desired: &MediaPlayerState,
    ) -> Result<ApplyOutcome, OsControlError> {
        // A relative action can only be verified against the state that was true
        // before it ran, so capture that first.
        if let Some(info) = self
            .transport
            .read_media_player(ctx.observation(), &request.player)
            .await?
        {
            let session = Self::media_session_key(ctx.observation());
            self.media_snapshots
                .lock()
                .expect("media snapshots poisoned")
                .insert(
                    session,
                    MediaSnapshot {
                        before: MediaPlayerState::from_info(&info, MediaFocus::Relative),
                        playback: request.playback,
                    },
                );
        } else {
            return Err(OsControlError::TargetChanged);
        }

        let steps = vec![AudioStep {
            executable: gdbus_executable()?,
            args: request.argv(),
            step: request.playback.step(),
        }];
        self.dispatch_steps(ctx, &request.action, &request.params, steps)
            .await
    }

    async fn verify(
        &self,
        ctx: &HostExecutionContext,
        request: &MediaRequest,
        desired: &MediaPlayerState,
    ) -> Result<VerificationReport<MediaPlayerState>, OsControlError> {
        let Some(info) = self
            .transport
            .read_media_player(ctx, &request.player)
            .await?
        else {
            return Ok(VerificationReport::Inconclusive {
                reason: SafeText::new(
                    "the player left the session bus before verification; its state is no longer observable",
                ),
            });
        };
        let observed = MediaPlayerState::from_info(&info, request.focus());

        let satisfied = match request.playback.expected_status() {
            // Absolute: the status must be exactly what the action promised.
            Some(_) => observed.observation_digest() == desired.observation_digest(),
            // Relative: compare against the captured prior state.
            None => {
                let snapshot = self
                    .media_snapshots
                    .lock()
                    .expect("media snapshots poisoned")
                    .get(&Self::media_session_key(ctx))
                    .cloned();
                let Some(snapshot) = snapshot else {
                    return Ok(VerificationReport::Inconclusive {
                        reason: SafeText::new(
                            "no prior player state was captured, so a relative action cannot be verified",
                        ),
                    });
                };
                match relative_satisfied(&snapshot, &observed) {
                    Some(result) => result,
                    None => {
                        return Ok(VerificationReport::Inconclusive {
                            reason: SafeText::new(
                                "the player publishes no track identity, so a track change cannot be proven",
                            ),
                        })
                    }
                }
            }
        };

        if satisfied {
            Ok(VerificationReport::Satisfied(SatisfyingVerification::new(
                OsEvidenceSource::StructuredCommandQuery,
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
        _ctx: &AdmittedMutationContext<'_>,
        _token: &RollbackToken,
    ) -> Result<ApplyOutcome, OsControlError> {
        // Playback control has no inverse to advertise: `next` cannot be undone
        // (the previous track's position is gone), and the frozen contract's
        // rollback claim for this operation is `None`. Report the effect as
        // uncompensable rather than dispatching a "reverse" command that would be
        // a new change dressed up as a rollback.
        Ok(ApplyOutcome::Uncertain(
            crate::os_control::receipt::UncertainDispatch::new(
                None,
                crate::os_control::receipt::UncertainEffectCause::Unobservable,
                crate::os_control::contract::BoundedVec::new(),
            ),
        ))
    }
}

/// Whether a relative action achieved its effect.
///
/// `None` means it cannot be decided from what the player publishes.
fn relative_satisfied(snapshot: &MediaSnapshot, observed: &MediaPlayerState) -> Option<bool> {
    match snapshot.playback {
        MediaPlaybackAction::Toggle => {
            let before = snapshot.before.playback_state.as_deref()?;
            let after = observed.playback_state.as_deref()?;
            Some((before == "Playing") != (after == "Playing"))
        }
        MediaPlaybackAction::Next | MediaPlaybackAction::Previous => {
            // Proving a track change needs a track identity on both sides.
            let before = snapshot.before.track_id.as_deref()?;
            let after = observed.track_id.as_deref()?;
            Some(before != after)
        }
        _ => Some(false),
    }
}

impl<T: AudioTransport> AudioControl<T> {
    /// The snapshot key for one media action.
    ///
    /// Keyed by the action id rather than the session id because `verify` only
    /// ever sees the observation context, and because two media actions in one
    /// session must not read each other's prior state.
    fn media_session_key(ctx: &HostExecutionContext) -> String {
        ctx.action_id.as_str().to_string()
    }
}

#[cfg(test)]
mod media_parse_tests {
    use super::*;

    #[test]
    fn playback_status_is_parsed() {
        assert_eq!(parse_playback_status("(<'Playing'>,)\n").unwrap(), "Playing");
        assert_eq!(parse_playback_status("(<'Paused'>,)").unwrap(), "Paused");
        assert_eq!(parse_playback_status("(<'Stopped'>,)").unwrap(), "Stopped");
    }

    #[test]
    fn bus_names_are_parsed_and_filtered_to_players() {
        let out = "([':1.0', 'org.freedesktop.DBus', 'org.mpris.MediaPlayer2.vlc', 'org.mpris.MediaPlayer2.firefox.instance_1_9'],)";
        let names = parse_bus_names(out).unwrap();
        assert!(names.contains(&"org.freedesktop.DBus".to_string()));
        let players = mpris_names(&names);
        assert_eq!(
            players,
            vec![
                "org.mpris.MediaPlayer2.firefox.instance_1_9".to_string(),
                "org.mpris.MediaPlayer2.vlc".to_string()
            ]
        );
    }

    #[test]
    fn metadata_title_and_trackid_are_parsed_including_apostrophes() {
        let out = "(<{'mpris:trackid': <objectpath '/org/mpris/MediaPlayer2/track/7'>, 'xesam:title': <'Don\\'t Stop'>, 'xesam:artist': <['A']>}>,)";
        assert_eq!(
            parse_track_title(out).unwrap(),
            Some("Don't Stop".to_string())
        );
        assert_eq!(
            parse_track_id(out).unwrap(),
            Some("/org/mpris/MediaPlayer2/track/7".to_string())
        );
    }

    #[test]
    fn a_player_publishing_no_title_is_absent_not_unknown() {
        let out = "(<{'mpris:length': <int64 1000>}>,)";
        assert_eq!(parse_track_title(out).unwrap(), None);
        assert_eq!(parse_track_id(out).unwrap(), None);
    }

    #[test]
    fn unrecognised_output_is_an_error_never_a_default() {
        // A format change must not become "Stopped".
        assert!(parse_playback_status("gdbus: error: no such interface").is_err());
        assert!(parse_playback_status("").is_err());
        // A status MPRIS does not define is not coerced.
        assert!(parse_playback_status("(<'Buffering'>,)").is_err());
        // An empty bus list is impossible; treat it as unreadable.
        assert!(parse_bus_names("()").is_err());
        // A present-but-unreadable title is an error, not "no title".
        assert!(parse_track_title("(<{'xesam:title': <unterminated>}>,)").is_err());
    }

    #[test]
    fn a_title_is_never_accepted_as_an_identity() {
        assert!(parse_player_id("Spotify").is_err());
        assert!(parse_player_id("org.mpris.MediaPlayer2.").is_err());
        assert!(parse_player_id("-org.mpris.MediaPlayer2.x").is_err());
        assert!(parse_player_id("org.mpris.MediaPlayer2.a b").is_err());
        assert!(parse_player_id("org.mpris.MediaPlayer2.vlc").is_ok());
    }

    #[test]
    fn app_label_comes_from_the_bus_name_not_the_title() {
        let id = parse_player_id("org.mpris.MediaPlayer2.firefox.instance_1_9").unwrap();
        assert_eq!(app_label(&id), "firefox");
        let vlc = parse_player_id("org.mpris.MediaPlayer2.vlc").unwrap();
        assert_eq!(app_label(&vlc), "vlc");
    }

    #[test]
    fn a_relative_action_can_never_be_short_circuited_as_unchanged() {
        let player = parse_player_id("org.mpris.MediaPlayer2.vlc").unwrap();
        let desired = MediaPlayerState::pending(player.clone());
        for status in ["Playing", "Paused", "Stopped"] {
            let observed = MediaPlayerState::from_info(
                &MediaPlayerInfo {
                    player: player.clone(),
                    app: "vlc".into(),
                    playback_state: status.into(),
                    track_label: None,
                    track_id: Some("/track/1".into()),
                },
                MediaFocus::Relative,
            );
            assert_ne!(
                desired.observation_digest(),
                observed.observation_digest(),
                "a pending relative action must not match an observed state"
            );
        }
    }

    #[test]
    fn next_is_only_satisfied_by_a_proven_track_change() {
        let player = parse_player_id("org.mpris.MediaPlayer2.vlc").unwrap();
        let before = MediaPlayerInfo {
            player: player.clone(),
            app: "vlc".into(),
            playback_state: "Playing".into(),
            track_label: None,
            track_id: Some("/track/1".into()),
        };
        let snapshot = MediaSnapshot {
            before: MediaPlayerState::from_info(&before, MediaFocus::Relative),
            playback: MediaPlaybackAction::Next,
        };
        let advanced = MediaPlayerState::from_info(
            &MediaPlayerInfo {
                track_id: Some("/track/2".into()),
                ..before.clone()
            },
            MediaFocus::Relative,
        );
        assert_eq!(relative_satisfied(&snapshot, &advanced), Some(true));
        let unchanged = MediaPlayerState::from_info(&before, MediaFocus::Relative);
        assert_eq!(relative_satisfied(&snapshot, &unchanged), Some(false));
        // No track identity → undecidable, never "satisfied".
        let untracked = MediaPlayerState::from_info(
            &MediaPlayerInfo {
                track_id: None,
                ..before
            },
            MediaFocus::Relative,
        );
        assert_eq!(relative_satisfied(&snapshot, &untracked), None);
    }

    #[test]
    fn toggle_is_satisfied_only_by_a_flip() {
        let player = parse_player_id("org.mpris.MediaPlayer2.vlc").unwrap();
        let playing = MediaPlayerInfo {
            player: player.clone(),
            app: "vlc".into(),
            playback_state: "Playing".into(),
            track_label: None,
            track_id: None,
        };
        let snapshot = MediaSnapshot {
            before: MediaPlayerState::from_info(&playing, MediaFocus::Relative),
            playback: MediaPlaybackAction::Toggle,
        };
        let paused = MediaPlayerState::from_info(
            &MediaPlayerInfo {
                playback_state: "Paused".into(),
                ..playing.clone()
            },
            MediaFocus::Relative,
        );
        assert_eq!(relative_satisfied(&snapshot, &paused), Some(true));
        let still_playing = MediaPlayerState::from_info(&playing, MediaFocus::Relative);
        assert_eq!(relative_satisfied(&snapshot, &still_playing), Some(false));
    }

    #[test]
    fn argv_golden() {
        assert_eq!(
            list_names_argv(),
            vec![
                "call",
                "--session",
                "--dest",
                "org.freedesktop.DBus",
                "--object-path",
                "/org/freedesktop/DBus",
                "--method",
                "org.freedesktop.DBus.ListNames"
            ]
        );
        assert_eq!(
            get_property_argv("org.mpris.MediaPlayer2.vlc", "PlaybackStatus"),
            vec![
                "call",
                "--session",
                "--dest",
                "org.mpris.MediaPlayer2.vlc",
                "--object-path",
                "/org/mpris/MediaPlayer2",
                "--method",
                "org.freedesktop.DBus.Properties.Get",
                "org.mpris.MediaPlayer2.Player",
                "PlaybackStatus"
            ]
        );
        assert_eq!(
            call_member_argv("org.mpris.MediaPlayer2.vlc", MPRIS_PLAYER_IFACE, "PlayPause")
                .last()
                .unwrap(),
            "org.mpris.MediaPlayer2.Player.PlayPause"
        );
    }

    #[test]
    fn paging_is_deterministic_and_cursor_checked() {
        let mk = |bus: &str| MediaPlayerInfo {
            player: parse_player_id(bus).unwrap(),
            app: app_label(&parse_player_id(bus).unwrap()),
            playback_state: "Playing".into(),
            track_label: None,
            track_id: None,
        };
        let all = vec![
            mk("org.mpris.MediaPlayer2.vlc"),
            mk("org.mpris.MediaPlayer2.firefox"),
        ];
        let first = page(all.clone(), None, Some(1)).expect("page");
        assert_eq!(first.items[0].app, "firefox");
        assert!(first.truncated);
        let cursor = first.next_cursor.clone().unwrap();
        let second = page(all.clone(), Some(&cursor), Some(1)).expect("page");
        assert_eq!(second.items[0].app, "vlc");
        assert!(page(all, Some("mp1.0.0000000000000000"), None).is_err());
    }
}
