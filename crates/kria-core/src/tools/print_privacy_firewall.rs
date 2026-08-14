//! Print, privacy and firewall tool handlers.
//!
//! linux-os-control-production tasks **4.3**, **4.7**, **5.3**
//! (OSC-017, OSC-021, OSC-029).
//!
//! Every handler routes through [`crate::tools::os_governed`].
//!
//! # What the risk levels here actually mean
//!
//! * `print_file` is RED and claims **no rollback**: a spooled job cannot be
//!   recalled, and cancelling one that already printed does not un-print paper.
//! * `cancel_print_job` is RED because a queue on a shared machine holds other
//!   users' jobs; the domain refuses anything the caller does not own.
//! * `set_firewall_enabled` is RED **when disabling** — that exposes every
//!   listening service at once, and packets that got in cannot be recalled.
//! * `get_privacy_state` is RED even though it changes nothing: reporting which
//!   sensors are open is itself sensitive.

use std::sync::Arc;

use async_trait::async_trait;

use crate::infra::ToolResult;
use crate::os_control::broker::protocol::ReviewedPrinterOptions;
use crate::os_control::firewall::{validate_app_id, FirewallOp, FirewallRequest, GrantDuration};
use crate::os_control::print::{
    PrintOp, PrintRequest, PrinterId, PrintJobId, ReviewedPrintOptions,
};
use crate::os_control::privacy::{parse_control, PrivacyRequest};
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

/// Read a required absolute path, refusing traversal.
fn required_path(params: &serde_json::Value, field: &str) -> Result<std::path::PathBuf, ToolResult> {
    let raw = params[field].as_str().unwrap_or("").trim();
    if raw.is_empty() {
        return Err(ToolResult::err(format!("`{field}` is required")));
    }
    if raw.chars().any(char::is_control) {
        return Err(ToolResult::err(format!("`{field}` contains control characters")));
    }
    let path = std::path::PathBuf::from(raw);
    if !path.is_absolute() {
        return Err(ToolResult::err(format!("`{field}` must be an absolute path")));
    }
    if path
        .components()
        .any(|c| matches!(c, std::path::Component::ParentDir))
    {
        return Err(ToolResult::err(format!("`{field}` must not contain `..`")));
    }
    Ok(path)
}

// ─────────────────────────────────────────────────────────────────────────────
// Printing
// ─────────────────────────────────────────────────────────────────────────────

/// Drive one governed print mutation.
async fn run_print(
    ctx: &ToolContext,
    tool: &str,
    params: serde_json::Value,
    op: PrintOp,
) -> ToolResult {
    let resolved = match gov::resolve(ctx, tool) {
        Ok(resolved) => resolved,
        Err(result) => return result,
    };
    let provider = match resolved.runtime.print_control(tool) {
        Ok(provider) => provider,
        Err(error) => return gov::os_error(&error),
    };
    let call = match gov::mutation_call(ctx, &resolved.runtime, tool) {
        Ok(call) => call,
        Err(result) => return result,
    };
    let request = PrintRequest {
        action: tool.to_string(),
        params,
        op,
    };
    // The pre-state comes from the domain's own observation, so the postcondition
    // is derived rather than assumed.
    let observed = match crate::os_control::contract::DesiredStateControl::observe(
        provider,
        call.observation(),
        &request,
    )
    .await
    {
        Ok(observed) => observed,
        Err(error) => return gov::os_error(&error),
    };
    let desired = request.desired_state(&observed);
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

struct ListPrinters;

#[async_trait]
impl ToolHandler for ListPrinters {
    async fn execute(&self, _params: serde_json::Value) -> ToolResult {
        gov::os_unavailable(None, "list_printers")
    }

    async fn execute_with_context(&self, params: serde_json::Value, ctx: ToolContext) -> ToolResult {
        let tool = "list_printers";
        let resolved = match gov::resolve(&ctx, tool) {
            Ok(resolved) => resolved,
            Err(result) => return result,
        };
        let provider = match resolved.runtime.print_control(tool) {
            Ok(provider) => provider,
            Err(error) => return gov::os_error(&error),
        };
        let call = match gov::read_call(&ctx, &resolved.runtime, tool) {
            Ok(call) => call,
            Err(result) => return result,
        };
        let limit = params["limit"].as_u64().and_then(|v| usize::try_from(v).ok());
        match provider
            .printers(call.observation(), params["cursor"].as_str(), limit)
            .await
        {
            Ok(page) => ToolResult::ok(serde_json::json!({
                "printers": page.items.iter().map(|p| serde_json::json!({
                    // The queue name is the identity; the description is display text.
                    "printer": p.printer.as_str(),
                    "description": p.description.as_str(),
                    "accepting": p.accepting,
                    "is_default": p.is_default,
                    "state": p.state,
                })).collect::<Vec<_>>(),
                "next_cursor": page.next_cursor.as_deref(),
                "truncated": page.truncated,
            })),
            Err(error) => gov::os_error(&error),
        }
    }
}

struct GetPrintQueue;

#[async_trait]
impl ToolHandler for GetPrintQueue {
    async fn execute(&self, _params: serde_json::Value) -> ToolResult {
        gov::os_unavailable(None, "get_print_queue")
    }

    async fn execute_with_context(&self, params: serde_json::Value, ctx: ToolContext) -> ToolResult {
        let tool = "get_print_queue";
        let printer = match params["printer"].as_str() {
            Some(raw) => match PrinterId::parse(raw) {
                Ok(id) => Some(id),
                Err(error) => return gov::os_error(&error),
            },
            None => None,
        };
        let resolved = match gov::resolve(&ctx, tool) {
            Ok(resolved) => resolved,
            Err(result) => return result,
        };
        let provider = match resolved.runtime.print_control(tool) {
            Ok(provider) => provider,
            Err(error) => return gov::os_error(&error),
        };
        let call = match gov::read_call(&ctx, &resolved.runtime, tool) {
            Ok(call) => call,
            Err(result) => return result,
        };
        let limit = params["limit"].as_u64().and_then(|v| usize::try_from(v).ok());
        match provider
            .queue(
                call.observation(),
                printer.as_ref(),
                params["cursor"].as_str(),
                limit,
            )
            .await
        {
            Ok(page) => ToolResult::ok(serde_json::json!({
                "jobs": page.items.iter().map(|j| serde_json::json!({
                    "job": j.job.as_str(),
                    "printer": j.printer.as_str(),
                    // Surfaced so a caller knows which jobs it may cancel at all.
                    "owned_by_you": j.owned_by_caller,
                    "state": j.state,
                    "size_bytes": j.size_bytes,
                })).collect::<Vec<_>>(),
                "next_cursor": page.next_cursor.as_deref(),
                "truncated": page.truncated,
            })),
            Err(error) => gov::os_error(&error),
        }
    }
}

struct PrintFile;

#[async_trait]
impl ToolHandler for PrintFile {
    async fn execute(&self, _params: serde_json::Value) -> ToolResult {
        gov::os_unavailable(None, "print_file")
    }

    async fn execute_with_context(&self, params: serde_json::Value, ctx: ToolContext) -> ToolResult {
        let tool = "print_file";
        let printer = match params["printer"].as_str() {
            Some(raw) => match PrinterId::parse(raw) {
                Ok(id) => id,
                Err(error) => return gov::os_error(&error),
            },
            None => return ToolResult::err("`printer` is required"),
        };
        let path = match required_path(&params, "path") {
            Ok(path) => path,
            Err(result) => return result,
        };
        // A closed option set. There is deliberately no pass-through for arbitrary
        // `lp -o` strings, which would be an injection point into the spooler.
        let options = ReviewedPrintOptions {
            copies: u8::try_from(params["options"]["copies"].as_u64().unwrap_or(1)).unwrap_or(0),
            duplex: params["options"]["duplex"].as_bool().unwrap_or(false),
        };
        let options = match options.validate() {
            Ok(options) => options,
            Err(error) => return gov::os_error(&error),
        };
        run_print(
            &ctx,
            tool,
            params,
            PrintOp::Submit {
                printer,
                path,
                options,
            },
        )
        .await
    }
}

struct CancelPrintJob;

#[async_trait]
impl ToolHandler for CancelPrintJob {
    async fn execute(&self, _params: serde_json::Value) -> ToolResult {
        gov::os_unavailable(None, "cancel_print_job")
    }

    async fn execute_with_context(&self, params: serde_json::Value, ctx: ToolContext) -> ToolResult {
        let tool = "cancel_print_job";
        let job = match params["job"].as_str() {
            Some(raw) => match PrintJobId::parse(raw) {
                Ok(id) => id,
                Err(error) => return gov::os_error(&error),
            },
            None => return ToolResult::err("`job` is required"),
        };
        // Ownership is enforced in the domain, immediately before cancelling: a
        // shared queue holds other users' work.
        run_print(&ctx, tool, params, PrintOp::CancelOwned { job }).await
    }
}

struct ConfigurePrinter;

#[async_trait]
impl ToolHandler for ConfigurePrinter {
    async fn execute(&self, _params: serde_json::Value) -> ToolResult {
        gov::os_unavailable(None, "configure_printer")
    }

    async fn execute_with_context(&self, params: serde_json::Value, ctx: ToolContext) -> ToolResult {
        let tool = "configure_printer";
        let Some(discovered_raw) = params["discovered"].as_str() else {
            return ToolResult::err("`discovered` is required (a discovered printer id)");
        };
        let discovered = match crate::os_control::broker::protocol::DiscoveredPrinterId::new(
            discovered_raw.trim(),
        ) {
            Ok(id) => id,
            Err(_) => {
                return ToolResult::err(
                    "`discovered` must be a bounded, control-character-free printer id",
                )
            }
        };
        // A closed, reviewed option set — three booleans, no driver strings.
        let options = ReviewedPrinterOptions {
            set_default: params["options"]["set_default"].as_bool().unwrap_or(false),
            shared: params["options"]["shared"].as_bool().unwrap_or(false),
            accept_jobs: params["options"]["accept_jobs"].as_bool().unwrap_or(true),
        };
        run_print(
            &ctx,
            tool,
            params,
            PrintOp::Configure {
                discovered,
                options,
            },
        )
        .await
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Privacy
// ─────────────────────────────────────────────────────────────────────────────

struct GetPrivacyState;

#[async_trait]
impl ToolHandler for GetPrivacyState {
    async fn execute(&self, _params: serde_json::Value) -> ToolResult {
        gov::os_unavailable(None, "get_privacy_state")
    }

    async fn execute_with_context(
        &self,
        _params: serde_json::Value,
        ctx: ToolContext,
    ) -> ToolResult {
        let tool = "get_privacy_state";
        let resolved = match gov::resolve(&ctx, tool) {
            Ok(resolved) => resolved,
            Err(result) => return result,
        };
        let provider = match resolved.runtime.privacy(tool) {
            Ok(provider) => provider,
            Err(error) => return gov::os_error(&error),
        };
        let call = match gov::read_call(&ctx, &resolved.runtime, tool) {
            Ok(call) => call,
            Err(result) => return result,
        };
        match provider.snapshot(call.observation()).await {
            // `null` means the setting could not be read. It is NOT reported as
            // `false`: telling the user a camera is off when it may be open is the
            // worst answer this tool can give.
            Ok(snapshot) => ToolResult::ok(serde_json::json!({
                "camera": snapshot.camera,
                "microphone": snapshot.microphone,
                "location": snapshot.location,
                "note": "a null value means the setting could not be read, not that access is off",
            })),
            Err(error) => gov::os_error(&error),
        }
    }
}

struct SetPrivacyControl;

#[async_trait]
impl ToolHandler for SetPrivacyControl {
    async fn execute(&self, _params: serde_json::Value) -> ToolResult {
        gov::os_unavailable(None, "set_privacy_control")
    }

    async fn execute_with_context(&self, params: serde_json::Value, ctx: ToolContext) -> ToolResult {
        let tool = "set_privacy_control";
        let control = match params["control"].as_str() {
            Some(raw) => match parse_control(raw) {
                Ok(control) => control,
                Err(error) => return gov::os_error(&error),
            },
            None => return ToolResult::err("`control` is required (camera, microphone, location)"),
        };
        let Some(enabled) = params["enabled"].as_bool() else {
            return ToolResult::err("`enabled` must be a boolean");
        };

        let resolved = match gov::resolve(&ctx, tool) {
            Ok(resolved) => resolved,
            Err(result) => return result,
        };
        let provider = match resolved.runtime.privacy(tool) {
            Ok(provider) => provider,
            Err(error) => return gov::os_error(&error),
        };
        let call = match gov::mutation_call(&ctx, &resolved.runtime, tool) {
            Ok(call) => call,
            Err(result) => return result,
        };
        let request = PrivacyRequest {
            action: tool.to_string(),
            params,
            control,
            enabled,
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
// Firewall
// ─────────────────────────────────────────────────────────────────────────────

async fn run_firewall(
    ctx: &ToolContext,
    tool: &str,
    params: serde_json::Value,
    op: FirewallOp,
) -> ToolResult {
    let resolved = match gov::resolve(ctx, tool) {
        Ok(resolved) => resolved,
        Err(result) => return result,
    };
    let provider = match resolved.runtime.firewall(tool) {
        Ok(provider) => provider,
        Err(error) => return gov::os_error(&error),
    };
    let call = match gov::mutation_call(ctx, &resolved.runtime, tool) {
        Ok(call) => call,
        Err(result) => return result,
    };
    let request = FirewallRequest {
        action: tool.to_string(),
        params,
        op,
    };
    let observed = match crate::os_control::contract::DesiredStateControl::observe(
        provider,
        call.observation(),
        &request,
    )
    .await
    {
        Ok(observed) => observed,
        Err(error) => return gov::os_error(&error),
    };
    let desired = request.desired_state(&observed);
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

struct GetFirewallStatus;

#[async_trait]
impl ToolHandler for GetFirewallStatus {
    async fn execute(&self, _params: serde_json::Value) -> ToolResult {
        gov::os_unavailable(None, "get_firewall_status")
    }

    async fn execute_with_context(
        &self,
        _params: serde_json::Value,
        ctx: ToolContext,
    ) -> ToolResult {
        let tool = "get_firewall_status";
        let resolved = match gov::resolve(&ctx, tool) {
            Ok(resolved) => resolved,
            Err(result) => return result,
        };
        let provider = match resolved.runtime.firewall(tool) {
            Ok(provider) => provider,
            Err(error) => return gov::os_error(&error),
        };
        let call = match gov::read_call(&ctx, &resolved.runtime, tool) {
            Ok(call) => call,
            Err(result) => return result,
        };
        match provider.status(call.observation()).await {
            // `enabled: null` is unknown, never "protected".
            Ok(facts) => ToolResult::ok(serde_json::json!({
                "provider": facts.provider.tag(),
                "enabled": facts.enabled,
                "default_incoming": facts.default_incoming,
                "rule_count": facts.rule_count,
                "note": "a null `enabled` means the state could not be read, not that the firewall is on",
            })),
            Err(error) => gov::os_error(&error),
        }
    }
}

struct SetFirewallEnabled;

#[async_trait]
impl ToolHandler for SetFirewallEnabled {
    async fn execute(&self, _params: serde_json::Value) -> ToolResult {
        gov::os_unavailable(None, "set_firewall_enabled")
    }

    async fn execute_with_context(&self, params: serde_json::Value, ctx: ToolContext) -> ToolResult {
        let tool = "set_firewall_enabled";
        let Some(enabled) = params["enabled"].as_bool() else {
            return ToolResult::err("`enabled` must be a boolean");
        };
        // Disabling is the dangerous direction: it exposes every listening service
        // at once, and the contract rates that RED while enabling is YELLOW.
        run_firewall(&ctx, tool, params, FirewallOp::SetEnabled(enabled)).await
    }
}

struct GrantTemporaryAppNetworkAccess;

#[async_trait]
impl ToolHandler for GrantTemporaryAppNetworkAccess {
    async fn execute(&self, _params: serde_json::Value) -> ToolResult {
        gov::os_unavailable(None, "grant_temporary_app_network_access")
    }

    async fn execute_with_context(&self, params: serde_json::Value, ctx: ToolContext) -> ToolResult {
        let tool = "grant_temporary_app_network_access";
        let app_id = match params["app_id"].as_str() {
            Some(raw) => match validate_app_id(raw) {
                Ok(id) => id,
                Err(error) => return gov::os_error(&error),
            },
            None => return ToolResult::err("`app_id` is required"),
        };
        let Some(duration_ms) = params["duration"].as_u64() else {
            return ToolResult::err(
                "`duration` is required in milliseconds: there is no unbounded grant",
            );
        };
        let duration = match GrantDuration::parse(duration_ms) {
            Ok(duration) => duration,
            Err(error) => return gov::os_error(&error),
        };
        run_firewall(
            &ctx,
            tool,
            params,
            FirewallOp::GrantTemporary { app_id, duration },
        )
        .await
    }
}

/// Register the print, privacy and firewall tool surface.
pub fn register(registry: &ToolRegistry) {
    let tools: Vec<(ToolDef, Arc<dyn ToolHandler>)> = vec![
        (
            ToolDef {
                name: "list_printers".into(),
                description: "List available printers".into(),
                category: "print".into(),
                default_tier: RiskLevel::Green,
                min_tier: "lite",
                parameters: vec![
                    param("cursor", "string", "Page cursor", false),
                    param("limit", "integer", "Maximum rows (max 256)", false),
                ],
            },
            Arc::new(ListPrinters),
        ),
        (
            ToolDef {
                name: "get_print_queue".into(),
                description: "List queued print jobs".into(),
                category: "print".into(),
                default_tier: RiskLevel::Green,
                min_tier: "lite",
                parameters: vec![
                    param("printer", "string", "Limit to one printer queue", false),
                    param("cursor", "string", "Page cursor", false),
                    param("limit", "integer", "Maximum rows (max 256)", false),
                ],
            },
            Arc::new(GetPrintQueue),
        ),
        (
            ToolDef {
                name: "print_file".into(),
                description: "Send a file to a printer — cannot be recalled once spooled".into(),
                category: "print".into(),
                default_tier: RiskLevel::Red,
                min_tier: "lite",
                parameters: vec![
                    param("printer", "string", "Printer queue name", true),
                    param("path", "string", "Absolute path to the file", true),
                    param(
                        "options",
                        "object",
                        "{copies: 1-99, duplex: bool}. Arbitrary driver options are not accepted.",
                        false,
                    ),
                ],
            },
            Arc::new(PrintFile),
        ),
        (
            ToolDef {
                name: "cancel_print_job".into(),
                description: "Cancel one of your own print jobs".into(),
                category: "print".into(),
                default_tier: RiskLevel::Red,
                min_tier: "lite",
                parameters: vec![param(
                    "job",
                    "string",
                    "Job id. A job owned by another user is refused.",
                    true,
                )],
            },
            Arc::new(CancelPrintJob),
        ),
        (
            ToolDef {
                name: "configure_printer".into(),
                description: "Configure a discovered printer".into(),
                category: "print".into(),
                default_tier: RiskLevel::Red,
                min_tier: "lite",
                parameters: vec![
                    param("discovered", "string", "Discovered printer id", true),
                    param(
                        "options",
                        "object",
                        "{set_default, shared, accept_jobs} — a closed reviewed set",
                        true,
                    ),
                ],
            },
            Arc::new(ConfigurePrinter),
        ),
        (
            ToolDef {
                name: "get_privacy_state".into(),
                description: "Read whether camera, microphone and location access are permitted"
                    .into(),
                category: "privacy".into(),
                // Reporting which sensors are open is itself sensitive.
                default_tier: RiskLevel::Red,
                min_tier: "lite",
                parameters: vec![],
            },
            Arc::new(GetPrivacyState),
        ),
        (
            ToolDef {
                name: "set_privacy_control".into(),
                description: "Allow or block camera, microphone, or location access".into(),
                category: "privacy".into(),
                default_tier: RiskLevel::Red,
                min_tier: "lite",
                parameters: vec![
                    param("control", "string", "camera, microphone, or location", true),
                    param("enabled", "boolean", "Whether access is permitted", true),
                ],
            },
            Arc::new(SetPrivacyControl),
        ),
        (
            ToolDef {
                name: "get_firewall_status".into(),
                description: "Read the firewall's state and default policy".into(),
                category: "firewall".into(),
                default_tier: RiskLevel::Green,
                min_tier: "lite",
                parameters: vec![],
            },
            Arc::new(GetFirewallStatus),
        ),
        (
            ToolDef {
                name: "set_firewall_enabled".into(),
                description: "Turn the firewall on or off — disabling exposes every service".into(),
                category: "firewall".into(),
                // The contract is conditional: RED to disable, YELLOW to enable.
                // The stricter default is declared here.
                default_tier: RiskLevel::Red,
                min_tier: "lite",
                parameters: vec![param("enabled", "boolean", "Desired firewall state", true)],
            },
            Arc::new(SetFirewallEnabled),
        ),
        (
            ToolDef {
                name: "grant_temporary_app_network_access".into(),
                description: "Let one application through the firewall for a bounded time".into(),
                category: "firewall".into(),
                default_tier: RiskLevel::Red,
                min_tier: "lite",
                parameters: vec![
                    param("app_id", "string", "The application's stable id", true),
                    param(
                        "duration",
                        "integer",
                        "Milliseconds, 1 second to 4 hours. There is no unbounded grant.",
                        true,
                    ),
                ],
            },
            Arc::new(GrantTemporaryAppNetworkAccess),
        ),
    ];

    for (def, handler) in tools {
        registry.register(def, handler);
    }
}
