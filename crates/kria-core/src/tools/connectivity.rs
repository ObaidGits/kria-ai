//! Connectivity tool handlers — VPN, network diagnosis, hotspot, proxy, and
//! saved connectivity credentials.
//!
//! linux-os-control-production tasks **4.2**, **5.3** and **5.6** (OSC-011).
//!
//! Every handler routes through [`crate::tools::os_governed`]; none of them runs
//! `nmcli` or `gsettings` directly.
//!
//! # The rules that shape this file
//!
//! * **A passphrase never becomes an argv element.** `/proc/<pid>/cmdline` is
//!   world-readable, so a Wi-Fi or VPN secret is passed by *reference* to a
//!   stored credential; the domain resolves it under the admitted mutation
//!   context and delivers it on the child's stdin.
//! * **A saved credential's VALUE is never returned.** The listing reports the
//!   profile, a label, the kind, and an opaque reference — enough to act on,
//!   nothing to leak.
//! * **A profile is a UUID, not a name.** Two saved connections can share a
//!   display name, so acting on a name could hit the wrong network.
//! * **"Offline" and "could not determine" are different answers.** The
//!   diagnosis reports each field as its own verdict rather than collapsing an
//!   unknown into a negative.

use std::sync::Arc;

use async_trait::async_trait;

use crate::infra::ToolResult;
use crate::os_control::connectivity::{
    ConnectivityOp, ConnectivityRequest, NetworkDeviceId, NetworkProfileId, ProxyEndpoint,
    ProxyProfile,
};
use crate::os_control::secrets::SecretRef;
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

/// Read a required identifier, rejecting option-looking and control-bearing
/// values rather than escaping them.
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

fn required_bool(params: &serde_json::Value, field: &str) -> Result<bool, ToolResult> {
    params[field]
        .as_bool()
        .ok_or_else(|| ToolResult::err(format!("`{field}` must be a boolean")))
}

/// Reject a plaintext secret supplied directly as a parameter.
///
/// The contract takes a *reference* to a stored credential. Accepting a raw
/// passphrase here would put it in the tool params, and therefore in the params
/// digest and the audit record — exactly what the reference indirection exists to
/// prevent.
fn refuse_inline_secret(params: &serde_json::Value, tool: &str) -> Option<ToolResult> {
    for field in ["password", "passphrase", "secret", "psk", "key"] {
        if !params[field].is_null() {
            return Some(ToolResult::err(format!(
                "`{tool}` does not accept `{field}` inline: store the secret first and pass \
                 `credential` (its reference) so the value never enters argv, params, or the audit record"
            )));
        }
    }
    None
}

/// Drive one governed connectivity mutation.
async fn run_mutation(
    ctx: &ToolContext,
    tool: &str,
    params: serde_json::Value,
    op: ConnectivityOp,
) -> ToolResult {
    let resolved = match gov::resolve(ctx, tool) {
        Ok(resolved) => resolved,
        Err(result) => return result,
    };
    let provider = match resolved.runtime.connectivity(tool) {
        Ok(provider) => provider,
        Err(error) => return gov::os_error(&error),
    };
    let call = match gov::mutation_call(ctx, &resolved.runtime, tool) {
        Ok(call) => call,
        Err(result) => return result,
    };
    let request = ConnectivityRequest {
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

/// Resolve the provider and a read-admitted observation context for one read.
macro_rules! read_setup {
    ($ctx:expr, $tool:expr) => {{
        let resolved = match gov::resolve(&$ctx, $tool) {
            Ok(resolved) => resolved,
            Err(result) => return result,
        };
        match resolved.runtime.connectivity($tool) {
            Ok(_) => {}
            Err(error) => return gov::os_error(&error),
        }
        let call = match gov::read_call(&$ctx, &resolved.runtime, $tool) {
            Ok(call) => call,
            Err(result) => return result,
        };
        (resolved, call)
    }};
}

// ─────────────────────────────────────────────────────────────────────────────
// Reads
// ─────────────────────────────────────────────────────────────────────────────

struct ListVpnProfiles;

#[async_trait]
impl ToolHandler for ListVpnProfiles {
    async fn execute(&self, _params: serde_json::Value) -> ToolResult {
        gov::os_unavailable(None, "list_vpn_profiles")
    }

    async fn execute_with_context(
        &self,
        _params: serde_json::Value,
        ctx: ToolContext,
    ) -> ToolResult {
        let tool = "list_vpn_profiles";
        let (resolved, call) = read_setup!(ctx, tool);
        let provider = match resolved.runtime.connectivity(tool) {
            Ok(provider) => provider,
            Err(error) => return gov::os_error(&error),
        };
        match provider.list_vpn_profiles(call.observation()).await {
            Ok(profiles) => ToolResult::ok(serde_json::json!({
                "profiles": profiles.iter().map(|p| serde_json::json!({
                    // The UUID is the identity a mutation must be addressed to.
                    "profile": p.profile.as_str(),
                    "label": p.label,
                    "connected": p.connected,
                })).collect::<Vec<_>>(),
            })),
            Err(error) => gov::os_error(&error),
        }
    }
}

struct DiagnoseNetwork;

#[async_trait]
impl ToolHandler for DiagnoseNetwork {
    async fn execute(&self, _params: serde_json::Value) -> ToolResult {
        gov::os_unavailable(None, "diagnose_network")
    }

    async fn execute_with_context(
        &self,
        _params: serde_json::Value,
        ctx: ToolContext,
    ) -> ToolResult {
        let tool = "diagnose_network";
        let (resolved, call) = read_setup!(ctx, tool);
        let provider = match resolved.runtime.connectivity(tool) {
            Ok(provider) => provider,
            Err(error) => return gov::os_error(&error),
        };
        // `false`: never probe an external target unless the caller asked. A silent
        // outbound request during a diagnosis would be a privacy surprise.
        let probe_internet = _params["probe_internet"].as_bool().unwrap_or(false);
        match provider.diagnose(call.observation(), probe_internet).await {
            // Each field carries its own verdict, including "undetermined".
            // Collapsing an unknown into "down" would report a network problem
            // that was never actually observed.
            Ok(facts) => ToolResult::ok(serde_json::json!({
                "link": facts.link,
                "address": facts.address,
                "route": facts.route,
                "gateway": facts.gateway,
                "dns": facts.dns,
                "internet": facts.internet,
                "captive_portal": facts.captive_portal,
                "target_probe_unavailable": facts.target_probe_unavailable,
            })),
            Err(error) => gov::os_error(&error),
        }
    }
}

struct GetHotspotState;

#[async_trait]
impl ToolHandler for GetHotspotState {
    async fn execute(&self, _params: serde_json::Value) -> ToolResult {
        gov::os_unavailable(None, "get_hotspot_state")
    }

    async fn execute_with_context(
        &self,
        _params: serde_json::Value,
        ctx: ToolContext,
    ) -> ToolResult {
        let tool = "get_hotspot_state";
        let (resolved, call) = read_setup!(ctx, tool);
        let provider = match resolved.runtime.connectivity(tool) {
            Ok(provider) => provider,
            Err(error) => return gov::os_error(&error),
        };
        // No device filter: report whichever device is serving a hotspot rather
        // than requiring the caller to already know.
        match provider.get_hotspot_state(call.observation(), None).await {
            Ok(facts) => ToolResult::ok(serde_json::json!({
                "enabled": facts.enabled,
                "device": facts.device.as_ref().map(|d| d.as_str()),
                "profile": facts.profile.as_ref().map(|p| p.as_str()),
            })),
            Err(error) => gov::os_error(&error),
        }
    }
}

struct GetProxyState;

#[async_trait]
impl ToolHandler for GetProxyState {
    async fn execute(&self, _params: serde_json::Value) -> ToolResult {
        gov::os_unavailable(None, "get_proxy_state")
    }

    async fn execute_with_context(
        &self,
        _params: serde_json::Value,
        ctx: ToolContext,
    ) -> ToolResult {
        let tool = "get_proxy_state";
        let (resolved, call) = read_setup!(ctx, tool);
        let provider = match resolved.runtime.connectivity(tool) {
            Ok(provider) => provider,
            Err(error) => return gov::os_error(&error),
        };
        match provider.get_proxy_state(call.observation()).await {
            Ok(facts) => ToolResult::ok(serde_json::json!({
                "mode": facts.mode,
                "profile": describe_proxy(facts.profile.as_ref()),
            })),
            Err(error) => gov::os_error(&error),
        }
    }
}

/// Describe a proxy profile without inventing fields it does not carry.
fn describe_proxy(profile: Option<&ProxyProfile>) -> serde_json::Value {
    match profile {
        None => serde_json::Value::Null,
        Some(ProxyProfile::Automatic { pac_uri }) => serde_json::json!({
            "kind": "automatic",
            "pac_uri": pac_uri,
        }),
        Some(ProxyProfile::Manual {
            http,
            https,
            socks,
            exclusions,
        }) => serde_json::json!({
            "kind": "manual",
            "http": endpoint_json(http.as_ref()),
            "https": endpoint_json(https.as_ref()),
            "socks": endpoint_json(socks.as_ref()),
            "exclusions": exclusions,
        }),
    }
}

fn endpoint_json(endpoint: Option<&ProxyEndpoint>) -> serde_json::Value {
    endpoint.map_or(serde_json::Value::Null, |e| {
        serde_json::json!({ "host": e.host, "port": e.port })
    })
}

struct ListSavedConnectivityCredentials;

#[async_trait]
impl ToolHandler for ListSavedConnectivityCredentials {
    async fn execute(&self, _params: serde_json::Value) -> ToolResult {
        gov::os_unavailable(None, "list_saved_connectivity_credentials")
    }

    async fn execute_with_context(
        &self,
        _params: serde_json::Value,
        ctx: ToolContext,
    ) -> ToolResult {
        let tool = "list_saved_connectivity_credentials";
        let (resolved, call) = read_setup!(ctx, tool);
        let provider = match resolved.runtime.connectivity(tool) {
            Ok(provider) => provider,
            Err(error) => return gov::os_error(&error),
        };
        // No kind filter: list every saved credential's metadata.
        match provider.list_saved_credentials(call.observation(), None).await {
            // Metadata only. The secret VALUE is never read, so it cannot leak
            // through this listing even by accident.
            Ok(rows) => ToolResult::ok(serde_json::json!({
                "credentials": rows.iter().map(|c| serde_json::json!({
                    "profile": c.profile.as_str(),
                    "label": c.label,
                    "kind": format!("{:?}", c.kind),
                    "reference": c.secret_ref,
                })).collect::<Vec<_>>(),
            })),
            Err(error) => gov::os_error(&error),
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Mutations
// ─────────────────────────────────────────────────────────────────────────────

struct SetVpnConnection;

#[async_trait]
impl ToolHandler for SetVpnConnection {
    async fn execute(&self, _params: serde_json::Value) -> ToolResult {
        gov::os_unavailable(None, "set_vpn_connection")
    }

    async fn execute_with_context(
        &self,
        params: serde_json::Value,
        ctx: ToolContext,
    ) -> ToolResult {
        let tool = "set_vpn_connection";
        if let Some(refusal) = refuse_inline_secret(&params, tool) {
            return refusal;
        }
        let profile = match required_id(&params, "profile") {
            Ok(profile) => profile,
            Err(result) => return result,
        };
        let connected = match required_bool(&params, "connected") {
            Ok(connected) => connected,
            Err(result) => return result,
        };
        run_mutation(
            &ctx,
            tool,
            params,
            ConnectivityOp::SetVpn {
                profile: NetworkProfileId::new(profile),
                connected,
            },
        )
        .await
    }
}

struct SetHotspot;

#[async_trait]
impl ToolHandler for SetHotspot {
    async fn execute(&self, _params: serde_json::Value) -> ToolResult {
        gov::os_unavailable(None, "set_hotspot")
    }

    async fn execute_with_context(
        &self,
        params: serde_json::Value,
        ctx: ToolContext,
    ) -> ToolResult {
        let tool = "set_hotspot";
        // A hotspot broadcasts a network from this machine. An inline passphrase
        // would land in argv and the audit record, so only a stored reference is
        // accepted.
        if let Some(refusal) = refuse_inline_secret(&params, tool) {
            return refusal;
        }
        let device = match required_id(&params, "device") {
            Ok(device) => device,
            Err(result) => return result,
        };
        let enabled = match required_bool(&params, "enabled") {
            Ok(enabled) => enabled,
            Err(result) => return result,
        };
        let profile = params["profile"]
            .as_str()
            .map(|p| NetworkProfileId::new(p.trim()));
        let credential = params["credential"]
            .as_str()
            .map(|c| SecretRef::new(c.trim()));
        run_mutation(
            &ctx,
            tool,
            params,
            ConnectivityOp::SetHotspot {
                device: NetworkDeviceId::new(device),
                enabled,
                profile,
                credential,
            },
        )
        .await
    }
}

struct SetProxyProfile;

#[async_trait]
impl ToolHandler for SetProxyProfile {
    async fn execute(&self, _params: serde_json::Value) -> ToolResult {
        gov::os_unavailable(None, "set_proxy_profile")
    }

    async fn execute_with_context(
        &self,
        params: serde_json::Value,
        ctx: ToolContext,
    ) -> ToolResult {
        let tool = "set_proxy_profile";
        let mode = match required_id(&params, "mode") {
            Ok(mode) => mode.to_ascii_lowercase(),
            Err(result) => return result,
        };

        // A proxy redirects EVERY application's traffic, so the mode is a closed
        // set and the profile must match it. A mismatched pair is refused rather
        // than partially applied.
        let profile = match mode.as_str() {
            "none" => None,
            "automatic" => {
                let pac_uri = match required_id(&params, "pac_uri") {
                    Ok(uri) => uri,
                    Err(result) => return result,
                };
                if !(pac_uri.starts_with("http://") || pac_uri.starts_with("https://")) {
                    return ToolResult::err(
                        "`pac_uri` must be an http:// or https:// URL",
                    );
                }
                Some(ProxyProfile::Automatic { pac_uri })
            }
            "manual" => Some(ProxyProfile::Manual {
                http: parse_endpoint(&params, "http"),
                https: parse_endpoint(&params, "https"),
                socks: parse_endpoint(&params, "socks"),
                exclusions: params["exclusions"]
                    .as_array()
                    .map(|rows| {
                        rows.iter()
                            .filter_map(|v| v.as_str())
                            .map(str::to_string)
                            .collect()
                    })
                    .unwrap_or_default(),
            }),
            other => {
                return ToolResult::err(format!(
                    "`mode` must be one of none, automatic, manual (got `{other}`)"
                ))
            }
        };

        run_mutation(&ctx, tool, params, ConnectivityOp::SetProxy { mode, profile }).await
    }
}

fn parse_endpoint(params: &serde_json::Value, field: &str) -> Option<ProxyEndpoint> {
    let host = params[field]["host"].as_str()?.trim();
    let port = params[field]["port"].as_u64()?;
    if host.is_empty() || host.chars().any(char::is_control) {
        return None;
    }
    Some(ProxyEndpoint {
        host: host.to_string(),
        port: u16::try_from(port).ok()?,
    })
}

struct ReplaceSavedConnectivityCredential;

#[async_trait]
impl ToolHandler for ReplaceSavedConnectivityCredential {
    async fn execute(&self, _params: serde_json::Value) -> ToolResult {
        gov::os_unavailable(None, "replace_saved_connectivity_credential")
    }

    async fn execute_with_context(
        &self,
        params: serde_json::Value,
        ctx: ToolContext,
    ) -> ToolResult {
        let tool = "replace_saved_connectivity_credential";
        if let Some(refusal) = refuse_inline_secret(&params, tool) {
            return refusal;
        }
        let profile = match required_id(&params, "profile") {
            Ok(profile) => profile,
            Err(result) => return result,
        };
        let credential = match required_id(&params, "credential") {
            Ok(credential) => credential,
            Err(result) => return result,
        };
        // The previous value is not recoverable, so the receipt claims no inverse.
        run_mutation(
            &ctx,
            tool,
            params,
            ConnectivityOp::ReplaceCredential {
                profile: NetworkProfileId::new(profile),
                credential: SecretRef::new(credential),
            },
        )
        .await
    }
}

struct DeleteSavedConnectivityCredential;

#[async_trait]
impl ToolHandler for DeleteSavedConnectivityCredential {
    async fn execute(&self, _params: serde_json::Value) -> ToolResult {
        gov::os_unavailable(None, "delete_saved_connectivity_credential")
    }

    async fn execute_with_context(
        &self,
        params: serde_json::Value,
        ctx: ToolContext,
    ) -> ToolResult {
        let tool = "delete_saved_connectivity_credential";
        let profile = match required_id(&params, "profile") {
            Ok(profile) => profile,
            Err(result) => return result,
        };
        // The profile survives; only its stored secret is cleared, which is
        // observable without ever reading the value.
        run_mutation(
            &ctx,
            tool,
            params,
            ConnectivityOp::DeleteCredential {
                profile: NetworkProfileId::new(profile),
            },
        )
        .await
    }
}

/// Register the connectivity tool surface.
pub fn register(registry: &ToolRegistry) {
    let profile_param = || {
        param(
            "profile",
            "string",
            "Saved connection UUID. A display NAME is not accepted: two profiles can share one.",
            true,
        )
    };

    let tools: Vec<(ToolDef, Arc<dyn ToolHandler>)> = vec![
        (
            ToolDef {
                name: "list_vpn_profiles".into(),
                description: "List saved VPN profiles and whether each is connected".into(),
                category: "connectivity".into(),
                default_tier: RiskLevel::Green,
                min_tier: "lite",
                parameters: vec![],
            },
            Arc::new(ListVpnProfiles),
        ),
        (
            ToolDef {
                name: "diagnose_network".into(),
                description: "Diagnose link, address, route, DNS and internet reachability".into(),
                category: "connectivity".into(),
                default_tier: RiskLevel::Green,
                min_tier: "lite",
                parameters: vec![],
            },
            Arc::new(DiagnoseNetwork),
        ),
        (
            ToolDef {
                name: "set_vpn_connection".into(),
                description: "Connect or disconnect a saved VPN profile".into(),
                category: "connectivity".into(),
                default_tier: RiskLevel::Yellow,
                min_tier: "lite",
                parameters: vec![
                    profile_param(),
                    param("connected", "boolean", "Desired connected state", true),
                ],
            },
            Arc::new(SetVpnConnection),
        ),
        (
            ToolDef {
                name: "get_hotspot_state".into(),
                description: "Read whether a Wi-Fi hotspot is running".into(),
                category: "connectivity".into(),
                default_tier: RiskLevel::Green,
                min_tier: "lite",
                parameters: vec![],
            },
            Arc::new(GetHotspotState),
        ),
        (
            ToolDef {
                name: "set_hotspot".into(),
                description: "Start or stop a Wi-Fi hotspot on a device".into(),
                category: "connectivity".into(),
                // RED: this broadcasts a network from the machine.
                default_tier: RiskLevel::Red,
                min_tier: "lite",
                parameters: vec![
                    param("device", "string", "Wi-Fi device name", true),
                    param("enabled", "boolean", "Whether the hotspot should run", true),
                    param("profile", "string", "Access-point profile UUID", false),
                    param(
                        "credential",
                        "string",
                        "Reference to a STORED passphrase. A plaintext passphrase is refused: it would enter argv and the audit record.",
                        false,
                    ),
                ],
            },
            Arc::new(SetHotspot),
        ),
        (
            ToolDef {
                name: "get_proxy_state".into(),
                description: "Read the desktop-wide proxy configuration".into(),
                category: "connectivity".into(),
                default_tier: RiskLevel::Green,
                min_tier: "lite",
                parameters: vec![],
            },
            Arc::new(GetProxyState),
        ),
        (
            ToolDef {
                name: "set_proxy_profile".into(),
                description: "Set the desktop-wide proxy (none, automatic, or manual)".into(),
                category: "connectivity".into(),
                // Redirects every application's traffic.
                default_tier: RiskLevel::Red,
                min_tier: "lite",
                parameters: vec![
                    param("mode", "string", "One of: none, automatic, manual", true),
                    param("pac_uri", "string", "PAC URL, for automatic mode", false),
                    param("http", "object", "{host, port} for manual mode", false),
                    param("https", "object", "{host, port} for manual mode", false),
                    param("socks", "object", "{host, port} for manual mode", false),
                    param("exclusions", "array", "Hosts to bypass the proxy", false),
                ],
            },
            Arc::new(SetProxyProfile),
        ),
        (
            ToolDef {
                name: "list_saved_connectivity_credentials".into(),
                description: "List saved network credentials as metadata only (never values)"
                    .into(),
                category: "connectivity".into(),
                // Privacy-sensitive: it enumerates which networks have stored secrets.
                default_tier: RiskLevel::Red,
                min_tier: "lite",
                parameters: vec![],
            },
            Arc::new(ListSavedConnectivityCredentials),
        ),
        (
            ToolDef {
                name: "replace_saved_connectivity_credential".into(),
                description: "Replace the credential stored on a saved network profile".into(),
                category: "connectivity".into(),
                default_tier: RiskLevel::Red,
                min_tier: "lite",
                parameters: vec![
                    profile_param(),
                    param(
                        "credential",
                        "string",
                        "Reference to the STORED replacement secret (never a plaintext value)",
                        true,
                    ),
                ],
            },
            Arc::new(ReplaceSavedConnectivityCredential),
        ),
        (
            ToolDef {
                name: "delete_saved_connectivity_credential".into(),
                description: "Clear the credential stored on a saved network profile".into(),
                category: "connectivity".into(),
                default_tier: RiskLevel::Red,
                min_tier: "lite",
                parameters: vec![profile_param()],
            },
            Arc::new(DeleteSavedConnectivityCredential),
        ),
    ];

    for (def, handler) in tools {
        registry.register(def, handler);
    }
}
