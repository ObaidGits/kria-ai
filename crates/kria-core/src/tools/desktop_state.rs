//! Desktop-state tool handlers — clipboard history, notification state,
//! capability reporting and graceful application close.
//!
//! linux-os-control-production **Task 4.9** (`get_clipboard_history`,
//! `clear_clipboard_history`, `configure_clipboard_history`,
//! `get_notification_state`, `set_do_not_disturb`), **Task 1.3**
//! (`get_os_capabilities`) and **Task 2.5** (`graceful_close_application`).
//!
//! Every handler routes through the governed runtime: reads take an admitted
//! observation context, mutations take a sealed mutation permit. Nothing here
//! spawns a process, opens a bus, or reads a device.
//!
//! # The clipboard history is the most privacy-sensitive read in KRIA
//!
//! The retained history is a rolling log of everything the user copied — which
//! routinely includes passwords they pasted into a login form seconds ago. So
//! `get_clipboard_history` reports entry **metadata** only (identity, type, size,
//! capture time, payload digest). The frozen `ClipboardHistoryPage` entry makes
//! `payload` optional and classes it as `Content`; this handler never populates
//! it. No history value reaches a log line, an error message, a digest input a
//! caller could grind, or a test fixture.
//!
//! # Risk shape (from the frozen manifest)
//!
//! * `get_clipboard_history` — **RED read**. Fixed RED because the payload class
//!   is `Content`: reading the copy log is as sensitive as reading a password
//!   store.
//! * `clear_clipboard_history`, `configure_clipboard_history` — fixed **RED**
//!   mutations. Clearing is **irreversible** (the payloads are destroyed) and
//!   narrowing retention drops entries, so neither advertises a rollback.
//! * `set_do_not_disturb` — **YELLOW**. Suppressing alerts can hide something the
//!   user is relying on, so the switch is only ever set from a state that was
//!   actually read, and the postcondition is a real re-read.
//! * `get_notification_state`, `get_os_capabilities` — **GREEN reads**.
//! * `graceful_close_application` — **YELLOW**. SIGTERM only; escalation to
//!   SIGKILL is the separate `kill_process` operation.
//!
//! # Absent is never confused with unknown
//!
//! An empty clipboard history, a session with no history store, and a history
//! that could not be read are three different facts. Only the first is ever
//! reported as "no entries"; the other two surface as the frozen `Unavailable` /
//! `Unsupported` envelope. The same rule governs do-not-disturb: an unreadable
//! switch is an error, never "alerts will be delivered".

use std::sync::Arc;

use async_trait::async_trait;

use crate::infra::ToolResult;
use crate::os_control::clipboard::{
    AllowedMime, ClipboardHistoryClearRequest, ClipboardHistoryConfigRequest,
    ClipboardHistoryConfigState, ClipboardHistoryPage, DEFAULT_CLIPBOARD_HISTORY_PAGE,
    MAX_CLIPBOARD_HISTORY_PAGE,
};
use crate::os_control::notifications::{DoNotDisturbRequest, NotificationSessionState};
use crate::safety::RiskLevel;
use crate::tools::os_governed as gov;
use crate::tools::registry::{ParamDef, ToolDef, ToolHandler, ToolRegistry};
use crate::tools::ToolContext;

// ─────────────────────────────────────────────────────────────────────────────
// Shared input validation (reject, never escape)
// ─────────────────────────────────────────────────────────────────────────────

/// Validate a caller-supplied identifier that will be matched against host
/// state.
///
/// Rejects rather than sanitizes: a control character or a leading `-` (which a
/// tool could read as an option) means the caller sent something this operation
/// cannot honour, and silently rewriting it would act on a target the caller did
/// not name.
fn validated_identifier(raw: &str, field: &str, max_chars: usize) -> Result<String, ToolResult> {
    let value = raw.trim();
    if value.is_empty() {
        return Err(ToolResult::err(format!("`{field}` must not be empty")));
    }
    if value.chars().count() > max_chars {
        return Err(ToolResult::err(format!(
            "`{field}` must be at most {max_chars} characters"
        )));
    }
    if value.chars().any(char::is_control) {
        return Err(ToolResult::err(format!(
            "`{field}` must not contain control characters"
        )));
    }
    if value.starts_with('-') {
        return Err(ToolResult::err(format!(
            "`{field}` must not start with `-`: it would be read as a command option"
        )));
    }
    Ok(value.to_string())
}

/// Read a bounded integer parameter, or the frozen validation failure.
fn optional_bounded_u64(
    params: &serde_json::Value,
    field: &str,
    min: u64,
    max: u64,
) -> Result<Option<u64>, ToolResult> {
    match params.get(field) {
        None | Some(serde_json::Value::Null) => Ok(None),
        Some(value) => {
            let Some(number) = value.as_u64() else {
                return Err(ToolResult::err(format!(
                    "`{field}` must be an integer between {min} and {max}"
                )));
            };
            if number < min || number > max {
                return Err(ToolResult::err(format!(
                    "`{field}` must be between {min} and {max}"
                )));
            }
            Ok(Some(number))
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// get_clipboard_history (RED read) — metadata only, never a payload
// ─────────────────────────────────────────────────────────────────────────────

struct GetClipboardHistory;

/// Project a history page into the frozen `ClipboardHistoryPage` shape.
///
/// `payload` is deliberately absent: the frozen entry marks it optional and
/// classes it as `Content`, and this surface has no reason to hand a caller the
/// text of something the user copied. `captured_at_ms` is emitted only when the
/// store actually recorded one — never substituted with the read time.
fn project_history_page(page: &ClipboardHistoryPage) -> serde_json::Value {
    let items: Vec<serde_json::Value> = page
        .entries
        .iter()
        .map(|entry| {
            let mut item = serde_json::json!({
                "item_id": entry.item_id.as_str(),
                "mime": entry.mime.as_str(),
                "byte_count": entry.byte_count,
                "payload_digest": entry.payload_digest.to_string(),
            });
            if let Some(captured_at_ms) = entry.captured_at_ms {
                item["captured_at_ms"] = serde_json::json!(captured_at_ms);
            }
            item
        })
        .collect();

    let mut out = serde_json::json!({
        "items": items,
        "truncated": page.truncated,
    });
    if let Some(cursor) = page.next_cursor.as_deref() {
        out["next_cursor"] = serde_json::json!(cursor);
    }
    out
}

#[async_trait]
impl ToolHandler for GetClipboardHistory {
    async fn execute(&self, _params: serde_json::Value) -> ToolResult {
        gov::os_unavailable(None, "get_clipboard_history")
    }

    async fn execute_with_context(
        &self,
        params: serde_json::Value,
        ctx: ToolContext,
    ) -> ToolResult {
        let tool = "get_clipboard_history";
        let limit = match optional_bounded_u64(
            &params,
            "limit",
            1,
            u64::from(MAX_CLIPBOARD_HISTORY_PAGE),
        ) {
            Ok(limit) => limit.unwrap_or(u64::from(DEFAULT_CLIPBOARD_HISTORY_PAGE)),
            Err(result) => return result,
        };
        // The cursor is opaque to this handler: it is validated for shape and
        // passed straight through, never interpreted.
        let cursor = match params.get("cursor") {
            None | Some(serde_json::Value::Null) => None,
            Some(value) => {
                let Some(raw) = value.as_str() else {
                    return ToolResult::err("`cursor` must be a string");
                };
                match validated_identifier(raw, "cursor", 512) {
                    Ok(cursor) => Some(cursor),
                    Err(result) => return result,
                }
            }
        };

        let resolved = match gov::resolve(&ctx, tool) {
            Ok(resolved) => resolved,
            Err(result) => return result,
        };
        let provider = match resolved.runtime.clipboard(tool) {
            Ok(provider) => provider,
            Err(error) => return gov::os_error(&error),
        };
        let call = match gov::read_call(&ctx, &resolved.runtime, tool) {
            Ok(call) => call,
            Err(result) => return result,
        };

        // `limit` is already clamped to the frozen maximum, so a caller cannot
        // request an unbounded dump of the copy log.
        #[allow(clippy::cast_possible_truncation)]
        let limit = limit as u32;
        match provider
            .history(call.observation(), limit, cursor.as_deref())
            .await
        {
            Ok(page) => ToolResult::ok(project_history_page(&page)),
            // The error envelope names the store and what it could not supply —
            // never an entry, a preview, or a payload.
            Err(error) => gov::os_error(&error),
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// clear_clipboard_history (RED mutation) — irreversible
// ─────────────────────────────────────────────────────────────────────────────

struct ClearClipboardHistory;

#[async_trait]
impl ToolHandler for ClearClipboardHistory {
    async fn execute(&self, _params: serde_json::Value) -> ToolResult {
        gov::os_unavailable(None, "clear_clipboard_history")
    }

    async fn execute_with_context(
        &self,
        params: serde_json::Value,
        ctx: ToolContext,
    ) -> ToolResult {
        let tool = "clear_clipboard_history";
        let resolved = match gov::resolve(&ctx, tool) {
            Ok(resolved) => resolved,
            Err(result) => return result,
        };
        let provider = match resolved.runtime.clipboard(tool) {
            Ok(provider) => provider,
            Err(error) => return gov::os_error(&error),
        };
        let call = match gov::mutation_call(&ctx, &resolved.runtime, tool) {
            Ok(call) => call,
            Err(result) => return result,
        };

        let request = ClipboardHistoryClearRequest {
            action: tool.to_string(),
            // The operation takes no parameters, and there is nothing about the
            // retained content to bind into the grant.
            // The caller's ORIGINAL parameters: the grant's params digest was
            // taken from these, and rebuilding the object here would make the
            // binding check fail with grant_invalid. The normalized value
            // travels in the typed desired-state instead.
            params: params.clone(),
        };
        let desired = request.desired_state();
        // `plan_for` leaves the rollback plan `Unavailable`, which is the truth
        // here: the destroyed payloads cannot be restored, and the frozen
        // contract declares `rollbackClaim: None`.
        let plan = gov::plan_for(resolved.provider_id, request.comparator(), None);

        gov::run_mutation(
            tool,
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

// ─────────────────────────────────────────────────────────────────────────────
// configure_clipboard_history (RED mutation)
// ─────────────────────────────────────────────────────────────────────────────

struct ConfigureClipboardHistory;

#[async_trait]
impl ToolHandler for ConfigureClipboardHistory {
    async fn execute(&self, _params: serde_json::Value) -> ToolResult {
        gov::os_unavailable(None, "configure_clipboard_history")
    }

    async fn execute_with_context(
        &self,
        params: serde_json::Value,
        ctx: ToolContext,
    ) -> ToolResult {
        let tool = "configure_clipboard_history";
        let Some(enabled) = params["enabled"].as_bool() else {
            return ToolResult::err("configure_clipboard_history requires a boolean `enabled`");
        };
        let ttl_seconds = match optional_bounded_u64(&params, "ttl_seconds", 60, 31_536_000) {
            Ok(value) => value,
            Err(result) => return result,
        };
        let max_items = match optional_bounded_u64(&params, "max_items", 1, 1_000) {
            Ok(value) => value,
            Err(result) => return result,
        };
        // An unrecognized MIME token is rejected, never dropped: silently
        // widening or narrowing the retained set would apply a policy the caller
        // did not ask for.
        let allowed_mimes = match params.get("allowed_mimes") {
            None | Some(serde_json::Value::Null) => Vec::new(),
            Some(serde_json::Value::Array(raw)) => {
                if raw.is_empty() || raw.len() > 256 {
                    return ToolResult::err(
                        "`allowed_mimes` must contain between 1 and 256 entries",
                    );
                }
                let mut mimes = Vec::with_capacity(raw.len());
                for value in raw {
                    let Some(token) = value.as_str() else {
                        return ToolResult::err("`allowed_mimes` entries must be strings");
                    };
                    let Some(mime) = AllowedMime::parse(token) else {
                        return ToolResult::err(
                            "`allowed_mimes` accepts only text/plain, text/html, image/png or image/jpeg",
                        );
                    };
                    mimes.push(mime);
                }
                mimes.sort_unstable();
                mimes.dedup();
                mimes
            }
            Some(_) => return ToolResult::err("`allowed_mimes` must be an array of MIME strings"),
        };

        let resolved = match gov::resolve(&ctx, tool) {
            Ok(resolved) => resolved,
            Err(result) => return result,
        };
        let provider = match resolved.runtime.clipboard(tool) {
            Ok(provider) => provider,
            Err(error) => return gov::os_error(&error),
        };
        let call = match gov::mutation_call(&ctx, &resolved.runtime, tool) {
            Ok(call) => call,
            Err(result) => return result,
        };

        let request = ClipboardHistoryConfigRequest {
            action: tool.to_string(),
            params: params.clone(),
            config: ClipboardHistoryConfigState {
                enabled,
                ttl_seconds,
                max_items,
                allowed_mimes,
            },
        };
        let desired = request.desired_state();
        // Entries already dropped by a narrower retention policy cannot be
        // brought back: `rollbackClaim: None`.
        let plan = gov::plan_for(resolved.provider_id, request.comparator(), None);

        gov::run_mutation(
            tool,
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

// ─────────────────────────────────────────────────────────────────────────────
// get_notification_state (GREEN read)
// ─────────────────────────────────────────────────────────────────────────────

struct GetNotificationState;

/// Project the session notification state into the frozen `NotificationState`
/// envelope.
///
/// `pending_count` is absent: no notification server exposes a pending count, and
/// the frozen `fields` object requires nothing, so the field is omitted rather
/// than reported as zero.
fn project_notification_state(
    state: &NotificationSessionState,
    revision: Option<u64>,
) -> serde_json::Value {
    serde_json::json!({
        "identity": "notification-state/session",
        "revision": revision,
        "availability": "Available",
        "fields": {
            "do_not_disturb": state.do_not_disturb.as_str(),
            "server_available": state.server.as_str(),
        },
    })
}

#[async_trait]
impl ToolHandler for GetNotificationState {
    async fn execute(&self, _params: serde_json::Value) -> ToolResult {
        gov::os_unavailable(None, "get_notification_state")
    }

    async fn execute_with_context(
        &self,
        _params: serde_json::Value,
        ctx: ToolContext,
    ) -> ToolResult {
        let tool = "get_notification_state";
        let resolved = match gov::resolve(&ctx, tool) {
            Ok(resolved) => resolved,
            Err(result) => return result,
        };
        let provider = match resolved.runtime.notifications(tool) {
            Ok(provider) => provider,
            Err(error) => return gov::os_error(&error),
        };
        let call = match gov::read_call(&ctx, &resolved.runtime, tool) {
            Ok(call) => call,
            Err(result) => return result,
        };
        let revision = resolved
            .runtime
            .capability_snapshot()
            .map(|snapshot| snapshot.revision.0);

        match provider.session_state(call.observation()).await {
            Ok(state) => ToolResult::ok(project_notification_state(&state, revision)),
            // A switch that could not be read is an error, never "alerts are
            // delivered": the caller must not be told they will be notified when
            // they may be silenced.
            Err(error) => gov::os_error(&error),
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// set_do_not_disturb (YELLOW mutation)
// ─────────────────────────────────────────────────────────────────────────────

struct SetDoNotDisturb;

#[async_trait]
impl ToolHandler for SetDoNotDisturb {
    async fn execute(&self, _params: serde_json::Value) -> ToolResult {
        gov::os_unavailable(None, "set_do_not_disturb")
    }

    async fn execute_with_context(
        &self,
        params: serde_json::Value,
        ctx: ToolContext,
    ) -> ToolResult {
        let tool = "set_do_not_disturb";
        let Some(enabled) = params["enabled"].as_bool() else {
            return ToolResult::err("set_do_not_disturb requires a boolean `enabled`");
        };

        let resolved = match gov::resolve(&ctx, tool) {
            Ok(resolved) => resolved,
            Err(result) => return result,
        };
        let provider = match resolved.runtime.notifications(tool) {
            Ok(provider) => provider,
            Err(error) => return gov::os_error(&error),
        };
        let call = match gov::mutation_call(&ctx, &resolved.runtime, tool) {
            Ok(call) => call,
            Err(result) => return result,
        };

        let request = DoNotDisturbRequest {
            action: tool.to_string(),
            // The caller's ORIGINAL parameters: the grant's params digest was
            // taken from these, and rebuilding the object here would make the
            // binding check fail with grant_invalid. The normalized value
            // travels in the typed desired-state instead.
            params: params.clone(),
            enabled,
        };
        let desired = request.desired_state();
        // The rollback token carries no prior switch position and the provider
        // holds no state of its own, so no inverse is advertised. Reverting is a
        // fresh call with the other value.
        let plan = gov::plan_for(resolved.provider_id, request.comparator(), None);

        gov::run_mutation(
            tool,
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

// ─────────────────────────────────────────────────────────────────────────────
// get_os_capabilities (GREEN read)
// ─────────────────────────────────────────────────────────────────────────────

struct GetOsCapabilities;

/// The frozen `DomainId` set. A caller-supplied domain outside it is rejected
/// rather than silently matching nothing, which would look like "that domain has
/// no capabilities".
const DOMAIN_IDS: [&str; 24] = [
    "files",
    "applications",
    "processes",
    "packages",
    "storage",
    "connectivity",
    "firewall",
    "audio",
    "media",
    "display",
    "power",
    "bluetooth",
    "health",
    "clipboard",
    "notifications",
    "search",
    "secrets",
    "printing",
    "scanning",
    "backup",
    "privacy",
    "automation",
    "hardware",
    "firmware",
];

#[async_trait]
impl ToolHandler for GetOsCapabilities {
    async fn execute(&self, _params: serde_json::Value) -> ToolResult {
        gov::os_unavailable(None, "get_os_capabilities")
    }

    async fn execute_with_context(
        &self,
        params: serde_json::Value,
        ctx: ToolContext,
    ) -> ToolResult {
        let tool = "get_os_capabilities";
        let domain = match params.get("domain") {
            None | Some(serde_json::Value::Null) => None,
            Some(value) => {
                let Some(raw) = value.as_str() else {
                    return ToolResult::err("`domain` must be a string");
                };
                let raw = raw.trim().to_ascii_lowercase();
                if !DOMAIN_IDS.contains(&raw.as_str()) {
                    return ToolResult::err(
                        "`domain` must be one of the frozen capability domains",
                    );
                }
                Some(raw)
            }
        };
        let include_unavailable = match params.get("include_unavailable") {
            None | Some(serde_json::Value::Null) => false,
            Some(value) => {
                let Some(flag) = value.as_bool() else {
                    return ToolResult::err("`include_unavailable` must be a boolean");
                };
                flag
            }
        };

        let resolved = match gov::resolve(&ctx, tool) {
            Ok(resolved) => resolved,
            Err(result) => return result,
        };
        let call = match gov::read_call(&ctx, &resolved.runtime, tool) {
            Ok(call) => call,
            Err(result) => return result,
        };
        // Bind the answer to the admitted read's own observation context, so a
        // capability report cannot be produced outside an admitted call.
        if call.observation().cancellation.is_cancelled() {
            return gov::os_error(&crate::os_control::OsControlError::CancelledBeforeMutation);
        }

        // The snapshot is the probe result the composed aggregate was built
        // against. When the host was never probed there is no capability fact to
        // report: that is `Unavailable` with a reason, not an empty capability
        // list, which would read as "this machine can do nothing".
        let Some(snapshot) = resolved.runtime.capability_snapshot() else {
            return ToolResult::ok(serde_json::json!({
                "identity": "capability-snapshot/host",
                "revision": serde_json::Value::Null,
                "availability": "Unavailable",
                "fields": {},
                "unavailable_reason": {
                    "code": "capability_snapshot_absent",
                    "remediation": "the host was not probed in this composition, so no capability facts exist to report",
                },
            }));
        };

        // Only operations a provider is actually composed for are counted:
        // `AvailabilityStatus` comes from the prober, so a domain whose provider
        // is absent is never reported as available.
        let selected: Vec<&crate::os_control::capability::CapabilityAvailability> = snapshot
            .operations
            .as_slice()
            .iter()
            .filter(|operation| {
                domain
                    .as_deref()
                    .is_none_or(|wanted| operation.domain.as_str() == wanted)
            })
            .filter(|operation| {
                include_unavailable
                    || !matches!(
                        operation.status,
                        crate::os_control::AvailabilityStatus::Unavailable
                    )
            })
            .collect();

        let mut domains: Vec<&str> = selected
            .iter()
            .map(|operation| operation.domain.as_str())
            .collect();
        domains.sort_unstable();
        domains.dedup();

        // A digest over exactly the reported set, in canonical order, so two
        // reports of the same capabilities are byte-identical and a change is
        // detectable without surfacing every operation.
        let mut lines: Vec<String> = selected
            .iter()
            .map(|operation| {
                format!(
                    "{}:{}",
                    operation.capability.as_str(),
                    serde_json::to_string(&operation.status).unwrap_or_default()
                )
            })
            .collect();
        lines.sort_unstable();
        let capability_digest =
            crate::os_control::contract::Digest::of_str(&lines.join("\n")).to_string();

        // `generated_at_ms` is deliberately absent: the snapshot records a
        // monotonic revision, not a probe wall-clock time, and reporting "now"
        // would claim a freshness nobody observed.
        ToolResult::ok(serde_json::json!({
            "identity": "capability-snapshot/host",
            "revision": snapshot.revision.0,
            "availability": if snapshot.degraded { "Degraded" } else { "Available" },
            "fields": {
                "snapshot_revision": snapshot.revision.0,
                "session_type": serde_json::to_value(snapshot.display_server)
                    .unwrap_or(serde_json::Value::Null),
                "desktop_family": serde_json::to_value(snapshot.desktop_family)
                    .unwrap_or(serde_json::Value::Null),
                "domain_count": domains.len(),
                "capability_digest": capability_digest,
            },
        }))
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// graceful_close_application (YELLOW mutation)
// ─────────────────────────────────────────────────────────────────────────────

struct GracefulCloseApplication;

#[async_trait]
impl ToolHandler for GracefulCloseApplication {
    async fn execute(&self, _params: serde_json::Value) -> ToolResult {
        gov::os_unavailable(None, "graceful_close_application")
    }

    async fn execute_with_context(
        &self,
        params: serde_json::Value,
        ctx: ToolContext,
    ) -> ToolResult {
        let tool = "graceful_close_application";
        // `app_id` is the stable identity the frozen contract names. A window
        // title is never accepted: it is neither unique nor stable, so closing by
        // title can close something the caller did not name.
        let Some(raw_app_id) = params["app_id"].as_str() else {
            return ToolResult::err(
                "graceful_close_application requires `app_id`; a window title is not an identity because it is neither unique nor stable",
            );
        };
        let app_id = match validated_identifier(raw_app_id, "app_id", 128) {
            Ok(app_id) => app_id,
            Err(result) => return result,
        };
        // An `instance_id` narrows the target to one running instance. The
        // composed provider matches on the application identity only, so
        // accepting the parameter and ignoring it would close *every* instance
        // when the caller asked for one. Fail closed instead.
        match params.get("instance_id") {
            None | Some(serde_json::Value::Null) => {}
            Some(_) => {
                return gov::os_error(&crate::os_control::OsControlError::Unsupported {
                    capability: crate::os_control::contract::CapabilityId::new(
                        "graceful_close_application.instance_id",
                    ),
                    reason: crate::os_control::contract::SafeText::new(
                        "the composed application-close provider matches on the application identity only; an instance-scoped close would close every instance and is refused rather than approximated",
                    ),
                })
            }
        }

        let resolved = match gov::resolve(&ctx, tool) {
            Ok(resolved) => resolved,
            Err(result) => return result,
        };
        let provider = match resolved.runtime.application_close(tool) {
            Ok(provider) => provider,
            Err(error) => return gov::os_error(&error),
        };
        let call = match gov::mutation_call(&ctx, &resolved.runtime, tool) {
            Ok(call) => call,
            Err(result) => return result,
        };

        // The provider owns the SIGTERM-only signal loop and the liveness
        // postcondition; escalation to SIGKILL is the separate `kill_process`
        // operation and never happens here.
        let request = crate::os_control::applications::ApplicationCloseRequest {
            action: tool.to_string(),
            params: params.clone(),
            name: app_id,
        };
        let desired = request.desired_state();
        let plan = gov::plan_for(resolved.provider_id, request.comparator(), None);

        gov::run_mutation(
            tool,
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

// ─────────────────────────────────────────────────────────────────────────────
// Registration
// ─────────────────────────────────────────────────────────────────────────────

/// Register the desktop-state tool surface.
pub fn register(registry: &ToolRegistry) {
    let tools: Vec<(ToolDef, Arc<dyn ToolHandler>)> = vec![
        (
            ToolDef {
                name: "get_clipboard_history".into(),
                description: "Read retained clipboard history entry metadata (identity, type, size, digest) — never the copied content".into(),
                category: "clipboard".into(),
                // RED: the copy history routinely contains passwords.
                default_tier: RiskLevel::Red,
                min_tier: "lite",
                parameters: vec![
                    param(
                        "limit",
                        "integer",
                        "Maximum entries to return (1-256)",
                        false,
                    ),
                    param(
                        "cursor",
                        "string",
                        "Opaque pagination cursor from a previous page",
                        false,
                    ),
                ],
            },
            Arc::new(GetClipboardHistory),
        ),
        (
            ToolDef {
                name: "clear_clipboard_history".into(),
                description: "Destroy the entire retained clipboard history — irreversible".into(),
                category: "clipboard".into(),
                default_tier: RiskLevel::Red,
                min_tier: "lite",
                parameters: vec![],
            },
            Arc::new(ClearClipboardHistory),
        ),
        (
            ToolDef {
                name: "configure_clipboard_history".into(),
                description: "Set clipboard-history retention (enabled, lifetime, entry cap, allowed types)".into(),
                category: "clipboard".into(),
                default_tier: RiskLevel::Red,
                min_tier: "lite",
                parameters: vec![
                    param(
                        "enabled",
                        "boolean",
                        "Whether the history should capture new entries",
                        true,
                    ),
                    param(
                        "ttl_seconds",
                        "integer",
                        "Entry lifetime in seconds (60-31536000)",
                        false,
                    ),
                    param("max_items", "integer", "Maximum retained entries (1-1000)", false),
                    param(
                        "allowed_mimes",
                        "array",
                        "Retained content types: text/plain, text/html, image/png, image/jpeg",
                        false,
                    ),
                ],
            },
            Arc::new(ConfigureClipboardHistory),
        ),
        (
            ToolDef {
                name: "get_notification_state".into(),
                description: "Read whether this session is suppressing notification alerts".into(),
                category: "notifications".into(),
                default_tier: RiskLevel::Green,
                min_tier: "lite",
                parameters: vec![],
            },
            Arc::new(GetNotificationState),
        ),
        (
            ToolDef {
                name: "set_do_not_disturb".into(),
                description: "Turn do-not-disturb on or off (suppresses notification alerts)".into(),
                category: "notifications".into(),
                default_tier: RiskLevel::Yellow,
                min_tier: "lite",
                parameters: vec![param(
                    "enabled",
                    "boolean",
                    "Whether alerts should be suppressed",
                    true,
                )],
            },
            Arc::new(SetDoNotDisturb),
        ),
        (
            ToolDef {
                name: "get_os_capabilities".into(),
                description: "Report which OS-control capabilities this machine actually has".into(),
                category: "system_info".into(),
                default_tier: RiskLevel::Green,
                min_tier: "lite",
                parameters: vec![
                    param(
                        "domain",
                        "string",
                        "Restrict the report to one capability domain",
                        false,
                    ),
                    param(
                        "include_unavailable",
                        "boolean",
                        "Include operations that are not available on this machine",
                        false,
                    ),
                ],
            },
            Arc::new(GetOsCapabilities),
        ),
        (
            ToolDef {
                name: "graceful_close_application".into(),
                description: "Ask an application to close (SIGTERM only; never a forced kill)".into(),
                category: "applications".into(),
                default_tier: RiskLevel::Yellow,
                min_tier: "lite",
                parameters: vec![
                    param(
                        "app_id",
                        "string",
                        "Stable application identity. A window title is not accepted: it is neither unique nor stable.",
                        true,
                    ),
                    param(
                        "instance_id",
                        "string",
                        "Restrict the close to one running instance",
                        false,
                    ),
                ],
            },
            Arc::new(GracefulCloseApplication),
        ),
    ];

    for (def, handler) in tools {
        registry.register(def, handler);
    }
}

fn param(name: &str, ty: &str, desc: &str, required: bool) -> ParamDef {
    ParamDef {
        name: name.into(),
        param_type: ty.into(),
        description: desc.into(),
        required,
        default: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // No test in this module contains a clipboard payload, a notification body,
    // or any other host content: the handlers surface metadata only, so the
    // fixtures do not need one.

    #[test]
    fn identifier_validation_rejects_option_like_and_control_values() {
        assert!(validated_identifier("firefox", "app_id", 128).is_ok());
        // An option-like value would be read as a flag by whatever tool receives
        // it, so it is rejected rather than escaped.
        assert!(validated_identifier("-rf", "app_id", 128).is_err());
        assert!(validated_identifier("fire\nfox", "app_id", 128).is_err());
        assert!(validated_identifier("   ", "app_id", 128).is_err());
        assert!(validated_identifier(&"a".repeat(129), "app_id", 128).is_err());
    }

    #[test]
    fn bounded_integers_reject_out_of_range() {
        let params = serde_json::json!({ "limit": 0 });
        assert!(optional_bounded_u64(&params, "limit", 1, 256).is_err());
        let params = serde_json::json!({ "limit": 257 });
        assert!(optional_bounded_u64(&params, "limit", 1, 256).is_err());
        let params = serde_json::json!({ "limit": 32 });
        assert_eq!(
            optional_bounded_u64(&params, "limit", 1, 256).unwrap(),
            Some(32)
        );
        // Absent and null are the same fact: the caller named no limit.
        assert_eq!(
            optional_bounded_u64(&serde_json::json!({}), "limit", 1, 256).unwrap(),
            None
        );
        assert_eq!(
            optional_bounded_u64(&serde_json::json!({ "limit": null }), "limit", 1, 256).unwrap(),
            None
        );
        // A negative or fractional value is not an entry count.
        assert!(optional_bounded_u64(&serde_json::json!({ "limit": -1 }), "limit", 1, 256).is_err());
        assert!(
            optional_bounded_u64(&serde_json::json!({ "limit": 1.5 }), "limit", 1, 256).is_err()
        );
    }

    #[test]
    fn allowed_mime_parsing_is_closed() {
        assert_eq!(AllowedMime::parse("text/plain"), Some(AllowedMime::TextPlain));
        assert_eq!(AllowedMime::parse("IMAGE/PNG"), Some(AllowedMime::ImagePng));
        // Anything outside the frozen enum is rejected, never coerced.
        assert_eq!(AllowedMime::parse("application/pdf"), None);
        assert_eq!(AllowedMime::parse(""), None);
    }

    #[test]
    fn empty_history_page_reports_no_entries_not_a_missing_field() {
        let page = ClipboardHistoryPage::default();
        let projected = project_history_page(&page);
        assert_eq!(projected["items"].as_array().map(Vec::len), Some(0));
        assert_eq!(projected["truncated"], serde_json::json!(false));
        // An absent cursor is absent, not an empty string.
        assert!(projected.get("next_cursor").is_none());
    }

    #[test]
    fn history_projection_never_carries_a_payload_field() {
        use crate::os_control::clipboard::{ClipboardHistoryEntry, ClipboardHistoryItemId};
        let page = ClipboardHistoryPage {
            entries: vec![ClipboardHistoryEntry {
                item_id: ClipboardHistoryItemId::new("42"),
                mime: AllowedMime::TextPlain,
                byte_count: 11,
                captured_at_ms: None,
                // A digest of a fixed non-secret marker: no payload appears here.
                payload_digest: crate::os_control::contract::Digest::of_str("fixture-marker"),
            }],
            next_cursor: Some("opaque-cursor".to_string()),
            truncated: true,
        };
        let projected = project_history_page(&page);
        let item = &projected["items"][0];
        assert_eq!(item["item_id"], serde_json::json!("42"));
        assert_eq!(item["mime"], serde_json::json!("text/plain"));
        assert_eq!(item["byte_count"], serde_json::json!(11));
        // The frozen entry marks `payload` optional and classes it `Content`;
        // this surface never populates it.
        assert!(item.get("payload").is_none());
        // A store that records no capture time reports none, rather than the
        // read time.
        assert!(item.get("captured_at_ms").is_none());
        assert_eq!(projected["next_cursor"], serde_json::json!("opaque-cursor"));
        assert_eq!(projected["truncated"], serde_json::json!(true));
    }

    #[test]
    fn notification_projection_distinguishes_unknown_server_from_absent() {
        use crate::os_control::notifications::{DoNotDisturb, ServerAvailability};
        let state = NotificationSessionState {
            do_not_disturb: DoNotDisturb::On,
            server: ServerAvailability::Unknown,
        };
        let projected = project_notification_state(&state, Some(7));
        assert_eq!(projected["fields"]["do_not_disturb"], serde_json::json!("on"));
        // "could not determine" is reported as such, never as "no server".
        assert_eq!(
            projected["fields"]["server_available"],
            serde_json::json!("unknown")
        );
        assert_eq!(projected["revision"], serde_json::json!(7));
        // No notification server exposes a pending count, so the field is absent
        // rather than zero.
        assert!(projected["fields"].get("pending_count").is_none());
    }
}
