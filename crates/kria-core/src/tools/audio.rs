//! Audio tool handlers — input endpoints, per-application streams, device
//! profiles, and MPRIS media control.
//!
//! linux-os-control-production tasks **3.6** and **5.2** (OSC-018).
//!
//! Every handler routes through [`crate::tools::os_governed`]; none of them
//! touches a process, a bus, or `wpctl` directly.
//!
//! # Two privacy/correctness rules shape this file
//!
//! * **Microphone mute is a privacy control.** Reporting "muted" because a read
//!   failed would tell the user their microphone is off while it is still live.
//!   Every mute read fails closed instead.
//! * **A stream is identified by its node id, never by application name.** Two
//!   windows of the same browser are two streams, names are not unique, and a
//!   name can change while the stream lives. The same holds for a media player:
//!   its MPRIS bus name is stable, its window title is not.

use std::sync::Arc;

use async_trait::async_trait;

use crate::infra::ToolResult;
use crate::os_control::audio::media::{MediaPlaybackAction, MediaRequest};
use crate::os_control::audio::streams::{AudioStreamOp, AudioStreamRequest};
use crate::os_control::audio::{
    AudioEndpointId, AudioEndpointKind, AudioOp, AudioProfileId, AudioRequest, AudioStreamId,
};
use crate::safety::RiskLevel;
use crate::tools::os_governed as gov;
use crate::tools::registry::{ParamDef, ToolDef, ToolHandler, ToolRegistry};
use crate::tools::ToolContext;

fn param(name: &str, ty: &str, desc: &str, required: bool) -> ParamDef {
    ParamDef {
        name: name.into(),
        param_type: ty.into(),
        description: desc.into(),
        required,
        default: None,
    }
}

/// Read a required string parameter, rejecting anything that could be read as a
/// command option or that carries control characters.
fn required_id(params: &serde_json::Value, field: &str) -> Result<String, ToolResult> {
    let raw = params[field].as_str().unwrap_or("").trim();
    if raw.is_empty() {
        return Err(ToolResult::err(format!("`{field}` is required")));
    }
    if raw.len() > 256 || raw.chars().any(char::is_control) {
        return Err(ToolResult::err(format!(
            "`{field}` is too long or contains control characters"
        )));
    }
    if raw.starts_with('-') {
        return Err(ToolResult::err(format!(
            "`{field}` must not start with `-`: it would be read as a command option"
        )));
    }
    Ok(raw.to_string())
}

/// Read a percentage, refusing an out-of-range value rather than clamping it.
///
/// Clamping would silently do something the user did not ask for; a rejected
/// request is honest and costs nothing.
fn required_percent(params: &serde_json::Value, field: &str) -> Result<u8, ToolResult> {
    let raw = params[field]
        .as_u64()
        .ok_or_else(|| ToolResult::err(format!("`{field}` must be an integer 0-100")))?;
    u8::try_from(raw)
        .ok()
        .filter(|value| *value <= 100)
        .ok_or_else(|| ToolResult::err(format!("`{field}` must be between 0 and 100")))
}

fn required_bool(params: &serde_json::Value, field: &str) -> Result<bool, ToolResult> {
    params[field]
        .as_bool()
        .ok_or_else(|| ToolResult::err(format!("`{field}` must be a boolean")))
}

// ─────────────────────────────────────────────────────────────────────────────
// Endpoint-level mutations (Task 3.6)
// ─────────────────────────────────────────────────────────────────────────────

/// Drive one governed endpoint-level audio mutation.
async fn run_endpoint(
    ctx: &ToolContext,
    tool: &str,
    params: serde_json::Value,
    op: AudioOp,
    endpoint: AudioEndpointKind,
) -> ToolResult {
    let resolved = match gov::resolve(ctx, tool) {
        Ok(resolved) => resolved,
        Err(result) => return result,
    };
    let provider = match resolved.runtime.audio(tool) {
        Ok(provider) => provider,
        Err(error) => return gov::os_error(&error),
    };
    let call = match gov::mutation_call(ctx, &resolved.runtime, tool) {
        Ok(call) => call,
        Err(result) => return result,
    };
    let request = AudioRequest {
        action: tool.to_string(),
        params,
        op,
        endpoint,
    };
    // A mutation always has a desired state; `None` would mean this op is a read,
    // which must never reach the mutation path.
    let Some(desired) = request.desired_state() else {
        return ToolResult::err(format!(
            "`{tool}` has no desired state and cannot be applied as a mutation"
        ));
    };
    let plan = gov::plan_for(resolved.provider_id, request.comparator(), request.tolerance());
    gov::run_mutation(
        tool,
        &resolved.runtime,
        provider,
        &call,
        &request,
        &desired,
        &plan,
    )
    .await
}

struct SetMicrophoneLevel;

#[async_trait]
impl ToolHandler for SetMicrophoneLevel {
    async fn execute(&self, _params: serde_json::Value) -> ToolResult {
        gov::os_unavailable(None, "set_microphone_level")
    }

    async fn execute_with_context(
        &self,
        params: serde_json::Value,
        ctx: ToolContext,
    ) -> ToolResult {
        let tool = "set_microphone_level";
        let level = match required_percent(&params, "level") {
            Ok(level) => level,
            Err(result) => return result,
        };
        // The op is the same shape as the output level; the endpoint kind is what
        // directs it at the capture device.
        run_endpoint(
            &ctx,
            tool,
            params,
            AudioOp::SetOutputLevel(level),
            AudioEndpointKind::Input,
        )
        .await
    }
}

struct SetMicrophoneMute;

#[async_trait]
impl ToolHandler for SetMicrophoneMute {
    async fn execute(&self, _params: serde_json::Value) -> ToolResult {
        gov::os_unavailable(None, "set_microphone_mute")
    }

    async fn execute_with_context(
        &self,
        params: serde_json::Value,
        ctx: ToolContext,
    ) -> ToolResult {
        let tool = "set_microphone_mute";
        let muted = match required_bool(&params, "muted") {
            Ok(muted) => muted,
            Err(result) => return result,
        };
        // Privacy-critical: the verification re-reads the real mute state, so an
        // unreadable microphone reports failure rather than "muted".
        run_endpoint(
            &ctx,
            tool,
            params,
            AudioOp::SetOutputMute(muted),
            AudioEndpointKind::Input,
        )
        .await
    }
}

struct SetDefaultAudioEndpoint {
    tool: &'static str,
    endpoint: AudioEndpointKind,
}

#[async_trait]
impl ToolHandler for SetDefaultAudioEndpoint {
    async fn execute(&self, _params: serde_json::Value) -> ToolResult {
        gov::os_unavailable(None, self.tool)
    }

    async fn execute_with_context(
        &self,
        params: serde_json::Value,
        ctx: ToolContext,
    ) -> ToolResult {
        let device = match required_id(&params, "device") {
            Ok(device) => device,
            Err(result) => return result,
        };
        // Changing the default endpoint means any volume previously read no longer
        // describes the active device; verification re-reads against the NEW
        // default rather than assuming the old reading still holds.
        run_endpoint(
            &ctx,
            self.tool,
            params,
            AudioOp::SetDefaultEndpoint(match AudioEndpointId::parse(&device) {
                Ok(id) => id,
                Err(error) => return gov::os_error(&error),
            }),
            self.endpoint,
        )
        .await
    }
}

struct SetAudioDeviceProfile;

#[async_trait]
impl ToolHandler for SetAudioDeviceProfile {
    async fn execute(&self, _params: serde_json::Value) -> ToolResult {
        gov::os_unavailable(None, "set_audio_device_profile")
    }

    async fn execute_with_context(
        &self,
        params: serde_json::Value,
        ctx: ToolContext,
    ) -> ToolResult {
        let tool = "set_audio_device_profile";
        let device = match required_id(&params, "device") {
            Ok(device) => device,
            Err(result) => return result,
        };
        let profile = match required_id(&params, "profile") {
            Ok(profile) => profile,
            Err(result) => return result,
        };
        run_endpoint(
            &ctx,
            tool,
            params,
            AudioOp::SetProfile {
                endpoint: match AudioEndpointId::parse(&device) {
                    Ok(id) => id,
                    Err(error) => return gov::os_error(&error),
                },
                profile: match AudioProfileId::parse(&profile) {
                    Ok(id) => id,
                    Err(error) => return gov::os_error(&error),
                },
                // Port selection is a separate concern; absent means "leave the
                // port as it is" rather than a guessed default.
                port: None,
            },
            AudioEndpointKind::Output,
        )
        .await
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Per-application streams (Task 5.2)
// ─────────────────────────────────────────────────────────────────────────────

async fn run_stream(
    ctx: &ToolContext,
    tool: &str,
    params: serde_json::Value,
    stream: AudioStreamId,
    op: AudioStreamOp,
) -> ToolResult {
    let resolved = match gov::resolve(ctx, tool) {
        Ok(resolved) => resolved,
        Err(result) => return result,
    };
    let provider = match resolved.runtime.audio(tool) {
        Ok(provider) => provider,
        Err(error) => return gov::os_error(&error),
    };
    let call = match gov::mutation_call(ctx, &resolved.runtime, tool) {
        Ok(call) => call,
        Err(result) => return result,
    };
    let request = AudioStreamRequest {
        action: tool.to_string(),
        params,
        stream,
        op,
    };
    let desired = request.desired_state();
    let plan = gov::plan_for(resolved.provider_id, request.comparator(), request.tolerance());
    gov::run_mutation(
        tool,
        &resolved.runtime,
        provider,
        &call,
        &request,
        &desired,
        &plan,
    )
    .await
}

struct ListAudioStreams;

#[async_trait]
impl ToolHandler for ListAudioStreams {
    async fn execute(&self, _params: serde_json::Value) -> ToolResult {
        gov::os_unavailable(None, "list_audio_streams")
    }

    async fn execute_with_context(
        &self,
        params: serde_json::Value,
        ctx: ToolContext,
    ) -> ToolResult {
        let tool = "list_audio_streams";
        let resolved = match gov::resolve(&ctx, tool) {
            Ok(resolved) => resolved,
            Err(result) => return result,
        };
        let provider = match resolved.runtime.audio(tool) {
            Ok(provider) => provider,
            Err(error) => return gov::os_error(&error),
        };
        let call = match gov::read_call(&ctx, &resolved.runtime, tool) {
            Ok(call) => call,
            Err(result) => return result,
        };
        let cursor = params["cursor"].as_str();
        let limit = params["limit"].as_u64().and_then(|v| usize::try_from(v).ok());
        match provider.stream_page(call.observation(), cursor, limit).await {
            Ok(page) => ToolResult::ok(serde_json::json!({
                "streams": page.items.iter().map(|s| serde_json::json!({
                    // The node id is the identity a mutation must be addressed to.
                    "stream": s.stream.as_str(),
                    "app": s.app.as_str(),
                    "level_percent": s.level_percent,
                    "muted": s.muted,
                })).collect::<Vec<_>>(),
                "next_cursor": page.next_cursor.as_deref(),
                "truncated": page.truncated,
            })),
            Err(error) => gov::os_error(&error),
        }
    }
}

struct SetApplicationVolume;

#[async_trait]
impl ToolHandler for SetApplicationVolume {
    async fn execute(&self, _params: serde_json::Value) -> ToolResult {
        gov::os_unavailable(None, "set_application_volume")
    }

    async fn execute_with_context(
        &self,
        params: serde_json::Value,
        ctx: ToolContext,
    ) -> ToolResult {
        let tool = "set_application_volume";
        let stream = match required_id(&params, "stream") {
            Ok(stream) => stream,
            Err(result) => return result,
        };
        let level = match required_percent(&params, "level") {
            Ok(level) => level,
            Err(result) => return result,
        };
        run_stream(
            &ctx,
            tool,
            params,
            match AudioStreamId::parse(&stream) {
                Ok(id) => id,
                Err(error) => return gov::os_error(&error),
            },
            AudioStreamOp::SetLevel(level),
        )
        .await
    }
}

struct SetApplicationMute;

#[async_trait]
impl ToolHandler for SetApplicationMute {
    async fn execute(&self, _params: serde_json::Value) -> ToolResult {
        gov::os_unavailable(None, "set_application_mute")
    }

    async fn execute_with_context(
        &self,
        params: serde_json::Value,
        ctx: ToolContext,
    ) -> ToolResult {
        let tool = "set_application_mute";
        let stream = match required_id(&params, "stream") {
            Ok(stream) => stream,
            Err(result) => return result,
        };
        let muted = match required_bool(&params, "muted") {
            Ok(muted) => muted,
            Err(result) => return result,
        };
        run_stream(
            &ctx,
            tool,
            params,
            match AudioStreamId::parse(&stream) {
                Ok(id) => id,
                Err(error) => return gov::os_error(&error),
            },
            AudioStreamOp::SetMute(muted),
        )
        .await
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// MPRIS media control (Task 5.2)
// ─────────────────────────────────────────────────────────────────────────────

struct ListMediaPlayers;

#[async_trait]
impl ToolHandler for ListMediaPlayers {
    async fn execute(&self, _params: serde_json::Value) -> ToolResult {
        gov::os_unavailable(None, "list_media_players")
    }

    async fn execute_with_context(
        &self,
        params: serde_json::Value,
        ctx: ToolContext,
    ) -> ToolResult {
        let tool = "list_media_players";
        let resolved = match gov::resolve(&ctx, tool) {
            Ok(resolved) => resolved,
            Err(result) => return result,
        };
        let provider = match resolved.runtime.audio(tool) {
            Ok(provider) => provider,
            Err(error) => return gov::os_error(&error),
        };
        let call = match gov::read_call(&ctx, &resolved.runtime, tool) {
            Ok(call) => call,
            Err(result) => return result,
        };
        let cursor = params["cursor"].as_str();
        let limit = params["limit"].as_u64().and_then(|v| usize::try_from(v).ok());
        match provider
            .media_player_page(call.observation(), cursor, limit)
            .await
        {
            Ok(page) => ToolResult::ok(serde_json::json!({
                "players": page.items.iter().map(|p| serde_json::json!({
                    // The MPRIS bus name is stable; a window title is not.
                    "player": p.player.as_str(),
                    "app": p.app.as_str(),
                    "playback_state": p.playback_state.as_str(),
                    // Track text is user content, so it is reported as-is only
                    // because the contract asks for it — never logged elsewhere.
                    "track": p.track_label.as_deref(),
                })).collect::<Vec<_>>(),
                "next_cursor": page.next_cursor.as_deref(),
                "truncated": page.truncated,
            })),
            Err(error) => gov::os_error(&error),
        }
    }
}

struct ControlMediaPlayback;

#[async_trait]
impl ToolHandler for ControlMediaPlayback {
    async fn execute(&self, _params: serde_json::Value) -> ToolResult {
        gov::os_unavailable(None, "control_media_playback")
    }

    async fn execute_with_context(
        &self,
        params: serde_json::Value,
        ctx: ToolContext,
    ) -> ToolResult {
        let tool = "control_media_playback";
        let player = match required_id(&params, "player") {
            Ok(player) => player,
            Err(result) => return result,
        };
        let action_raw = match required_id(&params, "action") {
            Ok(action) => action,
            Err(result) => return result,
        };
        // A closed action set: parsed, never passed through as a D-Bus member name.
        let playback = match MediaPlaybackAction::parse(&action_raw) {
            Ok(playback) => playback,
            Err(error) => return gov::os_error(&error),
        };
        let player = match crate::os_control::audio::media::parse_player_id(&player) {
            Ok(player) => player,
            Err(error) => return gov::os_error(&error),
        };

        let resolved = match gov::resolve(&ctx, tool) {
            Ok(resolved) => resolved,
            Err(result) => return result,
        };
        let provider = match resolved.runtime.audio(tool) {
            Ok(provider) => provider,
            Err(error) => return gov::os_error(&error),
        };
        let call = match gov::mutation_call(&ctx, &resolved.runtime, tool) {
            Ok(call) => call,
            Err(result) => return result,
        };
        let request = MediaRequest {
            action: tool.to_string(),
            params,
            player,
            playback,
        };
        let desired = request.desired_state();
        // Playback state is an exact match, so there is no tolerance to apply.
        let plan = gov::plan_for(resolved.provider_id, request.comparator(), None);
        gov::run_mutation(
            tool,
            &resolved.runtime,
            provider,
            &call,
            &request,
            &desired,
            &plan,
        )
        .await
    }
}

/// Register the audio tool surface.
pub fn register(registry: &ToolRegistry) {
    let stream_param = || {
        param(
            "stream",
            "string",
            "Audio stream node id from list_audio_streams. An application NAME is not accepted: two windows of the same app are different streams.",
            true,
        )
    };

    let tools: Vec<(ToolDef, Arc<dyn ToolHandler>)> = vec![
        (
            ToolDef {
                name: "set_microphone_level".into(),
                description: "Set the microphone (default input) level 0-100".into(),
                category: "audio".into(),
                default_tier: RiskLevel::Yellow,
                min_tier: "lite",
                parameters: vec![param("level", "integer", "Input level 0-100", true)],
            },
            Arc::new(SetMicrophoneLevel),
        ),
        (
            ToolDef {
                name: "set_microphone_mute".into(),
                description: "Mute or unmute the microphone".into(),
                category: "audio".into(),
                // Privacy control: unmuting exposes the room.
                default_tier: RiskLevel::Yellow,
                min_tier: "lite",
                parameters: vec![param("muted", "boolean", "Desired mute state", true)],
            },
            Arc::new(SetMicrophoneMute),
        ),
        (
            ToolDef {
                name: "set_default_audio_output".into(),
                description: "Choose the default audio output device".into(),
                category: "audio".into(),
                default_tier: RiskLevel::Yellow,
                min_tier: "lite",
                parameters: vec![param("device", "string", "Output device id", true)],
            },
            Arc::new(SetDefaultAudioEndpoint {
                tool: "set_default_audio_output",
                endpoint: AudioEndpointKind::Output,
            }),
        ),
        (
            ToolDef {
                name: "set_default_audio_input".into(),
                description: "Choose the default audio input device".into(),
                category: "audio".into(),
                default_tier: RiskLevel::Yellow,
                min_tier: "lite",
                parameters: vec![param("device", "string", "Input device id", true)],
            },
            Arc::new(SetDefaultAudioEndpoint {
                tool: "set_default_audio_input",
                endpoint: AudioEndpointKind::Input,
            }),
        ),
        (
            ToolDef {
                name: "set_audio_device_profile".into(),
                description: "Switch an audio device to a different profile".into(),
                category: "audio".into(),
                default_tier: RiskLevel::Yellow,
                min_tier: "lite",
                parameters: vec![
                    param("device", "string", "Audio device id", true),
                    param("profile", "string", "Profile id supported by the device", true),
                ],
            },
            Arc::new(SetAudioDeviceProfile),
        ),
        (
            ToolDef {
                name: "list_audio_streams".into(),
                description: "List live per-application audio streams".into(),
                category: "audio".into(),
                default_tier: RiskLevel::Green,
                min_tier: "lite",
                parameters: vec![
                    param("cursor", "string", "Page cursor", false),
                    param("limit", "integer", "Maximum rows", false),
                ],
            },
            Arc::new(ListAudioStreams),
        ),
        (
            ToolDef {
                name: "set_application_volume".into(),
                description: "Set one application's audio stream level 0-100".into(),
                category: "audio".into(),
                default_tier: RiskLevel::Yellow,
                min_tier: "lite",
                parameters: vec![
                    stream_param(),
                    param("level", "integer", "Stream level 0-100", true),
                ],
            },
            Arc::new(SetApplicationVolume),
        ),
        (
            ToolDef {
                name: "set_application_mute".into(),
                description: "Mute or unmute one application's audio stream".into(),
                category: "audio".into(),
                default_tier: RiskLevel::Yellow,
                min_tier: "lite",
                parameters: vec![
                    stream_param(),
                    param("muted", "boolean", "Desired mute state", true),
                ],
            },
            Arc::new(SetApplicationMute),
        ),
        (
            ToolDef {
                name: "list_media_players".into(),
                description: "List MPRIS media players and their playback state".into(),
                category: "audio".into(),
                default_tier: RiskLevel::Green,
                min_tier: "lite",
                parameters: vec![
                    param("cursor", "string", "Page cursor", false),
                    param("limit", "integer", "Maximum rows", false),
                ],
            },
            Arc::new(ListMediaPlayers),
        ),
        (
            ToolDef {
                name: "control_media_playback".into(),
                description: "Play, pause, stop, or skip in a media player".into(),
                category: "audio".into(),
                default_tier: RiskLevel::Yellow,
                min_tier: "lite",
                parameters: vec![
                    param(
                        "player",
                        "string",
                        "MPRIS player id from list_media_players (a window title is not accepted)",
                        true,
                    ),
                    param(
                        "action",
                        "string",
                        "One of: play, pause, play_pause, stop, next, previous",
                        true,
                    ),
                ],
            },
            Arc::new(ControlMediaPlayback),
        ),
    ];

    for (def, handler) in tools {
        registry.register(def, handler);
    }
}
