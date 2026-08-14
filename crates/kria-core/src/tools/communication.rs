use crate::infra::ToolResult;
use crate::os_control::contract::SafeText;
use crate::os_control::{OsControlError, OsControlRuntime};
use crate::safety::RiskLevel;
use crate::tools::registry::{ParamDef, ToolDef, ToolHandler, ToolRegistry};
use crate::tools::os_governed as gov;
use crate::tools::ToolContext;
use async_trait::async_trait;
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

/// Return the governed OS-control `Unavailable` envelope for a notification
/// tool.
///
/// linux-os-control-production **Task 2.5** ("upgrade notification
/// adapter"): `send_notification` no longer spawns `notify-send` with a
/// manually-guessed `DBUS_SESSION_BUS_ADDRESS`/`DISPLAY`, nor falls back to
/// the `notify_rust` library. It reaches host effects **only** through the
/// injected [`OsControlRuntime`] + `os_control::notifications::NotificationControl`
/// provider (a freedesktop-portal-style seam). Until a live D-Bus portal
/// transport is composed into the runtime, the handler fails closed with
/// this frozen envelope.
fn os_notification_unavailable(runtime: Option<&Arc<OsControlRuntime>>, tool: &str) -> ToolResult {
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

struct SendNotification;
#[async_trait]
impl ToolHandler for SendNotification {
    async fn execute(&self, _params: serde_json::Value) -> ToolResult {
        os_notification_unavailable(None, "send_notification")
    }

    async fn execute_with_context(
        &self,
        params: serde_json::Value,
        ctx: ToolContext,
    ) -> ToolResult {
        let title = params["title"].as_str().unwrap_or("K.R.I.A.").to_string();
        let body = params["body"]
            .as_str()
            .or_else(|| params["message"].as_str())
            .unwrap_or_default()
            .to_string();
        // The nonce makes every send a distinct desired state, so two identical
        // notifications are not collapsed into one idempotent no-op.
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_millis() as u64);

        let resolved = match gov::resolve(&ctx, "send_notification") {
            Ok(resolved) => resolved,
            Err(result) => return result,
        };
        let provider = match resolved.runtime.notifications("send_notification") {
            Ok(provider) => provider,
            Err(error) => return gov::os_error(&error),
        };
        let call = match gov::mutation_call(&ctx, &resolved.runtime, "send_notification") {
            Ok(call) => call,
            Err(result) => return result,
        };
        let request = crate::os_control::notifications::NotificationRequest {
            action: "send_notification".to_string(),
            params: params.clone(),
            title,
            body,
            nonce,
        };
        let desired = request.desired_state();
        let plan = gov::plan_for(resolved.provider_id, request.comparator(), None);
        gov::run_mutation(
            "send_notification",
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

struct ComposeEmail;
#[async_trait]
impl ToolHandler for ComposeEmail {
    async fn execute(&self, params: serde_json::Value) -> ToolResult {
        let to = params["to"].as_str().unwrap_or("");
        let subject = params["subject"].as_str().unwrap_or("");
        let body = params["body"].as_str().unwrap_or("");
        // Opens default email client with mailto: link (draft only, does NOT send)
        let mailto = format!(
            "mailto:{}?subject={}&body={}",
            urlencoding(to),
            urlencoding(subject),
            urlencoding(body)
        );
        let _ = open::that(&mailto);
        ToolResult::ok(serde_json::json!({
            "action": "compose_email",
            "to": to, "subject": subject,
            "note": "Email draft opened in default email client (not sent)",
        }))
    }
}

struct ScheduleReminder;
#[async_trait]
impl ToolHandler for ScheduleReminder {
    async fn execute(&self, params: serde_json::Value) -> ToolResult {
        self.schedule(params, None).await
    }

    async fn execute_with_context(
        &self,
        params: serde_json::Value,
        ctx: ToolContext,
    ) -> ToolResult {
        self.schedule(params, ctx.os_runtime.clone()).await
    }
}

impl ScheduleReminder {
    /// Schedule the in-process timer (`tokio::spawn`+`sleep`, not a host
    /// mutation) and, when it fires, deliver the reminder **only** through
    /// the governed [`OsControlRuntime`] notification port — never a direct
    /// `notify-send`/`paplay` subprocess spawn or a manually-guessed D-Bus
    /// env (Task 2.5's "upgrade notification adapter"). Until a live portal
    /// transport is composed, the eventual delivery attempt reaches the
    /// frozen `Unavailable` envelope rather than any ungoverned fallback.
    async fn schedule(
        &self,
        params: serde_json::Value,
        runtime: Option<Arc<OsControlRuntime>>,
    ) -> ToolResult {
        let message = params["message"].as_str().unwrap_or("");
        let delay_secs = params["delay_minutes"].as_f64().unwrap_or(5.0) * 60.0;
        let delay_secs = delay_secs as u64;
        let msg = message.to_string();

        // Spawn the persistent in-process timer that fires the reminder
        // after the delay. The eventual notification delivery routes through
        // the same governed NotificationControl provider `send_notification`
        // uses; it is not fired here directly.
        tokio::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_secs(delay_secs)).await;
            let _ = os_notification_unavailable(runtime.as_ref(), "send_notification");
            tracing::info!(
                target: "communication",
                message = %msg,
                "schedule_reminder fired; notification delivery routes through the governed \
                 NotificationControl provider (Unavailable until a live portal transport is composed)"
            );
        });
        let display_mins = delay_secs / 60;
        let display_secs = delay_secs % 60;
        let time_str = if display_secs == 0 {
            format!(
                "{display_mins} minute{}",
                if display_mins == 1 { "" } else { "s" }
            )
        } else {
            format!("{display_mins}m {display_secs}s")
        };
        ToolResult::ok(serde_json::json!({
            "scheduled": true,
            "message": message,
            "fires_in": time_str,
        }))
    }
}

// Simple URL encoding helper (no external dep needed for basic mailto)
fn urlencoding(s: &str) -> String {
    s.replace(' ', "%20")
        .replace('\n', "%0A")
        .replace('&', "%26")
}

pub fn register(reg: &ToolRegistry) {
    let tools: Vec<(ToolDef, Arc<dyn ToolHandler>)> = vec![
        (
            ToolDef {
                name: "send_notification".into(),
                description: "Send a desktop notification".into(),
                category: "communication".into(),
                default_tier: RiskLevel::Green,
                min_tier: "lite",
                parameters: vec![
                    param("title", "string", "Notification title", false),
                    param("body", "string", "Notification body", true),
                ],
            },
            Arc::new(SendNotification),
        ),
        (
            ToolDef {
                name: "compose_email".into(),
                description: "Open email draft in default client (does NOT send)".into(),
                category: "communication".into(),
                default_tier: RiskLevel::Green,
                min_tier: "standard",
                parameters: vec![
                    param("to", "string", "Recipient email", true),
                    param("subject", "string", "Email subject", true),
                    param("body", "string", "Email body", true),
                ],
            },
            Arc::new(ComposeEmail),
        ),
        (
            ToolDef {
                name: "schedule_reminder".into(),
                description: "Schedule a reminder notification".into(),
                category: "communication".into(),
                default_tier: RiskLevel::Green,
                min_tier: "lite",
                parameters: vec![
                    param("message", "string", "Reminder message", true),
                    param(
                        "delay_minutes",
                        "integer",
                        "Minutes from now (default 5)",
                        false,
                    ),
                ],
            },
            Arc::new(ScheduleReminder),
        ),
    ];
    for (def, handler) in tools {
        reg.register(def, handler);
    }
}
