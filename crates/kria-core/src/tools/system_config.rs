use crate::infra::ToolResult;
use crate::os_control::contract::SafeText;
use crate::os_control::{OsControlError, OsControlRuntime};
use crate::safety::RiskLevel;
use crate::tools::os_governed as gov;
use crate::tools::registry::{ParamDef, ToolDef, ToolHandler, ToolRegistry};
use crate::tools::ToolContext;
use async_trait::async_trait;
use schemars::JsonSchema;
use serde::de::{self, DeserializeOwned};
use serde::Deserialize;
use std::sync::Arc;

fn param(name: &str, ty: &str, desc: &str, required: bool) -> ParamDef {
    ParamDef {
        name: name.into(),
        param_type: ty.into(),
        description: desc.into(),
        required,
        default: None,
    }
}

fn parse_input<T: DeserializeOwned>(params: serde_json::Value) -> Result<T, ToolResult> {
    serde_json::from_value(params)
        .map_err(|error| ToolResult::err(format!("invalid parameters: {error}")))
}

#[derive(Debug, Clone, Copy, JsonSchema)]
#[schemars(description = "Percentage from 0 to 100. Accepts numbers and strings like '60%'.")]
struct PercentLevel(u8);

impl PercentLevel {
    fn as_u8(self) -> u8 {
        self.0
    }
}

impl<'de> Deserialize<'de> for PercentLevel {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum RawLevel {
            Int(u64),
            Float(f64),
            Text(String),
        }

        let raw = RawLevel::deserialize(deserializer)?;
        let value = match raw {
            RawLevel::Int(value) => value,
            RawLevel::Float(value) => {
                if !value.is_finite() {
                    return Err(de::Error::custom("level must be a finite number"));
                }
                if value < 0.0 {
                    0
                } else {
                    value.round() as u64
                }
            }
            RawLevel::Text(value) => parse_level_text(&value).map_err(de::Error::custom)?,
        };

        Ok(Self(clamp_percent(value)))
    }
}

fn clamp_percent(value: u64) -> u8 {
    value.min(100) as u8
}

fn parse_level_text(text: &str) -> Result<u64, String> {
    let cleaned = text.trim().trim_end_matches('%').trim();
    if cleaned.is_empty() {
        return Err("level cannot be empty".into());
    }
    cleaned
        .parse::<u64>()
        .map_err(|_| format!("invalid level '{text}'"))
}

fn default_percent_level() -> PercentLevel {
    PercentLevel(50)
}

fn default_wifi_enabled() -> bool {
    true
}

fn default_power_plan() -> String {
    "balanced".to_string()
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct EmptyInput {}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct SetVolumeInput {
    // Accept the frozen-manifest name `percent` as well as the pre-migration
    // `level` for wire compatibility.
    #[serde(default = "default_percent_level", alias = "percent")]
    level: PercentLevel,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct SetAudioMuteInput {
    muted: bool,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct SetBrightnessInput {
    // Accept the frozen-manifest name `percent` as well as the pre-migration
    // `level` for wire compatibility.
    #[serde(default = "default_percent_level", alias = "percent")]
    level: PercentLevel,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct ToggleWifiInput {
    #[serde(default = "default_wifi_enabled")]
    enable: bool,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct SetPowerPlanInput {
    #[serde(default = "default_power_plan")]
    plan: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct ConnectWifiInput {
    ssid: String,
    #[serde(default)]
    password: Option<String>,
    /// The frozen manifest's typed `credential?:SecretRef` parameter (Task
    /// 3.5, OSC-015.3): an opaque reference to a stored credential, resolved
    /// through `CredentialStore` — never a plaintext value. Mutually
    /// exclusive with `password`.
    #[serde(default)]
    credential: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct DisconnectWifiInput {
    device: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct ForgetWifiInput {
    profile: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct ActivateNetworkProfileInput {
    profile: String,
    #[serde(default)]
    device: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct SetEnvironmentVariableInput {
    name: String,
    value: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct EnvironmentVariableInput {
    name: String,
}

fn validate_env_name(name: &str) -> Result<(), String> {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        return Err("environment variable name is required".into());
    }

    let mut chars = trimmed.chars();
    let Some(first) = chars.next() else {
        return Err("environment variable name is required".into());
    };

    if !(first.is_ascii_alphabetic() || first == '_') {
        return Err("invalid environment variable name".into());
    }

    if !chars.all(|c| c.is_ascii_alphanumeric() || c == '_') {
        return Err("invalid environment variable name".into());
    }

    Ok(())
}

/// Return the governed OS-control `Unavailable` envelope for an audio tool.
///
/// Migrated audio handlers reach host effects **only** through the injected
/// [`OsControlRuntime`] + `os_control::audio::AudioControl` provider — never a
/// direct subprocess (Task 2.1 completion proof). Until a live audio provider is
/// composed into the runtime (desktop startup root), the handlers fail closed
/// with this frozen envelope rather than falling back to `wpctl`/`pactl`/`amixer`.
/// Render a governed mutation receipt as a tool result.
///
/// Shared by every canonical OS handler so the surfaced shape is identical across
/// domains. It reports only what the receipt actually proves — `changed` and the
/// lifecycle come from the runtime's verification, never from the fact that a
/// command was dispatched — and it states whether the terminal audit record landed
/// durably, so a caller can tell a recorded action from one pending recovery.
fn render_os_receipt<O>(
    tool: &str,
    outcome: &crate::os_control::governed::GovernedOutcome<O>,
) -> ToolResult {
    let summary = outcome.receipt.safe_summary();
    ToolResult::ok(serde_json::json!({
        "tool": tool,
        "lifecycle": summary.lifecycle().as_str(),
        "changed": summary.changed(),
        "verified": matches!(
            summary.lifecycle(),
            crate::os_control::ActionLifecycle::Verified
        ),
        "rollback_available": outcome.receipt.rollback_available(),
        "durably_recorded": outcome.durably_recorded(),
        "incident_codes": summary
            .incident_codes()
            .iter()
            .map(|code| code.as_str().to_string())
            .collect::<Vec<_>>(),
    }))
}

fn os_audio_unavailable(runtime: Option<&Arc<OsControlRuntime>>, tool: &str) -> ToolResult {
    let err = match runtime {
        Some(rt) => rt.unavailable(tool),
        None => OsControlError::Unavailable {
            provider: None,
            reason: SafeText::new("OS control runtime is not injected in this build"),
            retryable: false,
        },
    };
    ToolResult::err_with_data(err.code(), err.to_envelope())
}

struct SetVolume;

#[async_trait]
impl ToolHandler for SetVolume {
    async fn execute(&self, _params: serde_json::Value) -> ToolResult {
        // No context path: cannot reach the governed runtime; fail closed with
        // the frozen envelope rather than invoking any process directly.
        os_audio_unavailable(None, "set_volume")
    }

    async fn execute_with_context(
        &self,
        params: serde_json::Value,
        ctx: ToolContext,
    ) -> ToolResult {
        let input: SetVolumeInput = match parse_input(params.clone()) {
            Ok(input) => input,
            Err(error) => return error,
        };

        if !cfg!(target_os = "linux") {
            return ToolResult::err("set_volume not implemented for this OS");
        }

        // The governed AudioControl provider owns the actual set/verify/rollback;
        // this handler only parses, builds the domain request/plan, and renders the
        // receipt. It never touches a process, a bus, or the audio device.
        let requested = input.level.as_u8();

        let Some(runtime) = ctx.os_runtime.clone() else {
            return os_audio_unavailable(None, "set_volume");
        };
        let provider = match runtime.audio("set_volume") {
            Ok(provider) => provider,
            Err(error) => return ToolResult::err_with_data(error.code(), error.to_envelope()),
        };
        // No governed call means the policy gate did not admit a host mutation
        // (blocked, awaiting approval, or a non-admitted path). Fail closed with the
        // frozen envelope rather than mutating.
        let Some(call) = ctx.os_call() else {
            return os_audio_unavailable(Some(&runtime), "set_volume");
        };
        let provider_id = match runtime.probe_provider("set_volume") {
            Ok(id) => id,
            Err(error) => return ToolResult::err_with_data(error.code(), error.to_envelope()),
        };

        let request = crate::os_control::audio::AudioRequest {
            action: "set_volume".to_string(),
            // The caller's ORIGINAL parameters: the grant's params digest was
            // taken from these, and rebuilding the object here would make the
            // binding check fail with grant_invalid. The normalized value
            // travels in the typed desired-state instead.
            params: params.clone(),
            op: crate::os_control::audio::AudioOp::SetOutputLevel(requested),
            endpoint: crate::os_control::audio::AudioEndpointKind::Output,
        };
        let Some(desired) = request.desired_state() else {
            return ToolResult::err("set_volume produced no desired state");
        };

        let plan = crate::os_control::runtime::MutationPlan {
            receipt_id: crate::os_control::ReceiptId::new(uuid::Uuid::now_v7().to_string()),
            provider: provider_id,
            // Volume is numeric, so an observation within tolerance counts as
            // satisfied — this is what makes a repeat request idempotent instead of
            // dispatching a redundant command.
            comparator: crate::os_control::ComparatorKind::WithinTolerance,
            tolerance: Some(crate::os_control::Tolerance { abs: 2.0 }),
            deadline_ms: 500,
            // No rollback token is minted for volume yet; never advertise an
            // inverse the runtime cannot actually perform.
            rollback: crate::os_control::runtime::RollbackPlan::Unavailable,
            latency_ms: 0,
        };

        match crate::os_control::governed::execute_governed_mutation(
            &runtime,
            provider,
            call,
            crate::os_control::governed::audit_store(),
            &request,
            &desired,
            &plan,
        )
        .await
        {
            Ok(outcome) => render_os_receipt("set_volume", &outcome),
            Err(error) => ToolResult::err_with_data(error.code(), error.to_envelope()),
        }
    }
}

struct SetAudioMute;

#[async_trait]
impl ToolHandler for SetAudioMute {
    async fn execute(&self, _params: serde_json::Value) -> ToolResult {
        os_audio_unavailable(None, "set_audio_mute")
    }

    async fn execute_with_context(
        &self,
        params: serde_json::Value,
        ctx: ToolContext,
    ) -> ToolResult {
        let input: SetAudioMuteInput = match parse_input(params.clone()) {
            Ok(input) => input,
            Err(error) => return error,
        };

        if !cfg!(target_os = "linux") {
            return ToolResult::err("set_audio_mute not implemented for this OS");
        }

        // Mute is a boolean, so it verifies with the Exact comparator (no
        // tolerance): the observed state either matches the desired state or the
        // runtime treats it as a contradiction.
        let requested_muted = input.muted;

        let Some(runtime) = ctx.os_runtime.clone() else {
            return os_audio_unavailable(None, "set_audio_mute");
        };
        let provider = match runtime.audio("set_audio_mute") {
            Ok(provider) => provider,
            Err(error) => return ToolResult::err_with_data(error.code(), error.to_envelope()),
        };
        let Some(call) = ctx.os_call() else {
            return os_audio_unavailable(Some(&runtime), "set_audio_mute");
        };
        let provider_id = match runtime.probe_provider("set_audio_mute") {
            Ok(id) => id,
            Err(error) => return ToolResult::err_with_data(error.code(), error.to_envelope()),
        };

        let request = crate::os_control::audio::AudioRequest {
            action: "set_audio_mute".to_string(),
            // The caller's ORIGINAL parameters: the grant's params digest was
            // taken from these, and rebuilding the object here would make the
            // binding check fail with grant_invalid. The normalized value
            // travels in the typed desired-state instead.
            params: params.clone(),
            op: crate::os_control::audio::AudioOp::SetOutputMute(requested_muted),
            endpoint: crate::os_control::audio::AudioEndpointKind::Output,
        };
        let Some(desired) = request.desired_state() else {
            return ToolResult::err("set_audio_mute produced no desired state");
        };

        let plan = crate::os_control::runtime::MutationPlan {
            receipt_id: crate::os_control::ReceiptId::new(uuid::Uuid::now_v7().to_string()),
            provider: provider_id,
            comparator: crate::os_control::ComparatorKind::Exact,
            tolerance: None,
            deadline_ms: 500,
            rollback: crate::os_control::runtime::RollbackPlan::Unavailable,
            latency_ms: 0,
        };

        match crate::os_control::governed::execute_governed_mutation(
            &runtime,
            provider,
            call,
            crate::os_control::governed::audit_store(),
            &request,
            &desired,
            &plan,
        )
        .await
        {
            Ok(outcome) => render_os_receipt("set_audio_mute", &outcome),
            Err(error) => ToolResult::err_with_data(error.code(), error.to_envelope()),
        }
    }
}

struct GetAudioState;

#[async_trait]
impl ToolHandler for GetAudioState {
    async fn execute(&self, _params: serde_json::Value) -> ToolResult {
        os_audio_unavailable(None, "get_audio_state")
    }

    async fn execute_with_context(
        &self,
        params: serde_json::Value,
        ctx: ToolContext,
    ) -> ToolResult {
        let _input: EmptyInput = match parse_input(params.clone()) {
            Ok(input) => input,
            Err(error) => return error,
        };

        if !cfg!(target_os = "linux") {
            return ToolResult::ok(serde_json::json!({ "backend": "unsupported" }));
        }

        let resolved = match gov::resolve(&ctx, "get_audio_state") {
            Ok(resolved) => resolved,
            Err(result) => return result,
        };
        let provider = match resolved.runtime.audio("get_audio_state") {
            Ok(provider) => provider,
            Err(error) => return gov::os_error(&error),
        };
        let call = match gov::read_call(&ctx, &resolved.runtime, "get_audio_state") {
            Ok(call) => call,
            Err(result) => return result,
        };
        let request = crate::os_control::audio::AudioRequest {
            action: "get_audio_state".to_string(),
            // The caller's ORIGINAL parameters: the grant's params digest was
            // taken from these, and rebuilding the object here would make the
            // binding check fail with grant_invalid. The normalized value
            // travels in the typed desired-state instead.
            params: params.clone(),
            op: crate::os_control::audio::AudioOp::GetState,
            endpoint: crate::os_control::audio::AudioEndpointKind::Output,
        };
        gov::run_read(provider, call, &request, |state| {
            serde_json::json!({
                "volume_percent": state.volume_percent,
                "muted": state.muted,
            })
        })
        .await
    }
}

/// Return the governed OS-control `Unavailable` envelope for a display tool.
///
/// Migrated display handlers reach host effects **only** through the injected
/// [`OsControlRuntime`] + `os_control::display::DisplayControl` provider —
/// never a direct subprocess (Task 2.2 completion proof). Until a live display
/// provider is composed into the runtime (desktop startup root), the handlers
/// fail closed with this frozen envelope rather than falling back to
/// `gdbus`/`brightnessctl`/`xrandr`.
fn os_display_unavailable(runtime: Option<&Arc<OsControlRuntime>>, tool: &str) -> ToolResult {
    let err = match runtime {
        Some(rt) => rt.unavailable(tool),
        None => OsControlError::Unavailable {
            provider: None,
            reason: SafeText::new("OS control runtime is not injected in this build"),
            retryable: false,
        },
    };
    ToolResult::err_with_data(err.code(), err.to_envelope())
}

struct SetBrightness;

#[async_trait]
impl ToolHandler for SetBrightness {
    async fn execute(&self, _params: serde_json::Value) -> ToolResult {
        // No context path: cannot reach the governed runtime; fail closed with
        // the frozen envelope rather than invoking any process directly.
        os_display_unavailable(None, "set_brightness")
    }

    async fn execute_with_context(
        &self,
        params: serde_json::Value,
        ctx: ToolContext,
    ) -> ToolResult {
        let input: SetBrightnessInput = match parse_input(params.clone()) {
            Ok(input) => input,
            Err(error) => return error,
        };

        if !cfg!(target_os = "linux") {
            return ToolResult::err("set_brightness not implemented for this OS");
        }

        // The governed DisplayControl provider owns set/verify/rollback and the
        // backend choice — it distinguishes physical backlight from X11-only
        // XRandR gamma and never selects XRandR on Wayland (OSC-019.3/OSC-032.3).
        let requested = input.level.as_u8();

        let Some(runtime) = ctx.os_runtime.clone() else {
            return os_display_unavailable(None, "set_brightness");
        };
        let provider = match runtime.display("set_brightness") {
            Ok(provider) => provider,
            Err(error) => return ToolResult::err_with_data(error.code(), error.to_envelope()),
        };
        let Some(call) = ctx.os_call() else {
            return os_display_unavailable(Some(&runtime), "set_brightness");
        };
        let provider_id = match runtime.probe_provider("set_brightness") {
            Ok(id) => id,
            Err(error) => return ToolResult::err_with_data(error.code(), error.to_envelope()),
        };

        let request = crate::os_control::display::DisplayRequest {
            action: "set_brightness".to_string(),
            // The caller's ORIGINAL parameters: the grant's params digest was
            // taken from these, and rebuilding the object here would make the
            // binding check fail with grant_invalid. The normalized value
            // travels in the typed desired-state instead.
            params: params.clone(),
            op: crate::os_control::display::DisplayOp::SetBrightness(requested),
        };
        // The desired state depends on which backend actually applies, so it is
        // read from the composed port rather than assumed.
        let Some(desired) = request.desired_state(provider.backend()) else {
            return ToolResult::err("set_brightness produced no desired state");
        };

        let plan = crate::os_control::runtime::MutationPlan {
            receipt_id: crate::os_control::ReceiptId::new(uuid::Uuid::now_v7().to_string()),
            provider: provider_id,
            // Comparator and tolerance come from the request's own frozen rule
            // rather than being restated here.
            comparator: request.comparator(),
            tolerance: request.tolerance(),
            deadline_ms: 500,
            rollback: crate::os_control::runtime::RollbackPlan::Unavailable,
            latency_ms: 0,
        };

        match crate::os_control::governed::execute_governed_mutation(
            &runtime,
            provider,
            call,
            crate::os_control::governed::audit_store(),
            &request,
            &desired,
            &plan,
        )
        .await
        {
            Ok(outcome) => render_os_receipt("set_brightness", &outcome),
            Err(error) => ToolResult::err_with_data(error.code(), error.to_envelope()),
        }
    }
}

struct GetDisplayState;

#[async_trait]
impl ToolHandler for GetDisplayState {
    async fn execute(&self, _params: serde_json::Value) -> ToolResult {
        os_display_unavailable(None, "get_display_state")
    }

    async fn execute_with_context(
        &self,
        params: serde_json::Value,
        ctx: ToolContext,
    ) -> ToolResult {
        let _input: EmptyInput = match parse_input(params.clone()) {
            Ok(input) => input,
            Err(error) => return error,
        };

        if !cfg!(target_os = "linux") {
            return ToolResult::ok(serde_json::json!({ "backend": "unsupported" }));
        }

        let resolved = match gov::resolve(&ctx, "get_display_state") {
            Ok(resolved) => resolved,
            Err(result) => return result,
        };
        let provider = match resolved.runtime.display("get_display_state") {
            Ok(provider) => provider,
            Err(error) => return gov::os_error(&error),
        };
        let call = match gov::read_call(&ctx, &resolved.runtime, "get_display_state") {
            Ok(call) => call,
            Err(result) => return result,
        };
        let request = crate::os_control::display::DisplayRequest {
            action: "get_display_state".to_string(),
            // The caller's ORIGINAL parameters: the grant's params digest was
            // taken from these, and rebuilding the object here would make the
            // binding check fail with grant_invalid. The normalized value
            // travels in the typed desired-state instead.
            params: params.clone(),
            op: crate::os_control::display::DisplayOp::GetState,
        };
        gov::run_read(provider, call, &request, |state| {
            serde_json::json!({
                "brightness_percent": state.brightness_percent,
                "backend": state.backend.as_str(),
            })
        })
        .await
    }
}

/// Return the governed OS-control `Unavailable` envelope for a connectivity
/// (Wi-Fi) tool.
///
/// Migrated connectivity handlers reach host effects **only** through the
/// injected [`OsControlRuntime`] +
/// `os_control::connectivity::ConnectivityControl` provider — never a direct
/// subprocess (Task 2.3 completion proof). Until a live NetworkManager
/// provider is composed into the runtime (desktop startup root), the handlers
/// fail closed with this frozen envelope rather than falling back to `nmcli`.
/// This mirrors `os_audio_unavailable`/`os_display_unavailable`: `ToolContext`
/// deliberately does not carry the `ExecutionGrant`/resource-lease/audit-
/// admission plumbing a real mutation requires (Tasks 2.1/2.2 scoping), so a
/// tool handler's only reachable outcome is either this frozen envelope or, in
/// a later task that wires that plumbing, a governed receipt.
fn os_connectivity_unavailable(runtime: Option<&Arc<OsControlRuntime>>, tool: &str) -> ToolResult {
    let err = match runtime {
        Some(rt) => rt.unavailable(tool),
        None => OsControlError::Unavailable {
            provider: None,
            reason: SafeText::new("OS control runtime is not injected in this build"),
            retryable: false,
        },
    };
    ToolResult::err_with_data(err.code(), err.to_envelope())
}

/// Return the governed OS-control `Unavailable` envelope for a power-profile
/// tool.
///
/// Migrated power-profile handlers reach host effects **only** through the
/// injected [`OsControlRuntime`] + `os_control::power::PowerControl` provider
/// — never a direct subprocess (Task 2.3 completion proof). Until a live
/// power-profiles provider is composed into the runtime (desktop startup
/// root), the handlers fail closed with this frozen envelope rather than
/// falling back to `powerprofilesctl`.
fn os_power_unavailable(runtime: Option<&Arc<OsControlRuntime>>, tool: &str) -> ToolResult {
    let err = match runtime {
        Some(rt) => rt.unavailable(tool),
        None => OsControlError::Unavailable {
            provider: None,
            reason: SafeText::new("OS control runtime is not injected in this build"),
            retryable: false,
        },
    };
    ToolResult::err_with_data(err.code(), err.to_envelope())
}

struct ToggleWifi;

#[async_trait]
impl ToolHandler for ToggleWifi {
    async fn execute(&self, _params: serde_json::Value) -> ToolResult {
        // No context path: cannot reach the governed runtime; fail closed with
        // the frozen envelope rather than invoking any process directly.
        os_connectivity_unavailable(None, "toggle_wifi")
    }

    async fn execute_with_context(
        &self,
        params: serde_json::Value,
        ctx: ToolContext,
    ) -> ToolResult {
        let input: ToggleWifiInput = match parse_input(params.clone()) {
            Ok(input) => input,
            Err(error) => return error,
        };

        if !cfg!(target_os = "linux") {
            return ToolResult::err("toggle_wifi not implemented for this OS");
        }

        // The governed ConnectivityControl provider owns set/verify/rollback.
        let requested_enabled = input.enable;
        let resolved = match gov::resolve(&ctx, "toggle_wifi") {
            Ok(resolved) => resolved,
            Err(result) => return result,
        };
        let provider = match resolved.runtime.connectivity("toggle_wifi") {
            Ok(provider) => provider,
            Err(error) => return gov::os_error(&error),
        };
        let call = match gov::mutation_call(&ctx, &resolved.runtime, "toggle_wifi") {
            Ok(call) => call,
            Err(result) => return result,
        };
        let request = crate::os_control::connectivity::ConnectivityRequest {
            action: "toggle_wifi".to_string(),
            // The caller's ORIGINAL parameters: the grant's params digest was
            // taken from these, and rebuilding the object here would make the
            // binding check fail with grant_invalid. The normalized value
            // travels in the typed desired-state instead.
            params: params.clone(),
            op: crate::os_control::connectivity::ConnectivityOp::ToggleRadio(
                requested_enabled,
            ),
        };
        let desired = request.desired_state();
        let plan = gov::plan_for(resolved.provider_id, request.comparator(), None);
        gov::run_mutation(
            "toggle_wifi",
            &resolved.runtime,
            provider,
            call,
            &request,
            &desired,
            &plan,
        )
        .await
    }
}

struct ConnectWifi;

#[async_trait]
impl ToolHandler for ConnectWifi {
    async fn execute(&self, _params: serde_json::Value) -> ToolResult {
        os_connectivity_unavailable(None, "connect_wifi")
    }

    async fn execute_with_context(
        &self,
        params: serde_json::Value,
        ctx: ToolContext,
    ) -> ToolResult {
        let input: ConnectWifiInput = match parse_input(params.clone()) {
            Ok(input) => input,
            Err(error) => return error,
        };

        if !cfg!(target_os = "linux") {
            return ToolResult::err("connect_wifi not implemented for this OS");
        }

        let ssid = input.ssid.trim();
        if ssid.is_empty() {
            return ToolResult::err("ssid parameter is required");
        }

        if input.password.is_some() && input.credential.is_some() {
            return ToolResult::err("password and credential are mutually exclusive");
        }

        // The password (if any) is accepted only as an ephemeral value for
        // this one request; it is never logged, stored on `self`, or placed in
        // any DTO/plan field. `credential` (Task 3.5) is the frozen manifest's
        // typed `SecretRef` — the governed ConnectivityControl provider
        // resolves it through `CredentialStore::resolve_for_operation` under
        // the admitted mutation context, scoped to `SecretPurpose::
        // WifiPassword` and this SSID, before dispatch. In both cases the
        // secret value is used only as a literal, non-shell argv element for
        // this one dispatch, redacted from every captured summary/trace
        // (OSC-025.4, OSC-029).
        let _password_present = input.password.is_some();
        let _credential_present = input.credential.is_some();
        let resolved = match gov::resolve(&ctx, "connect_wifi") {
            Ok(resolved) => resolved,
            Err(result) => return result,
        };
        let provider = match resolved.runtime.connectivity("connect_wifi") {
            Ok(provider) => provider,
            Err(error) => return gov::os_error(&error),
        };
        let call = match gov::mutation_call(&ctx, &resolved.runtime, "connect_wifi") {
            Ok(call) => call,
            Err(result) => return result,
        };
        let ssid = params["ssid"].as_str().unwrap_or_default().to_string();
        // The raw password never enters the canonical params (OSC-025.4). When the
        // caller supplies a stored reference instead, the provider resolves it
        // through the Secret Service under the admitted mutation context, scoped to
        // WifiPassword — the plaintext never reaches the model, plan, or audit.
        let credential = params["credential"]
            .as_str()
            .map(crate::os_control::secrets::SecretRef::new);
        let password = params["password"]
            .as_str()
            .map(|raw| {
                crate::os_control::secrets::SecretPayload::new(raw.as_bytes().to_vec())
            });
        let request = crate::os_control::connectivity::ConnectivityRequest {
            action: "connect_wifi".to_string(),
            // Canonical params carry the SSID only — never the secret.
            // The caller's ORIGINAL parameters: the grant's params digest was
            // taken from these, and rebuilding the object here would make the
            // binding check fail with grant_invalid. The normalized value
            // travels in the typed desired-state instead.
            params: params.clone(),
            op: crate::os_control::connectivity::ConnectivityOp::ConnectWifi(
                crate::os_control::connectivity::ConnectWifiOp {
                    ssid: ssid.clone(),
                    password,
                    credential,
                },
            ),
        };
        let desired = request.desired_state();
        let plan = gov::plan_for(resolved.provider_id, request.comparator(), None);
        gov::run_mutation(
            "connect_wifi",
            &resolved.runtime,
            provider,
            call,
            &request,
            &desired,
            &plan,
        )
        .await
    }
}

struct GetWifiNetworks;

#[async_trait]
impl ToolHandler for GetWifiNetworks {
    async fn execute(&self, _params: serde_json::Value) -> ToolResult {
        os_connectivity_unavailable(None, "get_wifi_networks")
    }

    async fn execute_with_context(
        &self,
        params: serde_json::Value,
        ctx: ToolContext,
    ) -> ToolResult {
        let _input: EmptyInput = match parse_input(params.clone()) {
            Ok(input) => input,
            Err(error) => return error,
        };

        if !cfg!(target_os = "linux") {
            return ToolResult::ok(serde_json::json!({ "networks": [] }));
        }

        // Scanning is a read passthrough on the port, outside the mutation
        // lifecycle: it never seals a permit and never dispatches a command.
        let resolved = match gov::resolve(&ctx, "get_wifi_networks") {
            Ok(resolved) => resolved,
            Err(result) => return result,
        };
        let provider = match resolved.runtime.connectivity("get_wifi_networks") {
            Ok(provider) => provider,
            Err(error) => return gov::os_error(&error),
        };
        let call = match gov::read_call(&ctx, &resolved.runtime, "get_wifi_networks") {
            Ok(call) => call,
            Err(result) => return result,
        };
        match provider.scan_wifi(call.observation()).await {
            Ok(rows) => ToolResult::ok(serde_json::json!({
                "networks": rows
                    .iter()
                    .map(|row| serde_json::json!({
                        "ssid": row.ssid,
                        "bssid": row.bssid,
                        "signal_percent": row.signal_percent,
                        "security": row.security,
                    }))
                    .collect::<Vec<_>>(),
            })),
            Err(error) => gov::os_error(&error),
        }
    }
}

struct GetNetworkState;

#[async_trait]
impl ToolHandler for GetNetworkState {
    async fn execute(&self, _params: serde_json::Value) -> ToolResult {
        os_connectivity_unavailable(None, "get_network_state")
    }

    async fn execute_with_context(
        &self,
        params: serde_json::Value,
        ctx: ToolContext,
    ) -> ToolResult {
        let _input: EmptyInput = match parse_input(params.clone()) {
            Ok(input) => input,
            Err(error) => return error,
        };

        if !cfg!(target_os = "linux") {
            return ToolResult::ok(serde_json::json!({ "devices": [], "profiles": [] }));
        }

        // The governed ConnectivityControl provider owns the actual
        // device/profile catalog read through the runtime.
        let resolved = match gov::resolve(&ctx, "get_network_state") {
            Ok(resolved) => resolved,
            Err(result) => return result,
        };
        let provider = match resolved.runtime.connectivity("get_network_state") {
            Ok(provider) => provider,
            Err(error) => return gov::os_error(&error),
        };
        let call = match gov::read_call(&ctx, &resolved.runtime, "get_network_state") {
            Ok(call) => call,
            Err(result) => return result,
        };
        let devices = match provider.list_devices(call.observation()).await {
            Ok(rows) => rows,
            Err(error) => return gov::os_error(&error),
        };
        let profiles = match provider.list_profiles(call.observation()).await {
            Ok(rows) => rows,
            Err(error) => return gov::os_error(&error),
        };
        ToolResult::ok(serde_json::json!({
            "devices": devices
                .iter()
                .map(|d| serde_json::json!({
                    "name": d.name,
                    "type": d.device_type,
                    "state": d.state,
                    "connected": d.is_connected(),
                }))
                .collect::<Vec<_>>(),
            "profiles": profiles
                .iter()
                .map(|p| serde_json::json!({
                    "name": p.name,
                    "uuid": p.uuid,
                    "type": p.connection_type,
                    "device": p.device,
                }))
                .collect::<Vec<_>>(),
        }))
    }
}

struct DisconnectWifi;

#[async_trait]
impl ToolHandler for DisconnectWifi {
    async fn execute(&self, _params: serde_json::Value) -> ToolResult {
        os_connectivity_unavailable(None, "disconnect_wifi")
    }

    async fn execute_with_context(
        &self,
        params: serde_json::Value,
        ctx: ToolContext,
    ) -> ToolResult {
        let input: DisconnectWifiInput = match parse_input(params.clone()) {
            Ok(input) => input,
            Err(error) => return error,
        };

        if !cfg!(target_os = "linux") {
            return ToolResult::err("disconnect_wifi not implemented for this OS");
        }

        let device = input.device.trim();
        if device.is_empty() {
            return ToolResult::err("device parameter is required");
        }

        // The governed ConnectivityControl provider owns the actual
        // disconnect dispatch + fresh device-state verification through the
        // runtime. `disconnect_wifi` never claims rollback (design §13.1:
        // `RollbackClaim::None`).
        let resolved = match gov::resolve(&ctx, "disconnect_wifi") {
            Ok(resolved) => resolved,
            Err(result) => return result,
        };
        let provider = match resolved.runtime.connectivity("disconnect_wifi") {
            Ok(provider) => provider,
            Err(error) => return gov::os_error(&error),
        };
        let call = match gov::mutation_call(&ctx, &resolved.runtime, "disconnect_wifi") {
            Ok(call) => call,
            Err(result) => return result,
        };
        let device = crate::os_control::connectivity::NetworkDeviceId::new(
            params["device"].as_str().unwrap_or_default(),
        );
        let request = crate::os_control::connectivity::ConnectivityRequest {
            action: "disconnect_wifi".to_string(),
            params: params.clone(),
            // Never claims rollback: disconnecting has no reliably restorable
            // prior positive action (design §13.1).
            op: crate::os_control::connectivity::ConnectivityOp::DisconnectWifi(device),
        };
        let desired = request.desired_state();
        let plan = gov::plan_for(resolved.provider_id, request.comparator(), None);
        gov::run_mutation(
            "disconnect_wifi",
            &resolved.runtime,
            provider,
            call,
            &request,
            &desired,
            &plan,
        )
        .await
    }
}

struct ForgetWifi;

#[async_trait]
impl ToolHandler for ForgetWifi {
    async fn execute(&self, _params: serde_json::Value) -> ToolResult {
        os_connectivity_unavailable(None, "forget_wifi")
    }

    async fn execute_with_context(
        &self,
        params: serde_json::Value,
        ctx: ToolContext,
    ) -> ToolResult {
        let input: ForgetWifiInput = match parse_input(params.clone()) {
            Ok(input) => input,
            Err(error) => return error,
        };

        if !cfg!(target_os = "linux") {
            return ToolResult::err("forget_wifi not implemented for this OS");
        }

        let profile = input.profile.trim();
        if profile.is_empty() {
            return ToolResult::err("profile parameter is required");
        }

        // The governed ConnectivityControl provider owns the actual profile
        // deletion + fresh profile-catalog verification through the runtime.
        // `forget_wifi` is RED and never claims rollback (design §13.1:
        // `RollbackClaim::None`) — the forget confirmation is enforced by the
        // RED approval gate, not by a second in-tool prompt.
        let resolved = match gov::resolve(&ctx, "forget_wifi") {
            Ok(resolved) => resolved,
            Err(result) => return result,
        };
        let provider = match resolved.runtime.connectivity("forget_wifi") {
            Ok(provider) => provider,
            Err(error) => return gov::os_error(&error),
        };
        let call = match gov::mutation_call(&ctx, &resolved.runtime, "forget_wifi") {
            Ok(call) => call,
            Err(result) => return result,
        };
        let profile = crate::os_control::connectivity::NetworkProfileId::new(
            params["profile"]
                .as_str()
                .or_else(|| params["ssid"].as_str())
                .unwrap_or_default(),
        );
        let request = crate::os_control::connectivity::ConnectivityRequest {
            action: "forget_wifi".to_string(),
            params: params.clone(),
            // RED and irreversible (design §13.1): the saved profile is deleted, so
            // the gate must have obtained explicit confirmation before this point.
            op: crate::os_control::connectivity::ConnectivityOp::ForgetProfile(profile),
        };
        let desired = request.desired_state();
        let plan = gov::plan_for(resolved.provider_id, request.comparator(), None);
        gov::run_mutation(
            "forget_wifi",
            &resolved.runtime,
            provider,
            call,
            &request,
            &desired,
            &plan,
        )
        .await
    }
}

struct ActivateNetworkProfile;

#[async_trait]
impl ToolHandler for ActivateNetworkProfile {
    async fn execute(&self, _params: serde_json::Value) -> ToolResult {
        os_connectivity_unavailable(None, "activate_network_profile")
    }

    async fn execute_with_context(
        &self,
        params: serde_json::Value,
        ctx: ToolContext,
    ) -> ToolResult {
        let input: ActivateNetworkProfileInput = match parse_input(params.clone()) {
            Ok(input) => input,
            Err(error) => return error,
        };

        if !cfg!(target_os = "linux") {
            return ToolResult::err("activate_network_profile not implemented for this OS");
        }

        let profile = input.profile.trim();
        if profile.is_empty() {
            return ToolResult::err("profile parameter is required");
        }

        // The governed ConnectivityControl provider owns device resolution,
        // duplicate-device disambiguation, dispatch, and fresh active-profile
        // verification through the runtime. Ethernet activation reuses this
        // exact tool — there is no separate Ethernet "connect" tool.
        let _device = input.device;
        let resolved = match gov::resolve(&ctx, "activate_network_profile") {
            Ok(resolved) => resolved,
            Err(result) => return result,
        };
        let provider = match resolved.runtime.connectivity("activate_network_profile") {
            Ok(provider) => provider,
            Err(error) => return gov::os_error(&error),
        };
        let call = match gov::mutation_call(&ctx, &resolved.runtime, "activate_network_profile") {
            Ok(call) => call,
            Err(result) => return result,
        };
        let profile = crate::os_control::connectivity::NetworkProfileId::new(
            params["profile"].as_str().unwrap_or_default(),
        );
        // Ethernet has no separate connect operation — it is just another saved
        // profile activated through this same variant (OSC-015.7).
        let device = params["device"]
            .as_str()
            .map(crate::os_control::connectivity::NetworkDeviceId::new);
        let request = crate::os_control::connectivity::ConnectivityRequest {
            action: "activate_network_profile".to_string(),
            params: params.clone(),
            op: crate::os_control::connectivity::ConnectivityOp::ActivateProfile {
                profile,
                device,
            },
        };
        let desired = request.desired_state();
        let plan = gov::plan_for(resolved.provider_id, request.comparator(), None);
        gov::run_mutation(
            "activate_network_profile",
            &resolved.runtime,
            provider,
            call,
            &request,
            &desired,
            &plan,
        )
        .await
    }
}

struct SetPowerPlan;

#[async_trait]
impl ToolHandler for SetPowerPlan {
    async fn execute(&self, _params: serde_json::Value) -> ToolResult {
        os_power_unavailable(None, "set_power_plan")
    }

    async fn execute_with_context(
        &self,
        params: serde_json::Value,
        ctx: ToolContext,
    ) -> ToolResult {
        // Cloned because the canonical params are also bound into the governed
        // request, where they must reproduce the grant's parameter digest.
        let input: SetPowerPlanInput = match parse_input(params.clone()) {
            Ok(input) => input,
            Err(error) => return error,
        };

        if !cfg!(target_os = "linux") {
            return ToolResult::err("set_power_plan not implemented for this OS");
        }

        let requested = input.plan.trim();
        if requested.is_empty() {
            return ToolResult::err("plan parameter is required");
        }
        let Some(profile) = crate::os_control::PowerProfile::parse(requested) else {
            return ToolResult::err(format!("unrecognized power plan '{requested}'"));
        };

        // The governed PowerControl provider owns the actual set/verify/
        // rollback through the runtime.
        let resolved = match gov::resolve(&ctx, "set_power_plan") {
            Ok(resolved) => resolved,
            Err(result) => return result,
        };
        let provider = match resolved.runtime.power("set_power_plan") {
            Ok(provider) => provider,
            Err(error) => return gov::os_error(&error),
        };
        let call = match gov::mutation_call(&ctx, &resolved.runtime, "set_power_plan") {
            Ok(call) => call,
            Err(result) => return result,
        };
        let request = crate::os_control::power::PowerProfileRequest {
            action: "set_power_plan".to_string(),
            params: params.clone(),
            op: crate::os_control::power::PowerProfileOp::SetProfile(profile),
        };
        let Some(desired) = request.desired_state() else {
            return ToolResult::err("set_power_plan produced no desired state");
        };
        let plan = gov::plan_for(resolved.provider_id, request.comparator(), None);
        gov::run_mutation(
            "set_power_plan",
            &resolved.runtime,
            provider,
            call,
            &request,
            &desired,
            &plan,
        )
        .await
    }
}

struct GetPowerPlan;

#[async_trait]
impl ToolHandler for GetPowerPlan {
    async fn execute(&self, _params: serde_json::Value) -> ToolResult {
        os_power_unavailable(None, "get_power_plan")
    }

    async fn execute_with_context(
        &self,
        params: serde_json::Value,
        ctx: ToolContext,
    ) -> ToolResult {
        let _input: EmptyInput = match parse_input(params.clone()) {
            Ok(input) => input,
            Err(error) => return error,
        };

        if !cfg!(target_os = "linux") {
            return ToolResult::ok(serde_json::json!({ "power_plan": "unsupported" }));
        }

        let resolved = match gov::resolve(&ctx, "get_power_plan") {
            Ok(resolved) => resolved,
            Err(result) => return result,
        };
        let provider = match resolved.runtime.power("get_power_plan") {
            Ok(provider) => provider,
            Err(error) => return gov::os_error(&error),
        };
        let call = match gov::read_call(&ctx, &resolved.runtime, "get_power_plan") {
            Ok(call) => call,
            Err(result) => return result,
        };
        let request = crate::os_control::power::PowerProfileRequest {
            action: "get_power_plan".to_string(),
            // The caller's ORIGINAL parameters: the grant's params digest was
            // taken from these, and rebuilding the object here would make the
            // binding check fail with grant_invalid. The normalized value
            // travels in the typed desired-state instead.
            params: params.clone(),
            op: crate::os_control::power::PowerProfileOp::GetProfile,
        };
        gov::run_read(provider, call, &request, |state| {
            serde_json::json!({ "profile": state.profile.as_str() })
        })
        .await
    }
}

struct SetEnvironmentVariable;

#[async_trait]
impl ToolHandler for SetEnvironmentVariable {
    async fn execute(&self, params: serde_json::Value) -> ToolResult {
        let input: SetEnvironmentVariableInput = match parse_input(params.clone()) {
            Ok(input) => input,
            Err(error) => return error,
        };

        let name = input.name.trim();
        if let Err(error) = validate_env_name(name) {
            return ToolResult::err(error);
        }

        if std::env::var(name).ok().as_deref() == Some(input.value.as_str()) {
            return ToolResult::ok(serde_json::json!({
                "name": name,
                "value": input.value,
                "set": true,
                "changed": false,
                "already_in_desired_state": true,
            }));
        }

        std::env::set_var(name, &input.value);
        ToolResult::ok(serde_json::json!({
            "name": name,
            "value": input.value,
            "set": true,
            "changed": true,
            "already_in_desired_state": false,
        }))
    }
}

struct GetEnvironmentVariable;

#[async_trait]
impl ToolHandler for GetEnvironmentVariable {
    async fn execute(&self, params: serde_json::Value) -> ToolResult {
        let input: EnvironmentVariableInput = match parse_input(params.clone()) {
            Ok(input) => input,
            Err(error) => return error,
        };

        let name = input.name.trim();
        if let Err(error) = validate_env_name(name) {
            return ToolResult::err(error);
        }

        let value = std::env::var(name).ok();
        ToolResult::ok(serde_json::json!({ "name": name, "value": value }))
    }
}

struct ListEnvironmentVariables;

#[async_trait]
impl ToolHandler for ListEnvironmentVariables {
    async fn execute(&self, params: serde_json::Value) -> ToolResult {
        let _input: EmptyInput = match parse_input(params.clone()) {
            Ok(input) => input,
            Err(error) => return error,
        };

        let vars: Vec<serde_json::Value> = std::env::vars()
            .filter(|(key, _)| {
                !key.contains("KEY")
                    && !key.contains("SECRET")
                    && !key.contains("TOKEN")
                    && !key.contains("PASSWORD")
            })
            .map(|(name, value)| serde_json::json!({ "name": name, "value": value }))
            .collect();

        ToolResult::ok(serde_json::json!({
            "variables": vars,
            "count": vars.len(),
        }))
    }
}

pub fn register(reg: &ToolRegistry) {
    let tools: Vec<(ToolDef, Arc<dyn ToolHandler>)> = vec![
        // GREEN
        (
            ToolDef {
                name: "get_power_plan".into(),
                description: "Get current power plan".into(),
                category: "system_config".into(),
                default_tier: RiskLevel::Green,
                min_tier: "lite",
                parameters: vec![],
            },
            Arc::new(GetPowerPlan),
        ),
        (
            ToolDef {
                name: "get_environment_variable".into(),
                description: "Get an environment variable value".into(),
                category: "system_config".into(),
                default_tier: RiskLevel::Green,
                min_tier: "lite",
                parameters: vec![param("name", "string", "Variable name", true)],
            },
            Arc::new(GetEnvironmentVariable),
        ),
        (
            ToolDef {
                name: "list_environment_variables".into(),
                description: "List all environment variables (secrets filtered)".into(),
                category: "system_config".into(),
                default_tier: RiskLevel::Green,
                min_tier: "lite",
                parameters: vec![],
            },
            Arc::new(ListEnvironmentVariables),
        ),
        (
            ToolDef {
                name: "get_wifi_networks".into(),
                description: "List available WiFi networks".into(),
                category: "system_config".into(),
                default_tier: RiskLevel::Green,
                min_tier: "standard",
                parameters: vec![],
            },
            Arc::new(GetWifiNetworks),
        ),
        (
            ToolDef {
                name: "get_network_state".into(),
                description: "Get Wi-Fi/Ethernet adapters, saved profiles, active profile, and connectivity status using stable typed device/profile identifiers.".into(),
                category: "system_config".into(),
                default_tier: RiskLevel::Red,
                min_tier: "standard",
                parameters: vec![],
            },
            Arc::new(GetNetworkState),
        ),
        (
            ToolDef {
                name: "get_audio_state".into(),
                description: "Get the default audio output volume and mute state".into(),
                category: "system_config".into(),
                default_tier: RiskLevel::Green,
                min_tier: "lite",
                parameters: vec![],
            },
            Arc::new(GetAudioState),
        ),
        (
            ToolDef {
                name: "get_display_state".into(),
                description: "Get the current display brightness and backend".into(),
                category: "system_config".into(),
                default_tier: RiskLevel::Green,
                min_tier: "lite",
                parameters: vec![],
            },
            Arc::new(GetDisplayState),
        ),
        // YELLOW
        (
            ToolDef {
                name: "set_volume".into(),
                description: "Set system audio volume (0-100)".into(),
                category: "system_config".into(),
                default_tier: RiskLevel::Yellow,
                min_tier: "lite",
                parameters: vec![param("level", "integer", "Volume 0-100", true)],
            },
            Arc::new(SetVolume),
        ),
        (
            ToolDef {
                name: "set_audio_mute".into(),
                description: "Mute or unmute the default audio output".into(),
                category: "system_config".into(),
                default_tier: RiskLevel::Yellow,
                min_tier: "lite",
                parameters: vec![param("muted", "boolean", "true=mute, false=unmute", true)],
            },
            Arc::new(SetAudioMute),
        ),
        (
            ToolDef {
                name: "set_brightness".into(),
                description: "Set screen brightness (0-100)".into(),
                category: "system_config".into(),
                default_tier: RiskLevel::Yellow,
                min_tier: "lite",
                parameters: vec![param("level", "integer", "Brightness 0-100", true)],
            },
            Arc::new(SetBrightness),
        ),
        (
            ToolDef {
                name: "toggle_wifi".into(),
                description: "Enable or disable WiFi".into(),
                category: "system_config".into(),
                default_tier: RiskLevel::Yellow,
                min_tier: "lite",
                parameters: vec![param("enable", "boolean", "true=on, false=off", true)],
            },
            Arc::new(ToggleWifi),
        ),
        (
            ToolDef {
                name: "set_power_plan".into(),
                description: "Set power plan (balanced/performance/power-saver)".into(),
                category: "system_config".into(),
                default_tier: RiskLevel::Yellow,
                min_tier: "lite",
                parameters: vec![param("plan", "string", "Power plan name", true)],
            },
            Arc::new(SetPowerPlan),
        ),
        (
            ToolDef {
                name: "connect_wifi".into(),
                description: "Connect to a WiFi network".into(),
                category: "system_config".into(),
                default_tier: RiskLevel::Yellow,
                min_tier: "standard",
                parameters: vec![
                    param("ssid", "string", "Network name", true),
                    param("password", "string", "Network password", false),
                    param("credential", "string", "Opaque Secret_Reference to a stored Wi-Fi credential (mutually exclusive with password)", false),
                ],
            },
            Arc::new(ConnectWifi),
        ),
        (
            ToolDef {
                name: "disconnect_wifi".into(),
                description: "Disconnect a network device by its typed device identifier (from get_network_state).".into(),
                category: "system_config".into(),
                default_tier: RiskLevel::Yellow,
                min_tier: "standard",
                parameters: vec![param("device", "string", "Typed network device identifier (from get_network_state)", true)],
            },
            Arc::new(DisconnectWifi),
        ),
        (
            ToolDef {
                name: "activate_network_profile".into(),
                description: "Activate an existing saved Wi-Fi or Ethernet profile by its typed profile identifier (from get_network_state). Ethernet has no separate connect tool — this is it.".into(),
                category: "system_config".into(),
                default_tier: RiskLevel::Red,
                min_tier: "standard",
                parameters: vec![
                    param("profile", "string", "Typed saved network profile identifier (from get_network_state)", true),
                    param("device", "string", "Typed network device identifier to activate on, if the profile does not already bind one", false),
                ],
            },
            Arc::new(ActivateNetworkProfile),
        ),
        // RED
        (
            ToolDef {
                name: "forget_wifi".into(),
                description: "Permanently forget (delete) a saved Wi-Fi profile by its typed profile identifier (from get_network_state). Irreversible — never claims rollback.".into(),
                category: "system_config".into(),
                default_tier: RiskLevel::Red,
                min_tier: "standard",
                parameters: vec![param("profile", "string", "Typed saved network profile identifier (from get_network_state)", true)],
            },
            Arc::new(ForgetWifi),
        ),
        (
            ToolDef {
                name: "set_environment_variable".into(),
                description: "Set an environment variable".into(),
                category: "system_config".into(),
                default_tier: RiskLevel::Red,
                min_tier: "lite",
                parameters: vec![
                    param("name", "string", "Variable name", true),
                    param("value", "string", "Variable value", true),
                ],
            },
            Arc::new(SetEnvironmentVariable),
        ),
    ];

    for (def, handler) in tools {
        reg.register(def, handler);
    }
}
