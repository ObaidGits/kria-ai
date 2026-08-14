//! Power tool handlers — battery health, logout, and cancelling a pending
//! shutdown.
//!
//! linux-os-control-production task **3.8** (OSC-020).
//!
//! Every handler routes through [`crate::tools::os_governed`].
//!
//! # Why the display tools are not here
//!
//! Task 5.1's `set_display_configuration`, `confirm_display_configuration` and
//! `set_night_light` have **no port operation**: `DisplayOp` carries only
//! `GetState` and `SetBrightness`. A display configuration change also needs an
//! apply-then-confirm lifecycle with automatic revert — a bad mode can leave the
//! user with a black screen and nothing to click — which is domain work, not
//! something a handler can improvise. They stay unimplemented.
//!
//! # What makes these three delicate
//!
//! * `logout_session` destroys unsaved work in **every** open application.
//! * `cancel_scheduled_shutdown` must never report success against an unknown
//!   schedule; "nothing is pending" and "the schedule could not be read" are
//!   different answers.
//! * `get_battery_health` must report "no battery" as a fact, never 0% — a
//!   fabricated zero looks exactly like a dying battery.

use std::sync::Arc;

use async_trait::async_trait;

use crate::infra::ToolResult;
use crate::os_control::power::session::{PowerSessionOp, PowerSessionRequest};
use crate::os_control::power::BatteryHealth;
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

/// Drive one governed power-session mutation.
async fn run_session(
    ctx: &ToolContext,
    tool: &str,
    params: serde_json::Value,
    op: PowerSessionOp,
) -> ToolResult {
    let resolved = match gov::resolve(ctx, tool) {
        Ok(resolved) => resolved,
        Err(result) => return result,
    };
    let provider = match resolved.runtime.power_session(tool) {
        Ok(provider) => provider,
        Err(error) => return gov::os_error(&error),
    };
    let call = match gov::mutation_call(ctx, &resolved.runtime, tool) {
        Ok(call) => call,
        Err(result) => return result,
    };
    let request = PowerSessionRequest {
        action: tool.to_string(),
        params,
        op,
    };
    let desired = request.desired_state();
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

struct GetBatteryHealth;

#[async_trait]
impl ToolHandler for GetBatteryHealth {
    async fn execute(&self, _params: serde_json::Value) -> ToolResult {
        gov::os_unavailable(None, "get_battery_health")
    }

    async fn execute_with_context(
        &self,
        _params: serde_json::Value,
        ctx: ToolContext,
    ) -> ToolResult {
        let tool = "get_battery_health";
        let resolved = match gov::resolve(&ctx, tool) {
            Ok(resolved) => resolved,
            Err(result) => return result,
        };
        let provider = match resolved.runtime.power(tool) {
            Ok(provider) => provider,
            Err(error) => return gov::os_error(&error),
        };
        let call = match gov::read_call(&ctx, &resolved.runtime, tool) {
            Ok(call) => call,
            Err(result) => return result,
        };
        match provider.read_battery_health(call.observation()).await {
            // `Absent` is a positive fact from the power service's own device
            // inventory — reporting 0% here would look like a dying battery on a
            // desktop that simply has none.
            Ok(BatteryHealth::Absent) => ToolResult::ok(serde_json::json!({
                "battery_present": false,
            })),
            Ok(BatteryHealth::Present {
                capacity_percent,
                cycle_count,
                health_state,
            }) => ToolResult::ok(serde_json::json!({
                "battery_present": true,
                "capacity_percent": capacity_percent,
                // `None` means the driver does not report cycles; never 0.
                "cycle_count": cycle_count,
                "health_state": health_state,
            })),
            Err(error) => gov::os_error(&error),
        }
    }
}

struct LogoutSession;

#[async_trait]
impl ToolHandler for LogoutSession {
    async fn execute(&self, _params: serde_json::Value) -> ToolResult {
        gov::os_unavailable(None, "logout_session")
    }

    async fn execute_with_context(
        &self,
        params: serde_json::Value,
        ctx: ToolContext,
    ) -> ToolResult {
        let tool = "logout_session";
        // A supplied session id is cross-checked by the provider against the live
        // session manager rather than trusted, so this can never terminate
        // somebody else's session. Absent means "resolve my own".
        let session = params["session"]
            .as_str()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string);
        run_session(&ctx, tool, params, PowerSessionOp::Logout { session }).await
    }
}

struct CancelScheduledShutdown;

#[async_trait]
impl ToolHandler for CancelScheduledShutdown {
    async fn execute(&self, _params: serde_json::Value) -> ToolResult {
        gov::os_unavailable(None, "cancel_scheduled_shutdown")
    }

    async fn execute_with_context(
        &self,
        params: serde_json::Value,
        ctx: ToolContext,
    ) -> ToolResult {
        let tool = "cancel_scheduled_shutdown";
        // The schedule id is DERIVED from authoritative state, never invented by
        // the caller: cancelling by a guessed id could report success while a
        // different shutdown stays pending. An absent id means "cancel whatever
        // the live schedule turns out to be", which the provider resolves.
        let schedule_id = params["schedule_id"]
            .as_str()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .unwrap_or_default()
            .to_string();
        run_session(
            &ctx,
            tool,
            params,
            PowerSessionOp::CancelScheduledShutdown { schedule_id },
        )
        .await
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Display configuration and night light (Task 5.1)
// ─────────────────────────────────────────────────────────────────────────────

/// Drive one governed display-configuration mutation.
async fn run_display_config(
    ctx: &ToolContext,
    tool: &str,
    params: serde_json::Value,
    op: crate::os_control::display::configuration::DisplayConfigOp,
) -> ToolResult {
    use crate::os_control::display::configuration::{DisplayConfigRequest, DisplayConfigState};

    let resolved = match gov::resolve(ctx, tool) {
        Ok(resolved) => resolved,
        Err(result) => return result,
    };
    let provider = match resolved.runtime.display_configuration(tool) {
        Ok(provider) => provider,
        Err(error) => return gov::os_error(&error),
    };
    let call = match gov::mutation_call(ctx, &resolved.runtime, tool) {
        Ok(call) => call,
        Err(result) => return result,
    };

    // The postcondition is derived from observed facts, so an unrelated setting is
    // carried through unchanged rather than being asserted at a guessed value.
    let facts = match provider.facts(call.observation()).await {
        Ok(facts) => facts,
        Err(error) => return gov::os_error(&error),
    };
    let observed = DisplayConfigState {
        focus: op.focus(),
        night_light: facts.night_light,
        config_serial: facts.config_serial,
        awaiting_confirmation: facts.awaiting_confirmation,
    };

    let request = DisplayConfigRequest {
        action: tool.to_string(),
        params,
        op,
    };
    let desired = request.desired_state(&observed);
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

struct SetNightLight;

#[async_trait]
impl ToolHandler for SetNightLight {
    async fn execute(&self, _params: serde_json::Value) -> ToolResult {
        gov::os_unavailable(None, "set_night_light")
    }

    async fn execute_with_context(
        &self,
        params: serde_json::Value,
        ctx: ToolContext,
    ) -> ToolResult {
        use crate::os_control::display::configuration::DisplayConfigOp;

        let tool = "set_night_light";
        let Some(enabled) = params["enabled"].as_bool() else {
            return ToolResult::err("`enabled` must be a boolean");
        };
        run_display_config(&ctx, tool, params, DisplayConfigOp::SetNightLight(enabled)).await
    }
}

struct SetDisplayConfiguration;

#[async_trait]
impl ToolHandler for SetDisplayConfiguration {
    async fn execute(&self, _params: serde_json::Value) -> ToolResult {
        gov::os_unavailable(None, "set_display_configuration")
    }

    async fn execute_with_context(
        &self,
        params: serde_json::Value,
        ctx: ToolContext,
    ) -> ToolResult {
        use crate::os_control::display::configuration::{DisplayConfigOp, MonitorLayoutSelection};

        let tool = "set_display_configuration";
        let Some(layout_id) = params["layout"].as_str().map(|s| s.trim().to_string()) else {
            return ToolResult::err(
                "`layout` is required: name a layout the compositor already reports as available. \
                 An arbitrary mode/position set is not accepted, because an invalid one leaves the \
                 screen unusable.",
            );
        };
        if layout_id.is_empty() || layout_id.chars().any(char::is_control) {
            return ToolResult::err("`layout` is empty or contains control characters");
        }
        let Some(serial) = params["serial"].as_u64().and_then(|v| u32::try_from(v).ok()) else {
            return ToolResult::err(
                "`serial` is required: it is the configuration serial you read, and it stops a \
                 layout being applied against a monitor set that has since changed",
            );
        };

        // Applied TEMPORARILY, always. The compositor reverts it on its own unless
        // confirm_display_configuration follows — which is why a bad layout can
        // never permanently lock the user out of their screen.
        run_display_config(
            &ctx,
            tool,
            params,
            DisplayConfigOp::ApplyConfiguration(MonitorLayoutSelection { serial, layout_id }),
        )
        .await
    }
}

struct ConfirmDisplayConfiguration;

#[async_trait]
impl ToolHandler for ConfirmDisplayConfiguration {
    async fn execute(&self, _params: serde_json::Value) -> ToolResult {
        gov::os_unavailable(None, "confirm_display_configuration")
    }

    async fn execute_with_context(
        &self,
        params: serde_json::Value,
        ctx: ToolContext,
    ) -> ToolResult {
        use crate::os_control::display::configuration::DisplayConfigOp;

        let tool = "confirm_display_configuration";
        // Only meaningful while a temporary configuration is pending; the domain
        // refuses otherwise rather than reporting a layout permanent when the
        // compositor is about to revert it.
        run_display_config(&ctx, tool, params, DisplayConfigOp::ConfirmConfiguration).await
    }
}

/// Register the power tool surface.
pub fn register(registry: &ToolRegistry) {
    let tools: Vec<(ToolDef, Arc<dyn ToolHandler>)> = vec![
        (
            ToolDef {
                name: "get_battery_health".into(),
                description: "Read battery capacity, cycle count and health band".into(),
                category: "power".into(),
                default_tier: RiskLevel::Green,
                min_tier: "lite",
                parameters: vec![],
            },
            Arc::new(GetBatteryHealth),
        ),
        (
            ToolDef {
                name: "logout_session".into(),
                description: "Log out of the current desktop session".into(),
                category: "power".into(),
                // Destroys unsaved work in every open application.
                default_tier: RiskLevel::Red,
                min_tier: "lite",
                parameters: vec![param(
                    "session",
                    "string",
                    "Session id; must name your own current session (verified against the live session manager). Omit to resolve it automatically.",
                    false,
                )],
            },
            Arc::new(LogoutSession),
        ),
        (
            ToolDef {
                name: "cancel_scheduled_shutdown".into(),
                description: "Cancel a pending scheduled shutdown".into(),
                category: "power".into(),
                default_tier: RiskLevel::Yellow,
                min_tier: "lite",
                parameters: vec![param(
                    "schedule_id",
                    "string",
                    "Schedule id derived from live state. Omit to cancel whatever is currently pending.",
                    false,
                )],
            },
            Arc::new(CancelScheduledShutdown),
        ),
        (
            ToolDef {
                name: "set_night_light".into(),
                description: "Turn the night-light colour shift on or off".into(),
                category: "display".into(),
                default_tier: RiskLevel::Green,
                min_tier: "lite",
                parameters: vec![param("enabled", "boolean", "Desired night-light state", true)],
            },
            Arc::new(SetNightLight),
        ),
        (
            ToolDef {
                name: "set_display_configuration".into(),
                description:
                    "Apply a monitor layout temporarily; it reverts by itself unless confirmed"
                        .into(),
                category: "display".into(),
                // A wrong layout can leave the screen unusable until the revert.
                default_tier: RiskLevel::Red,
                min_tier: "lite",
                parameters: vec![
                    param(
                        "layout",
                        "string",
                        "A layout the compositor reports as available (not an arbitrary mode set)",
                        true,
                    ),
                    param(
                        "serial",
                        "integer",
                        "The configuration serial you read; stops applying against a changed monitor set",
                        true,
                    ),
                ],
            },
            Arc::new(SetDisplayConfiguration),
        ),
        (
            ToolDef {
                name: "confirm_display_configuration".into(),
                description: "Confirm the pending monitor layout, making it permanent".into(),
                category: "display".into(),
                default_tier: RiskLevel::Yellow,
                min_tier: "lite",
                parameters: vec![],
            },
            Arc::new(ConfirmDisplayConfiguration),
        ),
    ];

    for (def, handler) in tools {
        registry.register(def, handler);
    }
}
